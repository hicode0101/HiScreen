import { invoke } from "@tauri-apps/api/core";

export interface AppConfig {
  language: string;
  theme: string;
  port: number;
  quality: number;
  fps: number;
  audio: boolean;
  autoStart: boolean;
  serverAutoStart: boolean;
}

export interface DisplayMeta {
  id: number;
  name: string;
  width: number;
  height: number;
  primary: boolean;
}

export interface ViewerInfo {
  id: number;
  os: string;
  browser: string;
  addr: string;
  audioOn: boolean;
  latencyMs: number;
  connectedForS: number;
}

export interface ServerStatus {
  running: boolean;
  port: number;
  url: string;
  addresses: string[];
  qr: string;
  viewers: ViewerInfo[];
  sharing: boolean;
  display: DisplayMeta | null;
  fpsTarget: number;
  audioOn: boolean;
}

export interface LogEntry {
  ts: number;
  level: "info" | "success" | "warn" | "error";
  msg: string;
}

/** True when running inside the Tauri webview (vs. a plain browser). */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** Open an external URL with the system browser (window.open in plain browsers). */
export async function openExternal(url: string): Promise<void> {
  if (isTauri()) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener");
  }
}

export const api = {
  getConfig: () => invoke<AppConfig>("get_config"),
  saveConfig: (config: AppConfig) => invoke<void>("save_config", { config }),
  appVersion: () => invoke<string>("app_version"),
  listDisplays: () => invoke<DisplayMeta[]>("list_displays"),
  capturePreview: (displayId: number) =>
    invoke<string>("capture_preview", { displayId }),
  serverStatus: () => invoke<ServerStatus>("server_status"),
  serverStart: (port: number) => invoke<void>("server_start", { port }),
  serverStop: () => invoke<void>("server_stop"),
  updateShareParams: (quality: number, fps: number, audio: boolean) =>
    invoke<void>("update_share_params", { quality, fps, audio }),
  shareStart: (displayId: number) =>
    invoke<void>("share_start", { displayId }),
  shareStop: () => invoke<void>("share_stop"),
  broadcastNotice: (text: string) =>
    invoke<void>("broadcast_notice", { text }),
  kickViewer: (id: number) => invoke<void>("kick_viewer", { id }),
  getLogs: () => invoke<LogEntry[]>("get_logs"),
  clearLogs: () => invoke<void>("clear_logs"),
};
