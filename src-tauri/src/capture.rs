//! Screen capture: display enumeration, the capture/encode thread and
//! single-frame preview grabs for the host UI thumbnails.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use jpeg_encoder::{ColorType, Encoder};
use scrap::{Capturer, Display};

use crate::cursor;
use crate::hub::{DisplayMeta, Hub, LogLevel, FRAME_VIDEO};

/// Width cap for the host-side preview thumbnails; the persistent grabber
/// and the session preview publisher must agree.
pub const PREVIEW_MAX_WIDTH: u32 = 480;
/// The session publishes preview thumbnails at this cadence; mirrors the
/// frontend preview loop (share.ts PREVIEW_INTERVAL_MS).
const PREVIEW_PUBLISH_INTERVAL: Duration = Duration::from_secs(2);

/// One running capture session for a display; dropping it stops the thread.
pub struct CaptureHandle {
    pub display: DisplayMeta,
    /// Newest downscaled preview thumbnail published by the session thread.
    /// While the session owns the display's duplication it also feeds the
    /// host preview — a second duplication (the persistent grabber) would
    /// conflict with the live capture, so same-display previews come from
    /// here instead.
    pub preview: Arc<Mutex<Option<Vec<u8>>>>,
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
    let preview = Arc::new(Mutex::new(None));
    let preview2 = preview.clone();
    let hub2 = hub.clone();
    let thread = thread::Builder::new()
        .name("hiscreen-capture".into())
        .spawn(move || run_capture(hub2, display_id, stop2, preview2))?;
    Ok(CaptureHandle {
        display: meta,
        preview,
        stop,
        thread: Some(thread),
    })
}

