use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::Manager;

/// Application settings persisted to `{app_data_dir}/config.json`.
/// Field values mirror what the frontend stores, so both sides agree.
/// JSON uses camelCase (what the frontend sends); snake_case aliases keep
/// older config files written before the rename readable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppConfig {
    /// system | zh-CN | en-US
    pub language: String,
    /// light | dark
    pub theme: String,
    /// HTTP/WebSocket server listen port
    pub port: u16,
    /// JPEG video quality (1..=100)
    pub quality: u32,
    /// Target capture/encode frame rate
    pub fps: u32,
    /// Whether system audio is captured and streamed
    pub audio: bool,
    /// Whether the mouse cursor is composited into the shared video
    #[serde(alias = "show_cursor")]
    pub showCursor: bool,
    /// Microphone device name; empty = system default. The mic on/off state
    /// is session-only (always off at launch) and is not persisted.
    #[serde(alias = "mic_device")]
    pub micDevice: String,
    /// Mic volume in percent (0..=200)
    #[serde(alias = "mic_volume")]
    pub micVolume: u32,
    /// Start sharing the primary display when the app launches
    #[serde(alias = "auto_start")]
    pub autoStart: bool,
    /// Start the HTTP/WS server when the app launches
    #[serde(alias = "server_auto_start")]
    pub serverAutoStart: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            language: "system".into(),
            theme: "light".into(),
            port: 7870,
            quality: 80,
            fps: 30,
            audio: true,
            // Viewers normally expect to see the host mouse pointer.
            showCursor: true,
            micDevice: String::new(),
            micVolume: 100,
            // Sharing spawns the capture/encode threads; keep it opt-in so
            // the first launch stays instant.
            autoStart: false,
            serverAutoStart: true,
        }
    }
}

impl AppConfig {
    /// Clamp user-editable numeric fields into safe ranges.
    pub fn sanitized(mut self) -> Self {
        self.port = self.port.clamp(1024, 65535);
        self.quality = self.quality.clamp(1, 100);
        self.fps = self.fps.clamp(5, 60);
        self.micVolume = self.micVolume.clamp(0, 200);
        self
    }
}

pub struct AppState {
    pub config: std::sync::Mutex<AppConfig>,
}

impl AppConfig {
    fn config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
        let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(dir.join("config.json"))
    }

    pub fn load(app: &tauri::AppHandle) -> Result<Self, String> {
        let path = Self::config_path(app)?;
        if path.exists() {
            let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            serde_json::from_str(&raw)
                .map(Self::sanitized)
                .map_err(|e| e.to_string())
        } else {
            Ok(Self::default())
        }
    }

    pub fn save(&self, app: &tauri::AppHandle) -> Result<(), String> {
        let path = Self::config_path(app)?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| e.to_string())
    }
}
