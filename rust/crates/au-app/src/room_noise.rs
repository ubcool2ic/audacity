//! Room-noise profile capture, persistence, and live noise reduction for the app.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use au_core::AudioBuffer;
use au_effects::fx::fix_noisereduction::{
    NoiseProfile, NoiseReduction, StreamingNoiseReducer,
};

/// Duration of a room-noise profile capture, in seconds.
pub const CAPTURE_SECS: f64 = 10.0;

/// Settings for live noise suppression.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveSettings {
    /// Whether live noise suppression is enabled.
    pub enabled: bool,
    /// Whether live noise suppression is bypassed.
    pub bypass: bool,
    /// Noise reduction amount in decibels.
    pub reduction_db: f64,
    /// Noise classification sensitivity.
    pub sensitivity: f64,
    /// Frequency smoothing width in bands.
    pub smoothing: f64,
}

impl Default for LiveSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            bypass: false,
            reduction_db: 12.0,
            sensitivity: 6.0,
            smoothing: 3.0,
        }
    }
}

/// Result of feeding samples to an active capture.
#[derive(Clone, Debug, PartialEq)]
pub enum RoomNoiseEvent {
    /// No capture event occurred.
    None,
    /// A profile was captured successfully for the named device.
    CaptureFinished(String),
    /// A capture failed, with a message explaining why.
    CaptureFailed(String),
}

struct CaptureState {
    device: String,
    key: String,
    rate: u32,
    channels: usize,
    target_frames: usize,
    frames: usize,
    samples: Vec<Vec<f32>>,
}

struct LiveTake {
    reducer: StreamingNoiseReducer,
    rate: u32,
    channels: usize,
    latency: usize,
    input_frames: usize,
    processed: Vec<f32>,
    flush_input: Vec<f32>,
    flush_output: Vec<f32>,
    bypass_at_start: bool,
}

/// Room-noise profiles and capture/live-processing state.
pub struct RoomNoise {
    /// Profiles indexed by sanitized device key.
    pub profiles: BTreeMap<String, NoiseProfile>,
    /// Settings used for live noise suppression.
    pub live: LiveSettings,
    capture: Option<CaptureState>,
    live_take: Option<LiveTake>,
}

impl RoomNoise {
    /// Creates room-noise state and loads profiles from the user's profile directory.
    pub fn new() -> Self {
        let mut room = Self {
            profiles: BTreeMap::new(),
            live: LiveSettings::default(),
            capture: None,
            live_take: None,
        };
        room.load_profiles_from_disk();
        room
    }

