use crate::core::*;
use au_core::AudioBuffer;

/// Audacity's Auto Duck effect.
#[derive(Clone, Debug)]
pub struct AutoDuck {
    /// Duck gain in decibels.
    pub duck_amount_db: f64,
    /// Inner fade-down duration in seconds.
    pub inner_fade_down_len: f64,
    /// Inner fade-up duration in seconds.
    pub inner_fade_up_len: f64,
    /// Outer fade-down duration in seconds.
    pub outer_fade_down_len: f64,
    /// Outer fade-up duration in seconds.
    pub outer_fade_up_len: f64,
    /// Control-track threshold in decibels.
    pub threshold_db: f64,
    /// Maximum pause in the control track before a duck region ends.
    pub maximum_pause: f64,
}

impl Default for AutoDuck {
    fn default() -> Self {
        Self {
            duck_amount_db: -12.0,
            inner_fade_down_len: 0.0,
            inner_fade_up_len: 0.0,
            outer_fade_down_len: 0.5,
            outer_fade_up_len: 0.5,
            threshold_db: -30.0,
            maximum_pause: 1.0,
        }
    }
}

impl AutoDuck {
    fn definitions() -> Vec<ParamDef> {
        vec![
            ParamDef::float("DuckAmountDb", "Duck Amount", -24.0, 0.0, -12.0),
            ParamDef::float("InnerFadeDownLen", "Inner Fade Down Length", 0.0, 3.0, 0.0),
            ParamDef::float("InnerFadeUpLen", "Inner Fade Up Length", 0.0, 3.0, 0.0),
            ParamDef::float("OuterFadeDownLen", "Outer Fade Down Length", 0.0, 3.0, 0.5),
            ParamDef::float("OuterFadeUpLen", "Outer Fade Up Length", 0.0, 3.0, 0.5),
            ParamDef::float("ThresholdDb", "Threshold", -100.0, 0.0, -30.0),
            ParamDef::float("MaximumPause", "Maximum Pause", 0.0, f64::MAX, 1.0),
        ]
    }

    fn set_value(&mut self, key: &str, value: f64) -> bool {
        match key {
            "DuckAmountDb" => self.duck_amount_db = value,
            "InnerFadeDownLen" => self.inner_fade_down_len = value,
            "InnerFadeUpLen" => self.inner_fade_up_len = value,
            "OuterFadeDownLen" => self.outer_fade_down_len = value,
            "OuterFadeUpLen" => self.outer_fade_up_len = value,
            "ThresholdDb" => self.threshold_db = value,
            "MaximumPause" => self.maximum_pause = value,
            _ => return false,
        }
        true
    }

    fn get_value(&self, key: &str) -> Option<f64> {
        Some(match key {
            "DuckAmountDb" => self.duck_amount_db,
            "InnerFadeDownLen" => self.inner_fade_down_len,
            "InnerFadeUpLen" => self.inner_fade_up_len,
            "OuterFadeDownLen" => self.outer_fade_down_len,
            "OuterFadeUpLen" => self.outer_fade_up_len,
            "ThresholdDb" => self.threshold_db,
            "MaximumPause" => self.maximum_pause,
            _ => return None,
        })
    }
}

