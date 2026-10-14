// SPDX-License-Identifier: GPL-3.0-or-later
//! Audacity-compatible Beat Finder and Label Sounds analyzers.

use crate::core::*;
use au_core::AudioBuffer;

const MEASUREMENTS: &[&str] = &["Peak level", "Average level", "RMS level"];
const LABEL_TYPES: &[&str] = &[
    "Point before sound",
    "Point after sound",
    "Region around sounds",
    "Region between sounds",
];

/// Audacity's Beat Finder analyzer.
#[derive(Clone, Debug)]
pub struct BeatFinder {
    /// Threshold percentage, from 5 to 100.
    pub thresval: f64,
}

impl Default for BeatFinder {
    fn default() -> Self {
        Self { thresval: 65.0 }
    }
}

impl BuiltinEffect for BeatFinder {
    fn id(&self) -> &'static str {
        "BeatFinder"
    }

    fn name(&self) -> &'static str {
        "Beat Finder"
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Analyze
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![ParamDef::int(
            "THRESVAL",
            "Threshold Percentage",
            5.0,
            100.0,
            65.0,
        )]
    }

    fn get(&self, key: &str) -> Option<f64> {
        (key == "THRESVAL").then_some(self.thresval)
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        if key != "THRESVAL" {
            return false;
        }
        self.thresval = self.params()[0].clamp(value);
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.sample_rate == 0 {
            return Ok(());
        }
        let Some(mono) = mix_first_two_channels(buf) else {
            return Ok(());
        };
        if mono.is_empty() {
            return Ok(());
        }

        // These filters mirror Nyquist's `lp` calls. Nyquist's tone filter
        // keeps its recursive state in double precision and writes float
        // samples after each input sample.
        let bass = tone_lowpass(&mono, buf.sample_rate as f64, 50.0);
        let followed = follow_envelope(&bass, buf.sample_rate as f64);
        let control_rate = resample_linear(&followed, buf.sample_rate, 1000);
        let beats = tone_lowpass(&control_rate, 1000.0, 10.0);

        let peak = beats
            .iter()
            .fold(0.0_f64, |maximum, &sample| maximum.max((sample as f64).abs()));
        let threshold = (self.thresval / 100.0) * peak;

        let mut flag = true;
        // The forced 1000 Hz signal's sample-time mapping includes the
        // four-sample resampling origin used by Nyquist's forced-rate path.
        let mut time = 0.004_f64;
        for &value in &beats {
            let value = value as f64;
            if flag && value > threshold {
                ctx.labels.push(OutLabel {
                    t0: time,
                    t1: time,
                    text: "B".to_owned(),
                });
            }
            flag = value < threshold;
            time += 0.001;
        }

        Ok(())
    }
}

/// Audacity's Label Sounds analyzer.
#[derive(Clone, Debug)]
pub struct LabelSounds {
    /// Threshold level in dB.
    pub threshold: f64,
    /// Measurement mode: 0 = peak, 1 = average, 2 = RMS.
    pub measurement: f64,
    /// Minimum silence duration in seconds.
    pub sil_dur: f64,
    /// Minimum sound duration in seconds.
    pub snd_dur: f64,
    /// Label type: 0 = before, 1 = after, 2 = around, 3 = between.
    pub label_type: f64,
    /// Maximum leading silence to include in a label, in seconds.
    pub pre_offset: f64,
    /// Maximum trailing silence to include in a label, in seconds.
    pub post_offset: f64,
    /// Label text, which may contain a `##1`-style number placeholder.
    pub text: String,
}

impl Default for LabelSounds {
    fn default() -> Self {
        Self {
            threshold: -30.0,
            measurement: 0.0,
            sil_dur: 1.0,
            snd_dur: 1.0,
            label_type: 2.0,
            pre_offset: 0.0,
            post_offset: 0.0,
            text: "Sound ##1".to_owned(),
        }
    }
}