    /// Returns the directory used to store per-device room-noise profiles.
    pub fn user_profile_dir() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(|home| PathBuf::from(home).join(".config"))
            })
            .unwrap_or_else(|| PathBuf::from(".config"));

        base.join("audacity-rs").join("noise-profiles")
    }

    /// Converts a device name to a file-safe profile key.
    ///
    /// Empty names map to `"default"`. ASCII letters, digits, hyphens, and
    /// underscores are preserved; runs of other characters become one underscore.
    pub fn device_key(name: &str) -> String {
        let mut key = String::new();
        let mut in_replacement = false;

        for ch in name.trim().chars() {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                key.push(ch);
                in_replacement = false;
            } else if !in_replacement {
                key.push('_');
                in_replacement = true;
            }
        }

        let key = key.trim_matches('_').to_string();
        if key.is_empty() {
            "default".to_string()
        } else {
            key
        }
    }

    /// Starts capturing noise from a device.
    ///
    /// A new capture replaces any capture already in progress. Channel and rate
    /// values of zero are treated as one to keep capture state usable.
    pub fn start_capture(&mut self, device: &str, rate: u32, channels: usize) {
        let channels = channels.max(1);
        let rate = rate.max(1);
        let target_frames = (CAPTURE_SECS * rate as f64).round() as usize;

        self.capture = Some(CaptureState {
            device: device.to_string(),
            key: Self::device_key(device),
            rate,
            channels,
            target_frames,
            frames: 0,
            samples: (0..channels)
                .map(|_| Vec::with_capacity(target_frames))
                .collect(),
        });
    }

    /// Returns whether a noise capture is in progress.
    pub fn is_capturing(&self) -> bool {
        self.capture.is_some()
    }

    /// Returns capture progress in the range `0.0..=1.0`.
    pub fn capture_progress(&self) -> f32 {
        self.capture
            .as_ref()
            .map(|capture| {
                if capture.target_frames == 0 {
                    1.0
                } else {
                    (capture.frames as f64 / capture.target_frames as f64)
                        .clamp(0.0, 1.0) as f32
                }
            })
            .unwrap_or(0.0)
    }

    /// Cancels any capture in progress.
    pub fn cancel_capture(&mut self) {
        self.capture = None;
    }

    /// Feeds interleaved samples to an active capture.
    ///
    /// Once ten seconds of audio have been collected, the profile is learned,
    /// stored in memory, and written to the user's profile directory.
    pub fn feed(&mut self, interleaved: &[f32]) -> RoomNoiseEvent {
        let mut completed = false;

        if let Some(capture) = self.capture.as_mut() {
            let available_frames = interleaved.len() / capture.channels;
            let remaining = capture.target_frames.saturating_sub(capture.frames);
            let frames_to_copy = available_frames.min(remaining);

            for frame in 0..frames_to_copy {
                let base = frame * capture.channels;
                for channel in 0..capture.channels {
                    capture.samples[channel].push(interleaved[base + channel]);
                }
            }

            capture.frames += frames_to_copy;
            completed = capture.frames >= capture.target_frames;
        }

        if !completed {
            return RoomNoiseEvent::None;
        }

        let capture = match self.capture.take() {
            Some(capture) => capture,
            None => return RoomNoiseEvent::None,
        };

        let audio = AudioBuffer::from_channels(capture.rate, capture.samples);
        let mut effect = NoiseReduction::default();
        effect.window_size = 8; // 2048 samples
        effect.steps_per_window = 1; // 4 analysis steps

        if let Err(error) = effect.learn_profile(&audio) {
            return RoomNoiseEvent::CaptureFailed(error.to_string());
        }

        let profile = match effect.profile().cloned() {
            Some(profile) => profile,
            None => {
                return RoomNoiseEvent::CaptureFailed(
                    "Noise reduction did not produce a noise profile.".to_string(),
                )
            }
        };

        let directory = Self::user_profile_dir();
        if let Err(error) = fs::create_dir_all(&directory) {
            return RoomNoiseEvent::CaptureFailed(au_core::i18n::tr_format("Could not create noise-profile directory: %s", &[&error]));
        }

        let path = directory.join(format!("{}.aunp", capture.key));
        if let Err(error) = fs::write(path, profile.to_bytes()) {
            return RoomNoiseEvent::CaptureFailed(au_core::i18n::tr_format("Could not write noise profile: %s", &[&error]));
        }

        self.profiles.insert(capture.key, profile);
        RoomNoiseEvent::CaptureFinished(capture.device)
    }

    /// Returns a profile for the named device, if one is available.
    pub fn profile_for(&self, device: &str) -> Option<&NoiseProfile> {
        self.profiles.get(&Self::device_key(device))
    }

    /// Returns all profiles as project-storable `(device key, bytes)` pairs.
    pub fn project_blobs(&self) -> Vec<(String, Vec<u8>)> {
        self.profiles
            .iter()
            .map(|(key, profile)| (key.clone(), profile.to_bytes()))
            .collect()
    }

    /// Loads project-stored profiles, overriding profiles with matching keys.
    ///
    /// Invalid profile blobs are ignored.
    pub fn load_project_blobs(&mut self, blobs: &[(String, Vec<u8>)]) {
        for (key, bytes) in blobs {
            if let Ok(profile) = NoiseProfile::from_bytes(bytes) {
                self.profiles
                    .insert(Self::device_key(key), profile);
            }
        }
    }

    /// Creates a noise-reduction effect with this device's stored profile.
    pub fn offline_effect(&self, device: &str) -> Option<NoiseReduction> {
        let profile = self.profile_for(device)?.clone();
        let mut effect = NoiseReduction::default();
        effect.set_profile(profile);
        Some(effect)
    }

    /// Starts a live take if live suppression is enabled and a matching profile exists.
    ///
    /// The streaming reducer and its latency-flush buffers are allocated here so
    /// processing callbacks can operate on prepared state.
    pub fn begin_live_take(&mut self, device: &str, rate: u32, channels: usize) -> bool {
        self.live_take = None;

        if !self.live.enabled || rate == 0 || channels == 0 {
            return false;
        }

        let profile = match self.profile_for(device) {
            Some(profile) if profile.rate == rate as f64 => profile.clone(),
            _ => return false,
        };

        let mut settings = NoiseReduction::default();
        settings.noise_reduction = self.live.reduction_db;
        settings.sensitivity = self.live.sensitivity;
        settings.frequency_smoothing = self.live.smoothing;

        let mut reducer = StreamingNoiseReducer::new(&profile, &settings, channels);
        reducer.set_bypass(self.live.bypass);
        let latency = reducer.latency();
        let flush_samples = latency.saturating_mul(channels);

        self.live_take = Some(LiveTake {
            reducer,
            rate,
            channels,
            latency,
            input_frames: 0,
            processed: Vec::new(),
            flush_input: vec![0.0; flush_samples],
            flush_output: vec![0.0; flush_samples],
            bypass_at_start: self.live.bypass,
        });

        true
    }

    /// Processes and appends one interleaved block of live audio.
    ///
    /// Input samples are not modified. If no live take is active, the input is
    /// copied to `output`.
    pub fn process_live(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let Some(take) = self.live_take.as_mut() else {
            output.extend_from_slice(input);
            return;
        };

        take.reducer.set_bypass(self.live.bypass);
        let start = output.len();
        output.resize(start + input.len(), 0.0);
        take.reducer
            .process_block(input, &mut output[start..start + input.len()]);

        take.input_frames += input.len() / take.channels;
        take.processed
            .extend_from_slice(&output[start..start + input.len()]);
    }

    /// Flushes live-processing latency and returns the time-aligned denoised take.
    ///
    /// The returned audio has the same number of frames as the input take.
    pub fn end_live_take(&mut self) -> Option<AudioBuffer> {
        let mut take = self.live_take.take()?;

        if !take.flush_input.is_empty() {
            take.reducer
                .process_block(&take.flush_input, &mut take.flush_output);
            take.processed.extend_from_slice(&take.flush_output);
        }

        let channel_count = take.channels;
        let expected_samples = take.input_frames.saturating_mul(channel_count);
        let start = if take.bypass_at_start {
            0
        } else {
            take.latency.saturating_mul(channel_count)
        };
        let end = start.saturating_add(expected_samples);

        let mut channels = vec![Vec::with_capacity(take.input_frames); channel_count];
        for frame in 0..take.input_frames {
            for channel in 0..channel_count {
                let index = start + frame * channel_count + channel;
                let sample = if index < end && index < take.processed.len() {
                    take.processed[index]
                } else {
                    0.0
                };
                channels[channel].push(sample);
            }
        }

        Some(AudioBuffer::from_channels(take.rate, channels))
    }

    /// Draws room-profile capture and live-suppression controls.
    ///
    /// Returns `true` when the user presses “Capture Noise Profile (10 s)”.
    /// The caller should then call [`RoomNoise::start_capture`] with the active
    /// device's rate and channel count.
    pub fn ui(&mut self, ui: &mut eframe::egui::Ui, device: &str) -> bool {
        let mut capture_pressed = false;

        if let Some(profile) = self.profile_for(device) {
            ui.label(au_core::i18n::tr_format("Noise profile: %d Hz, %d-sample windows, %d windows", &[&(profile.rate as u32), &profile.window_size, &profile.total_windows]));
        } else {
            ui.label("No noise profile for this device.");
        }

        if self.is_capturing() {
            ui.add(
                eframe::egui::ProgressBar::new(self.capture_progress())
                    .text("Capturing room noise"),
            );
            if ui.button("Cancel capture").clicked() {
                self.cancel_capture();
            }
        } else {
            capture_pressed = ui
                .button("Capture Noise Profile (10 s)")
                .clicked();
        }

        ui.checkbox(&mut self.live.enabled, "Live noise suppression");
        ui.checkbox(&mut self.live.bypass, "Bypass");
        ui.add(
            eframe::egui::Slider::new(&mut self.live.reduction_db, 0.0..=48.0)
                .text("Reduction (dB)"),
        );
        ui.add(
            eframe::egui::Slider::new(&mut self.live.sensitivity, 0.01..=24.0)
                .text("Sensitivity"),
        );
        ui.add(
            eframe::egui::Slider::new(&mut self.live.smoothing, 0.0..=12.0)
                .text("Smoothing (bands)"),
        );

        capture_pressed
    }

    fn load_profiles_from_disk(&mut self) {
        let directory = Self::user_profile_dir();
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("aunp") {
                continue;
            }

            let Some(key) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            let Ok(bytes) = fs::read(&path) else {
                continue;
            };
            if let Ok(profile) = NoiseProfile::from_bytes(&bytes) {
                self.profiles.insert(key.to_string(), profile);
            }
        }
    }
}

