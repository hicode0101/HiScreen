import { reactive } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
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

/** Whether the app window is usable: focused AND not minimized. Preview sync
 * pauses while either fails (window minimized or pushed to the background)
 * and resumes when the user brings it back — an unused window has no reason
 * to keep grabbing frames. Both start "usable": the window is focused after
 * launch. */
let winFocused = true;
let winMinimized = false;
let windowActive = true;
let unlistenFocus: (() => void) | null = null;
let unlistenResize: (() => void) | null = null;
let unlistenMove: (() => void) | null = null;

function updateActive(): void {
  windowActive = winFocused && !winMinimized;
}

/** Follow the window's activation and size (Tauri window events; focus/blur
 * + visibility in the browser debug mode). */
function trackWindowFocus(): void {
  if (isTauri()) {
    const w = getCurrentWindow();
    void w
      .onFocusChanged(({ payload }) => {
        winFocused = payload;
        updateActive();
      })
      .then((fn) => {
        unlistenFocus = fn;
      });
    void w
      .onResized(({ payload }) => {
        // Some Windows builds report a 0×0 size while minimized. This only
        // ever SETS the flag — the moved handler below owns clearing it, so
        // the tiny non-zero minimized size (181×25) cannot overwrite it.
        if (payload.width === 0 || payload.height === 0) {
          winMinimized = true;
          updateActive();
        }
      })
      .then((fn) => {
        unlistenResize = fn;
      });
    void w
      .onMoved(({ payload }) => {
        // Windows parks minimized windows at −32000,−32000 (the size only
        // shrinks to a tiny strip) — the reliable minimize signal, because
        // an external minimize may leave the window active (no blur event
        // follows) and the size is not 0×0 either. A real position clears
        // the flag again.
        winMinimized = payload.x <= -32000 || payload.y <= -32000;
        updateActive();
      })
      .then((fn) => {
        unlistenMove = fn;
      });
    return;
  }
  // Browser debug mode: approximate the same behavior on page events.
  window.addEventListener("focus", () => {
    winFocused = true;
    updateActive();
  });
  window.addEventListener("blur", () => {
    winFocused = false;
    updateActive();
  });
  document.addEventListener("visibilitychange", () => {
    winMinimized = document.hidden;
    updateActive();
  });
}

export async function initShare(): Promise<void> {
  trackWindowFocus();
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
  if (unlistenFocus) {
    unlistenFocus();
    unlistenFocus = null;
  }
  if (unlistenResize) {
    unlistenResize();
    unlistenResize = null;
  }
  if (unlistenMove) {
    unlistenMove();
    unlistenMove = null;
  }
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
 * so the frame keeps refreshing every PREVIEW_INTERVAL_MS. Transient
 * failures keep the previous thumbnail on screen.
 *
 * Sync runs only while the app window is focused: a minimized or background
 * window stops grabbing frames and resumes on refocus. While sharing, the
 * capture session publishes the thumbnails itself (its duplication owns the
 * display), so preview updates no longer depend on the share start/stop.
 */
export async function loadPreview(id: number, force = false): Promise<void> {
  if (!isTauri() || previewBusy || !windowActive) return;
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

/** Preview refresh cadence; the ShareView tooltip shows the same number so
 * the UI copy can never drift from the actual loop interval. */
export const PREVIEW_INTERVAL_MS = 2000;

/** 2 s preview refresh; owned by ShareView so it only runs while visible.
 * Manual syncs (e.g. the source-refresh button) go through loadPreview
 * directly so the frame updates immediately. */
export function startPreviewLoop(): void {
  if (previewTimer !== null) return;
  previewTimer = window.setInterval(
    () => void loadPreview(share.selected, true),
    PREVIEW_INTERVAL_MS,
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
