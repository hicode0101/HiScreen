//! Screen capture: display enumeration, the capture/encode thread and
//! single-frame preview grabs for the host UI thumbnails.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use jpeg_encoder::{ColorType, Encoder};
use scrap::{Capturer, Display};

use crate::hub::{DisplayMeta, Hub, LogLevel, FRAME_VIDEO};

/// One running capture session for a display; dropping it stops the thread.
pub struct CaptureHandle {
    pub display: DisplayMeta,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn display_meta(id: usize, display: &Display) -> DisplayMeta {
    DisplayMeta {
        id,
        name: format!("Display {}", id + 1),
        width: display.width() as u32,
        height: display.height() as u32,
        // scrap itself treats the first enumerated display as "primary"
        // (its Display::primary() just takes the first of Display::all()).
        primary: id == 0,
    }
}

fn find_display(id: usize) -> io::Result<Display> {
    let displays = Display::all()?;
    displays
        .into_iter()
        .nth(id)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "display not found"))
}

/// Enumerate all displays for the host UI.
pub fn list_displays() -> io::Result<Vec<DisplayMeta>> {
    let displays = Display::all()?;
    Ok(displays
        .iter()
        .enumerate()
        .map(|(i, d)| display_meta(i, d))
        .collect())
}

/// Start the capture + JPEG encode thread for the given display.
pub fn start(hub: Arc<Hub>, display_id: usize) -> io::Result<CaptureHandle> {
    // Validate the display index up front so callers get an immediate error.
    let displays = Display::all()?;
    let meta = displays
        .get(display_id)
        .map(|d| display_meta(display_id, d))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "display not found"))?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let hub2 = hub.clone();
    let thread = thread::Builder::new()
        .name("hiscreen-capture".into())
        .spawn(move || run_capture(hub2, display_id, stop2))?;
    Ok(CaptureHandle {
        display: meta,
        stop,
        thread: Some(thread),
    })
}

/// Capture loop with automatic retry: a session error (display mode change,
/// device loss) is logged and the capture restarts until the stop flag sets.
fn run_capture(hub: Arc<Hub>, display_id: usize, stop: Arc<AtomicBool>) {
    let mut backoff = 500u64;
    while !stop.load(Ordering::Relaxed) {
        match capture_session(&hub, display_id, &stop) {
            Ok(()) => return, // stopped by flag
            Err(e) => {
                hub.log(
                    LogLevel::Error,
                    format!("屏幕捕获出错（{}ms 后重试）: {e}", backoff),
                );
                thread::sleep(Duration::from_millis(backoff));
                backoff = (backoff * 2).min(4000);
            }
        }
    }
}

fn capture_session(hub: &Hub, display_id: usize, stop: &AtomicBool) -> io::Result<()> {
    let display = find_display(display_id)?;
    let (w, h) = (display.width(), display.height());
    let mut capturer = Capturer::new(display)?;

    let mut next_tick = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        // Pick up live quality / fps changes each iteration. frame() polls
        // with a 0 ms internal timeout (WouldBlock when nothing changed),
        // so pacing below drives both the fps and the stop responsiveness.
        let frame_dur = Duration::from_millis(1000 / hub.fps.load(Ordering::Relaxed).max(1) as u64);
        match capturer.frame() {
            Ok(frame) => {
                if frame.len() < w * h * 4 {
                    // DXGI can hand back short buffers right after a
                    // resolution change; skip this frame.
                    continue;
                }
                encode_and_broadcast(hub, &frame, w, h);
            }
            Err(ref e)
                if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::TimedOut => {}
            Err(e) => return Err(e),
        }
        // Pace encoding to the target fps.
        let now = Instant::now();
        if now < next_tick {
            thread::sleep(next_tick - now);
        } else {
            next_tick = now;
        }
        next_tick += frame_dur;
    }
}

fn encode_and_broadcast(hub: &Hub, frame: &[u8], w: usize, h: usize) {
    let quality = hub.quality.load(Ordering::Relaxed).clamp(1, 100) as u8;
    let mut jpeg = Vec::with_capacity(96 * 1024);
    let encoder = Encoder::new(&mut jpeg, quality);
    if encoder.encode(frame, w as u16, h as u16, ColorType::Bgra).is_err() {
        return;
    }
    let mut msg = Vec::with_capacity(jpeg.len() + 1);
    msg.push(FRAME_VIDEO);
    msg.extend_from_slice(&jpeg);
    // send() fails only when no viewer is connected — that is fine, the host
    // still keeps capturing so previews and late joiners work.
    let _ = hub.video.send(Arc::from(msg));
}

/// Grab a single frame of the given display, downscaled to `max_width`,
/// encoded as JPEG — used for the host-side display selector thumbnails.
pub fn capture_preview(display_id: usize, max_width: u32) -> io::Result<Vec<u8>> {
    let display = find_display(display_id)?;
    let (w, h) = (display.width(), display.height());
    let mut capturer = Capturer::new(display)?;
    let frame = capturer.frame()?;
    if frame.len() < w * h * 4 {
        return Err(io::Error::new(io::ErrorKind::Other, "short frame buffer"));
    }
    let scale = ((w as f32 / max_width as f32).ceil() as usize).max(1);
    let (dw, dh) = ((w / scale).max(1), (h / scale).max(1));
    let small = downscale_bgra(&frame, w, h, dw, dh);
    let mut jpeg = Vec::with_capacity(32 * 1024);
    let encoder = Encoder::new(&mut jpeg, 70);
    encoder
        .encode(&small, dw as u16, dh as u16, ColorType::Bgra)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    Ok(jpeg)
}

/// Simple integer box-average downscale of a BGRA buffer (quality is ample
/// for thumbnails and keeps the pipeline dependency-free).
fn downscale_bgra(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut dst = vec![0u8; dw * dh * 4];
    for dy in 0..dh {
        let y0 = dy * sh / dh;
        let y1 = ((dy + 1) * sh / dh).max(y0 + 1);
        for dx in 0..dw {
            let x0 = dx * sw / dw;
            let x1 = ((dx + 1) * sw / dw).max(x0 + 1);
            let (mut b, mut g, mut r, mut a) = (0u32, 0u32, 0u32, 0u32);
            let mut count = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * sw + x) * 4;
                    b += src[i] as u32;
                    g += src[i + 1] as u32;
                    r += src[i + 2] as u32;
                    a += src[i + 3] as u32;
                    count += 1;
                }
            }
            let o = (dy * dw + dx) * 4;
            dst[o] = (b / count) as u8;
            dst[o + 1] = (g / count) as u8;
            dst[o + 2] = (r / count) as u8;
            dst[o + 3] = (a / count) as u8;
        }
    }
    dst
}