impl BuiltinEffect for LabelSounds {
    fn id(&self) -> &'static str {
        "LabelSounds"
    }

    fn name(&self) -> &'static str {
        "Label Sounds"
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Analyze
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("THRESHOLD", "Threshold level (dB)", -100.0, 0.0, -30.0),
            ParamDef::choice("MEASUREMENT", "Threshold measurement", MEASUREMENTS, 0),
            ParamDef::float("SIL-DUR", "Minimum silence duration", 0.01, 3600.0, 1.0),
            ParamDef::float("SND-DUR", "Minimum label interval", 0.01, 7200.0, 1.0),
            ParamDef::choice("TYPE", "Label type", LABEL_TYPES, 2),
            ParamDef::float("PRE-OFFSET", "Maximum leading silence", 0.0, f64::MAX, 0.0),
            ParamDef::float("POST-OFFSET", "Maximum trailing silence", 0.0, f64::MAX, 0.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "THRESHOLD" => Some(self.threshold),
            "MEASUREMENT" => Some(self.measurement),
            "SIL-DUR" => Some(self.sil_dur),
            "SND-DUR" => Some(self.snd_dur),
            "TYPE" => Some(self.label_type),
            "PRE-OFFSET" => Some(self.pre_offset),
            "POST-OFFSET" => Some(self.post_offset),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(definition) = self.params().into_iter().find(|param| param.key == key) else {
            return false;
        };
        let value = definition.clamp(value);
        match key {
            "THRESHOLD" => self.threshold = value,
            "MEASUREMENT" => self.measurement = value,
            "SIL-DUR" => self.sil_dur = value,
            "SND-DUR" => self.snd_dur = value,
            "TYPE" => self.label_type = value,
            "PRE-OFFSET" => self.pre_offset = value,
            "POST-OFFSET" => self.post_offset = value,
            _ => return false,
        }
        true
    }

    fn text_keys(&self) -> Vec<&'static str> {
        vec!["TEXT"]
    }

    fn get_text(&self, key: &str) -> Option<String> {
        (key == "TEXT").then(|| self.text.clone())
    }

    fn set_text(&mut self, key: &str, value: &str) -> bool {
        if key != "TEXT" {
            return false;
        }
        self.text = value.to_owned();
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.sample_rate == 0 {
            return Ok(());
        }

        let Some(reduced) = reduce_label_signal(buf, self.measurement as usize) else {
            return Ok(());
        };
        if reduced.samples.is_empty() || reduced.rate <= 0.0 {
            return Ok(());
        }

        let threshold = (self.threshold * (std::f64::consts::LN_10 / 20.0)).exp();
        let sounds = find_sound_intervals(
            &reduced.samples,
            reduced.rate,
            threshold,
            self.sil_dur,
            self.snd_dur,
        );
        if sounds.is_empty() {
            return Ok(());
        }

        let label_type = self.label_type as usize;
        if label_type == 3 && sounds.len() < 2 {
            return Ok(());
        }

        let (prefix, initial_number, suffix, digits) = parse_label_text(&self.text);
        let mut number = initial_number;
        let duration = buf.len() as f64 / buf.sample_rate as f64;
        let mut earliest_start = 0.0_f64;
        let mut labels = Vec::new();

        match label_type {
            3 => {
                for i in 0..sounds.len() - 1 {
                    let start_time = sounds[i].1;
                    let end_time = sounds[i + 1].0;

                    let mut start = end_time.min(start_time + self.pre_offset);
                    let mut end = start_time.max(end_time - self.post_offset);
                    if end - start < 0.0 {
                        start = (start + end) / 2.0;
                        end = start;
                    }

                    labels.push(OutLabel {
                        t0: start,
                        t1: end,
                        text: format_label_text(&prefix, number, digits, &suffix),
                    });
                    increment_label_number(&mut number);
                }
            }
            _ => {
                for (i, &(start_time, end_time)) in sounds.iter().enumerate() {
                    let label_start = 0.0_f64
                        .max(earliest_start)
                        .max(start_time - self.pre_offset);
                    let mut label_end = end_time + self.post_offset;
                    if let Some(next_sound) = sounds.get(i + 1) {
                        label_end = label_end.min(next_sound.0);
                    }

                    let (t0, t1) = match label_type {
                        0 => (label_start, label_start),
                        1 => (label_end, label_end),
                        _ => (label_start, label_end),
                    };
                    labels.push(OutLabel {
                        t0,
                        t1,
                        text: format_label_text(&prefix, number, digits, &suffix),
                    });

                    earliest_start = end_time;
                    increment_label_number(&mut number);
                }
            }
        }

        // Nyquist builds its label lists with `push`, so the returned list is
        // reverse chronological. Expose labels in Audacity's chronological order.
        labels.sort_by(|a, b| a.t0.total_cmp(&b.t0).then_with(|| a.t1.total_cmp(&b.t1)));

        // `duration` is the end of the selected sound; it is intentionally
        // not used to clip offsets, matching Label Sounds' Nyquist code.
        let _ = duration;
        ctx.labels.extend(labels);
        Ok(())
    }
}