/// Capture loop with automatic retry: a session error (display mode change,
/// device loss) is logged and the capture restarts until the stop flag sets.
fn run_capture(
    hub: Arc<Hub>,
    display_id: usize,
    stop: Arc<AtomicBool>,
    preview: Arc<Mutex<Option<Vec<u8>>>>,
) {
    let mut backoff = 500u64;
    while !stop.load(Ordering::Relaxed) {
        match capture_session(&hub, display_id, &stop, &preview) {
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

fn capture_session(
    hub: &Hub,
    display_id: usize,
    stop: &AtomicBool,
    preview: &Arc<Mutex<Option<Vec<u8>>>>,
) -> io::Result<()> {
    let display = find_display(display_id)?;
    let (w, h) = (display.width(), display.height());
    // The cursor overlay needs the captured monitor's virtual-screen origin
    // to translate the cursor's screen position into frame coordinates.
    let origin = cursor::monitor_origin(display_id);
    let mut capturer = Capturer::new(display)?;

    let mut next_tick = Instant::now();
    // Publish the first frame right away so the host preview does not sit
    // on the pre-share thumbnail for one interval.
    let mut last_publish = Instant::now() - PREVIEW_PUBLISH_INTERVAL;
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
                let mut owned: Vec<u8>;
                let frame_data: &[u8] =
                    if hub.cursor_on.load(Ordering::Relaxed) && origin.is_some() {
                        // The duplication image has no pointer in it;
                        // composite the live cursor before encoding (one
                        // buffer copy).
                        owned = frame.to_vec();
                        cursor::draw_cursor(&mut owned, w as i32, h as i32, origin.unwrap());
                        &owned
                    } else {
                        &frame[..]
                    };
                encode_and_broadcast(hub, frame_data, w, h);
                maybe_publish_preview(preview, frame_data, w, h, &mut last_publish);
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

/// Publish a downscaled preview thumbnail from the live capture session (at
/// most every PREVIEW_PUBLISH_INTERVAL) so the host preview keeps refreshing
/// while sharing without needing a second duplication. Black pre-composition
/// buffers are skipped so the preview never freezes on a black frame right
/// after the duplication starts.
fn maybe_publish_preview(
    slot: &Arc<Mutex<Option<Vec<u8>>>>,
    frame: &[u8],
    w: usize,
    h: usize,
    last_publish: &mut Instant,
) {
    if last_publish.elapsed() < PREVIEW_PUBLISH_INTERVAL {
        return;
    }
    if frame_mean_rgb(frame) <= 2.0 {
        return;
    }
    if let Ok(jpeg) = encode_thumbnail(frame, w, h, PREVIEW_MAX_WIDTH) {
        *slot.lock().unwrap() = Some(jpeg);
        *last_publish = Instant::now();
    }
}

/// Mean of sampled B+G+R bytes (the alpha byte at i%4==3 is skipped) —
/// near 0 for the black pre-composition buffers DXGI hands out first.
fn frame_mean_rgb(frame: &[u8]) -> f32 {
    let (mut sum, mut n) = (0u64, 0u64);
    let mut i = 0;
    while i + 3 < frame.len() {
        if i % 4 != 3 {
            sum += (frame[i] as u64) + (frame[i + 1] as u64) + (frame[i + 2] as u64);
            n += 1;
        }
        i += 4;
    }
    if n == 0 { 0.0 } else { sum as f32 / n as f32 / 3.0 }
}

/// Long-lived preview capturer: one DXGI duplication kept alive across calls
/// instead of a fresh D3D11 device per second (constant device churn degrades
/// the driver over time). `last_jpeg` is the newest successful thumbnail and
/// is reused when the desktop stays static between polls.
pub struct PreviewGrabber {
    display_id: usize,
    w: usize,
    h: usize,
    capturer: Capturer,
    last_jpeg: Option<Vec<u8>>,
}

impl PreviewGrabber {
    fn new(display_id: usize) -> io::Result<Self> {
        let display = find_display(display_id)?;
        let (w, h) = (display.width(), display.height());
        Ok(Self {
            display_id,
            w,
            h,
            capturer: Capturer::new(display)?,
            last_jpeg: None,
        })
    }
}

// SAFETY: scrap's Capturer holds raw D3D11/DXGI pointers and is not `Send`
// only because scrap never needs to cross threads itself. The grabber is
// exclusively accessed while holding the preview mutex — one thread at a
// time — and D3D11 interfaces are apartment-free, so serialized use from
// different threads is sound.
unsafe impl Send for PreviewGrabber {}

/// Grab the newest preview frame of the given display, downscaled to
/// `max_width`, encoded as JPEG — the host-side display thumbnail and live
/// preview.
///
/// A fresh DXGI duplication may first hand out a pre-composition (black)
/// buffer, and later frames only arrive with desktop updates, so poll briefly
/// for a frame that shows content. Once a thumbnail exists it is returned
/// unchanged whenever no newer frame arrives (static desktop), and failures
/// drop the grabber so the next call rebuilds a fresh duplication.
pub fn preview_frame(
    slot: &mut Option<PreviewGrabber>,
    display_id: usize,
    max_width: u32,
) -> io::Result<Vec<u8>> {
    const DEADLINE: Duration = Duration::from_millis(1500);
    if slot.as_ref().map(|g| g.display_id) != Some(display_id) {
        *slot = None;
    }
    if slot.is_none() {
        *slot = Some(PreviewGrabber::new(display_id)?);
    }

    let started = Instant::now();
    loop {
        let grabber = slot.as_mut().unwrap();
        match grabber.capturer.frame() {
            Ok(frame) if frame.len() >= grabber.w * grabber.h * 4 => {
                if frame_mean_rgb(&frame) > 2.0 {
                    let jpeg = encode_thumbnail(&frame, grabber.w, grabber.h, max_width)?;
                    grabber.last_jpeg = Some(jpeg.clone());
                    break Ok(jpeg);
                }
                // Pre-composition (black) buffer — keep waiting for content.
            }
            Ok(_) => {} // short buffer right after a resolution change; retry
            Err(ref e)
                if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::TimedOut =>
            {
                // No desktop update since the last poll — reuse the newest
                // thumbnail when one exists.
                if let Some(jpeg) = grabber.last_jpeg.clone() {
                    break Ok(jpeg);
                }
            }
            Err(e) => {
                // Duplication lost (mode change, locked desktop, driver
                // reset) — rebuild a fresh grabber on the next call.
                *slot = None;
                break Err(e);
            }
        }
        if started.elapsed() >= DEADLINE {
            break slot
                .as_mut()
                .unwrap()
                .last_jpeg
                .clone()
                .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "no frame captured"));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn encode_thumbnail(frame: &[u8], w: usize, h: usize, max_width: u32) -> io::Result<Vec<u8>> {
    let scale = ((w as f32 / max_width as f32).ceil() as usize).max(1);
    let (dw, dh) = ((w / scale).max(1), (h / scale).max(1));
    let small = downscale_bgra(frame, w, h, dw, dh);
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
