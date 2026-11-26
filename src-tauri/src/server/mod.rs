//! Embedded HTTP + WebSocket server that web viewers connect to.
//!
//! Routes:
//! - `GET /`     — the self-contained viewer page (embedded below).
//! - `GET /ws`   — WebSocket carrying JSON control messages (text) and
//!                 media frames (binary, first byte = payload type).
//! - `GET /health` — trivial liveness probe.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, State};
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::hub::{now_ms, Hub, LogLevel, ViewerEntry};

const VIEWER_HTML: &str = include_str!("viewer.html");

/// Handle to a running server instance (port + graceful shutdown trigger).
pub struct ServerHandle {
    pub port: u16,
    pub addr: Option<SocketAddr>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ServerHandle {
    pub fn offline() -> Self {
        Self {
            port: 0,
            addr: None,
            shutdown: None,
        }
    }

    pub fn running(&self) -> bool {
        self.addr.is_some()
    }

    /// Trigger graceful shutdown (the listener task finishes on its own).
    pub fn stop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        self.addr = None;
    }
}

/// Bind and spawn the axum server task; blocks briefly for the bind result.
pub fn start(hub: Arc<Hub>, port: u16) -> Result<ServerHandle, String> {
    let (bound_tx, bound_rx) = oneshot::channel::<Result<SocketAddr, String>>();
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let hub2 = hub.clone();

    tauri::async_runtime::spawn(async move {
        match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
            Ok(listener) => {
                let addr = listener.local_addr().ok();
                let _ = bound_tx.send(Ok(addr.expect("bound listener has an addr")));
                let app = Router::new()
                    .route("/", get(|| async { Html(VIEWER_HTML) }))
                    .route("/health", get(|| async { "ok" }))
                    .route("/ws", get(ws_route))
                    .with_state(hub2.clone());
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(async {
                    let _ = stop_rx.await;
                })
                .await
                .ok();
                hub2.log(LogLevel::Info, "网络服务已停止");
            }
            Err(e) => {
                let _ = bound_tx.send(Err(format!("端口 {port} 绑定失败: {e}")));
            }
        }
    });

    match bound_rx.blocking_recv() {
        Ok(Ok(addr)) => Ok(ServerHandle {
            port,
            addr: Some(addr),
            shutdown: Some(stop_tx),
        }),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("服务任务意外退出".into()),
    }
}

/// Best-effort local address discovery via the default route (no packets
/// are actually sent by a UDP "connect").
pub fn primary_ip() -> Option<IpAddr> {
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(a) = s.local_addr() {
                if !a.ip().is_loopback() {
                    return Some(a.ip());
                }
            }
        }
    }
    None
}

/// Render a QR code as an inline SVG string.
pub fn qr_svg(text: &str) -> String {
    use qrcode::render::svg;
    qrcode::QrCode::new(text)
        .map(|code| {
            code.render::<svg::Color>()
                .min_dimensions(200, 200)
                .dark_color(svg::Color("#1b1b1b"))
                .light_color(svg::Color("#ffffff"))
                .build()
        })
        .unwrap_or_default()
}

async fn ws_route(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(hub): State<Arc<Hub>>,
) -> axum::response::Response {
    ws.on_upgrade(move |socket| viewer_socket(socket, hub, addr.ip()))
}