impl Default for RoomNoise {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au_effects::core::{BuiltinEffect, EffectContext};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_DIR_ID: AtomicUsize = AtomicUsize::new(0);

    fn set_test_config_dir() -> PathBuf {
        let id = TEST_DIR_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "audacity-rs-room-noise-{}-{id}",
            std::process::id()
        ));
        std::env::set_var("XDG_CONFIG_HOME", &path);
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn noise_samples(length: usize, seed: &mut u32, amplitude: f32) -> Vec<f32> {
        (0..length)
            .map(|_| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 17;
                *seed ^= *seed << 5;
                let unit = (*seed as f32 / u32::MAX as f32) * 2.0 - 1.0;
                unit * amplitude
            })
            .collect()
    }

    fn learned_profile(rate: u32) -> NoiseProfile {
        let mut seed = 0x1234_5678;
        let samples = noise_samples(rate as usize * 2, &mut seed, 0.08);
        let mut effect = NoiseReduction::default();
        effect.window_size = 8;
        effect.steps_per_window = 1;
        effect
            .learn_profile(&AudioBuffer::from_channels(rate, vec![samples]))
            .unwrap();
        effect.profile().unwrap().clone()
    }

    #[test]
    fn captures_and_writes_a_ten_second_profile() {
        let config = set_test_config_dir();
        let mut room = RoomNoise::new();
        let rate = 8_000;
        room.start_capture("Test Microphone", rate, 1);

        let mut seed = 0x8765_4321;
        let total = rate as usize * CAPTURE_SECS as usize;
        let mut sent = 0;
        while sent < total {
            let count = (total - sent).min(4096);
            let samples = noise_samples(count, &mut seed, 0.1);
            let event = room.feed(&samples);
            sent += count;
            if sent == total {
                assert_eq!(
                    event,
                    RoomNoiseEvent::CaptureFinished("Test Microphone".to_string())
                );
            }
        }

        let key = RoomNoise::device_key("Test Microphone");
        assert!(room.profiles.contains_key(&key));
        assert!(RoomNoise::user_profile_dir()
            .join(format!("{key}.aunp"))
            .is_file());

        let _ = fs::remove_dir_all(config);
    }

    #[test]
    fn offline_effect_improves_tone_noise_snr() {
        fn lcg_noise(length: usize, seed: &mut u32, sigma: f32) -> Vec<f32> {
            (0..length)
                .map(|_| {
                    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let unit = (*seed as f64 / u32::MAX as f64) * 2.0 - 1.0;
                    (unit * 3.0_f64.sqrt() * sigma as f64) as f32
                })
                .collect()
        }

        fn spectral_snr(samples: &[f32], start: usize, length: usize, rate: u32) -> f64 {
            let segment = &samples[start..start + length];
            let mut total_power = 0.0_f64;
            let mut real = 0.0_f64;
            let mut imag = 0.0_f64;

            for (index, &sample) in segment.iter().enumerate() {
                let value = sample as f64;
                total_power += value * value;
                let phase = std::f64::consts::TAU * 1_000.0 * index as f64 / rate as f64;
                real += value * phase.cos();
                imag -= value * phase.sin();
            }

            let tone_power = 2.0 * (real * real + imag * imag) / (length as f64 * length as f64);
            let total_power = total_power / length as f64;
            let noise_power = (total_power - tone_power).max(f64::MIN_POSITIVE);
            10.0 * (tone_power / noise_power).log10()
        }

        let rate = 8_000;
        let mut profile_seed = 0x1357_2468;
        let profile_noise = lcg_noise(rate as usize * 2, &mut profile_seed, 0.05);
        let mut learner = NoiseReduction::default();
        learner.window_size = 8;
        learner.steps_per_window = 1;
        learner
            .learn_profile(&AudioBuffer::from_channels(rate, vec![profile_noise]))
            .unwrap();

        let mut effect = NoiseReduction::default();
        effect.set_profile(learner.profile().unwrap().clone());
        effect.set("NoiseReduction", 24.0);
        effect.set("Sensitivity", 6.0);
        effect.set("FrequencySmoothing", 3.0);

        let frames = rate as usize;
        let mut seed = 0x2468_1357;
        let noise = lcg_noise(frames, &mut seed, 0.05);
        let mut input = Vec::with_capacity(frames);
        for (index, noise_sample) in noise.into_iter().enumerate() {
            let tone = (std::f32::consts::TAU * 1_000.0 * index as f32 / rate as f32).sin()
                * 0.3;
            input.push(tone + noise_sample);
        }

        let mut audio = AudioBuffer::from_channels(rate, vec![input.clone()]);
        effect
            .process(&mut audio, &mut EffectContext::default())
            .unwrap();

        let segment_length = 4096;
        let start = (frames - segment_length) / 2;
        let output = &audio.channels[0];
        assert!(output.len() >= start + segment_length);
        let input_snr = spectral_snr(&input, start, segment_length, rate);
        let output_snr = spectral_snr(output, start, segment_length, rate);
        let improvement = output_snr - input_snr;

        assert!(
            improvement > 6.0, // Audacity-exact NR at 24 dB gives ~+7..8 dB spectral SNR here (fx_tool at 44.1 kHz: +7.2 dB)
            "SNR improvement was {improvement:.2} dB (input {input_snr:.2} dB, output {output_snr:.2} dB)"
        );
    }

    #[test]
    fn live_take_matches_offline_processing() {
        let rate = 8_000;
        let profile = learned_profile(rate);
        let mut room = RoomNoise::new();
        room.profiles
            .insert(RoomNoise::device_key("Test"), profile.clone());
        room.live.enabled = true;
        room.live.reduction_db = 12.0;
        room.live.sensitivity = 6.0;
        room.live.smoothing = 3.0;
        assert!(room.begin_live_take("Test", rate, 1));

        let mut seed = 0x2468_1357;
        let mut input = noise_samples(rate as usize, &mut seed, 0.05);
        for (index, sample) in input.iter_mut().enumerate() {
            *sample += (std::f32::consts::TAU * 440.0 * index as f32 / rate as f32).sin()
                * 0.25;
        }

        let mut live_output = Vec::new();
        for block in input.chunks(512) {
            let mut output = Vec::new();
            room.process_live(block, &mut output);
            live_output.extend_from_slice(&output);
        }
        let live_audio = room.end_live_take().unwrap();

        let mut offline_effect = NoiseReduction::default();
        offline_effect.set_profile(profile);
        offline_effect.set("NoiseReduction", room.live.reduction_db);
        offline_effect.set("Sensitivity", room.live.sensitivity);
        offline_effect.set("FrequencySmoothing", room.live.smoothing);
        let mut offline_audio = AudioBuffer::from_channels(rate, vec![input]);
        offline_effect
            .process(&mut offline_audio, &mut EffectContext::default())
            .unwrap();

        assert_eq!(live_audio.len(), offline_audio.len());
        assert_eq!(live_audio.channels[0].len(), offline_audio.channels[0].len());
        assert!(live_output.iter().all(|sample| sample.is_finite()));
        let max_difference = live_audio.channels[0]
            .iter()
            .zip(offline_audio.channels[0].iter())
            .map(|(live, offline)| (live - offline).abs())
            .fold(0.0_f32, f32::max);
        assert!(max_difference < 1.0e-6, "maximum difference: {max_difference}");
    }

    #[test]
    fn project_profiles_round_trip() {
        let profile = learned_profile(8_000);
        let mut source = RoomNoise::new();
        source
            .profiles
            .insert(RoomNoise::device_key("USB Mic"), profile.clone());

        let blobs = source.project_blobs();
        let mut destination = RoomNoise::new();
        destination.load_project_blobs(&blobs);

        assert_eq!(
            destination.profile_for("USB Mic"),
            Some(&profile)
        );
        assert_eq!(destination.project_blobs(), blobs);
    }
}
