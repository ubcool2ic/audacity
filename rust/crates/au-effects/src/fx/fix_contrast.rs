// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Contrast analyzer, measuring RMS level differences for WCAG 2.

use crate::core::*;
use au_core::AudioBuffer;
use std::time::{SystemTime, UNIX_EPOCH};

const INVALID_SELECTION: &str =
    "Invalid audio selection.\nPlease ensure that audio is selected.";
const NOTHING_TO_MEASURE: &str =
    "Nothing to measure.\nPlease select a section of a track.";

#[derive(Clone, Copy, Debug, PartialEq)]
/// Measurements and WCAG 2 result for a foreground/background pair.
pub struct ContrastResult {
    /// Foreground RMS level in dB.
    pub fg_db: f32,
    /// Background RMS level in dB.
    pub bg_db: f32,
    /// Signed difference, `fg_db - bg_db`, in dB.
    pub diff_db: f32,
    /// Whether the signed difference is strictly greater than 20 dB.
    pub pass: bool,
}

/// Measures RMS over a time range, combining channels by the mean of their
/// squared RMS values.
///
/// Times are seconds from the beginning of `buf`. They are converted to sample
/// indices using `floor(time * sample_rate + 0.5)`, as in Audacity's
/// `TimeToLongSamples`.
///
/// # Errors
///
/// Returns Audacity's Contrast error messages when there are no channels, the
/// selection is invalid, or no samples fall within the requested range.
pub fn measure(buf: &AudioBuffer, t0: f64, t1: f64) -> Result<f32, String> {
    if buf.channels.is_empty() {
        return Err("Please select an audio track.".to_string());
    }
    if !t0.is_finite() || !t1.is_finite() || t0 > t1 {
        return Err(INVALID_SELECTION.to_string());
    }

    let sample_rate = buf.sample_rate as f64;
    if sample_rate == 0.0 {
        return Err(NOTHING_TO_MEASURE.to_string());
    }

    let frame_count = buf.len();
    let duration = frame_count as f64 / sample_rate;
    let start_time = t0.max(0.0).min(duration);
    let end_time = t1.max(0.0).min(duration);

    let start = (start_time * sample_rate + 0.5).floor() as usize;
    let end = (end_time * sample_rate + 0.5).floor() as usize;

    if start > end {
        return Err(INVALID_SELECTION.to_string());
    }
    if start == end {
        return Err(NOTHING_TO_MEASURE.to_string());
    }

    let mut mean_sq = 0.0_f64;
    for channel in &buf.channels {
        let channel_start = start.min(channel.len());
        let channel_end = end.min(channel.len());
        if channel_start > channel_end {
            return Err(INVALID_SELECTION.to_string());
        }
        if channel_start == channel_end {
            return Err(NOTHING_TO_MEASURE.to_string());
        }

        let mut sum_sq = 0.0_f64;
        for &sample in &channel[channel_start..channel_end] {
            let sample = sample as f64;
            sum_sq += sample * sample;
        }

        let count = (channel_end - channel_start) as f64;
        let channel_rms = (sum_sq / count).sqrt() as f32;

        // ContrastBase accumulates rms * rms, where both operands are float,
        // into its double-precision meanSq.
        let channel_mean_sq: f32 = channel_rms * channel_rms;
        mean_sq += channel_mean_sq as f64;
    }

    let rms = if mean_sq > 0.0 {
        (mean_sq / buf.channels.len() as f64).sqrt() as f32
    } else {
        0.0_f32
    };

    Ok(if rms == 0.0 {
        f32::NEG_INFINITY
    } else {
        (20.0_f64 * (rms as f64).log10()) as f32
    })
}

/// Computes the signed foreground-minus-background difference and WCAG 2 result.
pub fn analyze(fg_db: f32, bg_db: f32) -> ContrastResult {
    let diff_db = fg_db - bg_db;
    ContrastResult {
        fg_db,
        bg_db,
        diff_db,
        pass: diff_db > 20.0,
    }
}

/// Audacity's Contrast analyzer.
#[derive(Clone, Debug)]
pub struct Contrast {
    /// Foreground range start, in seconds.
    pub fg_start: f64,
    /// Foreground range end, in seconds.
    pub fg_end: f64,
    /// Background range start, in seconds.
    pub bg_start: f64,
    /// Background range end, in seconds.
    pub bg_end: f64,
}

impl Default for Contrast {
    fn default() -> Self {
        Self {
            fg_start: 0.0,
            fg_end: 0.0,
            bg_start: 0.0,
            bg_end: 0.0,
        }
    }
}

impl Contrast {
    fn range_is_set(start: f64, end: f64) -> bool {
        end > start
    }

