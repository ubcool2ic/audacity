//! Audacity 3.7.3 Paulstretch effect.

use crate::core::*;
use au_core::AudioBuffer;
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::f32::consts::PI;
use std::sync::Arc;

const FLOAT_MAX: f64 = f32::MAX as f64;

/// Extreme time-stretch and time-smear effect.
#[derive(Clone, Debug)]
pub struct Paulstretch {
    /// Stretch factor.
    pub stretch_factor: f32,
    /// Analysis-window time resolution in seconds.
    pub time_resolution: f32,
}

impl Default for Paulstretch {
    fn default() -> Self {
        Self {
            stretch_factor: 10.0,
            time_resolution: 0.25,
        }
    }
}

impl BuiltinEffect for Paulstretch {
    fn id(&self) -> &'static str {
        "Paulstretch"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Stretch Factor", "Stretch Factor", 1.0, FLOAT_MAX, 10.0),
            ParamDef::float("Time Resolution", "Time Resolution", 0.00099, FLOAT_MAX, 0.25),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Stretch Factor" => Some(self.stretch_factor as f64),
            "Time Resolution" => Some(self.time_resolution as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(def) = self.params().into_iter().find(|param| param.key == key) else {
            return false;
        };
        let value = def.clamp(value) as f32;
        match key {
            "Stretch Factor" => self.stretch_factor = value,
            "Time Resolution" => self.time_resolution = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let frames = buf.len();
        if frames == 0 || buf.channels.is_empty() {
            return Ok(());
        }
        if buf.sample_rate == 0 {
            return Err(EffectError::Invalid(
                "Paulstretch requires a non-zero sample rate".into(),
            ));
        }

        let input_buffer_size = self.buffer_size(buf.sample_rate as f64)?;
        let pool_size = input_buffer_size
            .checked_mul(2)
            .ok_or_else(|| EffectError::Invalid("Paulstretch buffer is too large".into()))?;
        let min_duration = pool_size
            .checked_add(1)
            .ok_or_else(|| EffectError::Invalid("Paulstretch buffer is too large".into()))?;

        if frames < min_duration {
            return Err(EffectError::Invalid(
                "The audio selection is too short for the Paulstretch time resolution".into(),
            ));
        }

        // Audacity processes channels serially, using the process-global rand()
        // state for successive transforms. Use one equivalent generator here.
        let mut random = GlibcRand::new();
        let mut results = Vec::with_capacity(buf.channels.len());

        for channel in &buf.channels {
            results.push(self.process_channel(
                channel,
                frames,
                buf.sample_rate,
                input_buffer_size,
                pool_size,
                &mut random,
            )?);
        }

        buf.channels = results;
        Ok(())
    }
}

impl Paulstretch {
    fn buffer_size(&self, rate: f64) -> Result<usize, EffectError> {
        // Preserve GetBufferSize's float/double conversions and rounding.
        let mut tmp = (rate * self.time_resolution as f64 / 2.0) as f32;
        if !tmp.is_finite() || tmp <= 0.0 {
            return Err(EffectError::Invalid(
                "Invalid Paulstretch time resolution".into(),
            ));
        }

        tmp = (tmp.ln() as f64 / 2.0f64.ln()) as f32;
        tmp = 2.0f64.powf((tmp as f64 + 0.5).floor()) as f32;

        if !tmp.is_finite() || tmp < 0.0 || tmp > (1usize << 22) as f32 {
            return Err(EffectError::Invalid(
                "Requested Paulstretch buffer exceeds memory capacity".into(),
            ));
        }

        let size = tmp as usize;
        if size as f32 != tmp {
            return Err(EffectError::Invalid(
                "Requested Paulstretch buffer exceeds memory capacity".into(),
            ));
        }

        Ok(size.max(128))
    }