impl BuiltinEffect for AutoDuck {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "AutoDuck"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        Self::definitions()
    }

    fn get(&self, key: &str) -> Option<f64> {
        self.get_value(key)
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        self.set_value(key, value)
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let aux = ctx
            .aux
            .ok_or_else(|| EffectError::Invalid("Auto Duck requires a control track".into()));
        if buf.len() == 0 { return Ok(()); }
        let aux = aux?;
        let n = buf.len();
        if n == 0 || buf.channels.is_empty() {
            return Ok(());
        }

        let rate = buf.sample_rate as f64;
        let control_len = aux.channels.iter().map(Vec::len).min().unwrap_or(0);
        let control: Vec<f32> = (0..control_len)
            .map(|i| {
                if aux.channels.is_empty() {
                    0.0
                } else {
                    (aux.channels.iter().map(|c| c[i] as f64).sum::<f64>()
                        / aux.channels.len() as f64) as f32
                }
            })
            .collect();

        let start = (self.outer_fade_down_len * rate).round() as usize;
        let end = n
            .min((control_len as f64 - self.outer_fade_up_len * rate).max(0.0) as usize);
        if end <= start {
            return Ok(());
        }

        let threshold = 10.0_f64.powf(self.threshold_db / 20.0);
        let threshold_sum = threshold * threshold * 100.0;
        let max_pause = self
            .maximum_pause
            .max(self.outer_fade_down_len + self.outer_fade_up_len);
        let min_pause = (max_pause * rate).round() as usize;

        let mut window = [0.0f64; 100];
        let mut rms_sum = 0.0;
        let mut rms_pos = 0usize;
        let mut in_region = false;
        let mut region_start = 0usize;
        let mut pause = 0usize;
        let mut regions = Vec::new();

        for i in start..end {
            rms_sum -= window[rms_pos];
            let sample = control.get(i).copied().unwrap_or(0.0) as f64;
            window[rms_pos] = sample * sample;
            rms_sum += window[rms_pos];
            rms_pos = (rms_pos + 1) % 100;
            let exceeded = rms_sum > threshold_sum;

            if exceeded {
                pause = 0;
                if !in_region {
                    in_region = true;
                    region_start = i;
                }
            } else if in_region {
                pause += 1;
                if pause >= min_pause {
                    let region_end = i.saturating_sub(pause);
                    regions.push((
                        region_start as f64 / rate - self.outer_fade_down_len,
                        region_end as f64 / rate + self.outer_fade_up_len,
                    ));
                    in_region = false;
                }
            }
        }
        if in_region {
            let region_end = end.saturating_sub(pause);
            regions.push((
                region_start as f64 / rate - self.outer_fade_down_len,
                region_end as f64 / rate + self.outer_fade_up_len,
            ));
        }

        for channel in &mut buf.channels {
            for &(t0, t1) in &regions {
                let region_start = (t0 * rate).round() as isize;
                let region_end = (t1 * rate).round() as isize;
                let first = region_start.max(0) as usize;
                let last = (region_end.max(0) as usize).min(channel.len());
                if last <= first {
                    continue;
                }
                let down_samples = ((self.outer_fade_down_len + self.inner_fade_down_len)
                    * rate)
                    .round()
                    .max(1.0);
                let up_samples = ((self.outer_fade_up_len + self.inner_fade_up_len) * rate)
                    .round()
                    .max(1.0);
                for (i, sample) in channel.iter_mut().enumerate().take(last).skip(first) {
                    let down = self.duck_amount_db as f32
                        * ((i as isize - region_start) as f32 / down_samples as f32);
                    let up = self.duck_amount_db as f32
                        * ((region_end - i as isize) as f32 / up_samples as f32);
                    let gain_db = down.max(up).max(self.duck_amount_db as f32);
                    *sample *= 10.0_f32.powf(gain_db / 20.0);
                }
            }
        }
        Ok(())
    }
}

/// Audacity's Legacy Compressor effect.
#[derive(Clone, Debug)]
pub struct LegacyCompressor {
    /// Compression threshold in decibels.
    pub threshold: f64,
    /// Noise-floor level in decibels.
    pub noise_floor: f64,
    /// Compression ratio.
    pub ratio: f64,
    /// Attack time in seconds.
    pub attack_time: f64,
    /// Release time in seconds.
    pub release_time: f64,
    /// Normalize the result in a second pass.
    pub normalize: bool,
    /// Use peak detection instead of RMS detection.
    pub use_peak: bool,
}

impl Default for LegacyCompressor {
    fn default() -> Self {
        Self {
            threshold: -12.0,
            noise_floor: -40.0,
            ratio: 2.0,
            attack_time: 0.2,
            release_time: 1.0,
            normalize: true,
            use_peak: false,
        }
    }
}