    fn measure_range(
        buf: &AudioBuffer,
        start: f64,
        end: f64,
        origin: f64,
        use_range: bool,
    ) -> Result<f32, String> {
        let duration = if buf.sample_rate == 0 {
            0.0
        } else {
            buf.len() as f64 / buf.sample_rate as f64
        };

        if use_range {
            measure(buf, start - origin, end - origin)
        } else {
            measure(buf, 0.0, duration)
        }
    }

    fn rms_text(value: Option<f32>) -> String {
        match value {
            None => "RMS =  dB.".to_string(),
            Some(value) if value.abs() == f32::INFINITY => "RMS = zero.".to_string(),
            Some(value) => format!("RMS = {:.2} dB.", value),
        }
    }

    fn difference_text(diff_db: f32) -> String {
        if diff_db.is_nan() {
            "Difference is indeterminate.".to_string()
        } else if diff_db.abs() == f32::INFINITY {
            "Difference = infinite RMS dB.".to_string()
        } else {
            format!("Difference = {:.2} RMS dB.", diff_db)
        }
    }

    fn format_time(time: f64) -> String {
        // Contrast.cpp stores each exported time in a float before splitting it
        // into hours, minutes, and seconds.
        let t = time as f32;
        let hours = (t / 3600.0) as i32;
        let minutes = ((t - hours as f32 * 3600.0) / 60.0) as i32;
        let seconds =
            (t as f64 - hours as f64 * 3600.0 - minutes as f64 * 60.0) as f32;

        format!(
            "{:2} hour(s), {:2} minute(s), {:.2} seconds.",
            hours, minutes, seconds
        )
    }

    fn gathered_timestamp() -> String {
        let seconds_since_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs() as i64)
            .unwrap_or(0);
        let days = seconds_since_epoch.div_euclid(86_400);
        let seconds_of_day = seconds_since_epoch.rem_euclid(86_400);

        // Gregorian civil date from days since 1970-01-01.
        let z = days + 719_468;
        let era = if z >= 0 {
            z / 146_097
        } else {
            (z - 146_096) / 146_097
        };
        let day_of_era = z - era * 146_097;
        let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
            - day_of_era / 146_096)
            / 365;
        let mut year = year_of_era + era * 400;
        let day_of_year =
            day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_prime = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
        let month = month_prime + if month_prime < 10 { 3 } else { -9 };
        if month <= 2 {
            year += 1;
        }

        let month_name = match month {
            1 => "January",
            2 => "February",
            3 => "March",
            4 => "April",
            5 => "May",
            6 => "June",
            7 => "July",
            8 => "August",
            9 => "September",
            10 => "October",
            11 => "November",
            _ => "December",
        };
        let hour = seconds_of_day / 3_600;
        let minute = seconds_of_day % 3_600 / 60;
        let second = seconds_of_day % 60;

        format!(
            "{} {} {:02} {:02}h {:02}m {:02}s",
            day, month_name, year, hour, minute, second
        )
    }

    fn export_report(
        fg_start: f64,
        fg_end: f64,
        bg_start: f64,
        bg_end: f64,
        fg_db: f32,
        bg_db: f32,
        diff_db: f32,
    ) -> String {
        let pass_line = if diff_db > 20.0 {
            "Success Criteria 1.4.7 of WCAG 2.0: Pass"
        } else {
            "Success Criteria 1.4.7 of WCAG 2.0: Fail"
        };

        format!(
            "===================================\n\
WCAG 2.0 Success Criteria 1.4.7 Contrast Results\n\
\n\
Filename = .\n\
\n\
Foreground\n\
Time started = {}\n\
Time ended = {}\n\
{}\n\
\n\
Background\n\
Time started = {}\n\
Time ended = {}\n\
{}\n\
\n\
Results\n\
{}\n\
{}\n\
\n\
Data gathered\n\
{}\n\
===================================\n\n",
            Self::format_time(fg_start),
            Self::format_time(fg_end),
            Self::rms_text(Some(fg_db)),
            Self::format_time(bg_start),
            Self::format_time(bg_end),
            Self::rms_text(Some(bg_db)),
            Self::difference_text(diff_db),
            pass_line,
            Self::gathered_timestamp(),
        )
    }
}

