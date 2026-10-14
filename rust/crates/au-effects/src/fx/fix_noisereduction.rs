//! Audacity 3.7.3 two-pass noise reduction.

use crate::core::*;
use au_core::AudioBuffer;

/// Statistics learned from a noise-only selection.
#[derive(Clone, Debug, PartialEq)]
pub struct NoiseProfile {
    /// Sample rate used to learn the profile.
    pub rate: f64,
    /// FFT window size in samples.
    pub window_size: usize,
    /// Number of analysis steps per window.
    pub steps_per_window: usize,
    /// Window type index.
    pub window_types: usize,
    /// Number of windows represented by the profile.
    pub total_windows: usize,
    /// Number of windows in the current track's accumulator.
    pub track_windows: usize,
    /// Current track's accumulated power spectrum.
    pub sums: Vec<f32>,
    /// Mean power spectrum across completed tracks.
    pub means: Vec<f32>,
}

impl NoiseProfile {
    /// Serializes the profile to the versioned `AUNP1` little-endian format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.sums.len() * 8);
        out.extend_from_slice(b"AUNP1");
        out.extend_from_slice(&self.rate.to_le_bytes());
        put_u64(&mut out, self.window_size as u64);
        put_u64(&mut out, self.steps_per_window as u64);
        put_u64(&mut out, self.window_types as u64);
        put_u64(&mut out, self.total_windows as u64);
        put_u64(&mut out, self.track_windows as u64);
        put_u64(&mut out, self.sums.len() as u64);
        for value in &self.sums {
            out.extend_from_slice(&value.to_le_bytes());
        }
        put_u64(&mut out, self.means.len() as u64);
        for value in &self.means {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    /// Deserializes a profile from the versioned `AUNP1` little-endian format.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = ByteReader { bytes, pos: 0 };
        if reader.take(5)? != b"AUNP1" {
            return Err("Invalid noise profile magic or version.".into());
        }

        let rate = f64::from_le_bytes(reader.array::<8>()?);
        let window_size = reader.usize()?;
        let steps_per_window = reader.usize()?;
        let window_types = reader.usize()?;
        let total_windows = reader.usize()?;
        let track_windows = reader.usize()?;
        let sums_len = reader.usize()?;
        if sums_len > 1_048_577 {
            return Err("Noise profile is too large.".into());
        }
        let mut sums = Vec::with_capacity(sums_len);
        for _ in 0..sums_len {
            sums.push(f32::from_le_bytes(reader.array::<4>()?));
        }
        let means_len = reader.usize()?;
        if means_len > 1_048_577 {
            return Err("Noise profile is too large.".into());
        }
        let mut means = Vec::with_capacity(means_len);
        for _ in 0..means_len {
            means.push(f32::from_le_bytes(reader.array::<4>()?));
        }
        if reader.pos != bytes.len() {
            return Err("Trailing data in noise profile.".into());
        }
        if !rate.is_finite()
            || rate <= 0.0
            || window_size < 8
            || !window_size.is_power_of_two()
            || steps_per_window == 0
            || !steps_per_window.is_power_of_two()
            || steps_per_window > window_size
            || window_types >= WINDOW_CHOICES.len()
            || sums.len() != window_size / 2 + 1
            || means.len() != window_size / 2 + 1
        {
            return Err("Invalid noise profile data.".into());
        }

        Ok(Self {
            rate,
            window_size,
            steps_per_window,
            window_types,
            total_windows,
            track_windows,
            sums,
            means,
        })
    }
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> ByteReader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| "Invalid noise profile length.".to_string())?;
        let result = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| "Truncated noise profile.".to_string())?;
        self.pos = end;
        Ok(result)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        self.take(N)?
            .try_into()
            .map_err(|_| "Truncated noise profile.".to_string())
    }

    fn usize(&mut self) -> Result<usize, String> {
        let value = u64::from_le_bytes(self.array::<8>()?);
        usize::try_from(value).map_err(|_| "Invalid noise profile length.".to_string())
    }
}

/// Parameters for noise reduction.
#[derive(Clone, Debug)]
pub struct NoiseReduction {
    /// Reduction amount in dB.
    pub noise_reduction: f64,
    /// Noise-classification sensitivity.
    pub sensitivity: f64,
    /// Frequency smoothing width in bins.
    pub frequency_smoothing: f64,
    /// Output mode: reduce, isolate, or residue.
    pub noise_reduction_choice: usize,
    /// Analysis and synthesis window type.
    pub window_types: usize,
    /// FFT window-size choice, where choice 8 is 2048 samples.
    pub window_size: usize,
    /// Steps-per-window choice, where choice 1 is four steps.
    pub steps_per_window: usize,
    /// Classification method: median, second greatest, or old.
    pub method: usize,
    /// Sensitivity for the old classification method.
    pub old_sensitivity: f64,
    profile: Option<NoiseProfile>,
}

