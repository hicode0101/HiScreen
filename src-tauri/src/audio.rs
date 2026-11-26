//! System-audio and microphone capture feeding the hub's audio broadcast
//! channel.
//!
//! Output contract (both platforms): mono samples pushed through
//! [`AudioFeed`], which resamples to 48 kHz and routes finished 20 ms chunks
//! (960 samples) either straight to the broadcast channel (legacy
//! single-source path, used while mic mixing is off) or into [`AudioBus`]
//! where the mixer pacer sums system + mic audio before broadcasting.
//!
//! - System audio, Windows: WASAPI loopback of the default render device.
//! - System audio, macOS / Linux: cpal default input device; on Linux a
//!   PulseAudio/PipeWire ".monitor" source is preferred, otherwise the mic.
//! - Microphone (all platforms): cpal input device, see [`crate::mic`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::hub::{Hub, LogLevel, FRAME_AUDIO};

#[cfg(unix)]
use std::sync::mpsc;

const DST_RATE: f64 = 48_000.0;
/// Samples per emitted chunk: 20 ms at 48 kHz.
const CHUNK: usize = 960;

pub struct AudioHandle {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for AudioHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Which source a feed captures; decides where finished chunks are routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FeedKind {
    /// System audio (loopback / monitor). Routes to the mixer bus while mic
    /// mixing is on, otherwise broadcasts directly (legacy path).
    System,
    /// Microphone. Always feeds the mixer bus and drives the level meter.
    Mic,
}

/// Resamples any input rate down/up to 48 kHz mono and routes fixed-size
/// chunks according to [`FeedKind`].
pub(crate) struct AudioFeed {
    hub: Arc<Hub>,
    kind: FeedKind,
    src_rate: f64,
    buf: Vec<f64>,
    buf_pos: f64,
    /// Resampled 48 kHz mono samples, emitted one 20 ms chunk at a time.
    chunk: Vec<f32>,
}

impl AudioFeed {
    pub(crate) fn new(hub: Arc<Hub>, kind: FeedKind) -> Self {
        Self {
            hub,
            kind,
            src_rate: DST_RATE,
            buf: Vec::new(),
            buf_pos: 0.0,
            chunk: Vec::with_capacity(CHUNK),
        }
    }

    pub(crate) fn push(&mut self, samples: &[f32], rate: u32) {
        if samples.is_empty() {
            return;
        }
        if rate as f64 != self.src_rate {
            self.src_rate = rate as f64;
            self.buf_pos = 0.0;
        }
        self.buf
            .extend(samples.iter().map(|s| *s as f64));

        let ratio = self.src_rate / DST_RATE;
        let mut pos = self.buf_pos;
        while (pos as usize) + 1 < self.buf.len() {
            let i = pos as usize;
            let frac = pos - i as f64;
            let s = self.buf[i] + (self.buf[i + 1] - self.buf[i]) * frac;
            self.chunk.push(s.clamp(-1.0, 1.0) as f32);
            pos += ratio;
            if self.chunk.len() >= CHUNK {
                self.emit();
            }
        }
        // Drop consumed input samples, keep the fractional position aligned.
        let consumed = pos as usize;
        self.buf.drain(..consumed);
        self.buf_pos = pos - consumed as f64;
    }

    fn emit(&mut self) {
        match self.kind {
            FeedKind::Mic => self.emit_mic(),
            FeedKind::System => self.emit_system(),
        }
        self.chunk.clear();
    }

    /// System-audio chunk: into the mixer bus while mic mixing is on, else
    /// broadcast directly (the legacy single-source path, unchanged).
    fn emit_system(&mut self) {
        if self.hub.mic_on.load(Ordering::Relaxed) {
            self.hub.audio_bus.push_system(&self.chunk);
            return;
        }
        if !self.hub.audio_on.load(Ordering::Relaxed) {
            return;
        }
        let mut msg = Vec::with_capacity(CHUNK * 2 + 1);
        msg.push(FRAME_AUDIO);
        for s in &self.chunk {
            msg.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
        }
        let _ = self.hub.audio.send(Arc::from(msg));
    }

