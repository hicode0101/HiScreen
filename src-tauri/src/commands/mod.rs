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
pub fn capture_preview(display_id: usize) -> Result<String, String> {
    let jpeg = capture::capture_preview(display_id, 480).map_err(|e| e.to_string())?;
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    ))
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

/// Push live share parameters (quality / fps / audio) into the hub and
/// re-broadcast the meta object to viewers.
#[tauri::command]
pub fn update_share_params(state: State<MediaState>, quality: u32, fps: u32, audio: bool) {
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
    hub.broadcast_meta();
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
fn stop_threads(state: &MediaState) {
    *state.capture.lock().unwrap() = None;
    *state.audio.lock().unwrap() = None;
}

#[tauri::command]
pub fn broadcast_notice(state: State<MediaState>, text: String) {
    let text = text.trim().to_string();
    if text.is_empty() {
        return;
    }
    state.hub.broadcast_notice(&text);
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

// ------------------------------------------------------------- firewall

/// Add a Windows Defender Firewall inbound rule for the share port.
/// Needs an elevated shell to succeed; the error text says so otherwise.
#[tauri::command]
#[cfg(windows)]
pub fn add_firewall_rule(state: State<MediaState>, port: u16) -> Result<String, String> {
    use std::os::windows::process::CommandExt as _;

    let name = "HiScreen Screen Share";
    let output = std::process::Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &format!("name={name}"),
            "dir=in",
            "action=allow",
            "protocol=TCP",
            &format!("localport={port}"),
        ])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        state
            .hub
            .log(LogLevel::Success, format!("防火墙规则已添加 (TCP {port})"));
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        state
            .hub
            .log(LogLevel::Error, format!("防火墙规则添加失败: {err}"));
        Err(if err.is_empty() {
            "需要管理员权限运行".into()
        } else {
            err
        })
    }
}

/// Non-Windows platforms don't need a firewall helper (GUI prompts instead).
#[tauri::command]
#[cfg(not(windows))]
pub fn add_firewall_rule(state: State<MediaState>, port: u16) -> Result<String, String> {
    let _ = (state, port);
    Ok("此平台无需配置防火墙".into())
}

/// LAN addresses the viewer URL can use; the default-route address first.
fn lan_addresses(port: u16) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(ip) = server::primary_ip() {
        out.push(format!("http://{ip}:{port}"));
    }
    out
}
