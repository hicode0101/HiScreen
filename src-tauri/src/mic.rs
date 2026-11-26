//! Microphone capture feeding the shared audio bus ([`crate::audio::AudioBus`])
//! so the mixer can sum it with system audio, plus a live input-level meter
//! for the host UI.
//!
//! Unlike the system-audio thread (which lives only inside a share session),
//! the mic thread runs whenever the feature is enabled in settings — that is
//! what powers the host's "test your microphone" level bar while idle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio::{downmix, AudioFeed, FeedKind};
use crate::hub::{Hub, LogLevel};

pub struct MicHandle {
    stop: Arc<AtomicBool>,
    /// Requested device name ("" = system default); lets commands detect a
    /// device switch that needs the thread restarted.
    pub device: String,
    thread: Option<JoinHandle<()>>,
}

impl Drop for MicHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// All capture (microphone) device names, de-duplicated in host order.
pub fn list_devices() -> Result<Vec<String>, String> {
    use cpal::traits::{DeviceTrait, HostTrait};

    let host = cpal::default_host();
    let mut names = Vec::new();
    let devices = host.input_devices().map_err(|e| e.to_string())?;
    for d in devices {
        if let Ok(name) = d.name() {
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }
    Ok(names)
}

/// Start the mic capture thread (retrying device failures every 2 s until
/// stopped). `device` = `None`/empty for the system default input.
pub fn start(hub: Arc<Hub>, device: Option<String>) -> MicHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let hub2 = hub.clone();
    let device_name = device.unwrap_or_default();
    let wanted = device_name.clone();
    let thread = thread::Builder::new()
        .name("hiscreen-mic".into())
        .spawn(move || run(hub2, stop2, wanted))
        .expect("spawn mic thread");
    MicHandle {
        stop,
        device: device_name,
        thread: Some(thread),
    }
}

fn run(hub: Arc<Hub>, stop: Arc<AtomicBool>, device_name: String) {
    loop {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        match capture_session(&hub, &stop, &device_name) {
            Ok(()) => return, // stopped
            Err(e) => {
                hub.log(LogLevel::Error, format!("麦克风捕获失败（2 秒后重试）: {e}"));
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

fn capture_session(
    hub: &Arc<Hub>,
    stop: &AtomicBool,
    device_name: &str,
) -> Result<(), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let host = cpal::default_host();
    // Pick the requested device by name; fall back to the default input.
    let mut picked = None;
    if !device_name.is_empty() {
        if let Ok(devices) = host.input_devices() {
            for d in devices {
                if d.name().ok().as_deref() == Some(device_name) {
                    picked = Some(d);
                    break;
                }
            }
        }
    }
    let mut fell_back = false;
    let device = match picked {
        Some(d) => d,
        None => {
            fell_back = !device_name.is_empty();
            host.default_input_device().ok_or("未找到麦克风设备")?
        }
    };
    if fell_back {
        hub.log(
            LogLevel::Warn,
            format!("麦克风“{device_name}”不可用，已回退默认设备"),
        );
    }
    let Ok(config) = device.default_input_config() else {
        return Err("麦克风配置不可用".into());
    };

    let channels = config.channels() as usize;
    let rate = config.sample_rate().0;
    let (tx, rx) = mpsc::channel::<Vec<f32>>();
    let err_fn = |e| eprintln!("[hiscreen:mic] stream error: {e}");

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
        return Err("不支持的采样格式".into());
    };
    if stream.play().is_err() {
        return Err("设备启动失败".into());
    }
    let name = device.name().unwrap_or_else(|_| device_name.to_string());
    hub.log(
        LogLevel::Success,
        format!("麦克风已连接（{name} {rate}Hz/{channels}ch）"),
    );

    let mut feed = AudioFeed::new(hub.clone(), FeedKind::Mic);
    // Capture streams deliver packets continuously (even in silence), so a
    // long gap means the device died (unplugged / disabled) — retry.
    let mut last_packet = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(samples) => {
                last_packet = Instant::now();
                feed.push(&samples, rate);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_packet.elapsed() > Duration::from_secs(2) {
                    drop(stream);
                    return Err("麦克风无数据（设备可能已断开）".into());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("麦克风流已结束".into());
            }
        }
    }
    drop(stream);
    Ok(())
}