    /// Mic chunk: apply the UI volume, update the dB meter, feed the bus.
    fn emit_mic(&mut self) {
        let gain = self.hub.mic_gain_pct.load(Ordering::Relaxed) as f32 / 100.0;
        let mut sum2 = 0.0f32;
        for s in &mut self.chunk {
            *s = (*s * gain).clamp(-1.0, 1.0);
            sum2 += *s * *s;
        }
        let rms = (sum2 / self.chunk.len().max(1) as f32).sqrt();
        // log10(0) is -inf, clamped to the -90 dB meter floor.
        let db = (20.0 * rms.log10()).clamp(-90.0, 0.0);
        self.hub
            .mic_level_db10
            .store((db * 10.0).round() as i32, Ordering::Relaxed);
        self.hub.audio_bus.push_mic(&self.chunk);
    }
}

/// Sample queues feeding the mixer pacer ([`start_mixer`]). Capture threads
/// push resampled 48 kHz mono chunks here while mic mixing is active.
#[derive(Default)]
pub struct AudioBus {
    system: Mutex<Vec<f32>>,
    mic: Mutex<Vec<f32>>,
}

impl AudioBus {
    /// ~100 ms of samples per source; older samples are dropped so a device
    /// clock slightly faster than the pacer cannot build unbounded latency.
    const CAP: usize = 4800;

    pub fn push_system(&self, chunk: &[f32]) {
        Self::push(&self.system, chunk);
    }

    pub fn push_mic(&self, chunk: &[f32]) {
        Self::push(&self.mic, chunk);
    }

    fn push(q: &Mutex<Vec<f32>>, chunk: &[f32]) {
        let mut q = q.lock().unwrap();
        q.extend_from_slice(chunk);
        let excess = q.len().saturating_sub(Self::CAP);
        if excess > 0 {
            q.drain(..excess);
        }
    }

    fn drain(q: &Mutex<Vec<f32>>, cap: usize) -> Vec<f32> {
        let mut q = q.lock().unwrap();
        let n = q.len().min(cap);
        q.drain(..n).collect()
    }

    pub fn drain_system(&self, cap: usize) -> Vec<f32> {
        Self::drain(&self.system, cap)
    }

    pub fn drain_mic(&self, cap: usize) -> Vec<f32> {
        Self::drain(&self.mic, cap)
    }

    /// Discard everything (used when no share session is running so queues
    /// can't hold stale samples).
    pub fn clear(&self) {
        self.system.lock().unwrap().clear();
        self.mic.lock().unwrap().clear();
    }
}

/// Average all channels down to mono (expecting interleaved frames).
pub(crate) fn downmix<'a, T: Copy, F: Fn(T) -> f32>(
    data: &'a [T],
    channels: usize,
    to_f32: F,
) -> Vec<f32> {
    let channels = channels.max(1);
    let frames = data.len() / channels;
    let mut out = Vec::with_capacity(frames);
    for f in 0..frames {
        let base = f * channels;
        let sum: f32 = data[base..base + channels].iter().map(|&s| to_f32(s)).sum();
        out.push(sum / channels as f32);
    }
    out
}

/// Start the audio thread for the current platform. The thread keeps the
/// device open while sharing and skips broadcasting when audio is muted.
pub fn start(hub: Arc<Hub>) -> AudioHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let hub2 = hub.clone();
    let thread = thread::Builder::new()
        .name("hiscreen-audio".into())
        .spawn(move || {
            #[cfg(windows)]
            run_windows(hub2, stop2);
            #[cfg(unix)]
            run_unix(hub2, stop2);
        })
        .expect("spawn audio thread");
    AudioHandle {
        stop,
        thread: Some(thread),
    }
}

/// Spawn the 20 ms mixer pacer. It lives for the whole app run and idles
/// (a sleeping loop) while mic mixing is off — the legacy single-source path
/// broadcasts straight from its capture thread instead. While mic mixing is
/// on it sums the pending system + mic samples into one output chunk every
/// tick; system audio keeps its mute gate (`audio_on`), mic keeps its gain.
pub fn start_mixer(hub: Arc<Hub>) {
    thread::Builder::new()
        .name("hiscreen-mix".into())
        .spawn(move || {
            let mut next = Instant::now();
            loop {
                next += Duration::from_millis(20);
                let now = Instant::now();
                if next > now {
                    thread::sleep(next - now);
                } else {
                    next = now; // fell behind: resync instead of bursting
                }
                mix_tick(&hub);
            }
        })
        .expect("spawn mixer thread");
}

