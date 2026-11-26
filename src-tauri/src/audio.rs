//! System-audio capture feeding the hub's audio broadcast channel.
//!
//! Output contract (both platforms): mono samples pushed through
//! [`AudioFeed`], which resamples to 48 kHz, converts to i16 and emits
//! 20 ms chunks (960 samples) as binary frames (type byte + payload).
//!
//! - Windows: WASAPI loopback of the default render device (system audio).
//! - macOS / Linux: cpal default input device; on Linux a PulseAudio/PipeWire
//!   ".monitor" source is preferred when present, otherwise the microphone.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

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

/// Resamples any input rate down/up to 48 kHz mono and emits fixed-size
/// i16 chunks on the hub's audio channel (gated by the audio_on flag).
struct AudioFeed {
    hub: Arc<Hub>,
    src_rate: f64,
    buf: Vec<f64>,
    buf_pos: f64,
    out: Vec<i16>,
}

impl AudioFeed {
    fn new(hub: Arc<Hub>) -> Self {
        Self {
            hub,
            src_rate: DST_RATE,
            buf: Vec::new(),
            buf_pos: 0.0,
            out: Vec::with_capacity(CHUNK * 2),
        }
    }

    fn push(&mut self, samples: &[f32], rate: u32) {
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
            self.out.push(((s.clamp(-1.0, 1.0)) * 32767.0) as i16);
            pos += ratio;
            if self.out.len() >= CHUNK {
                self.flush();
            }
        }
        // Drop consumed input samples, keep the fractional position aligned.
        let consumed = pos as usize;
        self.buf.drain(..consumed);
        self.buf_pos = pos - consumed as f64;
    }

    fn flush(&mut self) {
        if self.out.len() < CHUNK {
            return;
        }
        if !self.hub.audio_on.load(Ordering::Relaxed) {
            self.out.clear();
            return;
        }
        let mut msg = Vec::with_capacity(CHUNK * 2 + 1);
        msg.push(FRAME_AUDIO);
        for s in self.out.drain(..CHUNK) {
            msg.extend_from_slice(&s.to_le_bytes());
        }
        let _ = self.hub.audio.send(Arc::from(msg));
    }
}

/// Average all channels down to mono (expecting interleaved frames).
fn downmix<'a, T: Copy, F: Fn(T) -> f32>(data: &'a [T], channels: usize, to_f32: F) -> Vec<f32> {
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

    let mut feed = AudioFeed::new(hub.clone());
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
        hub.log(LogLevel::Error, "音频捕获失败: 未找到输入设备".into());
        return;
    };
    let Ok(config) = device.default_input_config() else {
        hub.log(LogLevel::Error, "音频捕获失败: 输入设备配置不可用".into());
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
        hub.log(LogLevel::Error, "音频捕获失败: 不支持的采样格式".into());
        return;
    };
    if stream.play().is_err() {
        hub.log(LogLevel::Error, "音频捕获失败: 设备启动失败".into());
        return;
    }
    hub.log(
        LogLevel::Success,
        format!("音频设备已连接（{source_kind} {rate}Hz/{channels}ch）"),
    );

    let mut feed = AudioFeed::new(hub.clone());
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(samples) => feed.push(&samples, rate),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
    drop(stream);
}