struct ReducedSignal {
    samples: Vec<f32>,
    rate: f64,
}

fn mix_first_two_channels(buf: &AudioBuffer) -> Option<Vec<f32>> {
    let first = buf.channels.first()?;
    let Some(second) = buf.channels.get(1) else {
        return Some(first.clone());
    };
    let frames = first.len().min(second.len());
    Some(
        first[..frames]
            .iter()
            .zip(&second[..frames])
            .map(|(&left, &right)| left + right)
            .collect(),
    )
}

fn tone_lowpass(input: &[f32], sample_rate: f64, cutoff: f64) -> Vec<f32> {
    if input.is_empty() || sample_rate <= 0.0 {
        return Vec::new();
    }

    let b = 2.0 - (cutoff * std::f64::consts::TAU / sample_rate).cos();
    let c2 = b - ((b * b) - 1.0).sqrt();
    let c1 = 1.0 - c2;
    let mut previous = 0.0_f64;

    input
        .iter()
        .map(|&sample| {
            previous = c1 * sample as f64 + c2 * previous;
            previous as f32
        })
        .collect()
}

fn follow_envelope(input: &[f32], sample_rate: f64) -> Vec<f32> {
    if input.is_empty() || sample_rate <= 0.0 {
        return Vec::new();
    }

    let attack = (-1.0 / (0.01 * sample_rate)).exp();
    let release = (-1.0 / (0.1 * sample_rate)).exp();
    let mut envelope = 0.001_f64;

    input
        .iter()
        .map(|&sample| {
            let level = (sample as f64).abs().max(0.001);
            let coefficient = if level > envelope { attack } else { release };
            envelope = coefficient * envelope + (1.0 - coefficient) * level;
            envelope.max(0.001) as f32
        })
        .collect()
}

fn resample_linear(input: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if input.is_empty() || source_rate == 0 || target_rate == 0 {
        return Vec::new();
    }

    let output_len =
        ((input.len() as f64 * target_rate as f64 / source_rate as f64).round() as usize).max(1);
    let ratio = source_rate as f64 / target_rate as f64;

    (0..output_len)
        .map(|i| {
            let position = i as f64 * ratio;
            let index = position.floor() as usize;
            let fraction = (position - index as f64) as f32;
            let a = input[index.min(input.len() - 1)];
            let b = input[(index + 1).min(input.len() - 1)];
            a + (b - a) * fraction
        })
        .collect()
}

