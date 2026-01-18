//! Audio runtime implementation.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Audio configuration
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// Sample rate (Hz)
    pub sample_rate: u32,
    /// Channel count (1 or 2)
    pub channels: u16,
    /// Target bitrate (bps)
    pub bitrate: u32,
}

/// Encoded audio packet
#[derive(Debug, Clone)]
pub struct AudioPacket {
    pub data: Vec<u8>,
}

#[cfg(all(not(feature = "audio"), not(feature = "pulseaudio")))]
pub fn run_audio_capture(
    config: AudioConfig,
    _sender: broadcast::Sender<AudioPacket>,
    running: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let _ = (config.sample_rate, config.channels, config.bitrate);
    while running.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(())
}

#[cfg(all(feature = "audio", not(feature = "pulseaudio")))]
pub fn run_audio_capture(
    config: AudioConfig,
    sender: broadcast::Sender<AudioPacket>,
    running: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use opus::{Application, Bitrate, Channels, Encoder};
    use std::collections::VecDeque;

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or("No default input device")?;
    let supported_config = {
        let mut selected = None;
        if let Ok(configs) = device.supported_input_configs() {
            for cfg in configs {
                if cfg.channels() == config.channels {
                    let min_rate = cfg.min_sample_rate().0;
                    let max_rate = cfg.max_sample_rate().0;
                    if config.sample_rate >= min_rate && config.sample_rate <= max_rate {
                        selected = Some(cfg.with_sample_rate(cpal::SampleRate(config.sample_rate)));
                        break;
                    }
                }
            }
        }
        match selected {
            Some(cfg) => cfg,
            None => device.default_input_config()?,
        }
    };

    let sample_rate = supported_config.sample_rate().0;
    let channel_count = supported_config.channels();
    let channels = match channel_count {
        1 => Channels::Mono,
        2 => Channels::Stereo,
        _ => return Err("Unsupported channel count".into()),
    };

    let mut encoder = Encoder::new(sample_rate, channels, Application::Audio)?;
    encoder.set_bitrate(Bitrate::Bits(config.bitrate as i32))?;

    let frame_size = (sample_rate / 50) as usize; // 20ms
    let samples_per_frame = frame_size * channel_count as usize;
    let buffer = Arc::new(std::sync::Mutex::new(VecDeque::<i16>::new()));

    let err_fn = |err| {
        eprintln!("Audio stream error: {:?}", err);
    };

    let buffer_clone = buffer.clone();
    let sender_clone = sender.clone();
    let running_clone = running.clone();

    let stream = match supported_config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &supported_config.config(),
            move |data: &[f32], _| {
                if !running_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let mut buf = buffer_clone.lock().unwrap();
                for sample in data {
                    let s = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
                    buf.push_back(s);
                }
                encode_ready_frames(&mut encoder, &mut buf, frame_size, samples_per_frame, &sender_clone);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            &supported_config.config(),
            move |data: &[i16], _| {
                if !running_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let mut buf = buffer_clone.lock().unwrap();
                for sample in data {
                    buf.push_back(*sample);
                }
                encode_ready_frames(&mut encoder, &mut buf, frame_size, samples_per_frame, &sender_clone);
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_input_stream(
            &supported_config.config(),
            move |data: &[u16], _| {
                if !running_clone.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let mut buf = buffer_clone.lock().unwrap();
                for sample in data {
                    let s = (*sample as i32 - 32768) as i16;
                    buf.push_back(s);
                }
                encode_ready_frames(&mut encoder, &mut buf, frame_size, samples_per_frame, &sender_clone);
            },
            err_fn,
            None,
        )?,
    };

    stream.play()?;
    while running.load(std::sync::atomic::Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    drop(stream);

    Ok(())
}

#[cfg(feature = "pulseaudio")]
pub fn run_audio_capture(
    config: AudioConfig,
    sender: broadcast::Sender<AudioPacket>,
    running: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error>> {
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::Direction;
    use libpulse_simple_binding::Simple;
    use opus::{Application, Bitrate, Channels, Encoder};
    use std::collections::VecDeque;

    let channels = match config.channels {
        1 => Channels::Mono,
        2 => Channels::Stereo,
        _ => return Err("Unsupported channel count".into()),
    };

    let spec = Spec {
        format: Format::S16le,
        rate: config.sample_rate,
        channels: config.channels as u8,
    };

    let simple = Simple::new(
        None,
        "selkies-core",
        Direction::Record,
        None,
        "capture",
        &spec,
        None,
        None,
    )?;

    let mut encoder = Encoder::new(config.sample_rate, channels, Application::Audio)?;
    encoder.set_bitrate(Bitrate::Bits(config.bitrate as i32))?;

    let frame_size = (config.sample_rate / 50) as usize; // 20ms
    let samples_per_frame = frame_size * config.channels as usize;
    let mut buffer = VecDeque::<i16>::new();
    let mut read_buf = vec![0u8; samples_per_frame * 2];

    while running.load(std::sync::atomic::Ordering::Relaxed) {
        simple.read(&mut read_buf)?;
        for chunk in read_buf.chunks_exact(2) {
            buffer.push_back(i16::from_le_bytes([chunk[0], chunk[1]]));
        }
        encode_ready_frames(&mut encoder, &mut buffer, frame_size, samples_per_frame, &sender);
    }

    Ok(())
}

#[cfg(any(feature = "audio", feature = "pulseaudio"))]
fn encode_ready_frames(
    encoder: &mut opus::Encoder,
    buffer: &mut std::collections::VecDeque<i16>,
    frame_size: usize,
    samples_per_frame: usize,
    sender: &broadcast::Sender<AudioPacket>,
) {
    while buffer.len() >= samples_per_frame {
        let mut frame = Vec::with_capacity(samples_per_frame);
        for _ in 0..samples_per_frame {
            if let Some(sample) = buffer.pop_front() {
                frame.push(sample);
            }
        }
        let mut out = vec![0u8; 4000];
        if let Ok(len) = encoder.encode(&frame, frame_size, &mut out) {
            out.truncate(len);
            let _ = sender.send(AudioPacket { data: out });
        }
    }
}