impl LegacyCompressor {
    fn definitions() -> Vec<ParamDef> {
        vec![
            ParamDef::float("Threshold", "Threshold", -60.0, -1.0, -12.0),
            ParamDef::float("NoiseFloor", "Noise Floor", -80.0, -20.0, -40.0),
            ParamDef::float("Ratio", "Ratio", 1.1, 10.0, 2.0),
            ParamDef::float("AttackTime", "Attack Time", 0.1, 5.0, 0.2),
            ParamDef::float("ReleaseTime", "Release Time", 1.0, 30.0, 1.0),
            ParamDef::boolean("Normalize", "Normalize", true),
            ParamDef::boolean("UsePeak", "Use Peak", false),
        ]
    }

    fn set_value(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Threshold" => self.threshold = value,
            "NoiseFloor" => self.noise_floor = value,
            "Ratio" => self.ratio = value,
            "AttackTime" => self.attack_time = value,
            "ReleaseTime" => self.release_time = value,
            "Normalize" => self.normalize = value != 0.0,
            "UsePeak" => self.use_peak = value != 0.0,
            _ => return false,
        }
        true
    }

    fn get_value(&self, key: &str) -> Option<f64> {
        Some(match key {
            "Threshold" => self.threshold,
            "NoiseFloor" => self.noise_floor,
            "Ratio" => self.ratio,
            "AttackTime" => self.attack_time,
            "ReleaseTime" => self.release_time,
            "Normalize" => self.normalize as u8 as f64,
            "UsePeak" => self.use_peak as u8 as f64,
            _ => return None,
        })
    }

    fn compress_channel(&self, channel: &mut [f32], rate: f64, max_seen: &mut f64) {
        let threshold = 10.0_f64.powf(self.threshold / 20.0);
        let noise_floor = 10.0_f64.powf(self.noise_floor / 20.0);
        let attack_inverse = (threshold.ln() / (rate * self.attack_time + 0.5)).exp();
        let attack_factor = 1.0 / attack_inverse;
        let decay_factor = (threshold.ln() / (rate * self.release_time + 0.5)).exp();
        let compression = if self.ratio > 1.0 {
            1.0 - 1.0 / self.ratio
        } else {
            0.0
        };

        let mut circle = [0.0f64; 100];
        let mut circle_pos = 0usize;
        let mut rms_sum = 0.0f64;
        let mut last = threshold;
        for &sample in channel.iter() {
            if last < (sample as f64).abs() {
                last = (sample as f64).abs();
            }
        }
        let mut noise_counter = 100i32;
        let mut envelope = vec![0.0f32; channel.len()];

        for (i, &sample) in channel.iter().enumerate() {
            let level = if self.use_peak {
                (sample as f64).abs()
            } else {
                rms_sum -= circle[circle_pos];
                let sq = (sample as f64) * (sample as f64);
                circle[circle_pos] = sq;
                rms_sum += sq;
                circle_pos = (circle_pos + 1) % 100;
                (rms_sum / 100.0).sqrt()
            };
            if level < noise_floor {
                noise_counter += 1;
            } else {
                noise_counter = 0;
            }
            if noise_counter < 100 {
                last *= decay_factor;
                if last < threshold {
                    last = threshold;
                }
                if level > last {
                    last = level;
                }
            }
            envelope[i] = last as f32;
        }

        for i in (0..envelope.len()).rev() {
            last *= attack_inverse;
            if last < threshold {
                last = threshold;
            }
            if (envelope[i] as f64) < last {
                envelope[i] = last as f32;
            } else {
                last = envelope[i] as f64;
            }
        }

        for (sample, &env) in channel.iter_mut().zip(&envelope) {
            let env = env as f64;
            let factor = if self.use_peak {
                (1.0 / env).powf(compression)
            } else {
                (threshold / env).powf(compression)
            };
            let out = *sample * factor as f32;
            *max_seen = max_seen.max((out as f64).abs());
            *sample = out;
        }

        // Keep the attack factor used above available in the same precision as
        // the original envelope algorithm.
        let _ = attack_factor;
    }
}