impl Default for NoiseReduction {
    fn default() -> Self {
        Self {
            noise_reduction: 6.0,
            sensitivity: 6.0,
            frequency_smoothing: 6.0,
            noise_reduction_choice: 0,
            window_types: 2,
            window_size: 8,
            steps_per_window: 1,
            method: 1,
            old_sensitivity: 0.0,
            profile: None,
        }
    }
}

const OUTPUT_CHOICES: &[&str] = &["Reduce", "Isolate", "Residue"];
const WINDOW_CHOICES: &[&str] = &[
    "none, Hann (2.0.6 behavior)",
    "Hann, none",
    "Hann, Hann (default)",
    "Blackman, Hann",
    "Hamming, none",
    "Hamming, Hann",
];
const WINDOW_SIZE_CHOICES: &[&str] = &[
    "8", "16", "32", "64", "128", "256", "512", "1024", "2048", "4096", "8192",
    "16384",
];
const STEPS_CHOICES: &[&str] = &["2", "4", "8", "16", "32"];
const METHOD_CHOICES: &[&str] = &["Median", "Second greatest", "Old"];

impl NoiseReduction {
    fn window_size_samples(&self) -> usize {
        1usize << (3 + self.window_size.clamp(0, 11))
    }

    fn steps_per_window_count(&self) -> usize {
        1usize << (1 + self.steps_per_window.clamp(0, 4))
    }

    /// Returns the currently learned profile, if any.
    pub fn profile(&self) -> Option<&NoiseProfile> {
        self.profile.as_ref()
    }

    /// Replaces the currently learned profile.
    pub fn set_profile(&mut self, profile: NoiseProfile) {
        self.profile = Some(profile);
    }

    /// Learns noise statistics from every channel in `noise`.
    pub fn learn_profile(&mut self, noise: &AudioBuffer) -> Result<(), EffectError> {
        let size = self.window_size_samples();
        let steps = self.steps_per_window_count();
        if steps > size {
            return Err(EffectError::Invalid(
                "Steps per block cannot exceed the window size.".into(),
            ));
        }
        let hop = size / steps;
        let windows = make_windows(size, steps, self.window_types);
        let fft = RealFft::new(size);
        let bins = size / 2 + 1;
        let mut profile = NoiseProfile {
            rate: noise.sample_rate as f64,
            window_size: size,
            steps_per_window: steps,
            window_types: self.window_types,
            total_windows: 0,
            track_windows: 0,
            sums: vec![0.0; bins],
            means: vec![0.0; bins],
        };
        let mut buffer = vec![0.0f32; size];

        // Profiling has neither leading nor trailing padding. A window is
        // visited only when all its samples came from the selected audio.
        for channel in &noise.channels {
            let mut start = 0;
            while start + size <= channel.len() {
                for i in 0..size {
                    buffer[i] = channel[start + i] * windows.input[i];
                }
                fft.forward(&mut buffer);
                profile.sums[0] += ((buffer[0] as f64) * (buffer[0] as f64)) as f32;
                for bin in 1..size / 2 {
                    let k = fft.bit_reversed[bin];
                    let re = buffer[k] as f64;
                    let im = buffer[k + 1] as f64;
                    profile.sums[bin] += (re * re + im * im) as f32;
                }
                profile.sums[size / 2] +=
                    ((buffer[1] as f64) * (buffer[1] as f64)) as f32;
                profile.track_windows += 1;
                start += hop;
            }

            // Worker::FinishTrackStatistics combines completed tracks using
            // float arithmetic, then clears the per-track accumulator.
            let count = profile.track_windows;
            if count != 0 {
                let previous = profile.total_windows;
                let denominator = previous + count;
                for i in 0..bins {
                    profile.means[i] = (profile.means[i] * previous as f32
                        + profile.sums[i])
                        / denominator as f32;
                    profile.sums[i] = 0.0;
                }
                profile.track_windows = 0;
                profile.total_windows = denominator;
            }
        }

        if profile.total_windows == 0 {
            return Err(EffectError::Invalid(
                "Selected noise profile is too short.".into(),
            ));
        }
        self.profile = Some(profile);
        Ok(())
    }