impl BuiltinEffect for Contrast {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Contrast"
    }) }

    fn name(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Contrast"
    }) }

    fn kind(&self) -> EffectKind {
        EffectKind::Analyze
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("FgStart", "Foreground start", 0.0, f64::MAX, 0.0),
            ParamDef::float("FgEnd", "Foreground end", 0.0, f64::MAX, 0.0),
            ParamDef::float("BgStart", "Background start", 0.0, f64::MAX, 0.0),
            ParamDef::float("BgEnd", "Background end", 0.0, f64::MAX, 0.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "FgStart" => Some(self.fg_start),
            "FgEnd" => Some(self.fg_end),
            "BgStart" => Some(self.bg_start),
            "BgEnd" => Some(self.bg_end),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let definition = match self.params().into_iter().find(|param| param.key == key) {
            Some(definition) => definition,
            None => return false,
        };
        let value = definition.clamp(value);
        match key {
            "FgStart" => self.fg_start = value,
            "FgEnd" => self.fg_end = value,
            "BgStart" => self.bg_start = value,
            "BgEnd" => self.bg_end = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if buf.channels.is_empty() {
            return Err(EffectError::Invalid(
                "Please select an audio track.".to_string(),
            ));
        }

        let fg_has_range = Self::range_is_set(self.fg_start, self.fg_end);
        let fg_db = Self::measure_range(
            buf,
            self.fg_start,
            self.fg_end,
            ctx.t0,
            fg_has_range,
        )
        .map_err(EffectError::Invalid)?;

        let (bg_db, bg_start, bg_end) = if let Some(aux) = ctx.aux {
            let bg_has_range = Self::range_is_set(self.bg_start, self.bg_end);
            let measured = Self::measure_range(
                aux,
                self.bg_start,
                self.bg_end,
                ctx.t0,
                bg_has_range,
            )
            .map_err(EffectError::Invalid)?;
            let duration = if aux.sample_rate == 0 {
                0.0
            } else {
                aux.len() as f64 / aux.sample_rate as f64
            };
            (
                measured,
                if bg_has_range { self.bg_start } else { ctx.t0 },
                if bg_has_range {
                    self.bg_end
                } else {
                    ctx.t0 + duration
                },
            )
        } else {
            let bg_has_range = Self::range_is_set(self.bg_start, self.bg_end);
            let measured = Self::measure_range(
                buf,
                self.bg_start,
                self.bg_end,
                ctx.t0,
                bg_has_range,
            )
            .map_err(EffectError::Invalid)?;
            let duration = if buf.sample_rate == 0 {
                0.0
            } else {
                buf.len() as f64 / buf.sample_rate as f64
            };
            (
                measured,
                if bg_has_range { self.bg_start } else { ctx.t0 },
                if bg_has_range {
                    self.bg_end
                } else {
                    ctx.t0 + duration
                },
            )
        };

        let fg_duration = if buf.sample_rate == 0 {
            0.0
        } else {
            buf.len() as f64 / buf.sample_rate as f64
        };
        let fg_start = if fg_has_range {
            self.fg_start
        } else {
            ctx.t0
        };
        let fg_end = if fg_has_range {
            self.fg_end
        } else {
            ctx.t0 + fg_duration
        };

        let result = analyze(fg_db, bg_db);
        ctx.report = Self::export_report(
            fg_start,
            fg_end,
            bg_start,
            bg_end,
            result.fg_db,
            result.bg_db,
            result.diff_db,
        );

        // Contrast is an analyzer and does not alter the selected samples.
        Ok(())
    }
}

/// Returns the default Contrast analyzer.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(Contrast::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::set_param;

    fn stereo_buffer() -> AudioBuffer {
        let left: Vec<f32> = (0..128)
            .map(|i| (i as f32 * 0.17).sin() * 0.5)
            .collect();
        let right: Vec<f32> = (0..128)
            .map(|i| (i as f32 * 0.13).sin() * 0.25)
            .collect();
        AudioBuffer::from_channels(44_100, vec![left, right])
    }

    #[test]
    fn default_runs_on_stereo_audio_without_modifying_it() {
        let mut effect = Contrast::default();
        let mut buffer = stereo_buffer();
        let original = buffer.channels.clone();
        let mut context = EffectContext::default();

        effect.process(&mut buffer, &mut context).unwrap();

        assert_eq!(buffer.channels, original);
        assert!(context.report.contains("WCAG 2.0 Success Criteria"));
        assert!(context.report.contains("Difference ="));
    }

    #[test]
    fn empty_buffer_returns_an_error_without_panicking() {
        let mut effect = Contrast::default();
        let mut buffer = AudioBuffer::from_channels(44_100, vec![Vec::new(), Vec::new()]);
        let mut context = EffectContext::default();

        assert!(effect.process(&mut buffer, &mut context).is_err());
    }

    #[test]
    fn measurements_are_repeatable() {
        let buffer = stereo_buffer();
        let first = measure(&buffer, 0.0, buffer.len() as f64 / buffer.sample_rate as f64)
            .unwrap();
        let second = measure(&buffer, 0.0, buffer.len() as f64 / buffer.sample_rate as f64)
            .unwrap();

        assert_eq!(first, second);
    }

    #[test]
    fn parameters_clamp() {
        let mut effect = Contrast::default();

        assert!(set_param(&mut effect, "FgStart", -10.0));
        assert_eq!(effect.fg_start, 0.0);
        assert!(set_param(&mut effect, "BgEnd", f64::INFINITY));
        assert_eq!(effect.bg_end, f64::MAX);
        assert!(!set_param(&mut effect, "Unknown", 1.0));
    }
}