async fn viewer_socket(socket: WebSocket, hub: Arc<Hub>, ip: IpAddr) {
    let (mut sink, mut stream) = socket.split();
    let id = hub.next_viewer_id.fetch_add(1, Ordering::Relaxed);
    let (kick_tx, mut kick_rx) = mpsc::unbounded_channel::<()>();
    // Replies produced by the reader half (pongs) go through here so the
    // single writer loop stays the only socket writer.
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Message>();

    let entry = Arc::new(ViewerEntry {
        id,
        os: Mutex::new(String::new()),
        browser: Mutex::new(String::new()),
        addr: ip.to_string(),
        audio_on: AtomicBool::new(true),
        latency_ms: AtomicU32::new(0),
        since_ms: now_ms(),
        kick: kick_tx,
    });
    hub.viewers.lock().unwrap().push(entry.clone());
    hub.log(LogLevel::Info, format!("观众加入: {ip}"));

    let mut video = hub.video.subscribe();
    let mut audio = hub.audio.subscribe();
    let mut control = hub.control.subscribe();

    if sink
        .send(Message::text(hub.meta_json()))
        .await
        .is_err()
    {
        remove_viewer(&hub, &entry);
        return;
    }

    let mut ping = tokio::time::interval(Duration::from_secs(2));
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            maybe = stream.next() => {
                match maybe {
                    Some(Ok(Message::Text(txt))) => {
                        handle_control(&hub, &entry, txt.as_str(), &reply_tx);
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {} // protocol-level ping/pong handled by axum
                    Some(Err(_)) => break,
                }
            }
            reply = reply_rx.recv() => {
                match reply {
                    Some(m) => if sink.send(m).await.is_err() { break },
                    None => break,
                }
            }
            frame = video.recv() => {
                match frame {
                    Ok(f) => {
                        if sink.send(Message::Binary(Bytes::from_owner(f))).await.is_err() { break }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            chunk = audio.recv() => {
                match chunk {
                    Ok(c) => {
                        if sink.send(Message::Binary(Bytes::from_owner(c))).await.is_err() { break }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            ctl = control.recv() => {
                match ctl {
                    Ok(c) => {
                        if sink.send(Message::text(c)).await.is_err() { break }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            }
            _ = ping.tick() => {
                let t = now_ms();
                let msg = json!({ "type": "ping", "t": t }).to_string();
                if sink.send(Message::text(msg)).await.is_err() { break }
            }
            _ = kick_rx.recv() => break,
        }
    }

    remove_viewer(&hub, &entry);
    let _ = sink.close().await;
}

fn remove_viewer(hub: &Hub, entry: &Arc<ViewerEntry>) {
    hub.viewers.lock().unwrap().retain(|v| v.id != entry.id);
    let browser = entry.browser_ref();
    let os = entry.os.lock().unwrap().clone();
    let who = match (os.is_empty(), browser.is_empty()) {
        (false, false) => format!("{os} · {browser}"),
        (true, false) => browser,
        _ => entry.addr.clone(),
    };
    hub.log(LogLevel::Info, format!("观众离开: {who}"));
}

/// Interpret one control message from a viewer.
fn handle_control(
    hub: &Hub,
    entry: &Arc<ViewerEntry>,
    text: &str,
    reply_tx: &mpsc::UnboundedSender<Message>,
) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("hello") => {
            let os = v.get("os").and_then(|x| x.as_str()).unwrap_or("Unknown");
            let browser = v.get("browser").and_then(|x| x.as_str()).unwrap_or("Browser");
            *entry.os.lock().unwrap() = os.to_string();
            *entry.browser.lock().unwrap() = browser.to_string();
            hub.log(LogLevel::Info, format!("观众识别: {os} · {browser} ({})", entry.addr));
        }
        Some("ping") => {
            // Client-initiated latency probe: echo the timestamp back.
            let t = v.get("t").cloned().unwrap_or(serde_json::Value::Null);
            let msg = json!({ "type": "pong", "t": t }).to_string();
            let _ = reply_tx.send(Message::text(msg));
        }
        Some("pong") => {
            if let Some(t) = v.get("t").and_then(|x| x.as_u64()) {
                let rtt = now_ms().saturating_sub(t).min(99_999) as u32;
                entry.latency_ms.store(rtt, Ordering::Relaxed);
            }
        }
        Some("stats") => {
            if let Some(l) = v.get("latency").and_then(|x| x.as_u64()) {
                entry.latency_ms.store(l.min(99_999) as u32, Ordering::Relaxed);
            }
            if let Some(a) = v.get("audio").and_then(|x| x.as_bool()) {
                entry.audio_on.store(a, Ordering::Relaxed);
            }
        }
        _ => {}
    }
}