fn mix_tick(hub: &Hub) {
    if !hub.mic_on.load(Ordering::Relaxed) {
        return;
    }
    // Mic capture also runs for the host's level meter while idle; only
    // broadcast to viewers during an active share session.
    if !hub.share.lock().unwrap().sharing {
        hub.audio_bus.clear();
        return;
    }
    let audio_on = hub.audio_on.load(Ordering::Relaxed);
    let sys = if audio_on {
        hub.audio_bus.drain_system(CHUNK)
    } else {
        hub.audio_bus.drain_system(usize::MAX); // muted: drop the backlog
        Vec::new()
    };
    let mic = hub.audio_bus.drain_mic(CHUNK);
    if sys.is_empty() && mic.is_empty() {
        return;
    }
    let mut msg = Vec::with_capacity(CHUNK * 2 + 1);
    msg.push(FRAME_AUDIO);
    for i in 0..CHUNK {
        let a = sys.get(i).copied().unwrap_or(0.0);
        let b = mic.get(i).copied().unwrap_or(0.0);
        let s = (a + b).clamp(-1.0, 1.0);
        msg.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
    }
    let _ = hub.audio.send(Arc::from(msg));
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod wasapi {
    use windows::Win32::Media::Audio::{
        eConsole, eRender, EDataFlow, ERole, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    };

    pub const RENDER: EDataFlow = eRender;
    pub const CONSOLE: ERole = eConsole;

    /// Parse a mix format: (channels, sample rate, bits per sample, is-f32).
    /// Fields are read via `addr_of!` because WAVEFORMATEX is a packed struct.
    pub unsafe fn parse_format(fmt: *mut WAVEFORMATEX) -> (usize, u32, u16, bool) {
        const EXTENSIBLE_TAG: u16 = 0xFFFE;
        const IEEE_FLOAT_TAG: u16 = 0x0003;
        const IEEE_FLOAT_GUID: windows::core::GUID =
            windows::core::GUID::from_values(0x0000_0003, 0x0000, 0x0010, [
                0x80, 0x00, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71,
            ]);
        let tag = std::ptr::addr_of!((*fmt).wFormatTag).read();
        let bits = std::ptr::addr_of!((*fmt).wBitsPerSample).read();
        let channels = std::ptr::addr_of!((*fmt).nChannels).read();
        let rate = std::ptr::addr_of!((*fmt).nSamplesPerSec).read();
        let cb_size = std::ptr::addr_of!((*fmt).cbSize).read();
        let float = if tag == EXTENSIBLE_TAG && cb_size as usize >= 22 {
            let sub =
                std::ptr::addr_of!((*(fmt as *const WAVEFORMATEXTENSIBLE)).SubFormat).read();
            sub == IEEE_FLOAT_GUID && bits == 32
        } else {
            tag == IEEE_FLOAT_TAG && bits == 32
        };
        (channels as usize, rate, bits, float)
    }

    pub unsafe fn free_format(fmt: *mut WAVEFORMATEX) {
        windows::Win32::System::Com::CoTaskMemFree(Some(fmt.cast()));
    }
}

#[cfg(windows)]
fn run_windows(hub: Arc<Hub>, stop: Arc<AtomicBool>) {
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        match unsafe { wasapi_loopback(&hub, &stop) } {
            Ok(()) => return, // stopped
            Err(e) => {
                hub.log(LogLevel::Error, format!("音频捕获失败（2 秒后重试）: {e}"));
                for _ in 0..20 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

#[cfg(windows)]
unsafe fn wasapi_loopback(hub: &Arc<Hub>, stop: &AtomicBool) -> Result<(), String> {
    use std::slice;
    use windows::Win32::Media::Audio::{
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
        IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
    // S_FALSE (already initialized) and RPC_E_CHANGED_MODE are both fine to
    // continue from: WASAPI accepts either apartment in practice.
    if hr.is_err()
        && hr != windows::core::HRESULT(1)
        && hr != windows::core::HRESULT(0x8001_0106u32 as i32)
    {
        return Err(format!("CoInitializeEx: {hr:?}"));
    }

    let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
        .map_err(|e| e.to_string())?;
    let device = enumerator
        .GetDefaultAudioEndpoint(wasapi::RENDER, wasapi::CONSOLE)
        .map_err(|e| e.to_string())?;
    let client: IAudioClient = device
        .Activate(CLSCTX_ALL, None)
        .map_err(|e| e.to_string())?;

    let mix_format: *mut windows::Win32::Media::Audio::WAVEFORMATEX = client
        .GetMixFormat()
        .map_err(|e| e.to_string())?;
    let (channels, rate, bits, float_samples) = wasapi::parse_format(mix_format);

    if let Err(e) = client.Initialize(
        AUDCLNT_SHAREMODE_SHARED,
        AUDCLNT_STREAMFLAGS_LOOPBACK,
        0,
        0,
        mix_format,
        None,
    ) {
        wasapi::free_format(mix_format);
        return Err(format!("Initialize: {e}"));
    }
    let capture_client: IAudioCaptureClient = client.GetService().map_err(|e| e.to_string())?;
    client.Start().map_err(|e| e.to_string())?;
    hub.log(
        LogLevel::Success,
        format!("音频设备已连接（系统声音 {rate}Hz/{channels}ch/{bits}bit）"),
    );

    let mut feed = AudioFeed::new(hub.clone(), FeedKind::System);
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = client.Stop();
            wasapi::free_format(mix_format);
            return Ok(());
        }
        let Ok(mut frames) = capture_client.GetNextPacketSize() else {
            wasapi::free_format(mix_format);
            return Err("capture session ended".into());
        };
        if frames == 0 {
            thread::sleep(Duration::from_millis(8));
            continue;
        }
        let mut data: *mut u8 = std::ptr::null_mut();
        let mut flags: u32 = 0;
        if capture_client
            .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
            .is_err()
        {
            wasapi::free_format(mix_format);
            return Err("GetBuffer failed".into());
        }
        if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) == 0 && !data.is_null() {
            if float_samples {
                let f32s: &[f32] =
                    slice::from_raw_parts(data.cast(), frames as usize * channels);
                let mono = downmix(f32s, channels, |s| s);
                feed.push(&mono, rate);
            } else if bits == 16 {
                let i16s: &[i16] =
                    slice::from_raw_parts(data.cast(), frames as usize * channels);
                let mono = downmix(i16s, channels, |s| s as f32 / 32768.0);
                feed.push(&mono, rate);
            }
        }
        let _ = capture_client.ReleaseBuffer(frames);
    }
}

// ------------------------------------------------------------- macOS/Linux

#[cfg(unix)]
fn run_unix(hub: Arc<Hub>, stop: Arc<AtomicBool>) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    // Prefer a PulseAudio/PipeWire ".monitor" source (captures system audio
    // like WASAPI loopback); fall back to the default input (microphone).
    let mut device = host.default_input_device();
    let mut source_kind = "microphone";
    if let Ok(devices) = host.input_devices() {
        for d in devices {
            if let Ok(name) = d.name() {
                if name.to_lowercase().contains("monitor") {
                    device = Some(d);
                    source_kind = "monitor";
                    break;
                }
            }
        }
    }
    let Some(device) = device else {
        hub.log(LogLevel::Error, "音频捕获失败: 未找到输入设备");
        return;
    };
    let Ok(config) = device.default_input_config() else {
        hub.log(LogLevel::Error, "音频捕获失败: 输入设备配置不可用");
        return;
    };

    let channels = config.channels() as usize;
    let rate = config.sample_rate().0;
    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let err_fn = |e| eprintln!("[hiscreen:audio] stream error: {e}");

    let stream_result = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.into(),
            move |data: &[f32], _| {
                let _ = tx.send(downmix(data, channels, |s| s));
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config.into(),
            move |data: &[i16], _| {
                let _ = tx.send(downmix(data, channels, |s| s as f32 / 32768.0));
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config.into(),
            move |data: &[u16], _| {
                let _ = tx.send(downmix(data, channels, |s| (s as f32 - 32768.0) / 32768.0));
            },
            err_fn,
            None,
        ),
        _ => Err(cpal::BuildStreamError::StreamConfigNotSupported),
    };
    let Ok(stream) = stream_result else {
        hub.log(LogLevel::Error, "音频捕获失败: 不支持的采样格式");
        return;
    };
    if stream.play().is_err() {
        hub.log(LogLevel::Error, "音频捕获失败: 设备启动失败");
        return;
    }
    hub.log(
        LogLevel::Success,
        format!("音频设备已连接（{source_kind} {rate}Hz/{channels}ch）"),
    );

    let mut feed = AudioFeed::new(hub.clone(), FeedKind::System);
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(samples) => feed.push(&samples, rate),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
    drop(stream);
}
