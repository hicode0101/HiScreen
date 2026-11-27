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
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::hub::{now_ms, Hub, LogLevel, PwFail, ViewerEntry, PW_BAN_DURATION, PW_MAX_FAILS};

const VIEWER_HTML: &str = include_str!("viewer.html");

/// How long an unauthenticated viewer may sit on the password prompt before
/// its socket is closed (the viewer page reconnects on its own).
const AUTH_TIMEOUT: Duration = Duration::from_secs(120);
/// Grace period for draining frames after a rejected connection closes, so
/// the close completes as a FIN instead of a reset (see `close_gracefully`).
const CLOSE_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);

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

    // An IP the host kicked is rejected outright for a while: the remaining
    // lockout is the only state consulted (the password gate never runs),
    // and the reply goes out before anything else happens.
    // (Bind first — a lock() temporary would otherwise be held across the
    // await below, making the future !Send.)
    let kick_secs = hub.kick_bans.lock().unwrap().ban_left_secs(&ip);
    if let Some(secs) = kick_secs {
        hub.log(
            LogLevel::Warn,
            format!("已被断开的观众尝试重连，拒绝: {ip}（剩 {secs} 秒）"),
        );
        let msg = json!({ "type": "auth_banned", "secs": secs, "reason": "kick" }).to_string();
        if sink.send(Message::text(msg)).await.is_ok() {
            close_gracefully(&mut sink, &mut stream).await;
        }
        return;
    }

    // Connection-password gate: viewers register (and appear in the host UI)
    // only after presenting the password, so unauthenticated sockets never
    // receive frames or count towards the audience list.
    if hub.pw_on.load(Ordering::Relaxed) {
        // An IP that exhausted its attempts is locked out: connections are
        // dropped outright without the password ever being consulted.
        // (Bind first — a lock() temporary would otherwise be held across
        // the await below, making the future !Send.)
        let ban_secs = hub.pw_guard.lock().unwrap().ban_left_secs(&ip);
        if let Some(secs) = ban_secs {
            hub.log(LogLevel::Warn, format!("IP 已被锁定，拒绝连接: {ip}"));
            let msg = json!({ "type": "auth_banned", "secs": secs }).to_string();
            if sink.send(Message::text(msg)).await.is_ok() {
                close_gracefully(&mut sink, &mut stream).await;
            }
            return;
        }
        let need = json!({ "type": "need_auth" }).to_string();
        if sink.send(Message::text(need)).await.is_err() {
            return;
        }
        if !auth_gate(&hub, &mut sink, &mut stream, ip).await {
            close_gracefully(&mut sink, &mut stream).await;
            return;
        }
    }

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

/// Close the socket and briefly drain incoming frames. Dropping a socket
/// with unread client data makes the OS send a TCP reset instead of a FIN,
/// and resets wipe buffered received data on the peer — a rejection reply
/// sent right before closing would never reach the viewer (observed as
/// close code 1006 with no message). Draining lets the close handshake
/// complete normally.
async fn close_gracefully(
    sink: &mut SplitSink<WebSocket, Message>,
    stream: &mut SplitStream<WebSocket>,
) {
    let _ = sink.close().await;
    let _ = tokio::time::timeout(CLOSE_DRAIN_TIMEOUT, async {
        while stream.next().await.is_some() {}
    })
    .await;
}

/// Ask the viewer for the connection password and wait for a matching
/// `auth` message. Failure counts and bans are tracked per IP on the hub,
/// so they survive reconnects; a correct answer wipes the count. Returns
/// true once the viewer may proceed to registration; false means the
/// socket should be dropped unregistered.
async fn auth_gate(
    hub: &Hub,
    sink: &mut SplitSink<WebSocket, Message>,
    stream: &mut SplitStream<WebSocket>,
    ip: IpAddr,
) -> bool {
    loop {
        let msg = match tokio::time::timeout(AUTH_TIMEOUT, stream.next()).await {
            Ok(Some(m)) => m,
            Ok(None) => return false, // socket closed by the viewer
            Err(_) => {
                hub.log(LogLevel::Warn, format!("观众密码输入超时，已断开: {ip}"));
                return false;
            }
        };
        match msg {
            Ok(Message::Text(txt)) => {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(txt.as_str()) else {
                    continue;
                };
                if v.get("type").and_then(|t| t.as_str()) != Some("auth") {
                    continue; // hello/ping before auth carry no privileges
                }
                let attempt = v.get("pw").and_then(|x| x.as_str()).unwrap_or("");
                let expected = hub.password.lock().unwrap().clone();
                if !expected.is_empty() && attempt == expected {
                    // Success wipes the IP's failure record.
                    hub.pw_guard.lock().unwrap().clear_fails(&ip);
                    let ok = json!({ "type": "auth_ok" }).to_string();
                    if sink.send(Message::text(ok)).await.is_err() {
                        return false;
                    }
                    hub.log(LogLevel::Info, format!("观众密码验证通过: {ip}"));
                    return true;
                }
                // Bind before matching: a lock() temporary in the scrutinee
                // would be held across the awaits in the arms (!Send).
                let outcome = hub.pw_guard.lock().unwrap().add_fail(&ip);
                match outcome {
                    PwFail::Banned(secs) => {
                        hub.log(
                            LogLevel::Warn,
                            format!(
                                "观众密码错误次数过多，IP 已锁定 {} 分钟: {ip}",
                                PW_BAN_DURATION.as_secs() / 60
                            ),
                        );
                        let reply = json!({ "type": "auth_banned", "secs": secs }).to_string();
                        let _ = sink.send(Message::text(reply)).await;
                        return false;
                    }
                    PwFail::Counted(fails) => {
                        hub.log(
                            LogLevel::Warn,
                            format!("观众密码错误（{fails}/{PW_MAX_FAILS}）: {ip}"),
                        );
                        let reply = json!({ "type": "auth_fail", "left": PW_MAX_FAILS - fails })
                            .to_string();
                        if sink.send(Message::text(reply)).await.is_err() {
                            return false;
                        }
                    }
                }
            }
            Ok(Message::Close(_)) | Err(_) => return false,
            Ok(_) => {} // binary frames / protocol pings before auth
        }
    }
}

fn remove_viewer(hub: &Hub, entry: &Arc<ViewerEntry>) {
    hub.viewers.lock().unwrap().retain(|v| v.id != entry.id);
    // Lead with the IP so join/leave entries line up in the log; the
    // OS · browser identity follows once the hello message has filled it in.
    let os = entry.os.lock().unwrap().clone();
    let browser = entry.browser_ref();
    let who = match (os.is_empty(), browser.is_empty()) {
        (false, false) => format!(" ({os} · {browser})"),
        (true, false) => format!(" ({browser})"),
        _ => String::new(),
    };
    hub.log(LogLevel::Info, format!("观众离开: {}{who}", entry.addr));
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
