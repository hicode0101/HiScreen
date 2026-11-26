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
  showCursor: true,
  micEnabled: false,
  micDevice: "",
  micVolume: 100,
  // Connection password: session-only (off at every launch, never written
  // to config.json). Enabling generates a fresh random code each time.
  pwEnabled: false,
  pw: "",
  autoStart: false,
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
export function clampMicVolume(v: number): number {
  return Math.min(200, Math.max(0, Math.round(v)));
}

function normalize(cfg: Partial<typeof settings>): void {
  settings.language = normalizeLangOption(cfg.language);
  settings.theme = normalizeThemeOption(cfg.theme);
  settings.port = clampPort(Number(cfg.port) || 7870);
  settings.quality = clampQuality(Number(cfg.quality) || 80);
  settings.fps = clampFps(Number(cfg.fps) || 30);
  settings.audio = cfg.audio !== false;
  // Only an explicit false hides the mouse cursor from viewers.
  settings.showCursor = cfg.showCursor !== false;
  // Mic is session-only: it always starts OFF each launch (privacy default);
  // only the device and volume are remembered.
  settings.micEnabled = false;
  settings.micDevice = typeof cfg.micDevice === "string" ? cfg.micDevice : "";
  const micVol = Number(cfg.micVolume);
  settings.micVolume = Number.isFinite(micVol) ? clampMicVolume(micVol) : 100;
  // Opt-in: only an explicit true enables auto-start sharing.
  settings.autoStart = cfg.autoStart === true;
  settings.serverAutoStart = cfg.serverAutoStart !== false;
}

export async function loadSettings(): Promise<void> {
  try {
    const cfg = await api.getConfig();
    normalize(cfg as unknown as Partial<typeof settings>);
  } catch {
    // Running outside Tauri (e.g. plain browser) — keep defaults.
  }
  // Sync the connection-password switch with the backend hub, so the UI
  // matches reality after a webview reload (it is not part of config.json).
  try {
    const pw = await api.getPasswordState();
    settings.pwEnabled = pw.enabled;
    settings.pw = pw.password;
  } catch {
    // Outside Tauri — keep the session defaults.
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
      showCursor: settings.showCursor,
      micDevice: settings.micDevice,
      micVolume: settings.micVolume,
      autoStart: settings.autoStart,
      serverAutoStart: settings.serverAutoStart,
    });
  } catch {
    // Ignore persistence failures outside Tauri.
  }
}

/** Push live share parameters (quality/fps/audio/cursor) to the backend. */
export async function applyShareParams(): Promise<void> {
  if (!settings.loaded) return;
  try {
    await api.updateShareParams(
      settings.quality,
      settings.fps,
      settings.audio,
      settings.showCursor,
    );
  } catch {
    // Outside Tauri — nothing to apply.
  }
}

/** Push mic settings (enable/device/volume) to the backend capture thread. */
export async function applyMicParams(): Promise<void> {
  if (!settings.loaded) return;
  try {
    await api.updateMicParams(
      settings.micEnabled,
      settings.micDevice,
      settings.micVolume,
    );
  } catch {
    // Outside Tauri — nothing to apply.
  }
}

/** Push the connection-password switch to the backend WS auth gate. */
export async function applyPassword(): Promise<void> {
  if (!settings.loaded) return;
  try {
    await api.setPassword(settings.pwEnabled, settings.pw);
  } catch {
    // Outside Tauri, or the backend rejected an invalid password — the UI
    // validates the 4–6 letter/digit rule before applying, so failures here
    // are environmental, not user input problems.
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
