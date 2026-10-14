// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use au_core::AudioBuffer;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IoError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("encode error: {0}")]
    Encode(String),
    #[error("device error: {0}")]
    Device(String),
    #[error("unsupported format: {0}")]
    Unsupported(String),
}

pub fn import_file(path: &Path) -> Result<AudioBuffer, IoError> {
    use symphonia::core::{
        audio::SampleBuffer,
        codecs::DecoderOptions,
        errors::Error as SymphoniaError,
        formats::FormatOptions,
        io::MediaSourceStream,
        meta::MetadataOptions,
        probe::Hint,
    };

    let file = std::fs::File::open(path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|ext| ext.to_str()) {
        hint.with_extension(extension);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| IoError::Decode(error.to_string()))?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| IoError::Decode("no default audio track".to_owned()))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| IoError::Decode(error.to_string()))?;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| IoError::Decode("missing sample rate".to_owned()))?;
    let mut channels: Vec<Vec<f32>> = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::ResetRequired) => continue,
            Err(_) => break,
        };
        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => {
                let channel_count = decoded.spec().channels.count();
                if channels.is_empty() {
                    channels.resize_with(channel_count, Vec::new);
                }
                let mut samples =
                    SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                samples.copy_interleaved_ref(decoded);
                for (frame_index, frame) in samples.samples().chunks(channel_count).enumerate() {
                    for channel in 0..channel_count {
                        if let Some(output) = channels.get_mut(channel) {
                            if let Some(&sample) = frame.get(channel) {
                                let _ = frame_index;
                                output.push(sample);
                            }
                        }
                    }
                }
            }
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(SymphoniaError::ResetRequired) => continue,
            Err(error) => return Err(IoError::Decode(error.to_string())),
        }
    }

    Ok(AudioBuffer::from_channels(sample_rate, channels))
}

pub fn export_wav(path: &Path, buffer: &AudioBuffer, bits: u16) -> Result<(), IoError> {
    use hound::{SampleFormat, WavSpec, WavWriter};

    if !matches!(bits, 16 | 24 | 32) {
        return Err(IoError::Unsupported(format!(
            "WAV bit depth {bits}; expected 16, 24, or 32"
        )));
    }
    let channels = u16::try_from(buffer.n_channels())
        .map_err(|_| IoError::Unsupported("too many WAV channels".to_owned()))?;
    let float = bits == 32;
    let spec = WavSpec {
        channels,
        sample_rate: buffer.sample_rate,
        bits_per_sample: bits,
        sample_format: if float {
            SampleFormat::Float
        } else {
            SampleFormat::Int
        },
    };
    let mut writer =
        WavWriter::create(path, spec).map_err(|error| IoError::Encode(error.to_string()))?;

    for sample in buffer.interleaved() {
        let sample = sample.clamp(-1.0, 1.0);
        if float {
            writer
                .write_sample(sample)
                .map_err(|error| IoError::Encode(error.to_string()))?;
        } else if bits == 16 {
            let value = (sample * i16::MAX as f32).round() as i16;
            writer
                .write_sample(value)
                .map_err(|error| IoError::Encode(error.to_string()))?;
        } else {
            let value = (sample * 8_388_607.0).round() as i32;
            writer
                .write_sample(value)
                .map_err(|error| IoError::Encode(error.to_string()))?;
        }
    }
    writer
        .finalize()
        .map_err(|error| IoError::Encode(error.to_string()))
}

