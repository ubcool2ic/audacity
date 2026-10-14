// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use au_core::AudioBuffer;
use std::path::Path;
use thiserror::Error;

pub mod dsp;
pub mod ffmpeg;
pub mod midi;
pub mod engine;
pub mod record;

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
    #[error("{0}")]
    FfmpegMissing(String),
}

pub fn import_file(path: &Path) -> Result<AudioBuffer, IoError> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if ffmpeg::handles_extension(&ext) {
        return ffmpeg::get()?.import(path);
    }
    match import_native(path) {
        Ok(buffer) => Ok(buffer),
        // Like Audacity, fall back to FFmpeg for anything the built-in importers reject.
        Err(error) => match ffmpeg::get() {
            Ok(ffmpeg) => ffmpeg.import(path).map_err(|_| error),
            Err(_) => Err(error),
        },
    }
}

fn import_native(path: &Path) -> Result<AudioBuffer, IoError> {
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

/// Exports WAV with the default dither (triangular) when reducing to 16/24-bit.
pub fn export_wav(path: &Path, buffer: &AudioBuffer, bits: u16) -> Result<(), IoError> {
    export_wav_dithered(path, buffer, bits, dsp::Dither::default())
}

/// Exports WAV; integer depths are quantized with the selected [`dsp::Dither`].
pub fn export_wav_dithered(
    path: &Path,
    buffer: &AudioBuffer,
    bits: u16,
    dither: dsp::Dither,
) -> Result<(), IoError> {
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

    if float {
        for sample in buffer.interleaved() {
            writer
                .write_sample(sample.clamp(-1.0, 1.0))
                .map_err(|error| IoError::Encode(error.to_string()))?;
        }
    } else {
        let q = dsp::quantize_buffer(buffer, bits as u32, dither, 0x1234_5678);
        for i in 0..buffer.len() {
            for ch in &q {
                let r = if bits == 16 { writer.write_sample(ch[i] as i16) } else { writer.write_sample(ch[i]) };
                r.map_err(|error| IoError::Encode(error.to_string()))?;
            }
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
    use super::IoError;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{
        atomic::Ordering,
        Arc,
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

    /// Lock-free playback through the Stage 3 [`crate::engine`].
    ///
    /// Tracks are resampled with rubato to the device rate before the stream
    /// starts, so the realtime callback only mixes preallocated buffers: no
    /// allocation, no locks, commands arrive through an SPSC ring.
    pub struct Player {
        _stream: cpal::Stream,
        /// Lock-free command/meter handle shared with the audio thread.
        pub handle: crate::engine::EngineHandle,
        /// Device sample rate the mix runs at.
        pub device_rate: u32,
        /// Ratio device_rate / project_rate, for converting positions.
        pub rate_ratio: f64,
    }

    impl Player {
        /// Starts the engine on the default output device. `tracks` are at
        /// `project_rate`; `start` is a project-rate frame.
        pub fn start(
            tracks: Vec<crate::engine::MixTrack>,
            project_rate: u32,
            master_gain: f32,
            start: usize,
            speed: f64,
            loop_range: Option<(usize, usize)>,
        ) -> Result<Player, IoError> {
            Self::start_with_effects(tracks, Vec::new(), None, project_rate, master_gain, start, speed, loop_range)
        }

        /// Like [`Player::start`], with realtime effect stacks per track and on the master bus.
        #[allow(clippy::too_many_arguments)]
        pub fn start_with_effects(
            tracks: Vec<crate::engine::MixTrack>,
            track_fx: Vec<Option<au_effects::stack::EffectStack>>,
            master_fx: Option<au_effects::stack::EffectStack>,
            project_rate: u32,
            master_gain: f32,
            start: usize,
            speed: f64,
            loop_range: Option<(usize, usize)>,
        ) -> Result<Player, IoError> {
            let host = cpal::default_host();
            let device = host
                .default_output_device()
                .ok_or_else(|| IoError::Device("no default output device".to_owned()))?;
            let config = device
                .default_output_config()
                .map_err(|error| IoError::Device(error.to_string()))?;
            if config.sample_format() != cpal::SampleFormat::F32 {
                return Err(IoError::Unsupported("output device must support f32".to_owned()));
            }
            let device_rate = config.sample_rate().0;
            let channels = config.channels() as usize;
            let config: cpal::StreamConfig = config.into();
            let ratio = device_rate as f64 / project_rate.max(1) as f64;
            let conv = |f: usize| (f as f64 * ratio).round() as usize;
            let tracks: Vec<_> = tracks
                .into_iter()
                .map(|mut t| {
                    if t.audio.sample_rate != device_rate {
                        t.audio = Arc::new(crate::dsp::resample(&t.audio, device_rate));
                    }
                    t.offset = conv(t.offset);
                    t
                })
                .collect();
            let mut mixer = crate::engine::Mixer::new(tracks, master_gain);
            for (i, fx) in track_fx.into_iter().enumerate() { mixer.set_track_effects(i, fx); }
            mixer.set_master_effects(master_fx);
            mixer.set_position(conv(start));
            mixer.set_speed(speed);
            mixer.set_loop(loop_range.map(|(a, b)| (conv(a), conv(b))));
            let (mut handle, mut callback) = crate::engine::engine(mixer);
            let stream = device
                .build_output_stream(
                    &config,
                    move |output: &mut [f32], _| callback.render(output, channels),
                    |error| eprintln!("audio output error: {error}"),
                    None,
                )
                .map_err(|error| IoError::Device(error.to_string()))?;
            stream.play().map_err(|error| IoError::Device(error.to_string()))?;
            handle.send(crate::engine::EngineCommand::Play);
            Ok(Player { _stream: stream, handle, device_rate, rate_ratio: ratio })
        }

        /// Playback position in project-rate frames.
        pub fn position(&self) -> usize {
            (self.handle.position() / self.rate_ratio).max(0.0) as usize
        }

        /// Whether playback reached the end (or was stopped).
        pub fn is_finished(&self) -> bool {
            !self.handle.is_playing()
        }

        /// Stops playback.
        pub fn stop(&mut self) {
            self.handle.send(crate::engine::EngineCommand::Stop);
        }
    }

    /// Where a recording captures from.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum InputKind {
        /// The default input device (microphone / line-in).
        Default,
        /// System audio ("what you hear"). Windows: WASAPI loopback on the
        /// default output device. Linux: not native; choose the PulseAudio /
        /// PipeWire "Monitor of ..." source as the input device instead.
        /// macOS: unsupported (needs a virtual device such as BlackHole).
        Loopback,
    }

    /// Capture stream. The audio callback pushes samples into an SPSC ring
    /// (no locks, no allocation); call [`Recorder::poll`] from the UI/worker
    /// thread to drain them.
    pub struct Recorder {
        _stream: cpal::Stream,
        rx: rtrb::Consumer<f32>,
        peak: Arc<std::sync::atomic::AtomicU32>,
        /// Interleaved channel count of the capture.
        pub channels: u16,
        /// Capture sample rate.
        pub sample_rate: u32,
    }

    impl Recorder {
        /// Starts capture. `channels` is a request; the device's own count is used.
        pub fn start(kind: InputKind) -> Result<Recorder, IoError> {
            let host = cpal::default_host();
            let (device, supported) = match kind {
                InputKind::Default => {
                    let d = host
                        .default_input_device()
                        .ok_or_else(|| IoError::Device("no default input device".to_owned()))?;
                    let c = d.default_input_config().map_err(|e| IoError::Device(e.to_string()))?;
                    (d, c)
                }
                InputKind::Loopback => {
                    if !cfg!(target_os = "windows") {
                        return Err(IoError::Unsupported(
                            "system-audio loopback is only native on Windows (WASAPI); on Linux pick a \"Monitor of\" input, on macOS use a virtual device".to_owned(),
                        ));
                    }
                    let d = host
                        .default_output_device()
                        .ok_or_else(|| IoError::Device("no default output device".to_owned()))?;
                    let c = d.default_output_config().map_err(|e| IoError::Device(e.to_string()))?;
                    (d, c)
                }
            };
            if supported.sample_format() != cpal::SampleFormat::F32 {
                return Err(IoError::Unsupported("input device must support f32".to_owned()));
            }
            let sample_rate = supported.sample_rate().0;
            let channels = supported.channels();
            let config: cpal::StreamConfig = supported.into();
            // ~20 s of headroom; the UI drains every frame.
            let (mut tx, rx) = rtrb::RingBuffer::<f32>::new(sample_rate as usize * channels as usize * 20);
            let peak = Arc::new(std::sync::atomic::AtomicU32::new(0));
            let peak_cb = peak.clone();
            let stream = device
                .build_input_stream(
                    &config,
                    move |input: &[f32], _| {
                        let mut p = f32::from_bits(peak_cb.load(Ordering::Relaxed));
                        for &s in input {
                            p = p.max(s.abs());
                            let _ = tx.push(s);
                        }
                        peak_cb.store(p.to_bits(), Ordering::Relaxed);
                    },
                    |error| eprintln!("audio input error: {error}"),
                    None,
                )
                .map_err(|error| IoError::Device(error.to_string()))?;
            stream.play().map_err(|error| IoError::Device(error.to_string()))?;
            Ok(Recorder { _stream: stream, rx, peak, channels, sample_rate })
        }

        /// Drains captured interleaved samples into `out`.
        pub fn poll(&mut self, out: &mut Vec<f32>) {
            while let Ok(s) = self.rx.pop() {
                out.push(s);
            }
        }

        /// Returns and resets the input peak since the last call.
        pub fn take_peak(&self) -> f32 {
            f32::from_bits(self.peak.swap(0, Ordering::Relaxed))
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
