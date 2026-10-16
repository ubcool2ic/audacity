// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use au_core::AudioBuffer;
use std::path::Path;
use thiserror::Error;

pub mod dsp;
pub mod ffmpeg;
pub mod midi;
pub mod engine;
pub mod export;
pub mod record;
pub mod input_gain;

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
    /// The progress callback asked to stop (user pressed Cancel).
    #[error("cancelled")]
    Cancelled,
}

pub fn import_file(path: &Path) -> Result<AudioBuffer, IoError> {
    let mut progress = |_: u64, _: u64| true;
    import_file_with_progress(path, &mut progress)
}

/// Imports an audio file, reporting progress and allowing the import to be cancelled.
pub fn import_file_with_progress(
    path: &Path,
    progress: crate::export::ProgressFn,
) -> Result<AudioBuffer, IoError> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if ffmpeg::handles_extension(&ext) {
        if !progress(0, 0) {
            return Err(IoError::Cancelled);
        }
        let buffer = ffmpeg::get()?.import(path)?;
        if !progress(1, 1) {
            return Err(IoError::Cancelled);
        }
        return Ok(buffer);
    }
    match import_native_with_progress(path, progress) {
        Ok(buffer) => Ok(buffer),
        Err(IoError::Cancelled) => Err(IoError::Cancelled),
        // Like Audacity, fall back to FFmpeg for anything the built-in importers reject.
        Err(error) => match ffmpeg::get() {
            Ok(ffmpeg) => {
                if !progress(0, 0) {
                    return Err(IoError::Cancelled);
                }
                let buffer = match ffmpeg.import(path) {
                    Ok(buffer) => buffer,
                    Err(_) => return Err(error),
                };
                if !progress(1, 1) {
                    return Err(IoError::Cancelled);
                }
                Ok(buffer)
            }
            Err(_) => Err(error),
        },
    }
}