fn reduce_label_signal(buf: &AudioBuffer, measurement: usize) -> Option<ReducedSignal> {
    let first = buf.channels.first()?;
    let channel_count = buf.channels.len().min(2);
    let frames = buf
        .channels
        .iter()
        .take(channel_count)
        .map(Vec::len)
        .min()
        .unwrap_or(0);

    // Nyquist uses ROUND(*sound-srate* / 100) for both block and step size.
    let ratio = ((buf.sample_rate as f64 / 100.0).round() as usize).max(1);
    let output_count = frames / ratio;
    let mut output = Vec::with_capacity(output_count);

    for block in 0..output_count {
        let start = block * ratio;
        let end = start + ratio;

        let value = match measurement {
            0 => {
                let mut peak = 0.0_f32;
                for i in start..end {
                    let sample = if channel_count > 1 {
                        first[i].abs().max(buf.channels[1][i].abs())
                    } else {
                        first[i].abs()
                    };
                    if sample > peak {
                        peak = sample;
                    }
                }
                peak
            }
            1 => {
                let mut sum = 0.0_f64;
                for i in start..end {
                    let sample = if channel_count > 1 {
                        (first[i].abs() + buf.channels[1][i].abs()) * 0.5
                    } else {
                        first[i].abs()
                    };
                    sum += sample as f64;
                }
                (sum / ratio as f64) as f32
            }
            _ if channel_count > 1 => {
                let mut left_sum = 0.0_f64;
                let mut right_sum = 0.0_f64;
                for i in start..end {
                    let left = first[i] * first[i];
                    let right = buf.channels[1][i] * buf.channels[1][i];
                    left_sum += left as f64;
                    right_sum += right as f64;
                }

                let left_mean = (left_sum / ratio as f64) as f32;
                let right_mean = (right_sum / ratio as f64) as f32;
                let mean = (left_mean + right_mean) * 0.5_f32;
                mean.sqrt()
            }
            _ => {
                let mut sum = 0.0_f64;
                for &sample in &first[start..end] {
                    sum += (sample * sample) as f64;
                }
                (sum / ratio as f64).sqrt() as f32
            }
        };
        output.push(value);
    }

    Some(ReducedSignal {
        samples: output,
        rate: buf.sample_rate as f64 / ratio as f64,
    })
}

fn find_sound_intervals(
    samples: &[f32],
    sample_rate: f64,
    threshold: f64,
    silence_duration: f64,
    sound_duration: f64,
) -> Vec<(f64, f64)> {
    let sound_samples = sound_duration * sample_rate;
    let silence_samples = silence_duration * sample_rate;
    let mut sample_count = 0_usize;
    let mut silence_count = 0_usize;
    let mut sound_count = 0_usize;
    let mut sound_start = 0_usize;
    let mut intervals = Vec::new();

    for &value in samples {
        if (value as f64) < threshold {
            if silence_count as f64 >= silence_samples && sound_count as f64 >= sound_samples {
                intervals.push((
                    sound_start as f64 / sample_rate,
                    sample_count.saturating_sub(silence_count) as f64 / sample_rate,
                ));
                sound_count = 0;
                if intervals.len() == 10_000 {
                    return intervals;
                }
            }
            if sound_count > 0 {
                sound_count += 1;
            }
            silence_count += 1;
        } else {
            if sound_count == 0 {
                sound_start = sample_count;
            }
            silence_count = 0;
            sound_count += 1;
        }
        sample_count += 1;
    }

    if sound_count > 0 && intervals.len() < 10_000 {
        intervals.push((
            sound_start as f64 / sample_rate,
            sample_count.saturating_sub(silence_count) as f64 / sample_rate,
        ));
    }

    intervals
}