impl BuiltinEffect for LegacyCompressor {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "LegacyCompressor"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        Self::definitions()
    }

    fn get(&self, key: &str) -> Option<f64> {
        self.get_value(key)
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        self.set_value(key, value)
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.len() == 0 {
            return Ok(());
        }
        let mut max_seen = 0.0f64;
        for channel in &mut buf.channels {
            self.compress_channel(channel, buf.sample_rate as f64, &mut max_seen);
        }
        if self.normalize && max_seen != 0.0 {
            for channel in &mut buf.channels {
                for sample in channel {
                    *sample /= max_seen as f32;
                }
            }
        }
        Ok(())
    }
}

/// Returns the built-in effects implemented by this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(AutoDuck::default()), Box::new(LegacyCompressor::default())]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine() -> AudioBuffer {
        let rate = 8000;
        let left: Vec<f32> = (0..256)
            .map(|i| (i as f32 * 0.08).sin() * 0.5)
            .collect();
        let right = left.iter().map(|x| x * 0.7).collect();
        AudioBuffer::from_channels(rate, vec![left, right])
    }

    fn empty() -> AudioBuffer {
        AudioBuffer::from_channels(8000, vec![Vec::new(), Vec::new()])
    }

    #[test]
    fn auto_duck_smoke_empty_repeatability_and_params() {
        let mut effect = AutoDuck::default();
        assert!(set_param(&mut effect, "DuckAmountDb", -100.0));
        assert_eq!(effect.duck_amount_db, -24.0);
        assert!(set_param(&mut effect, "ThresholdDb", 10.0));
        assert_eq!(effect.threshold_db, 0.0);

        let mut a = sine();
        let mut b = a.clone();
        let control = sine();
        let mut ca = EffectContext { aux: Some(&control), ..Default::default() };
        let mut cb = EffectContext { aux: Some(&control), ..Default::default() };
        effect.process(&mut a, &mut ca).unwrap();
        effect.process(&mut b, &mut cb).unwrap();
        assert_eq!(a.channels, b.channels);

        let mut empty = empty();
        effect.process(&mut empty, &mut EffectContext::default()).unwrap();
        assert!(effect.process(&mut a, &mut EffectContext::default()).is_err());
    }

    #[test]
    fn auto_duck_reduces_gain() {
        let mut effect = AutoDuck::default();
        effect.outer_fade_down_len = 0.0;
        effect.outer_fade_up_len = 0.0;
        effect.maximum_pause = 0.0;
        effect.threshold_db = -60.0;
        let mut audio = AudioBuffer::from_channels(8000, vec![vec![1.0; 256]]);
        let control = AudioBuffer::from_channels(8000, vec![vec![1.0; 256]]);
        effect.process(
            &mut audio,
            &mut EffectContext { aux: Some(&control), ..Default::default() },
        ).unwrap();
        assert!(audio.channels[0][200] < 1.0);
    }

    #[test]
    fn compressor_smoke_empty_repeatability_and_params() {
        let mut effect = LegacyCompressor::default();
        assert!(set_param(&mut effect, "Ratio", 100.0));
        assert_eq!(effect.ratio, 10.0);

        let mut a = sine();
        let mut b = a.clone();
        effect.process(&mut a, &mut EffectContext::default()).unwrap();
        effect.process(&mut b, &mut EffectContext::default()).unwrap();
        assert_eq!(a.channels, b.channels);

        let mut empty = empty();
        effect.process(&mut empty, &mut EffectContext::default()).unwrap();
    }

    #[test]
    fn compressor_normalizes_output() {
        let mut effect = LegacyCompressor::default();
        let mut audio = AudioBuffer::from_channels(8000, vec![vec![0.25; 128]]);
        effect.process(&mut audio, &mut EffectContext::default()).unwrap();
        let peak = audio.channels[0].iter().fold(0.0f32, |a, &b| a.max(b.abs()));
        assert!((peak - 1.0).abs() < 1e-5);
    }
}
