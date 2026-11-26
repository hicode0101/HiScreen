//! Tauri IPC commands. Components never `invoke()` directly — the frontend
//! wraps these in `src/lib/tauri.ts`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::audio;
use crate::capture;
use crate::config::{AppConfig, AppState};
use crate::hub::{now_ms, DisplayMeta, LogEntry, LogLevel, ViewerEntry};
use crate::mic;
use crate::server;
use crate::MediaState;

#[tauri::command]
pub fn get_config(state: State<AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_config(
    app: AppHandle,
    state: State<AppState>,
    config: AppConfig,
) -> Result<(), String> {
    let config = config.sanitized();
    *state.config.lock().unwrap() = config.clone();
    config.save(&app)
}

/// App version from Cargo.toml, shown in the status bar and About row.
#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

// ------------------------------------------------------------- displays

#[tauri::command]
pub fn list_displays() -> Result<Vec<DisplayMeta>, String> {
    capture::list_displays().map_err(|e| e.to_string())
}

/// One downscaled JPEG frame (base64 data URL) for display thumbnails.
#[tauri::command]
pub fn capture_preview(state: State<MediaState>, display_id: usize) -> Result<String, String> {
    // While a capture session runs it owns the display duplication; a
    // concurrent preview grab would fail with a driver conflict. The
    // frontend pauses its preview loop while sharing — this is the
    // backend-side guard against races around the share start/stop.
    if state.capture.lock().unwrap().is_some() {
        return Err("capture session in progress".into());
    }
    let mut slot = state.preview.lock().unwrap();
    match capture::preview_frame(&mut slot, display_id, 480) {
        Ok(jpeg) => Ok(format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(jpeg)
        )),
        // "No frame within the deadline" means the desktop is frozen; the
        // frontend keeps showing the last thumbnail, so don't spam the log.
        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => Err(e.to_string()),
        Err(e) => {
            // A lost desktop (locked screen, driver reset) fails every
            // attempt until it recovers — log one entry per minute at most.
            const WARN_INTERVAL_MS: u64 = 60_000;
            let now = now_ms();
            if now.saturating_sub(state.preview_warn_ms.load(Ordering::Relaxed)) > WARN_INTERVAL_MS
            {
                state.preview_warn_ms.store(now, Ordering::Relaxed);
                state.hub.log(LogLevel::Warn, format!("预览采集失败: {e}"));
            }
            Err(e.to_string())
        }
    }
}

// --------------------------------------------------------------- server

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewerInfo {
    pub id: usize,
    pub os: String,
    pub browser: String,
    pub addr: String,
    pub audio_on: bool,
    pub latency_ms: u32,
    pub connected_for_s: u64,
}

