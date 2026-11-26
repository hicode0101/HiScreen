import { reactive } from "vue";
import { api } from "@/lib/tauri";
import {
  applyTheme,
  normalizeThemeOption,
  type ThemeOption,
} from "@/lib/theme";
import { normalizeLangOption, setLanguage, type LangOption } from "@/lib/i18n";

/** App settings; persisted to `{app_data_dir}/config.json` via Rust. */
export const settings = reactive({
  loaded: false,
  language: "system" as LangOption,
  theme: "light" as ThemeOption,
  port: 7870,
  quality: 80,
  fps: 30,
  audio: true,
  autoStart: true,
  serverAutoStart: true,
});

/** Clamp helpers kept in one place so all editors agree. */
export function clampPort(port: number): number {
  return Math.min(65535, Math.max(1024, Math.round(port)));
}
export function clampQuality(q: number): number {
  return Math.min(100, Math.max(10, Math.round(q)));
}
export function clampFps(fps: number): number {
  return Math.min(60, Math.max(5, Math.round(fps)));
}

function normalize(cfg: Partial<typeof settings>): void {
  settings.language = normalizeLangOption(cfg.language);
  settings.theme = normalizeThemeOption(cfg.theme);
  settings.port = clampPort(Number(cfg.port) || 7870);
  settings.quality = clampQuality(Number(cfg.quality) || 80);
  settings.fps = clampFps(Number(cfg.fps) || 30);
  settings.audio = cfg.audio !== false;
  settings.autoStart = cfg.autoStart !== false;
  settings.serverAutoStart = cfg.serverAutoStart !== false;
}

export async function loadSettings(): Promise<void> {
  try {
    const cfg = await api.getConfig();
    normalize(cfg as unknown as Partial<typeof settings>);
  } catch {
    // Running outside Tauri (e.g. plain browser) — keep defaults.
  }
  settings.loaded = true;
}

export async function persistSettings(): Promise<void> {
  try {
    await api.saveConfig({
      language: settings.language,
      theme: settings.theme,
      port: settings.port,
      quality: settings.quality,
      fps: settings.fps,
      audio: settings.audio,
      autoStart: settings.autoStart,
      serverAutoStart: settings.serverAutoStart,
    });
  } catch {
    // Ignore persistence failures outside Tauri.
  }
}

/** Push live share parameters (quality/fps/audio) to the backend. */
export async function applyShareParams(): Promise<void> {
  if (!settings.loaded) return;
  try {
    await api.updateShareParams(settings.quality, settings.fps, settings.audio);
  } catch {
    // Outside Tauri — nothing to apply.
  }
}

export async function updateTheme(pref: ThemeOption): Promise<void> {
  if (settings.theme === pref) return;
  settings.theme = pref;
  applyTheme(pref);
  await persistSettings();
}

export async function updateLanguage(pref: LangOption): Promise<void> {
  if (settings.language === pref) return;
  settings.language = pref;
  await setLanguage(pref);
  await persistSettings();
}