pub fn export_flac(path: &Path, buffer: &AudioBuffer, bits: usize) -> Result<(), IoError> {
    use flacenc::component::BitRepr;

    if !matches!(bits, 8 | 16 | 24) {
        return Err(IoError::Unsupported(format!(
            "FLAC bit depth {bits}; expected 8, 16, or 24"
        )));
    }
    let channels = buffer.n_channels();
    if channels == 0 || channels > u8::MAX as usize {
        return Err(IoError::Unsupported("unsupported FLAC channel count".to_owned()));
    }
    let scale = ((1_i64 << (bits - 1)) - 1) as f32;
    let ints: Vec<i32> = buffer
        .interleaved()
        .into_iter()
        .map(|sample| (sample.clamp(-1.0, 1.0) * scale).round() as i32)
        .collect();

    use flacenc::error::Verify;
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|error| IoError::Encode(format!("{error:?}")))?;
    let source =
        flacenc::source::MemSource::from_samples(&ints, channels, bits, buffer.sample_rate as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|error| IoError::Encode(format!("{error:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|error| IoError::Encode(format!("{error:?}")))?;
    std::fs::write(path, sink.as_slice())?;
    Ok(())
}

#[cfg(feature = "device")]
pub mod device {
    use super::{AudioBuffer, IoError};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    pub fn output_devices() -> Vec<String> {
        cpal::default_host()
            .output_devices()
            .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
            .unwrap_or_default()
    }

    pub fn input_devices() -> Vec<String> {
        cpal::default_host()
            .input_devices()
            .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
            .unwrap_or_default()
    }

    pub struct Player {
        stream: cpal::Stream,
        position: Arc<AtomicUsize>,
        finished: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Player {
        pub fn play(buf: Arc<AudioBuffer>, start: usize) -> Result<Player, IoError> {
            let host = cpal::default_host();
            let device = host
                .default_output_device()
                .ok_or_else(|| IoError::Device("no default output device".to_owned()))?;
            let config = device
                .default_output_config()
                .map_err(|error| IoError::Device(error.to_string()))?;
            let channels = config.channels() as usize;
            let sample_format = config.sample_format();
            let config: cpal::StreamConfig = config.into();
            let position = Arc::new(AtomicUsize::new(start.min(buf.len())));
            let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cursor = position.clone();
            let done = finished.clone();
            let buffer = buf.clone();

            let stream = match sample_format {
                cpal::SampleFormat::F32 => device.build_output_stream(
                    &config,
                    move |output: &mut [f32], _| {
                        for frame in output.chunks_mut(channels) {
                            let index = cursor.load(Ordering::Relaxed);
                            if index >= buffer.len() {
                                frame.fill(0.0);
                                done.store(true, Ordering::Relaxed);
                                continue;
                            }
                            for (channel, sample) in frame.iter_mut().enumerate() {
                                let source_channel = channel.min(buffer.n_channels().saturating_sub(1));
                                *sample = buffer.channels
                                    .get(source_channel)
                                    .and_then(|data| data.get(index))
                                    .copied()
                                    .unwrap_or(0.0);
                            }
                            cursor.store(index + 1, Ordering::Relaxed);
                        }
                    },
                    |error| eprintln!("audio output error: {error}"),
                    None,
                ),
                _ => return Err(IoError::Unsupported("output device must support f32".to_owned())),
            }
            .map_err(|error| IoError::Device(error.to_string()))?;
            stream
                .play()
                .map_err(|error| IoError::Device(error.to_string()))?;
            // TODO: resample when the device rate differs from the buffer rate.
            Ok(Player {
                stream,
                position,
                finished,
            })
        }

        pub fn position(&self) -> usize {
            self.position.load(Ordering::Relaxed)
        }

        pub fn is_finished(&self) -> bool {
            self.finished.load(Ordering::Relaxed)
        }

        pub fn stop(&self) {
            let _ = &self.stream;
            self.finished.store(true, Ordering::Relaxed);
        }
    }

    pub struct Recorder {
        stream: cpal::Stream,
        samples: Arc<Mutex<Vec<f32>>>,
        channels: u16,
        sample_rate: u32,
    }

    impl Recorder {
        pub fn start(channels: u16) -> Result<Recorder, IoError> {
            let host = cpal::default_host();
            let device = host
                .default_input_device()
                .ok_or_else(|| IoError::Device("no default input device".to_owned()))?;
            let supported = device
                .default_input_config()
                .map_err(|error| IoError::Device(error.to_string()))?;
            if supported.sample_format() != cpal::SampleFormat::F32 {
                return Err(IoError::Unsupported("input device must support f32".to_owned()));
            }
            let sample_rate = supported.sample_rate().0;
            let config: cpal::StreamConfig = supported.into();
            let samples = Arc::new(Mutex::new(Vec::new()));
            let captured = samples.clone();
            let stream = device
                .build_input_stream(
                    &config,
                    move |input: &[f32], _| {
                        if let Ok(mut output) = captured.lock() {
                            output.extend_from_slice(input);
                        }
                    },
                    |error| eprintln!("audio input error: {error}"),
                    None,
                )
                .map_err(|error| IoError::Device(error.to_string()))?;
            stream
                .play()
                .map_err(|error| IoError::Device(error.to_string()))?;
            Ok(Recorder {
                stream,
                samples,
                channels: channels.max(1),
                sample_rate,
            })
        }

        pub fn stop(self) -> AudioBuffer {
            let _ = &self.stream;
            let samples = self.samples.lock().map(|samples| samples.clone()).unwrap_or_default();
            let channels = self.channels as usize;
            let mut output = vec![Vec::new(); channels];
            for frame in samples.chunks(channels) {
                for (channel, sample) in frame.iter().enumerate() {
                    output[channel].push(*sample);
                }
            }
            AudioBuffer::from_channels(self.sample_rate, output)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au_core::AudioBuffer;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn path(extension: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("au_io_{nonce}.{extension}"))
    }

    fn sample_buffer() -> AudioBuffer {
        AudioBuffer::from_channels(
            44_100,
            vec![vec![-0.7, -0.1, 0.0, 0.25, 0.9], vec![0.5, 0.2, -0.3, 0.8, -1.0]],
        )
    }

    #[test]
    fn wav_16_bit_roundtrip() {
        let path = path("wav");
        let original = sample_buffer();
        export_wav(&path, &original, 16).unwrap();
        let decoded = import_file(&path).unwrap();
        for (a, b) in original.interleaved().iter().zip(decoded.interleaved()) {
            assert!((a - b).abs() <= 1.0e-3);
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn wav_float_roundtrip_is_exact() {
        let path = path("wav");
        let original = sample_buffer();
        export_wav(&path, &original, 32).unwrap();
        let decoded = import_file(&path).unwrap();
        assert_eq!(original, decoded);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn flac_roundtrip() {
        let path = path("flac");
        let original = sample_buffer();
        export_flac(&path, &original, 24).unwrap();
        let decoded = import_file(&path).unwrap();
        for (a, b) in original.interleaved().iter().zip(decoded.interleaved()) {
            assert!((a - b).abs() <= 1.0e-5);
        }
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "device")]
    #[test]
    fn listing_output_devices_does_not_panic() {
        let _ = crate::device::output_devices();
    }
}