    fn process_channel(
        &self,
        input: &[f32],
        frames: usize,
        sample_rate: u32,
        input_buffer_size: usize,
        pool_size: usize,
        random: &mut GlibcRand,
    ) -> Result<Vec<f32>, EffectError> {
        let dlen = frames as f64;
        let stretch_buffer_size = input_buffer_size as f64;
        let adjust_amount = dlen / (dlen - stretch_buffer_size * 2.0);
        let amount = (1.0 + (self.stretch_factor as f64 - 1.0) * adjust_amount) as f32;

        let mut stretch =
            PaulStretchEngine::new(amount, input_buffer_size, sample_rate as f32);
        let fade_len = 100usize.min(pool_size / 2 - 1);

        let mut output = Vec::new();
        let mut position = 0usize;
        let mut nget = stretch.get_nsamples_for_fill();
        let mut first_time = true;

        while position < frames {
            let mut input_block = vec![0.0f32; pool_size];
            for (i, sample) in input_block.iter_mut().take(nget).enumerate() {
                *sample = input
                    .get(position.saturating_add(i))
                    .copied()
                    .unwrap_or(0.0);
            }

            stretch.process(&input_block, nget, random);

            if first_time {
                // The first call fills the analysis pool. The second call,
                // with no new samples, establishes the initial overlap state.
                stretch.process(&input_block, 0, random);
            }

            position = position.saturating_add(nget);

            if first_time {
                first_time = false;
                for i in 0..fade_len {
                    let fi = i as f32 / fade_len as f32;
                    let original = input.get(i).copied().unwrap_or(0.0);
                    stretch.out_buf[i] =
                        (stretch.out_buf[i] * fi + (1.0 - fi) * original) as f32;
                }
            }

            if position >= frames {
                for i in 0..fade_len {
                    let fi = i as f32 / fade_len as f32;
                    let output_index = pool_size / 2 - 1 - i;
                    let source_index = frames - fade_len + fade_len - 1 - i;
                    let original = input.get(source_index).copied().unwrap_or(0.0);
                    stretch.out_buf[output_index] =
                        (stretch.out_buf[output_index] * fi + (1.0 - fi) * original) as f32;
                }
            }

            output
                .try_reserve(stretch.out_bufsize)
                .map_err(|_| EffectError::Invalid("Paulstretch output exceeds memory capacity".into()))?;
            output.extend_from_slice(&stretch.out_buf);

            nget = stretch.get_nsamples();
            if nget == 0 {
                return Err(EffectError::Invalid(
                    "Paulstretch produced a zero-sized input hop".into(),
                ));
            }
        }

        Ok(output)
    }
}

/// Paulstretch analysis/synthesis state for one channel.
struct PaulStretchEngine {
    _samplerate: f32,
    rap: f32,
    out_bufsize: usize,
    out_buf: Vec<f32>,
    old_out_smp_buf: Vec<f32>,
    poolsize: usize,
    in_pool: Vec<f32>,
    remained_samples: f64,
    fft_smps: Vec<f32>,
    fft_c: Vec<f32>,
    fft_s: Vec<f32>,
    fft_freq: Vec<f32>,
    fft_tmp: Vec<f32>,
    fft_plan: Arc<dyn Fft<f32>>,
    ifft_plan: Arc<dyn Fft<f32>>,
    complex: Vec<Complex<f32>>,
    inverse: Vec<Complex<f32>>,
}

impl PaulStretchEngine {
    fn new(rap: f32, in_bufsize: usize, samplerate: f32) -> Self {
        let poolsize = in_bufsize * 2;
        let out_bufsize = 8usize.max(in_bufsize);
        let mut planner = FftPlanner::<f32>::new();
        let fft_plan = planner.plan_fft_forward(poolsize);
        let ifft_plan = planner.plan_fft_inverse(poolsize);

        Self {
            _samplerate: samplerate,
            rap: 1.0f32.max(rap),
            out_bufsize,
            out_buf: vec![0.0; out_bufsize],
            old_out_smp_buf: vec![0.0; out_bufsize * 2],
            poolsize,
            in_pool: vec![0.0; poolsize],
            remained_samples: 0.0,
            fft_smps: vec![0.0; poolsize],
            fft_c: vec![0.0; poolsize],
            fft_s: vec![0.0; poolsize],
            fft_freq: vec![0.0; poolsize],
            fft_tmp: vec![0.0; poolsize],
            fft_plan,
            ifft_plan,
            complex: vec![Complex::new(0.0, 0.0); poolsize],
            inverse: vec![Complex::new(0.0, 0.0); poolsize],
        }
    }

