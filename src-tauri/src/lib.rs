// The lib name follows the template convention `<ProjectName>_lib`, which is
// intentionally PascalCase to match the product name.
#![allow(non_snake_case)]

pub mod audio;
pub mod capture;
pub mod commands;
pub mod config;
pub mod cursor;
pub mod hub;
pub mod mic;
pub mod server;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use config::AppConfig;
use hub::{Hub, LogLevel};
use server::ServerHandle;
use tauri::Manager;

use crate::config::AppState;

/// Runtime handles for the sharing subsystem: the shared hub, the HTTP/WS
/// server, the capture / audio threads (None while not sharing) and the
/// long-lived preview grabber (dropped while sharing so the stream owns the
/// display duplication).
pub struct MediaState {
    pub hub: Arc<Hub>,
    pub server: Mutex<ServerHandle>,
    pub capture: Mutex<Option<capture::CaptureHandle>>,
    pub audio: Mutex<Option<audio::AudioHandle>>,
    /// Mic capture thread; independent of the share session (also runs while
    /// idle so the host can watch the live level meter).
    pub mic: Mutex<Option<mic::MicHandle>>,
    pub preview: Mutex<Option<capture::PreviewGrabber>>,
    /// Last timestamp a preview failure was logged (rate limit: one per min).
    pub preview_warn_ms: AtomicU64,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Load persisted settings once and share them via managed state.
            let cfg = AppConfig::load(app.handle())?;
            app.manage(AppState {
                config: std::sync::Mutex::new(cfg.clone()),
            });

            let hub = Arc::new(Hub::new(cfg.fps, cfg.quality, cfg.audio, cfg.showCursor));
            hub.mic_gain_pct.store(cfg.micVolume, Ordering::Relaxed);
            app.manage(MediaState {
                hub: hub.clone(),
                server: std::sync::Mutex::new(ServerHandle::offline()),
                capture: std::sync::Mutex::new(None),
                audio: std::sync::Mutex::new(None),
                mic: std::sync::Mutex::new(None),
                preview: std::sync::Mutex::new(None),
                preview_warn_ms: AtomicU64::new(0),
            });
            // The mixer pacer idles until mic mixing (mic_on) is enabled; it
            // owns the audio broadcast so system + mic never double-send.
            audio::start_mixer(hub.clone());
            // The mic always starts OFF: enabling it is a session-only choice
            // (update_mic_params), never restored from the config file.
            hub.log(LogLevel::Info, "HiScreen 已启动");

            // Bring the HTTP/WS server up before the window shows so the
            // viewer URL works right away (bind failure is non-fatal).
            if cfg.serverAutoStart {
                match server::start(hub.clone(), cfg.port) {
                    Ok(handle) => {
                        let addr = handle
                            .addr
                            .map(|a| a.to_string())
                            .unwrap_or_else(|| format!("0.0.0.0:{}", cfg.port));
                        *app.state::<MediaState>().server.lock().unwrap() = handle;
                        hub.log(LogLevel::Success, format!("网络服务已启动: {addr}"));
                    }
                    Err(e) => hub.log(LogLevel::Error, format!("网络服务启动失败: {e}")),
                }
            }

            // Hide the WebView2 default (browser) context menu globally.
            // Disabling it at the engine level also suppresses native menus
            // such as spellcheck suggestions.
            #[cfg(target_os = "windows")]
            {
                let main = app
                    .get_webview_window("main")
                    .expect("main window must exist");
                main.with_webview(|webview| unsafe {
                    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings6;
                    use windows::core::Interface;

                    let Ok(core) = webview.controller().CoreWebView2() else {
                        return;
                    };
                    let Ok(settings) = core.Settings() else {
                        return;
                    };
                    let Ok(settings6) = settings.cast::<ICoreWebView2Settings6>() else {
                        return;
                    };
                    let _ = settings6.SetAreDefaultContextMenusEnabled(false);
                })
                .expect("failed to disable WebView2 default context menu");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::save_config,
            commands::app_version,
            commands::list_displays,
            commands::capture_preview,
            commands::server_status,
            commands::server_start,
            commands::server_stop,
            commands::update_share_params,
            commands::list_mics,
            commands::get_mic_level,
            commands::update_mic_params,
            commands::share_start,
            commands::share_stop,
            commands::broadcast_notice,
            commands::kick_viewer,
            commands::get_logs,
            commands::clear_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
