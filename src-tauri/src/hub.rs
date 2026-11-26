//! Shared runtime state ("hub") for screen sharing: encoded-frame broadcast
//! channels, the viewer registry, the ring-buffer event log and the current
//! share status. Every long-lived thread (capture, audio, HTTP server) and
//! every WebSocket task talks to the hub instead of to each other.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::broadcast;

/// First byte of every binary WebSocket frame identifies the payload type.
pub const FRAME_VIDEO: u8 = 1;
pub const FRAME_AUDIO: u8 = 2;

const MAX_LOGS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum LogLevel {
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "success")]
    Success,
    #[serde(rename = "warn")]
    Warn,
    #[serde(rename = "error")]
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// Wall-clock timestamp formatted as `HH:MM:SS`.
    pub ts: u64,
    pub level: LogLevel,
    pub msg: String,
}

/// Metadata about one physical display (id = index into scrap's enumeration).
#[derive(Debug, Clone, Serialize)]
pub struct DisplayMeta {
    pub id: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

/// One connected web viewer. Latency / audio use atomics because the per-viewer
/// WebSocket task updates them while commands read the whole registry.
pub struct ViewerEntry {
    pub id: usize,
    pub os: Mutex<String>,
    pub browser: Mutex<String>,
    pub addr: String,
    pub audio_on: AtomicBool,
    pub latency_ms: AtomicU32,
    pub since_ms: u64,
    /// Dropping the receiving half of this channel closes the viewer's socket.
    pub kick: tokio::sync::mpsc::UnboundedSender<()>,
}

impl ViewerEntry {
    pub fn browser_ref(&self) -> String {
        self.browser.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone)]
pub struct ShareState {
    pub sharing: bool,
    pub display: Option<DisplayMeta>,
}

pub struct Hub {
    /// Encoded JPEG frames (type byte + jpeg bytes) shared by all viewers.
    pub video: broadcast::Sender<Arc<[u8]>>,
    /// Raw PCM i16 48 kHz mono chunks (type byte + payload).
    pub audio: broadcast::Sender<Arc<[u8]>>,
    /// JSON control messages (meta updates, broadcast notices).
    pub control: broadcast::Sender<String>,
    pub viewers: Mutex<Vec<Arc<ViewerEntry>>>,
    pub next_viewer_id: AtomicUsize,
    pub logs: Mutex<VecDeque<LogEntry>>,
    pub share: Mutex<ShareState>,
    /// Whether audio streaming is enabled (toggled live).
    pub audio_on: AtomicBool,
    /// Whether the mouse cursor is composited into the video frames.
    pub cursor_on: AtomicBool,
    pub fps: AtomicU32,
    pub quality: AtomicU32,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Hub {
    pub fn new(fps: u32, quality: u32, audio_on: bool, cursor_on: bool) -> Self {
        let (video_tx, _) = broadcast::channel(4);
        let (audio_tx, _) = broadcast::channel(64);
        let (control_tx, _) = broadcast::channel(32);
        Self {
            video: video_tx,
            audio: audio_tx,
            control: control_tx,
            viewers: Mutex::new(Vec::new()),
            next_viewer_id: AtomicUsize::new(1),
            logs: Mutex::new(VecDeque::new()),
            share: Mutex::new(ShareState {
                sharing: false,
                display: None,
            }),
            audio_on: AtomicBool::new(audio_on),
            cursor_on: AtomicBool::new(cursor_on),
            fps: AtomicU32::new(fps),
            quality: AtomicU32::new(quality),
        }
    }

    /// Append an event to the ring-buffer log (also goes to stderr in debug).
    pub fn log(&self, level: LogLevel, msg: impl Into<String>) {
        let msg = msg.into();
        #[cfg(debug_assertions)]
        eprintln!("[hiscreen:{level:?}] {msg}");
        let mut logs = self.logs.lock().unwrap();
        logs.push_back(LogEntry {
            // Epoch millis; the frontend renders it in the local timezone.
            ts: now_ms(),
            level,
            msg,
        });
        while logs.len() > MAX_LOGS {
            logs.pop_front();
        }
    }

    /// Publish a control JSON string to every connected viewer.
    pub fn push_control(&self, json: String) {
        let _ = self.control.send(json);
    }

    /// Rebuild and broadcast the current meta object (share state + params).
    pub fn broadcast_meta(&self) {
        let share = self.share.lock().unwrap();
        let json = serde_json::json!({
            "type": "meta",
            "sharing": share.sharing,
            "display": share.display,
            "fps": self.fps.load(Ordering::Relaxed),
            "audio": self.audio_on.load(Ordering::Relaxed),
        });
        drop(share);
        self.push_control(json.to_string());
    }

    /// Send a text notice to every viewer (and log it locally).
    /// `duration_secs` is how long viewers keep the banner on screen.
    pub fn broadcast_notice(&self, text: &str, duration_secs: u64) {
        let json = serde_json::json!({
            "type": "notice",
            "text": text,
            "duration": duration_secs,
        });
        self.push_control(json.to_string());
        self.log(LogLevel::Info, format!("广播通知({duration_secs}s): {text}"));
    }

    /// Serialize the current meta object (used as the first WS message).
    pub fn meta_json(&self) -> String {
        let share = self.share.lock().unwrap();
        let json = serde_json::json!({
            "type": "meta",
            "sharing": share.sharing,
            "display": share.display,
            "fps": self.fps.load(Ordering::Relaxed),
            "audio": self.audio_on.load(Ordering::Relaxed),
        });
        json.to_string()
    }
}