impl ViewerInfo {
    fn from_entry(e: &Arc<ViewerEntry>) -> Self {
        Self {
            id: e.id,
            os: e.os.lock().unwrap().clone(),
            browser: e.browser_ref(),
            addr: e.addr.clone(),
            audio_on: e.audio_on.load(Ordering::Relaxed),
            latency_ms: e.latency_ms.load(Ordering::Relaxed),
            connected_for_s: now_ms().saturating_sub(e.since_ms) / 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub running: bool,
    pub port: u16,
    /// Primary LAN URL, e.g. `http://192.168.1.10:7870`.
    pub url: String,
    /// All reachable LAN addresses (primary first).
    pub addresses: Vec<String>,
    /// QR code of `url`, rendered as an inline SVG string.
    pub qr: String,
    pub viewers: Vec<ViewerInfo>,
    pub sharing: bool,
    pub display: Option<DisplayMeta>,
    pub fps_target: u32,
    pub audio_on: bool,
}

#[tauri::command]
pub fn server_status(state: State<MediaState>, cfg: State<AppState>) -> ServerStatus {
    let hub = &state.hub;
    let server = state.server.lock().unwrap();
    let running = server.running();
    let port = if running {
        server.port
    } else {
        cfg.config.lock().unwrap().port
    };
    let addresses = lan_addresses(port);
    let url = addresses
        .first()
        .cloned()
        .unwrap_or_else(|| format!("http://127.0.0.1:{port}"));
    let viewers = hub
        .viewers
        .lock()
        .unwrap()
        .iter()
        .map(ViewerInfo::from_entry)
        .collect();
    let share = hub.share.lock().unwrap();
    ServerStatus {
        running,
        port,
        qr: if running { server::qr_svg(&url) } else { String::new() },
        url,
        addresses,
        viewers,
        sharing: share.sharing,
        display: share.display.clone(),
        fps_target: hub.fps.load(Ordering::Relaxed),
        audio_on: hub.audio_on.load(Ordering::Relaxed),
    }
}

/// Start (or restart on a new port) the embedded HTTP/WS server.
#[tauri::command]
pub fn server_start(state: State<MediaState>, port: u16) -> Result<(), String> {
    let port = port.clamp(1024, 65535);
    let mut handle = state.server.lock().unwrap();
    if handle.running() && handle.port == port {
        return Ok(());
    }
    handle.stop();
    let new_handle = server::start(state.hub.clone(), port)?;
    state.hub.log(
        LogLevel::Success,
        format!("网络服务已启动: {}", new_handle.addr.map(|a| a.to_string()).unwrap_or_default()),
    );
    *handle = new_handle;
    Ok(())
}

#[tauri::command]
pub fn server_stop(state: State<MediaState>) -> Result<(), String> {
    let mut handle = state.server.lock().unwrap();
    if !handle.running() {
        return Ok(());
    }
    handle.stop();
    Ok(())
}

/// Push live share parameters (quality / fps / audio / cursor) into the hub
/// and re-broadcast the meta object to viewers.
#[tauri::command]
pub fn update_share_params(
    state: State<MediaState>,
    quality: u32,
    fps: u32,
    audio: bool,
    show_cursor: bool,
) {
    let hub = &state.hub;
    hub.quality.store(quality.clamp(1, 100), Ordering::Relaxed);
    hub.fps.store(fps.clamp(5, 60), Ordering::Relaxed);
    let was_on = hub.audio_on.swap(audio, Ordering::Relaxed);
    if was_on != audio {
        state.hub.log(
            LogLevel::Info,
            if audio { "音频已开启" } else { "音频已关闭" },
        );
    }
    let cursor_was = hub.cursor_on.swap(show_cursor, Ordering::Relaxed);
    if cursor_was != show_cursor {
        state.hub.log(
            LogLevel::Info,
            if show_cursor { "已显示鼠标光标" } else { "已隐藏鼠标光标" },
        );
    }
    hub.broadcast_meta();
}

// ------------------------------------------------------------ microphone

/// All capture (microphone) device names for the host UI dropdown.
#[tauri::command]
pub fn list_mics() -> Result<Vec<String>, String> {
    mic::list_devices().map_err(|e| e.to_string())
}

/// Latest mic input level in dBFS × 10 (-900..=0) for the live meter.
#[tauri::command]
pub fn get_mic_level(state: State<MediaState>) -> i32 {
    state.hub.mic_level_db10.load(Ordering::Relaxed)
}

/// Apply mic settings to the live capture thread. Persistence is handled
/// separately by the frontend via save_config (mirroring share params).
#[tauri::command]
pub fn update_mic_params(state: State<MediaState>, enabled: bool, device: String, volume: u32) {
    let hub = &state.hub;
    hub.mic_gain_pct
        .store(volume.clamp(0, 200), Ordering::Relaxed);

    let device = device.trim().to_string();
    let was_on = hub.mic_on.swap(enabled, Ordering::Relaxed);
    let mut slot = state.mic.lock().unwrap();

    if enabled {
        // (Re)open the device when the thread is missing or a different
        // device was picked; a pure volume change just uses the new gain.
        let needs_restart = slot.as_ref().map(|h| h.device != device).unwrap_or(true);
        if needs_restart {
            *slot = None; // dropping the handle stops the old thread
            *slot = Some(mic::start(hub.clone(), Some(device)));
        }
        if !was_on {
            hub.log(LogLevel::Info, "麦克风已开启");
        }
    } else if was_on || slot.is_some() {
        *slot = None;
        hub.mic_level_db10.store(-900, Ordering::Relaxed);
        hub.log(LogLevel::Info, "麦克风已停用");
    }
}

// ---------------------------------------------------------------- share

#[tauri::command]
pub fn share_start(state: State<MediaState>, display_id: usize) -> Result<(), String> {
    let hub = state.hub.clone();
    stop_threads(&state);

    let handle = capture::start(hub.clone(), display_id).map_err(|e| e.to_string())?;
    let meta = handle.display.clone();
    *state.capture.lock().unwrap() = Some(handle);

    // The audio thread keeps the device open and self-gates on audio_on.
    let mut audio_slot = state.audio.lock().unwrap();
    if audio_slot.is_none() {
        *audio_slot = Some(audio::start(hub.clone()));
    }
    drop(audio_slot);

    {
        let mut share = hub.share.lock().unwrap();
        share.sharing = true;
        share.display = Some(meta.clone());
    }
    hub.broadcast_meta();
    hub.log(
        LogLevel::Success,
        format!("屏幕分享已开始: {} ({}×{})", meta.name, meta.width, meta.height),
    );
    Ok(())
}

#[tauri::command]
pub fn share_stop(state: State<MediaState>) -> Result<(), String> {
    let hub = state.hub.clone();
    stop_threads(&state);
    {
        let mut share = hub.share.lock().unwrap();
        share.sharing = false;
        share.display = None;
    }
    hub.broadcast_meta();
    hub.log(LogLevel::Info, "屏幕分享已停止");
    Ok(())
}

/// Stop capture and audio threads (drops their handles, which signal stop).
/// The preview grabber is dropped too so the display duplication is free for
/// the capture session (share_start) and rebuilt on the next preview call
/// (share_stop).
fn stop_threads(state: &MediaState) {
    *state.capture.lock().unwrap() = None;
    *state.audio.lock().unwrap() = None;
    *state.preview.lock().unwrap() = None;
}

#[tauri::command]
pub fn broadcast_notice(state: State<MediaState>, text: String, duration_secs: u64) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    // Clamp to a sane banner lifetime (the UI offers 5s..60min).
    let duration = duration_secs.clamp(1, 3600);
    state.hub.broadcast_notice(&text, duration);
}

#[tauri::command]
pub fn kick_viewer(state: State<MediaState>, id: usize) -> Result<(), String> {
    let viewers = state.hub.viewers.lock().unwrap();
    if let Some(entry) = viewers.iter().find(|v| v.id == id) {
        let _ = entry.kick.send(());
        Ok(())
    } else {
        Err("viewer not found".into())
    }
}

// ----------------------------------------------------------------- logs

#[tauri::command]
pub fn get_logs(state: State<MediaState>) -> Vec<LogEntry> {
    state.hub.logs.lock().unwrap().iter().cloned().collect()
}

#[tauri::command]
pub fn clear_logs(state: State<MediaState>) {
    state.hub.logs.lock().unwrap().clear();
}

/// LAN addresses the viewer URL can use; the default-route address first.
fn lan_addresses(port: u16) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(ip) = server::primary_ip() {
        out.push(format!("http://{ip}:{port}"));
    }
    out
}