    fn validate_profile(&self, rate: u32) -> Result<&NoiseProfile, EffectError> {
        let profile = self.profile.as_ref().ok_or_else(|| {
            EffectError::Invalid(
                "You must first run Noise Reduction on the noise profile.".into(),
            )
        })?;
        if profile.rate != rate as f64 {
            return Err(EffectError::Invalid(
                "The sample rate of the noise profile must match that of the sound to be processed."
                    .into(),
            ));
        }
        if profile.window_size != self.window_size_samples() {
            return Err(EffectError::Invalid(
                "You must specify the same window size for steps 1 and 2.".into(),
            ));
        }
        Ok(profile)
    }

    fn process_audio(&self, buf: &mut AudioBuffer) -> Result<(), EffectError> {
        let profile = self.validate_profile(buf.sample_rate)?;
        if buf.channels.is_empty() || buf.len() == 0 {
            return Ok(());
        }

        let mut reducer = StreamingNoiseReducer::new(profile, self, buf.channels.len());
        let frames = buf.len();
        let latency = reducer.latency();
        let channels = buf.channels.len();
        let mut interleaved = vec![0.0f32; (frames + latency) * channels];
        for frame in 0..frames {
            for ch in 0..channels {
                interleaved[frame * channels + ch] = buf.channels[ch][frame];
            }
        }
        let mut processed = vec![0.0f32; interleaved.len()];
        reducer.process_block(&interleaved, &mut processed);
        for frame in 0..frames {
            for ch in 0..channels {
                buf.channels[ch][frame] = processed[(frame + latency) * channels + ch];
            }
        }
        Ok(())
    }
}