    fn process(&mut self, samples: &[f32], nsmps: usize, random: &mut GlibcRand) {
        if nsmps != 0 {
            let nsmps = nsmps.min(self.poolsize);
            let nleft = self.poolsize - nsmps;
            self.in_pool.copy_within(nsmps..self.poolsize, 0);
            for i in 0..nsmps {
                self.in_pool[nleft + i] = samples.get(i).copied().unwrap_or(0.0);
            }
        }

        self.fft_smps.copy_from_slice(&self.in_pool);

        let denominator = (self.poolsize - 1) as f64;
        for i in 0..self.poolsize {
            let window =
                0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / denominator).cos();
            self.fft_smps[i] *= window as f32;
            self.complex[i] = Complex::new(self.fft_smps[i], 0.0);
        }

        self.fft_plan.process(&mut self.complex);

        for i in 0..self.poolsize {
            self.fft_c[i] = self.complex[i].re;
            self.fft_s[i] = self.complex[i].im;
        }

        for i in 0..self.poolsize / 2 {
            self.fft_freq[i] =
                (self.fft_c[i] * self.fft_c[i] + self.fft_s[i] * self.fft_s[i]).sqrt();
        }

        let inv_2p15_2pi = (1.0f64 / 16384.0 * PI as f64) as f32;
        for i in 1..self.poolsize / 2 {
            let phase = (random.rand() & 0x7fff) as f32 * inv_2p15_2pi;
            let magnitude = self.fft_freq[i];
            let s = magnitude * phase.sin();
            let c = magnitude * phase.cos();

            self.fft_c[i] = c;
            self.fft_c[self.poolsize - i] = c;
            self.fft_s[i] = s;
            self.fft_s[self.poolsize - i] = -s;
        }

        self.fft_c[0] = 0.0;
        self.fft_s[0] = 0.0;
        self.fft_c[self.poolsize / 2] = 0.0;
        self.fft_s[self.poolsize / 2] = 0.0;

        for i in 0..self.poolsize {
            self.inverse[i] = Complex::new(self.fft_c[i], self.fft_s[i]);
        }
        self.ifft_plan.process(&mut self.inverse);

        let scale = 1.0f32 / self.poolsize as f32;
        for i in 0..self.poolsize {
            self.fft_smps[i] = self.inverse[i].re * scale;
            self.fft_tmp[i] = self.inverse[i].im * scale;
        }

        let tmp = (1.0f64 / self.out_bufsize as f64 * std::f64::consts::PI) as f32;
        let hinv_sqrt2 = 0.853553390593f32;
        let ampfactor = if self.rap < 1.0 {
            self.rap * 0.707
        } else {
            (self.out_bufsize as f32 / self.poolsize as f32) * 4.0
        };

        for i in 0..self.out_bufsize {
            let a = (0.5 + 0.5 * ((i as f32 * tmp).cos() as f64)) as f32;
            let out = (self.fft_smps[i + self.out_bufsize] as f64 * (1.0 - a as f64)
                + self.old_out_smp_buf[i] as f64 * a as f64) as f32;
            let modulation = hinv_sqrt2 as f64
                - (1.0 - hinv_sqrt2 as f64)
                    * (i as f64 * 2.0 * tmp as f64).cos();
            self.out_buf[i] = (out as f64 * modulation * ampfactor as f64) as f32;
        }

        self.old_out_smp_buf
            .copy_from_slice(&self.fft_smps[..self.out_bufsize * 2]);
    }

