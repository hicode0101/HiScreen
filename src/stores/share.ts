import { reactive } from "vue";
import {
  api,
  isTauri,
  type DisplayMeta,
  type LogEntry,
  type ServerStatus,
} from "@/lib/tauri";
import { applyShareParams, settings } from "./settings";

/**
 * Runtime sharing state: display enumeration, thumbnails, the polled server
 * status (viewers, URL, QR, sharing flag) and the event log. All server
 * data is refreshed by one 2s poller while the app runs.
 */
export const share = reactive({
  displays: [] as DisplayMeta[],
  selected: 0,
  previews: {} as Record<number, string>,
  status: {
    running: false,
    port: 7870,
    url: "",
    addresses: [],
    qr: "",
    viewers: [],
    sharing: false,
    display: null,
    fpsTarget: 30,
    audioOn: true,
  } as unknown as ServerStatus,
  logs: [] as LogEntry[],
  busy: false,
});

let statusTimer: number | null = null;
let previewTimer: number | null = null;
let previewBusy = false;

export async function initShare(): Promise<void> {
  await refreshDisplays();
  if (!statusTimer) {
    void refreshStatus();
    statusTimer = window.setInterval(refreshStatus, 2000);
  }
  // 自动开始分享: share the primary display when the app launches.
  if (
    isTauri() &&
    settings.autoStart &&
    !share.status.sharing &&
    share.displays.length > 0
  ) {
    const primary = share.displays.find((d) => d.primary) ?? share.displays[0];
    void startShare(primary.id);
  }
}

export function disposeShare(): void {
  if (statusTimer !== null) {
    clearInterval(statusTimer);
    statusTimer = null;
  }
  stopPreviewLoop();
}

/**
 * Re-enumerate displays. Resolves `true` when the backend list was fetched
 * and `false` when unavailable (no Tauri / IPC error — the mock list is
 * applied so the layout keeps working), letting UI callers report success.
 */
export async function refreshDisplays(): Promise<boolean> {
  if (isTauri()) {
    try {
      share.displays = await api.listDisplays();
      // A display may have been unplugged since the last enumeration.
      if (share.selected >= share.displays.length) {
        share.selected = 0;
      }
      return true;
    } catch {
      // fall through to the mock below
    }
  }
  // Browser mock so the layout stays verifiable without the backend.
  share.displays = [
    { id: 0, name: "Display 1", width: 1920, height: 1080, primary: true },
  ];
  return false;
}

/**
 * Thumbnail for the given display. Without `force` this only fetches when no
 * thumbnail exists yet (first paint); the live preview loop passes `force`
 * so the frame keeps refreshing at ~1 FPS. Transient failures keep the
 * previous thumbnail on screen.
 *
 * Skipped entirely while sharing: the capture session owns the display's
 * DXGI duplication, and a concurrent preview grab would clash with it.
 */
export async function loadPreview(id: number, force = false): Promise<void> {
  if (!isTauri() || previewBusy || share.status.sharing) return;
  if (!force && share.previews[id]) return;
  previewBusy = true;
  try {
    share.previews[id] = await api.capturePreview(id);
  } catch {
    // Thumbnail fetch failed — keep whatever is already on screen.
  } finally {
    previewBusy = false;
  }
}

/** 1 Hz preview refresh; owned by ShareView so it only runs while visible. */
export function startPreviewLoop(): void {
  if (previewTimer !== null) return;
  previewTimer = window.setInterval(
    () => void loadPreview(share.selected, true),
    1000,
  );
}

export function stopPreviewLoop(): void {
  if (previewTimer !== null) {
    clearInterval(previewTimer);
    previewTimer = null;
  }
}

export async function refreshStatus(): Promise<void> {
  if (isTauri()) {
    try {
      share.status = await api.serverStatus();
    } catch {
      // Transient IPC hiccup — keep the previous snapshot.
    }
    try {
      share.logs = await api.getLogs();
    } catch {
      // Ignore log fetch failures the same way.
    }
  }
}

export async function startShare(displayId = share.selected): Promise<void> {
  if (share.busy) return;
  share.busy = true;
  try {
    await api.shareStart(displayId);
    await applyShareParams();
    await refreshStatus();
  } catch {
    // Errors surface via the event log.
  } finally {
    share.busy = false;
  }
}

export async function stopShare(): Promise<void> {
  if (share.busy) return;
  share.busy = true;
  try {
    await api.shareStop();
    await refreshStatus();
  } catch {
    // Errors surface via the event log.
  } finally {
    share.busy = false;
  }
}

/** Apply the configured port to a running server (restart) or start it. */
export async function applyPort(): Promise<void> {
  if (!isTauri()) return;
  try {
    await api.serverStart(settings.port);
    await refreshStatus();
  } catch {
    // Errors surface via the event log.
  }
}

export function selectDisplay(id: number): void {
  share.selected = id;
  void loadPreview(id);
  // Switching the source while live restarts capture; viewers just get a
  // meta update and keep watching.
  if (isTauri() && share.status.sharing) {
    void startShare(id);
  }
}