impl BuiltinEffect for NoiseReduction {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "NoiseReduction"
    }) }

    fn name(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Noise Reduction"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("NoiseReduction", "Noise reduction (dB)", 0.0, 48.0, 6.0),
            ParamDef::float("Sensitivity", "Sensitivity", 0.01, 24.0, 6.0),
            ParamDef::int(
                "FrequencySmoothing",
                "Frequency smoothing (bands)",
                0.0,
                12.0,
                6.0,
            ),
            ParamDef::choice("NoiseReductionChoice", "Reduce noise", OUTPUT_CHOICES, 0),
            ParamDef::choice("WindowTypes", "Window types", WINDOW_CHOICES, 2),
            ParamDef::choice("WindowSize", "Window size", WINDOW_SIZE_CHOICES, 8),
            ParamDef::choice("StepsPerWindow", "Steps per window", STEPS_CHOICES, 1),
            ParamDef::choice("Method", "Method", METHOD_CHOICES, 1),
            ParamDef::float("OldSensitivity", "Old sensitivity", -20.0, 20.0, 0.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "NoiseReduction" => Some(self.noise_reduction),
            "Sensitivity" => Some(self.sensitivity),
            "FrequencySmoothing" => Some(self.frequency_smoothing),
            "NoiseReductionChoice" => Some(self.noise_reduction_choice as f64),
            "WindowTypes" => Some(self.window_types as f64),
            "WindowSize" => Some(self.window_size as f64),
            "StepsPerWindow" => Some(self.steps_per_window as f64),
            "Method" => Some(self.method as f64),
            "OldSensitivity" => Some(self.old_sensitivity),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let value = if value.is_nan() { 0.0 } else { value };
        match key {
            "NoiseReduction" => self.noise_reduction = value.clamp(0.0, 48.0),
            "Sensitivity" => self.sensitivity = value.clamp(0.01, 24.0),
            "FrequencySmoothing" => {
                self.frequency_smoothing = value.round().clamp(0.0, 12.0)
            }
            "NoiseReductionChoice" => {
                self.noise_reduction_choice = value.round().clamp(0.0, 2.0) as usize
            }
            "WindowTypes" => self.window_types = value.round().clamp(0.0, 5.0) as usize,
            "WindowSize" => self.window_size = value.round().clamp(0.0, 11.0) as usize,
            "StepsPerWindow" => self.steps_per_window = value.round().clamp(0.0, 4.0) as usize,
            "Method" => self.method = value.round().clamp(0.0, 2.0) as usize,
            "OldSensitivity" => self.old_sensitivity = value.clamp(-20.0, 20.0),
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        self.process_audio(buf)
    }

    fn learn_profile(&mut self, noise: &AudioBuffer) -> Result<(), EffectError> {
        NoiseReduction::learn_profile(self, noise)
    }

    fn supports_realtime(&self) -> bool {
        false
    }
}

/// Stateful, block-based noise reducer for interleaved audio.
pub struct StreamingNoiseReducer {
    states: Vec<StreamChannel>,
    channels: usize,
    latency: usize,
    bypass: bool,
}

impl StreamingNoiseReducer {
    /// Constructs a reducer for `channels` interleaved channels.
    pub fn new(profile: &NoiseProfile, settings: &NoiseReduction, channels: usize) -> Self {
        let size = profile.window_size;
        let steps = settings.steps_per_window_count().min(size).max(1);
        let hop = size / steps;
        let rate = profile.rate;
        let attack_blocks = 1 + (settings_attack_time() * rate / hop as f64) as usize;
        let n_windows = if settings.method == 2 {
            (0.05 * rate / hop as f64) as usize
        } else {
            1 + steps
        };
        let n_windows = if settings.method == 2 {
            n_windows.max(2)
        } else {
            n_windows
        };
        let center = n_windows / 2;
        let history_len = n_windows.max(center + attack_blocks);
        let latency = (history_len + steps - 1) * hop;
        let windows = make_windows(size, steps, settings.window_types);
        let states = (0..channels)
            .map(|_| StreamChannel::new(profile, settings, &windows, history_len, n_windows))
            .collect();

        Self {
            states,
            channels,
            latency,
            bypass: false,
        }
    }

    /// Returns the fixed processing latency in frames.
    pub fn latency(&self) -> usize {
        self.latency
    }

    /// Enables or disables bypass.
    pub fn set_bypass(&mut self, bypass: bool) {
        self.bypass = bypass;
    }

    /// Processes one interleaved block without allocating.
    ///
    /// Input and output must contain the same number of interleaved samples.
    pub fn process_block(&mut self, input: &[f32], output: &mut [f32]) {
        let count = input.len().min(output.len());
        if self.bypass || self.channels == 0 {
            output[..count].copy_from_slice(&input[..count]);
            output[count..].fill(0.0);
            return;
        }

        let frames = count / self.channels;
        for frame in 0..frames {
            for ch in 0..self.channels {
                let index = frame * self.channels + ch;
                output[index] = self.states[ch].process_sample(input[index]);
            }
        }
        output[frames * self.channels..].fill(0.0);
    }
}

fn settings_attack_time() -> f64 {
    0.02
}

struct SpectrumWindow {
    real: Vec<f32>,
    imag: Vec<f32>,
    spectrum: Vec<f32>,
    gains: Vec<f32>,
}

impl SpectrumWindow {
    fn new(size: usize, attenuation: f32) -> Self {
        Self {
            real: vec![0.0; size / 2],
            imag: vec![0.0; size / 2],
            spectrum: vec![0.0; size / 2 + 1],
            gains: vec![attenuation; size / 2 + 1],
        }
    }
}

struct StreamChannel {
    size: usize,
    steps: usize,
    hop: usize,
    input: Vec<f32>,
    input_pos: usize,
    overlap: Vec<f32>,
    fft_buffer: Vec<f32>,
    fft: RealFft,
    output_window: Vec<f32>,
    input_window: Vec<f32>,
    queue: Vec<SpectrumWindow>,
    out_step_count: isize,
    n_windows: usize,
    center: usize,
    means: Vec<f32>,
    attenuation: f32,
    attack: f32,
    release: f32,
    sensitivity: f64,
    smoothing_bins: usize,
    smoothing_scratch: Vec<f32>,
    choice: usize,
    method: usize,
    fifo: Vec<f32>,
    fifo_read: usize,
    fifo_write: usize,
    fifo_count: usize,
}

impl StreamChannel {
    fn new(
        profile: &NoiseProfile,
        settings: &NoiseReduction,
        windows: &Windows,
        history_len: usize,
        n_windows: usize,
    ) -> Self {
        let size = profile.window_size;
        let steps = settings.steps_per_window_count().min(size).max(1);
        let hop = size / steps;
        let rate = profile.rate;
        let attack_blocks = 1 + (0.02 * rate / hop as f64) as usize;
        let release_blocks = 1 + (0.10 * rate / hop as f64) as usize;
        let noise_gain = -settings.noise_reduction;
        let attenuation = 10.0f64.powf(noise_gain / 20.0) as f32;
        let attack = 10.0f64.powf(noise_gain / attack_blocks as f64 / 20.0) as f32;
        let release = 10.0f64.powf(noise_gain / release_blocks as f64 / 20.0) as f32;
        let bins = size / 2 + 1;

        Self {
            size,
            steps,
            hop,
            input: vec![0.0; size],
            input_pos: size - hop,
            overlap: vec![0.0; size],
            fft_buffer: vec![0.0; size],
            fft: RealFft::new(size),
            output_window: windows.output.clone(),
            input_window: windows.input.clone(),
            queue: (0..history_len)
                .map(|_| SpectrumWindow::new(size, attenuation))
                .collect(),
            out_step_count: -((history_len - 1 + steps - 1) as isize),
            n_windows,
            center: n_windows / 2,
            means: profile.means.clone(),
            attenuation,
            attack,
            release,
            sensitivity: settings.sensitivity * 10.0f64.ln(),
            smoothing_bins: settings.frequency_smoothing.max(0.0) as usize,
            smoothing_scratch: vec![0.0; bins],
            choice: settings.noise_reduction_choice,
            method: settings.method,
            fifo: vec![0.0; size * 2 + 1],
            fifo_read: 0,
            fifo_write: 0,
            fifo_count: 0,
        }
    }

    fn process_sample(&mut self, sample: f32) -> f32 {
        let output = self.pop_fifo();
        self.input[self.input_pos] = sample;
        self.input_pos += 1;
        if self.input_pos == self.size {
            self.process_window();
            self.input.copy_within(self.hop..self.size, 0);
            self.input_pos -= self.hop;
        }
        output
    }

    fn pop_fifo(&mut self) -> f32 {
        if self.fifo_count == 0 {
            return 0.0;
        }
        let value = self.fifo[self.fifo_read];
        self.fifo_read = (self.fifo_read + 1) % self.fifo.len();
        self.fifo_count -= 1;
        value
    }

    fn push_fifo(&mut self, value: f32) {
        if self.fifo_count < self.fifo.len() {
            self.fifo[self.fifo_write] = value;
            self.fifo_write = (self.fifo_write + 1) % self.fifo.len();
            self.fifo_count += 1;
        }
    }

    fn process_window(&mut self) {
        for i in 0..self.size {
            self.fft_buffer[i] = self.input[i] * self.input_window[i];
        }
        self.fft.forward(&mut self.fft_buffer);

        let newest = &mut self.queue[0];
        newest.real[0] = self.fft_buffer[0];
        newest.imag[0] = self.fft_buffer[1];
        newest.spectrum[0] =
            ((newest.real[0] as f64) * (newest.real[0] as f64)) as f32;
        for bin in 1..self.size / 2 {
            let k = self.fft.bit_reversed[bin];
            let re = self.fft_buffer[k];
            let im = self.fft_buffer[k + 1];
            newest.real[bin] = re;
            newest.imag[bin] = im;
            newest.spectrum[bin] = (re as f64 * re as f64 + im as f64 * im as f64) as f32;
        }
        newest.spectrum[self.size / 2] =
            ((newest.imag[0] as f64) * (newest.imag[0] as f64)) as f32;

        self.reduce_noise();

        if self.queue_is_full() {
            let record = self.queue.last().expect("nonempty history");
            self.fft_buffer[0] = record.real[0];
            self.fft_buffer[1] = record.imag[0];
            for bin in 1..self.size / 2 {
                self.fft_buffer[2 * bin] = record.real[bin];
                self.fft_buffer[2 * bin + 1] = record.imag[bin];
            }
            self.fft.inverse(&mut self.fft_buffer);
            for pair in 0..self.size / 2 {
                let k = self.fft.bit_reversed[pair];
                let i = pair * 2;
                self.overlap[i] += self.fft_buffer[k] * self.output_window[i];
                self.overlap[i + 1] +=
                    self.fft_buffer[k + 1] * self.output_window[i + 1];
            }
            if self.out_step_count >= 0 {
                for i in 0..self.hop {
                    self.push_fifo(self.overlap[i]);
                }
            }
            self.overlap.copy_within(self.hop..self.size, 0);
            self.overlap[self.size - self.hop..].fill(0.0);
        }

        self.out_step_count += 1;
        self.queue.rotate_right(1);
    }

    fn current_queue_size(&self) -> usize {
        let available = self.out_step_count
            + self.queue.len() as isize
            + (self.steps - 1) as isize;
        available.max(0).min(self.queue.len() as isize) as usize
    }

    fn queue_is_full(&self) -> bool {
        self.out_step_count >= -((self.steps - 1) as isize)
    }

    fn classify(&self, n_windows: usize, bin: usize) -> bool {
        if self.method == 0 && n_windows > 3 {
            if n_windows > 5 {
                return true;
            }
            let mut greatest = 0.0f32;
            let mut second = 0.0f32;
            let mut third = 0.0f32;
            for record in self.queue.iter().take(n_windows) {
                let power = record.spectrum[bin];
                if power >= greatest {
                    third = second;
                    second = greatest;
                    greatest = power;
                } else if power >= second {
                    third = second;
                    second = power;
                } else if power >= third {
                    third = power;
                }
            }
            return third as f64 <= self.sensitivity * self.means[bin] as f64;
        }

        // The old discrimination method is unavailable in Audacity's normal
        // build; its settings are mapped to the default method.
        let mut greatest = 0.0f32;
        let mut second = 0.0f32;
        for record in self.queue.iter().take(n_windows) {
            let power = record.spectrum[bin];
            if power >= greatest {
                second = greatest;
                greatest = power;
            } else if power >= second {
                second = power;
            }
        }
        second as f64 <= self.sensitivity * self.means[bin] as f64
    }

    fn smooth_oldest_gains(&mut self) {
        if self.smoothing_bins == 0 {
            return;
        }
        let gains = &mut self.queue.last_mut().expect("nonempty history").gains;
        for gain in gains.iter_mut() {
            *gain = gain.ln();
        }
        for i in 0..gains.len() {
            let first = i.saturating_sub(self.smoothing_bins);
            let last = (i + self.smoothing_bins).min(gains.len() - 1);
            let mut sum = 0.0f32;
            for gain in &gains[first..=last] {
                sum += *gain;
            }
            self.smoothing_scratch[i] = sum / (last - first + 1) as f32;
        }
        for (gain, value) in gains.iter_mut().zip(&self.smoothing_scratch) {
            *gain = value.exp();
        }
    }

    fn reduce_noise(&mut self) {
        let history_len = self.current_queue_size();
        let n_windows = self.n_windows.min(history_len);
        let bins = self.size / 2 + 1;

        if self.choice != 1 {
            self.queue[0].gains.fill(self.attenuation);
        }

        if n_windows > self.center {
            for bin in 0..bins {
                let is_noise = self.classify(n_windows, bin);
                if self.choice == 1 {
                    self.queue[self.center].gains[bin] = if is_noise { 1.0 } else { 0.0 };
                } else if !is_noise {
                    self.queue[self.center].gains[bin] = 1.0;
                }
            }
        }

        if self.choice != 1 {
            for bin in 0..bins {
                for i in self.center + 1..history_len {
                    let minimum = self
                        .attenuation
                        .max(self.queue[i - 1].gains[bin] * self.attack);
                    if self.queue[i].gains[bin] < minimum {
                        self.queue[i].gains[bin] = minimum;
                    } else {
                        break;
                    }
                }
            }

            for bin in 0..bins {
                let minimum = self
                    .attenuation
                    .max(self.queue[self.center].gains[bin] * self.release);
                self.queue[self.center - 1].gains[bin] =
                    self.queue[self.center - 1].gains[bin].max(minimum);
            }
        }

        if self.queue_is_full() {
            if self.choice != 1 {
                self.smooth_oldest_gains();
            }
            let record = self.queue.last_mut().expect("nonempty history");
            for bin in 1..self.size / 2 {
                // Audacity promotes the float gain to double before the
                // residue subtraction and coefficient multiplication.
                let gain = if self.choice == 2 {
                    record.gains[bin] as f64 - 1.0
                } else {
                    record.gains[bin] as f64
                };
                record.real[bin] = (record.real[bin] as f64 * gain) as f32;
                record.imag[bin] = (record.imag[bin] as f64 * gain) as f32;
            }
            let dc_gain = if self.choice == 2 {
                record.gains[0] as f64 - 1.0
            } else {
                record.gains[0] as f64
            };
            let nyquist_gain = if self.choice == 2 {
                record.gains[self.size / 2] as f64 - 1.0
            } else {
                record.gains[self.size / 2] as f64
            };
            record.real[0] = (record.real[0] as f64 * dc_gain) as f32;
            record.imag[0] = (record.imag[0] as f64 * nyquist_gain) as f32;
        }
    }
}

struct Windows {
    input: Vec<f32>,
    output: Vec<f32>,
}

fn make_windows(size: usize, steps: usize, window_type: usize) -> Windows {
    // 0: rectangular, 1: Hann, 2: Hamming, 3: Blackman.
    let (in_kind, out_kind) = match window_type {
        0 => (0, 1),
        1 => (1, 0),
        3 => (3, 1),
        4 => (2, 0),
        5 => (2, 1),
        _ => (1, 1),
    };
    let mut input = window_values(size, in_kind);
    let mut output = window_values(size, out_kind);
    let hop = (size / steps.max(1)).max(1);
    let mut denom = 0.0f64;
    for i in (0..size).step_by(hop) {
        denom += input[i] as f64 * output[i] as f64;
    }
    if denom != 0.0 {
        let target = if in_kind == 0 {
            &mut output
        } else {
            &mut input
        };
        for value in target {
            *value = (*value as f64 / denom) as f32;
        }
    }
    Windows { input, output }
}

fn window_values(size: usize, kind: usize) -> Vec<f32> {
    if kind == 0 {
        return vec![1.0; size];
    }
    let multiplier = 2.0 * std::f64::consts::PI / size as f64;
    (0..size)
        .map(|i| {
            let phase = i as f64 * multiplier;
            match kind {
                2 => (0.54 - 0.46 * phase.cos()) as f32,
                3 => {
                    let multiplier2 = 2.0 * multiplier;
                    (0.42 - 0.5 * phase.cos()
                        + 0.08 * (i as f64 * multiplier2).cos()) as f32
                }
                _ => (0.5 - 0.5 * phase.cos()) as f32,
            }
        })
        .collect()
}

/// The bit-reversed, interleaved, single-precision FFT used by Audacity's
/// SpectrumTransformer. Its forward transform is unnormalized; its inverse
/// returns samples in bit-reversed pair order.
struct RealFft {
    points: usize,
    bit_reversed: Vec<usize>,
    sin_table: Vec<f32>,
}

impl RealFft {
    fn new(size: usize) -> Self {
        let points = size / 2;
        let mut bit_reversed = vec![0; points];
        let mut sin_table = vec![0.0; size];
        for i in 0..points {
            let mut temp = 0usize;
            let mut mask = points / 2;
            while mask > 0 {
                temp = (temp >> 1) + if i & mask != 0 { points } else { 0 };
                mask >>= 1;
            }
            bit_reversed[i] = temp;
        }
        for i in 0..points {
            let angle = 2.0 * std::f64::consts::PI * i as f64 / size as f64;
            sin_table[bit_reversed[i]] = (-angle.sin()) as f32;
            sin_table[bit_reversed[i] + 1] = (-angle.cos()) as f32;
        }
        Self {
            points,
            bit_reversed,
            sin_table,
        }
    }

    fn forward(&self, buffer: &mut [f32]) {
        let size = self.points * 2;
        let mut butterflies = self.points / 2;
        while butterflies > 0 {
            let mut a = 0;
            let mut b = butterflies * 2;
            let mut sp = 0;
            while a < size {
                let sin = self.sin_table[sp];
                let cos = self.sin_table[sp + 1];
                let end = b;
                while a < end {
                    let v1 = buffer[b] * cos + buffer[b + 1] * sin;
                    let v2 = buffer[b] * sin - buffer[b + 1] * cos;
                    buffer[b] = buffer[a] + v1;
                    buffer[a] = buffer[b] - 2.0 * v1;
                    a += 1;
                    b += 1;
                    buffer[b] = buffer[a] - v2;
                    buffer[a] = buffer[b] + 2.0 * v2;
                    a += 1;
                    b += 1;
                }
                a = b;
                b += butterflies * 2;
                sp += 2;
            }
            butterflies >>= 1;
        }

        let mut left = 1;
        let mut right = self.points - 1;
        while left < right {
            let ai = self.bit_reversed[left];
            let bi = self.bit_reversed[right];
            let sin = self.sin_table[ai];
            let cos = self.sin_table[ai + 1];
            let hr_minus = buffer[ai] - buffer[bi];
            let hr_plus = hr_minus + buffer[bi] * 2.0;
            let hi_minus = buffer[ai + 1] - buffer[bi + 1];
            let hi_plus = hi_minus + buffer[bi + 1] * 2.0;
            let v1 = sin * hr_minus - cos * hi_plus;
            let v2 = cos * hr_minus + sin * hi_plus;
            buffer[ai] = (hr_plus + v1) * 0.5;
            buffer[bi] = buffer[ai] - v1;
            buffer[ai + 1] = (hi_minus + v2) * 0.5;
            buffer[bi + 1] = buffer[ai + 1] - hi_minus;
            left += 1;
            right -= 1;
        }
        let center_imag = self.bit_reversed[left] + 1;
        buffer[center_imag] = -buffer[center_imag];
        let v1 = buffer[0] - buffer[1];
        buffer[0] += buffer[1];
        buffer[1] = v1;
    }

    fn inverse(&self, buffer: &mut [f32]) {
        let size = self.points * 2;
        let mut a = 2;
        let mut b = size - 2;
        let mut br = 1;
        while a < b {
            let k = self.bit_reversed[br];
            let sin = self.sin_table[k];
            let cos = self.sin_table[k + 1];
            let hr_minus = buffer[a] - buffer[b];
            let hr_plus = hr_minus + buffer[b] * 2.0;
            let hi_minus = buffer[a + 1] - buffer[b + 1];
            let hi_plus = hi_minus + buffer[b + 1] * 2.0;
            let v1 = sin * hr_minus + cos * hi_plus;
            let v2 = cos * hr_minus - sin * hi_plus;
            buffer[a] = (hr_plus + v1) * 0.5;
            buffer[b] = buffer[a] - v1;
            buffer[a + 1] = (hi_minus - v2) * 0.5;
            buffer[b + 1] = buffer[a + 1] - hi_minus;
            a += 2;
            b -= 2;
            br += 1;
        }
        buffer[a + 1] = -buffer[a + 1];
        let v1 = 0.5f32 * (buffer[0] + buffer[1]);
        let v2 = 0.5f32 * (buffer[0] - buffer[1]);
        buffer[0] = v1;
        buffer[1] = v2;

        let mut butterflies = self.points / 2;
        while butterflies > 0 {
            let mut a = 0;
            let mut b = butterflies * 2;
            let mut sp = 0;
            while a < size {
                let sin = self.sin_table[sp];
                let cos = self.sin_table[sp + 1];
                sp += 2;
                let end = b;
                while a < end {
                    let v1 = buffer[b] * cos - buffer[b + 1] * sin;
                    let v2 = buffer[b] * sin + buffer[b + 1] * cos;
                    buffer[b] = (buffer[a] + v1) * 0.5;
                    buffer[a] = buffer[b] - v1;
                    a += 1;
                    b += 1;
                    buffer[b] = (buffer[a] + v2) * 0.5;
                    buffer[a] = buffer[b] - v2;
                    a += 1;
                    b += 1;
                }
                a = b;
                b += butterflies * 2;
            }
            butterflies >>= 1;
        }
    }
}

/// Returns one default instance of each effect in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(NoiseReduction::default())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stereo_buffer(rate: u32, frames: usize) -> AudioBuffer {
        let mut left = Vec::with_capacity(frames);
        let mut right = Vec::with_capacity(frames);
        for i in 0..frames {
            let phase = 2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32;
            left.push(phase.sin() * 0.2);
            right.push((phase + 0.2).sin() * 0.2);
        }
        AudioBuffer::from_channels(rate, vec![left, right])
    }

    #[test]
    fn default_runs_and_empty_is_safe() {
        let mut effect = NoiseReduction::default();
        let noise = stereo_buffer(8000, 8192);
        effect.learn_profile(&noise).unwrap();

        let mut audio = stereo_buffer(8000, 4096);
        effect
            .process(&mut audio, &mut EffectContext::default())
            .unwrap();

        let mut empty = AudioBuffer::from_channels(8000, vec![Vec::new(), Vec::new()]);
        effect
            .process(&mut empty, &mut EffectContext::default())
            .unwrap();
    }

    #[test]
    fn profile_serialization_round_trip() {
        let mut effect = NoiseReduction::default();
        effect.learn_profile(&stereo_buffer(8000, 8192)).unwrap();
        let profile = effect.profile().unwrap();
        let decoded = NoiseProfile::from_bytes(&profile.to_bytes()).unwrap();
        assert_eq!(*profile, decoded);
    }

    #[test]
    fn streaming_is_repeatable_and_offline_runs() {
        let mut effect = NoiseReduction::default();
        effect.learn_profile(&stereo_buffer(8000, 8192)).unwrap();
        let input = stereo_buffer(8000, 4096);
        let mut first = input.clone();
        let mut second = input;
        effect
            .process(&mut first, &mut EffectContext::default())
            .unwrap();
        effect
            .process(&mut second, &mut EffectContext::default())
            .unwrap();
        assert_eq!(first.channels, second.channels);
    }

    #[test]
    fn parameter_clamping() {
        let mut effect = NoiseReduction::default();
        assert!(set_param(&mut effect, "NoiseReduction", 100.0));
        assert_eq!(effect.noise_reduction, 48.0);
        assert!(set_param(&mut effect, "WindowSize", -3.0));
        assert_eq!(effect.window_size, 0);
        assert!(!set_param(&mut effect, "Unknown", 1.0));
    }

    #[test]
    fn real_fft_round_trip() {
        let fft = RealFft::new(2048);
        let original: Vec<f32> = (0..2048)
            .map(|i| (i as f32 * 0.123).sin() * 0.3)
            .collect();
        let mut buffer = original.clone();
        fft.forward(&mut buffer);
        let mut natural = vec![0.0f32; 2048];
        natural[0] = buffer[0];
        natural[1] = buffer[1];
        for bin in 1..1024 {
            let k = fft.bit_reversed[bin];
            natural[2 * bin] = buffer[k];
            natural[2 * bin + 1] = buffer[k + 1];
        }
        fft.inverse(&mut natural);
        for pair in 0..1024 {
            let k = fft.bit_reversed[pair];
            assert!((natural[k] - original[2 * pair]).abs() < 2e-5);
            assert!((natural[k + 1] - original[2 * pair + 1]).abs() < 2e-5);
        }
    }
}