    fn get_nsamples(&mut self) -> usize {
        // The C++ expression divides as float before assigning to double.
        let r = (self.out_bufsize as f32 / self.rap) as f64;
        let floor_r = r.floor();
        let mut ri = floor_r as usize;
        self.remained_samples += r - floor_r;

        if self.remained_samples >= 1.0 {
            let extra = self.remained_samples.floor();
            ri = ri.saturating_add(extra as usize);
            self.remained_samples -= extra;
        }

        ri.min(self.poolsize)
    }

    fn get_nsamples_for_fill(&self) -> usize {
        self.poolsize
    }
}

/// Linux glibc `rand()` with the default `srand(1)` TYPE_3 state.
struct GlibcRand {
    state: [u32; 31],
    front: usize,
    rear: usize,
}

impl GlibcRand {
    fn new() -> Self {
        let mut state = [0u32; 31];
        state[0] = 1;
        let mut word = 1i64;

        for slot in state.iter_mut().skip(1) {
            let high = word / 127_773;
            let low = word % 127_773;
            word = 16_807 * low - 2_836 * high;
            if word < 0 {
                word += 2_147_483_647;
            }
            *slot = word as u32;
        }

        let mut result = Self {
            state,
            front: 3,
            rear: 0,
        };
        for _ in 0..310 {
            result.next_value();
        }
        result
    }

    fn next_value(&mut self) -> u32 {
        let sum = self.state[self.front].wrapping_add(self.state[self.rear]);
        self.state[self.front] = sum;
        let value = (sum >> 1) & 0x7fff_ffff;

        self.front += 1;
        if self.front == self.state.len() {
            self.front = 0;
        }
        self.rear += 1;
        if self.rear == self.state.len() {
            self.rear = 0;
        }

        value
    }

    fn rand(&mut self) -> u32 {
        self.next_value()
    }
}

/// Returns this module's built-in effects.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(Paulstretch::default())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_buffer(frames: usize) -> AudioBuffer {
        let sample_rate = 44_100;
        let channel: Vec<f32> = (0..frames)
            .map(|i| {
                (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sample_rate as f32).sin()
            })
            .collect();
        AudioBuffer::from_channels(sample_rate, vec![channel.clone(), channel])
    }

    #[test]
    fn default_stereo_processing_does_not_panic() {
        let mut effect = Paulstretch::default();
        let mut buffer = sine_buffer(10_000);
        let mut context = EffectContext::default();
        let _ = effect.process(&mut buffer, &mut context);
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let mut effect = Paulstretch::default();
        let mut buffer = AudioBuffer::from_channels(44_100, vec![Vec::new(), Vec::new()]);
        let mut context = EffectContext::default();
        assert!(effect.process(&mut buffer, &mut context).is_ok());
        assert_eq!(buffer.len(), 0);
    }

    #[test]
    fn processing_is_repeatable() {
        let mut first = sine_buffer(10_000);
        let mut second = sine_buffer(10_000);
        let mut effect_a = Paulstretch::default();
        let mut effect_b = Paulstretch::default();
        let mut context_a = EffectContext::default();
        let mut context_b = EffectContext::default();

        effect_a.process(&mut first, &mut context_a).unwrap();
        effect_b.process(&mut second, &mut context_b).unwrap();
        assert_eq!(first.channels, second.channels);
    }

    #[test]
    fn parameters_clamp() {
        let mut effect = Paulstretch::default();
        assert!(set_param(&mut effect, "Stretch Factor", 0.0));
        assert_eq!(effect.stretch_factor, 1.0);
        assert!(set_param(&mut effect, "Time Resolution", f64::INFINITY));
        assert_eq!(effect.time_resolution, f32::MAX);
        assert!(!set_param(&mut effect, "unknown", 1.0));
    }

    #[test]
    fn glibc_rand_matches_default_sequence() {
        let mut random = GlibcRand::new();
        assert_eq!(random.rand(), 1_804_289_383);
        assert_eq!(random.rand(), 846_930_886);
        assert_eq!(random.rand(), 1_681_692_777);
    }
}