fn parse_label_text(text: &str) -> (String, Option<i64>, String, usize) {
    let chars: Vec<char> = text.chars().collect();
    let Some(hash_start) = chars.iter().position(|&character| character == '#') else {
        return (text.to_owned(), None, String::new(), 0);
    };

    let mut hash_end = hash_start;
    while hash_end < chars.len() && chars[hash_end] == '#' {
        hash_end += 1;
    }
    let digits = hash_end - hash_start;
    let mut position = hash_end;
    let mut negative = false;

    if position < chars.len() && chars[position] == '-' {
        negative = true;
        position += 1;
    }

    let digit_start = position;
    let mut number = 0_i64;
    while position < chars.len() && chars[position].is_ascii_digit() {
        number = number
            .saturating_mul(10)
            .saturating_add(chars[position].to_digit(10).unwrap_or(0) as i64);
        position += 1;
    }

    if position == digit_start {
        // Without a number, the hash run is literal text.
        return (
            chars[..hash_start].iter().collect(),
            None,
            chars[hash_start..].iter().collect(),
            0,
        );
    }

    if negative {
        number = -number;
    }

    (
        chars[..hash_start].iter().collect(),
        Some(number),
        chars[position..].iter().collect(),
        digits,
    )
}

fn format_label_text(
    prefix: &str,
    number: Option<i64>,
    digits: usize,
    suffix: &str,
) -> String {
    let Some(number) = number else {
        return format!("{prefix}{suffix}");
    };

    let negative = number < 0;
    let mut number_text = number.unsigned_abs().to_string();
    while number_text.len() < digits {
        number_text.insert(0, '0');
    }
    if negative {
        number_text.insert(0, '-');
    }

    format!("{prefix}{number_text}{suffix}")
}

fn increment_label_number(number: &mut Option<i64>) {
    if let Some(number) = number {
        *number = number.saturating_add(1);
    }
}

/// Returns the default Beat Finder and Label Sounds analyzers.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(BeatFinder::default()),
        Box::new(LabelSounds::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{set_param, EffectContext};

    fn sine_buffer(frames: usize) -> AudioBuffer {
        let sample_rate = 44_100;
        let left: Vec<f32> = (0..frames)
            .map(|i| {
                (std::f32::consts::TAU * 440.0 * i as f32 / sample_rate as f32).sin() * 0.2
            })
            .collect();
        let right: Vec<f32> = left.iter().map(|sample| sample * 0.8).collect();
        AudioBuffer::from_channels(sample_rate, vec![left, right])
    }

    #[test]
    fn analyzers_run_on_short_stereo_audio_without_modifying_it() {
        let mut buffer = sine_buffer(4096);
        let original = buffer.channels.clone();
        let mut context = EffectContext::default();

        BeatFinder::default()
            .process(&mut buffer, &mut context)
            .unwrap();
        LabelSounds::default()
            .process(&mut buffer, &mut context)
            .unwrap();

        assert_eq!(buffer.channels, original);
    }

    #[test]
    fn empty_buffers_do_not_panic() {
        let mut buffer = AudioBuffer::from_channels(44_100, vec![Vec::new(), Vec::new()]);
        let mut context = EffectContext::default();

        BeatFinder::default()
            .process(&mut buffer, &mut context)
            .unwrap();
        LabelSounds::default()
            .process(&mut buffer, &mut context)
            .unwrap();

        assert!(context.labels.is_empty());
    }

    #[test]
    fn analyzer_results_are_repeatable() {
        let buffer = sine_buffer(44_100);
        let mut first = buffer.clone();
        let mut second = buffer;
        let mut first_context = EffectContext::default();
        let mut second_context = EffectContext::default();

        LabelSounds::default()
            .process(&mut first, &mut first_context)
            .unwrap();
        LabelSounds::default()
            .process(&mut second, &mut second_context)
            .unwrap();

        assert_eq!(first_context.labels, second_context.labels);
    }

    #[test]
    fn parameters_clamp_to_their_ranges() {
        let mut beat = BeatFinder::default();
        assert!(set_param(&mut beat, "THRESVAL", 500.0));
        assert_eq!(beat.thresval, 100.0);

        let mut sounds = LabelSounds::default();
        assert!(set_param(&mut sounds, "THRESHOLD", -500.0));
        assert_eq!(sounds.threshold, -100.0);
        assert!(set_param(&mut sounds, "TYPE", 99.0));
        assert_eq!(sounds.label_type, 3.0);
    }
}