fn import_native_with_progress(
    path: &Path,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<AudioBuffer, IoError> {
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
    let total_frames = track.codec_params.n_frames.unwrap_or(0);
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| IoError::Decode(error.to_string()))?;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| IoError::Decode("missing sample rate".to_owned()))?;
    let mut channels: Vec<Vec<f32>> = Vec::new();
    let mut decoded_frames = 0_u64;

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::ResetRequired) => continue,
            Err(_) => break,
        };
        if packet.track_id() == track_id {
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
                        decoded_frames += 1;
                    }
                }
                Err(SymphoniaError::DecodeError(_)) => {}
                Err(SymphoniaError::ResetRequired) => {}
                Err(error) => return Err(IoError::Decode(error.to_string())),
            }
        }
        if !progress(decoded_frames, total_frames) {
            return Err(IoError::Cancelled);
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
                .write_sample(sample) // float WAV is unclamped, like Audacity (libsndfile float)
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

/// Exports planar float audio in the given format with metadata tags and progress reporting.
pub fn export_any_with_progress(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    format: &export::ExportFormat,
    tags: &au_core::tags::Tags,
    progress: export::ProgressFn,
) -> Result<(), IoError> {
    match format {
        export::ExportFormat::Pcm(o) => {
            export::intfmt::export_pcm_with_progress(path, channels, sample_rate, o, tags, progress)
        }
        export::ExportFormat::Flac(o) => {
            export::intfmt::export_flac_with_progress(path, channels, sample_rate, o, tags, progress)
        }
        export::ExportFormat::WavPack(o) => {
            export::intfmt::export_wavpack_with_progress(path, channels, sample_rate, o, tags, progress)
        }
        export::ExportFormat::Mp3(o) => {
            export::mp3::export_mp3_with_progress(path, channels, sample_rate, o, tags, progress)
        }
        export::ExportFormat::Vorbis(o) => {
            export::ogg_opus::export_vorbis_with_progress(path, channels, sample_rate, o, tags, progress)
        }
        export::ExportFormat::Opus(o) => {
            export::ogg_opus::export_opus_with_progress(path, channels, sample_rate, o, tags, progress)
        }
    }
}

#[cfg(feature = "device")]
pub mod device {
    use super::IoError;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    };

    /// Returns the names of audio hosts available through cpal.
    pub fn hosts() -> Vec<String> {
        cpal::available_hosts()
            .into_iter()
            .map(|host| host.name().to_owned())
            .collect()
    }

    fn resolve_host(name: &str) -> cpal::Host {
        if !name.is_empty() {
            if let Some(id) = cpal::available_hosts()
                .into_iter()
                .find(|host| host.name() == name)
            {
                if let Ok(host) = cpal::host_from_id(id) {
                    return host;
                }
            }
        }
        cpal::default_host()
    }

    /// Returns output device names for a host, or the default host when `host` is empty or unavailable.
    pub fn output_devices_on(host: &str) -> Vec<String> {
        resolve_host(host)
            .output_devices()
            .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
            .unwrap_or_default()
    }

    /// Returns input device names for a host, or the default host when `host` is empty or unavailable.
    pub fn input_devices_on(host: &str) -> Vec<String> {
        resolve_host(host)
            .input_devices()
            .map(|devices| devices.filter_map(|device| device.name().ok()).collect())
            .unwrap_or_default()
    }

    /// Returns output devices on the default host.
    pub fn output_devices() -> Vec<String> {
        output_devices_on("")
    }

    /// Returns input devices on the default host.
    pub fn input_devices() -> Vec<String> {
        input_devices_on("")
    }

    /// Selects an audio host and device. Empty names select the system default;
    /// missing saved host or device names fall back to the corresponding default.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub struct DeviceChoice {
        /// Host name, or empty to use the default host.
        pub host: String,
        /// Device name, or empty to use the default device.
        pub device: String,
    }

    fn resolve_output_device(choice: &DeviceChoice) -> Result<cpal::Device, IoError> {
        let host = resolve_host(&choice.host);
        let selected = if choice.device.is_empty() {
            None
        } else {
            host.output_devices()
                .ok()
                .and_then(|devices| {
                    devices.into_iter().find(|device| {
                        device
                            .name()
                            .map(|name| name == choice.device)
                            .unwrap_or(false)
                    })
                })
        };
        selected
            .or_else(|| host.default_output_device())
            .ok_or_else(|| IoError::Device("no default output device".to_owned()))
    }

    fn resolve_input_device(choice: &DeviceChoice) -> Result<cpal::Device, IoError> {
        let host = resolve_host(&choice.host);
        let selected = if choice.device.is_empty() {
            None
        } else {
            host.input_devices()
                .ok()
                .and_then(|devices| {
                    devices.into_iter().find(|device| {
                        device
                            .name()
                            .map(|name| name == choice.device)
                            .unwrap_or(false)
                    })
                })
        };
        selected
            .or_else(|| host.default_input_device())
            .ok_or_else(|| IoError::Device("no default input device".to_owned()))
    }

    fn choose_config(
        default: cpal::SupportedStreamConfig,
        ranges: Vec<cpal::SupportedStreamConfigRange>,
        requested_channels: Option<u16>,
    ) -> cpal::SupportedStreamConfig {
        let Some(requested_channels) = requested_channels else {
            return default;
        };
        let sample_rate = default.sample_rate();
        let supports_rate = |range: &cpal::SupportedStreamConfigRange| {
            range.min_sample_rate() <= sample_rate && range.max_sample_rate() >= sample_rate
        };
        let max_channels = ranges
            .iter()
            .filter(|range| supports_rate(range))
            .map(|range| range.channels())
            .max()
            .unwrap_or(default.channels());
        let target_channels = requested_channels.min(max_channels);
        ranges
            .into_iter()
            .find(|range| {
                range.channels() == target_channels
                    && supports_rate(range)
                    && range.sample_format() == cpal::SampleFormat::F32
            })
            .map(|range| range.with_sample_rate(sample_rate))
            .unwrap_or(default)
    }

    fn choose_input_config(
        device: &cpal::Device,
        requested_channels: Option<u16>,
    ) -> Result<cpal::SupportedStreamConfig, IoError> {
        let default = device
            .default_input_config()
            .map_err(|error| IoError::Device(error.to_string()))?;
        let ranges = device
            .supported_input_configs()
            .map(|configs| configs.collect())
            .unwrap_or_default();
        Ok(choose_config(default, ranges, requested_channels))
    }

    fn choose_output_config(
        device: &cpal::Device,
        requested_channels: Option<u16>,
    ) -> Result<cpal::SupportedStreamConfig, IoError> {
        let default = device
            .default_output_config()
            .map_err(|error| IoError::Device(error.to_string()))?;
        let ranges = device
            .supported_output_configs()
            .map(|configs| configs.collect())
            .unwrap_or_default();
        Ok(choose_config(default, ranges, requested_channels))
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
            Self::start_with_effects_on(
                &DeviceChoice::default(),
                tracks,
                track_fx,
                master_fx,
                project_rate,
                master_gain,
                start,
                speed,
                loop_range,
            )
        }

        /// Starts playback on the selected host and output device.
        #[allow(clippy::too_many_arguments)]
        pub fn start_with_effects_on(
            choice: &DeviceChoice,
            tracks: Vec<crate::engine::MixTrack>,
            track_fx: Vec<Option<au_effects::stack::EffectStack>>,
            master_fx: Option<au_effects::stack::EffectStack>,
            project_rate: u32,
            master_gain: f32,
            start: usize,
            speed: f64,
            loop_range: Option<(usize, usize)>,
        ) -> Result<Player, IoError> {
            let device = resolve_output_device(choice)?;
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
        /// Software playthrough output (Recording > Audible input monitoring).
        _monitor: Option<cpal::Stream>,
        rx: rtrb::Consumer<f32>,
        peak: Arc<std::sync::atomic::AtomicU32>,
        dropped_samples: Arc<AtomicU64>,
        /// Software input gain applied in the capture callback (Audacity's emulated
        /// recording volume when no OS mixer is available). 1.0 = untouched.
        input_gain: crate::input_gain::InputGain,
        /// Interleaved channel count of the capture.
        pub channels: u16,
        /// Capture sample rate.
        pub sample_rate: u32,
    }

    impl Recorder {
        /// Starts capture on the default host and device. `channels` is the
        /// device's default channel count.
        pub fn start(kind: InputKind) -> Result<Recorder, IoError> {
            Self::start_on(kind, &DeviceChoice::default(), None)
        }

        /// Starts capture from the chosen host and device. `channels` requests
        /// a channel count when supported; devices with fewer channels use
        /// their highest available count.
        pub fn start_on(
            kind: InputKind,
            choice: &DeviceChoice,
            channels: Option<u16>,
        ) -> Result<Recorder, IoError> {
            Self::start_monitored(kind, choice, channels, None)
        }

        /// Like [`Recorder::start_on`], and with `playthrough` also plays the input on that
        /// output device (Audacity's software playthrough, /AudioIO/SWPlaythrough). The monitor
        /// is skipped when the output cannot run at the capture rate.
        pub fn start_monitored(
            kind: InputKind,
            choice: &DeviceChoice,
            channels: Option<u16>,
            playthrough: Option<&DeviceChoice>,
        ) -> Result<Recorder, IoError> {
            let (device, supported) = match kind {
                InputKind::Default => {
                    let device = resolve_input_device(choice)?;
                    let config = choose_input_config(&device, channels)?;
                    (device, config)
                }
                InputKind::Loopback => {
                    if !cfg!(target_os = "windows") {
                        return Err(IoError::Unsupported(
                            "system-audio loopback is only native on Windows (WASAPI); on Linux pick a \"Monitor of\" input, on macOS use a virtual device".to_owned(),
                        ));
                    }
                    let device = resolve_output_device(choice)?;
                    let config = choose_output_config(&device, channels)?;
                    (device, config)
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
            let dropped_samples = Arc::new(AtomicU64::new(0));
            let dropped_samples_cb = dropped_samples.clone();
            let in_channels = channels as usize;
            let input_gain = crate::input_gain::InputGain::default();
            let gain_cb = input_gain.clone();
            let mut scratch: Vec<f32> = Vec::with_capacity(sample_rate as usize * in_channels);
            let (monitor, mut monitor_tx) = match playthrough.map(|c| start_playthrough(c, sample_rate, in_channels)) {
                Some(Ok((stream, tx))) => (Some(stream), Some(tx)),
                _ => (None, None),
            };
            let stream = device
                .build_input_stream(
                    &config,
                    move |raw: &[f32], _| {
                        let g = gain_cb.get();
                        let input: &[f32] = if g == 1.0 {
                            raw
                        } else {
                            scratch.clear();
                            scratch.extend_from_slice(raw);
                            crate::input_gain::apply_input_gain(&mut scratch, g);
                            &scratch
                        };
                        let mut p = f32::from_bits(peak_cb.load(Ordering::Relaxed));
                        if let Some(tx) = monitor_tx.as_mut() {
                            for &s in input {
                                let _ = tx.push(s);
                            }
                        }
                        for &s in input {
                            p = p.max(s.abs());
                            if tx.push(s).is_err() {
                                dropped_samples_cb.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        peak_cb.store(p.to_bits(), Ordering::Relaxed);
                    },
                    |error| eprintln!("audio input error: {error}"),
                    None,
                )
                .map_err(|error| IoError::Device(error.to_string()))?;
            stream.play().map_err(|error| IoError::Device(error.to_string()))?;
            Ok(Recorder { _stream: stream, _monitor: monitor, rx, peak, dropped_samples, input_gain, channels, sample_rate })
        }

        /// Sets the software recording gain (0..=1) applied to captured samples,
        /// meters and playthrough. Use only when no OS mixer handles the volume.
        pub fn set_input_gain(&self, gain: f32) {
            self.input_gain.set(gain);
        }

        /// Handle to this capture's software gain (shareable with a [`crate::input_gain::RecordVolume`]).
        pub fn input_gain(&self) -> crate::input_gain::InputGain {
            self.input_gain.clone()
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

        /// Returns the cumulative number of input samples dropped because the ring was full.
        pub fn dropped_samples(&self) -> u64 {
            self.dropped_samples.load(Ordering::Relaxed)
        }
    }

    /// Output stream for software playthrough: plays interleaved input samples (`in_channels`
    /// per frame) from a ring, mono input to every output channel, extra input channels dropped.
    fn start_playthrough(
        choice: &DeviceChoice,
        rate: u32,
        in_channels: usize,
    ) -> Result<(cpal::Stream, rtrb::Producer<f32>), IoError> {
        let device = resolve_output_device(choice)?;
        let supported = device
            .supported_output_configs()
            .map_err(|error| IoError::Device(error.to_string()))?
            .filter(|c| c.sample_format() == cpal::SampleFormat::F32)
            .find(|c| c.min_sample_rate().0 <= rate && rate <= c.max_sample_rate().0)
            .ok_or_else(|| IoError::Unsupported("no output config at the capture rate".to_owned()))?
            .with_sample_rate(cpal::SampleRate(rate));
        let out_channels = supported.channels() as usize;
        let config: cpal::StreamConfig = supported.into();
        // ~0.5 s ring; anything older is dropped rather than building latency.
        let (tx, mut rx) = rtrb::RingBuffer::<f32>::new((rate as usize / 2).max(1024) * in_channels.max(1));
        let in_ch = in_channels.max(1);
        let stream = device
            .build_output_stream(
                &config,
                move |out: &mut [f32], _| {
                    out.fill(0.0);
                    for frame in out.chunks_mut(out_channels.max(1)) {
                        if rx.slots() < in_ch {
                            break;
                        }
                        let mut v = [0.0f32; 2];
                        for c in 0..in_ch {
                            let s = rx.pop().unwrap_or(0.0);
                            if c < 2 {
                                v[c] = s;
                            }
                        }
                        if in_ch == 1 {
                            v[1] = v[0];
                        }
                        for (c, o) in frame.iter_mut().enumerate() {
                            *o = v[c.min(1)];
                        }
                    }
                },
                |error| eprintln!("audio playthrough error: {error}"),
                None,
            )
            .map_err(|error| IoError::Device(error.to_string()))?;
        stream.play().map_err(|error| IoError::Device(error.to_string()))?;
        Ok((stream, tx))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn hosts_and_device_listings_do_not_panic() {
            for host in hosts() {
                let _ = input_devices_on(&host);
                let _ = output_devices_on(&host);
            }
            let _ = input_devices();
            let _ = output_devices();
        }

        #[test]
        fn device_choice_defaults_to_system_defaults() {
            assert_eq!(
                DeviceChoice::default(),
                DeviceChoice {
                    host: String::new(),
                    device: String::new(),
                }
            );
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

    #[test]
    fn wav_import_progress_and_samples() {
        let path = path("wav");
        let original = sample_buffer();
        export_wav(&path, &original, 32).unwrap();
        let expected = import_file(&path).unwrap();
        let mut updates = Vec::new();
        let mut callback = |done, total| {
            updates.push((done, total));
            true
        };
        let imported = import_file_with_progress(&path, &mut callback).unwrap();
        assert!(!updates.is_empty());
        assert!(updates.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        let &(done, total) = updates.last().unwrap();
        assert_eq!(done, total);
        assert_eq!(imported, expected);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn wav_import_progress_can_cancel() {
        let path = path("wav");
        export_wav(&path, &sample_buffer(), 32).unwrap();
        let mut callback = |_, _| false;
        assert!(matches!(
            import_file_with_progress(&path, &mut callback),
            Err(IoError::Cancelled)
        ));
        let _ = std::fs::remove_file(path);
    }

    #[cfg(feature = "device")]
    #[test]
    fn listing_output_devices_does_not_panic() {
        let _ = crate::device::output_devices();
    }
}
