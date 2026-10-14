// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Filter Curve EQ and Graphic EQ effects.

use crate::core::*;
use au_core::AudioBuffer;
use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

const FFT_SIZE: usize = 16_384;
const MIN_FILTER_LENGTH: usize = 21;
const MAX_FILTER_LENGTH: usize = 8_191;
const INTERPOLATIONS: &[&str] = &["B-spline", "Cosine", "Cubic"];

const GRAPHIC_FREQUENCIES: [f64; 31] = [
    20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0,
    400.0, 500.0, 630.0, 800.0, 1_000.0, 1_250.0, 1_600.0, 2_000.0, 2_500.0, 3_150.0,
    4_000.0, 5_000.0, 6_300.0, 8_000.0, 10_000.0, 12_500.0, 16_000.0, 20_000.0,
];

const GRAPHIC_LABELS: [&str; 31] = [
    "20 Hz", "25 Hz", "31.5 Hz", "40 Hz", "50 Hz", "63 Hz", "80 Hz", "100 Hz", "125 Hz",
    "160 Hz", "200 Hz", "250 Hz", "315 Hz", "400 Hz", "500 Hz", "630 Hz", "800 Hz",
    "1 kHz", "1.25 kHz", "1.6 kHz", "2 kHz", "2.5 kHz", "3.15 kHz", "4 kHz", "5 kHz",
    "6.3 kHz", "8 kHz", "10 kHz", "12.5 kHz", "16 kHz", "20 kHz",
];

/// A point in an equalization curve, represented as `(frequency_hz, gain_db)`.
pub type EqPoint = (f64, f64);

/// Build a finite, symmetric FIR response from an equalization curve.
///
/// The returned spectrum uses the same full-complex representation as
/// `rustfft`; `impulse` contains the causal FIR taps.
fn build_filter(
    points: &[EqPoint],
    filter_length: usize,
    interpolate_linear_frequency: bool,
    interpolation_method: usize,
    sample_rate: u32,
) -> (Vec<f32>, Vec<Complex32>) {
    let length = filter_length.clamp(MIN_FILTER_LENGTH, MAX_FILTER_LENGTH);
    let nyquist = (sample_rate as f64 / 2.0).max(20.000_001);
    let lo_log = 20.0_f64.log10();
    let hi_log = nyquist.log10();
    let log_denom = (hi_log - lo_log).max(f64::EPSILON);

    let mut envelope_points: Vec<(f64, f64)> = points
        .iter()
        .filter_map(|&(frequency, db)| {
            if !frequency.is_finite() || frequency <= 0.0 || !db.is_finite() {
                return None;
            }
            let x = if interpolate_linear_frequency {
                frequency / nyquist
            } else {
                (frequency.log10() - lo_log) / log_denom
            };
            Some((x, db))
        })
        .collect();
    envelope_points.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut unique_points: Vec<(f64, f64)> = Vec::with_capacity(envelope_points.len());
    for point in envelope_points {
        if let Some(last) = unique_points.last_mut() {
            if (last.0 - point.0).abs() < 1.0e-14 {
                *last = point;
                continue;
            }
        }
        unique_points.push(point);
    }

    let value_at = |x: f64| -> f64 {
        envelope_value(&unique_points, x, interpolation_method)
    };
    let value0 = value_at(0.0);
    let value1 = value_at(1.0);
    let delta = nyquist / (FFT_SIZE as f64 / 2.0);

    let mut response = vec![0.0_f32; FFT_SIZE];
    response[0] = db_to_linear(value0);
    let mut frequency = delta;
    for i in 1..=FFT_SIZE / 2 {
        let when = if interpolate_linear_frequency {
            frequency / nyquist
        } else {
            (frequency.log10() - lo_log) / log_denom
        };
        response[i] = db_to_linear(value_at(when));
        frequency += delta;
    }
    response[FFT_SIZE / 2] = db_to_linear(value1);

    for i in 1..FFT_SIZE / 2 {
        response[FFT_SIZE - i] = response[i];
    }

    let mut planner = FftPlanner::<f32>::new();
    let inverse = planner.plan_fft_inverse(FFT_SIZE);
    let forward = planner.plan_fft_forward(FFT_SIZE);

    let mut impulse_work: Vec<Complex32> = response
        .iter()
        .map(|&value| Complex32::new(value, 0.0))
        .collect();
    inverse.process(&mut impulse_work);
    for value in &mut impulse_work {
        *value /= FFT_SIZE as f32;
    }

    let half = (length - 1) / 2;
    for i in 0..=half {
        let position = i as f64 + (length - 1) as f64 / 2.0;
        let denominator = (length - 1) as f64;
        let window = 0.42
            - 0.5 * (2.0 * std::f64::consts::PI * position / denominator).cos()
            + 0.08 * (4.0 * std::f64::consts::PI * position / denominator).cos();

        impulse_work[i].re *= window as f32;
        impulse_work[i].im *= window as f32;
        if i != 0 {
            let mirrored = FFT_SIZE - i;
            impulse_work[mirrored].re *= window as f32;
            impulse_work[mirrored].im *= window as f32;
        }
    }

    for i in half + 1..=FFT_SIZE / 2 {
        impulse_work[i] = Complex32::new(0.0, 0.0);
        impulse_work[FFT_SIZE - i] = Complex32::new(0.0, 0.0);
    }

    // Shift the circular impulse response into the causal FIR order used by
    // EqualizationFilter::CalcFilter.
    let mut impulse = vec![0.0_f32; length];
    let mut i = 0;
    while i < half {
        impulse[half + i] = impulse_work[i].re;
        impulse[i] = impulse_work[FFT_SIZE - half + i].re;
        i += 1;
    }
    if half + i < length {
        impulse[half + i] = impulse_work[i].re;
    }

    let mut spectrum = vec![Complex32::new(0.0, 0.0); FFT_SIZE];
    for (dst, &tap) in spectrum.iter_mut().zip(impulse.iter()) {
        dst.re = tap;
    }
    forward.process(&mut spectrum);

    (impulse, spectrum)
}

fn db_to_linear(db: f64) -> f32 {
    10.0_f64.powf(db / 20.0) as f32
}

fn envelope_value(points: &[(f64, f64)], x: f64, method: usize) -> f64 {
    if points.is_empty() {
        return 0.0;
    }
    if points.len() == 1 || x <= points[0].0 {
        return points[0].1;
    }
    if x >= points[points.len() - 1].0 {
        return points[points.len() - 1].1;
    }

    let upper = points.partition_point(|point| point.0 < x);
    let left = upper - 1;
    let (x0, y0) = points[left];
    let (x1, y1) = points[upper];
    let width = x1 - x0;
    if width <= 0.0 {
        return y1;
    }
    let t = ((x - x0) / width).clamp(0.0, 1.0);

    match method {
        1 => {
            let eased = (1.0 - (std::f64::consts::PI * t).cos()) * 0.5;
            y0 + (y1 - y0) * eased
        }
        2 => {
            let m0 = envelope_slope(points, left);
            let m1 = envelope_slope(points, upper);
            let t2 = t * t;
            let t3 = t2 * t;
            (2.0 * t3 - 3.0 * t2 + 1.0) * y0
                + (t3 - 2.0 * t2 + t) * width * m0
                + (-2.0 * t3 + 3.0 * t2) * y1
                + (t3 - t2) * width * m1
        }
        _ => {
            // Smooth cubic interpolation with endpoint tangents estimated from
            // adjacent secants, as used for the default B-spline-style curve.
            let m0 = envelope_slope(points, left);
            let m1 = envelope_slope(points, upper);
            let t2 = t * t;
            let t3 = t2 * t;
            (2.0 * t3 - 3.0 * t2 + 1.0) * y0
                + (t3 - 2.0 * t2 + t) * width * m0
                + (-2.0 * t3 + 3.0 * t2) * y1
                + (t3 - t2) * width * m1
        }
    }
}

fn envelope_slope(points: &[(f64, f64)], index: usize) -> f64 {
    if points.len() < 2 {
        return 0.0;
    }
    if index == 0 {
        let dx = points[1].0 - points[0].0;
        return if dx == 0.0 {
            0.0
        } else {
            (points[1].1 - points[0].1) / dx
        };
    }
    if index + 1 == points.len() {
        let dx = points[index].0 - points[index - 1].0;
        return if dx == 0.0 {
            0.0
        } else {
            (points[index].1 - points[index - 1].1) / dx
        };
    }

    let dx0 = points[index].0 - points[index - 1].0;
    let dx1 = points[index + 1].0 - points[index].0;
    if dx0 <= 0.0 || dx1 <= 0.0 {
        return 0.0;
    }
    let slope0 = (points[index].1 - points[index - 1].1) / dx0;
    let slope1 = (points[index + 1].1 - points[index].1) / dx1;
    (slope0 * dx1 + slope1 * dx0) / (dx0 + dx1)
}

fn fft_convolution_block(
    input: &[f32],
    spectrum: &[Complex32],
    forward: &Arc<dyn Fft<f32>>,
    inverse: &Arc<dyn Fft<f32>>,
) -> Vec<f32> {
    let mut work = vec![Complex32::new(0.0, 0.0); FFT_SIZE];
    for (dst, &sample) in work.iter_mut().zip(input.iter()) {
        dst.re = sample;
    }
    forward.process(&mut work);
    for (value, coefficient) in work.iter_mut().zip(spectrum.iter()) {
        *value *= *coefficient;
    }
    inverse.process(&mut work);

    work.into_iter()
        .map(|value| value.re / FFT_SIZE as f32)
        .collect()
}

fn convolve_offline(
    input: &[f32],
    filter_length: usize,
    spectrum: &[Complex32],
    forward: &Arc<dyn Fft<f32>>,
    inverse: &Arc<dyn Fft<f32>>,
) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }

    let length = filter_length.clamp(MIN_FILTER_LENGTH, MAX_FILTER_LENGTH);
    let block_size = FFT_SIZE - (length - 1);
    let mut result = vec![0.0_f32; input.len() + length - 1];

    let mut offset = 0;
    while offset < input.len() {
        let count = block_size.min(input.len() - offset);
        let transformed =
            fft_convolution_block(&input[offset..offset + count], spectrum, forward, inverse);
        let valid = count + length - 1;
        for i in 0..valid {
            result[offset + i] += transformed[i];
        }
        offset += count;
    }
    result
}

fn process_offline(
    buffer: &mut AudioBuffer,
    points: &[EqPoint],
    filter_length: usize,
    interpolate_linear_frequency: bool,
    interpolation_method: usize,
) {
    if buffer.channels.is_empty() || buffer.len() == 0 {
        return;
    }

    let length = filter_length.clamp(MIN_FILTER_LENGTH, MAX_FILTER_LENGTH);
    let (_, spectrum) = build_filter(
        points,
        length,
        interpolate_linear_frequency,
        interpolation_method,
        buffer.sample_rate,
    );
    let mut planner = FftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(FFT_SIZE);
    let inverse = planner.plan_fft_inverse(FFT_SIZE);
    let delay = (length - 1) / 2;

    for channel in &mut buffer.channels {
        if channel.is_empty() {
            continue;
        }
        let convolution = convolve_offline(
            channel,
            length,
            &spectrum,
            &forward,
            &inverse,
        );
        for (index, sample) in channel.iter_mut().enumerate() {
            *sample = convolution[index + delay];
        }
    }
}

/// Filter Curve EQ effect.
pub struct FilterCurveEq {
    /// FIR filter length in samples.
    pub filter_length: usize,
    /// Interpolate the curve on a linear-frequency axis instead of a log axis.
    pub interpolate_lin: bool,
    /// Interpolation method: 0 = B-spline, 1 = Cosine, 2 = Cubic.
    pub interpolation_method: usize,
    /// Equalization curve points as `(frequency_hz, gain_db)`.
    pub points: Vec<EqPoint>,
    rt_spectrum: Vec<Complex32>,
    rt_tails: Vec<Vec<f32>>,
}

impl Default for FilterCurveEq {
    fn default() -> Self {
        Self {
            filter_length: 8_191,
            interpolate_lin: false,
            interpolation_method: 0,
            points: Vec::new(),
            rt_spectrum: Vec::new(),
            rt_tails: Vec::new(),
        }
    }
}

impl FilterCurveEq {
    /// Parses an Audacity-style curve string such as
    /// `f0="20" v0="0" f1="100" v1="6"`.
    ///
    /// Empty text clears the curve. Returns `false` if no valid indexed point
    /// can be parsed from non-empty text.
    pub fn set_curve(&mut self, text: &str) -> bool {
        if text.trim().is_empty() {
            self.points.clear();
            return true;
        }

        let mut frequencies = std::collections::BTreeMap::<usize, f64>::new();
        let mut values = std::collections::BTreeMap::<usize, f64>::new();
        for token in text.split_whitespace() {
            let Some((key, raw_value)) = token.split_once('=') else {
                continue;
            };
            let Some(index) = key.get(1..).and_then(|suffix| suffix.parse::<usize>().ok()) else {
                continue;
            };
            let Some(prefix) = key.chars().next() else {
                continue;
            };
            let cleaned = raw_value.trim_matches('"');
            let Ok(value) = cleaned.parse::<f64>() else {
                continue;
            };
            match prefix {
                'f' => {
                    frequencies.insert(index, value);
                }
                'v' => {
                    values.insert(index, value);
                }
                _ => {}
            }
        }

        let parsed: Vec<EqPoint> = frequencies
            .into_iter()
            .filter_map(|(index, frequency)| {
                values.get(&index).map(|&db| (frequency, db))
            })
            .collect();
        if parsed.is_empty() {
            return false;
        }
        self.points = parsed;
        self.points
            .sort_by(|a, b| a.0.total_cmp(&b.0));
        true
    }

    /// Serializes the curve into Audacity-style indexed `fN`/`vN` text.
    pub fn curve(&self) -> String {
        self.points
            .iter()
            .enumerate()
            .map(|(index, (frequency, db))| {
                format!("f{index}=\"{frequency}\" v{index}=\"{db}\"")
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn numeric_params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int(
                "FilterLength",
                "Filter length",
                MIN_FILTER_LENGTH as f64,
                MAX_FILTER_LENGTH as f64,
                8_191.0,
            ),
            ParamDef::boolean("InterpolateLin", "Interpolate linearly", false),
            ParamDef::choice(
                "InterpolationMethod",
                "Interpolation method",
                INTERPOLATIONS,
                0,
            ),
        ]
    }

    fn set_numeric(&mut self, key: &str, value: f64) -> bool {
        match key {
            "FilterLength" => {
                self.filter_length = value
                    .round()
                    .clamp(MIN_FILTER_LENGTH as f64, MAX_FILTER_LENGTH as f64)
                    as usize;
                true
            }
            "InterpolateLin" => {
                self.interpolate_lin = value >= 0.5;
                true
            }
            "InterpolationMethod" => {
                self.interpolation_method = value.round().clamp(0.0, 2.0) as usize;
                true
            }
            _ => false,
        }
    }

    fn graphic_points(&self) -> Vec<EqPoint> {
        self.points.clone()
    }
}

impl BuiltinEffect for FilterCurveEq {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Filter Curve EQ"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        self.numeric_params()
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "FilterLength" => Some(self.filter_length as f64),
            "InterpolateLin" => Some(if self.interpolate_lin { 1.0 } else { 0.0 }),
            "InterpolationMethod" => Some(self.interpolation_method as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        self.set_numeric(key, value)
    }

    fn process(
        &mut self,
        buffer: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_offline(
            buffer,
            &self.graphic_points(),
            self.filter_length,
            self.interpolate_lin,
            self.interpolation_method,
        );
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        let (_, spectrum) = build_filter(
            &self.points,
            self.filter_length,
            self.interpolate_lin,
            self.interpolation_method,
            sample_rate,
        );
        self.rt_spectrum = spectrum;
        self.rt_tails = vec![vec![0.0; self.filter_length.saturating_sub(1)]; channels];
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        if self.rt_spectrum.is_empty() {
            self.rt_reset(44_100, block.len());
        }
        process_realtime_block(
            block,
            self.filter_length,
            &self.rt_spectrum,
            &mut self.rt_tails,
        );
    }

    fn rt_latency(&self) -> usize {
        self.filter_length.saturating_sub(1) / 2
    }
}

/// Graphic EQ effect with 31 ISO third-octave bands.
pub struct GraphicEq {
    /// Gain in dB for the 31 bands, from 20 Hz through 20 kHz.
    pub bands: [f64; 31],
    /// FIR filter length in samples.
    pub filter_length: usize,
    rt_spectrum: Vec<Complex32>,
    rt_tails: Vec<Vec<f32>>,
}

impl Default for GraphicEq {
    fn default() -> Self {
        Self {
            bands: [0.0; 31],
            filter_length: 8_191,
            rt_spectrum: Vec::new(),
            rt_tails: Vec::new(),
        }
    }
}

impl GraphicEq {
    fn curve_points(&self) -> Vec<EqPoint> {
        GRAPHIC_FREQUENCIES
            .iter()
            .copied()
            .zip(self.bands.iter().copied())
            .collect()
    }

    fn set_band(&mut self, index: usize, value: f64) {
        if let Some(band) = self.bands.get_mut(index) {
            *band = value.clamp(-20.0, 20.0);
        }
    }

    fn band_index(key: &str) -> Option<usize> {
        let suffix = key.strip_prefix('f')?;
        if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        suffix.parse::<usize>().ok().filter(|&index| index < 31)
    }
}

impl BuiltinEffect for GraphicEq {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
        "Graphic EQ"
    }) }

    fn params(&self) -> Vec<ParamDef> {
        let mut parameters = Vec::with_capacity(32);
        for (index, label) in GRAPHIC_LABELS.iter().enumerate() {
            parameters.push(ParamDef::float(
                match index {
                    0 => "f0",
                    1 => "f1",
                    2 => "f2",
                    3 => "f3",
                    4 => "f4",
                    5 => "f5",
                    6 => "f6",
                    7 => "f7",
                    8 => "f8",
                    9 => "f9",
                    10 => "f10",
                    11 => "f11",
                    12 => "f12",
                    13 => "f13",
                    14 => "f14",
                    15 => "f15",
                    16 => "f16",
                    17 => "f17",
                    18 => "f18",
                    19 => "f19",
                    20 => "f20",
                    21 => "f21",
                    22 => "f22",
                    23 => "f23",
                    24 => "f24",
                    25 => "f25",
                    26 => "f26",
                    27 => "f27",
                    28 => "f28",
                    29 => "f29",
                    _ => "f30",
                },
                label,
                -20.0,
                20.0,
                0.0,
            ));
        }
        parameters.push(ParamDef::int(
            "FilterLength",
            "Filter length",
            MIN_FILTER_LENGTH as f64,
            MAX_FILTER_LENGTH as f64,
            8_191.0,
        ));
        parameters
    }

    fn get(&self, key: &str) -> Option<f64> {
        if key == "FilterLength" {
            return Some(self.filter_length as f64);
        }
        Self::band_index(key).map(|index| self.bands[index])
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        if key == "FilterLength" {
            self.filter_length = value
                .round()
                .clamp(MIN_FILTER_LENGTH as f64, MAX_FILTER_LENGTH as f64)
                as usize;
            return true;
        }
        if let Some(index) = Self::band_index(key) {
            self.set_band(index, value);
            return true;
        }
        false
    }

    fn process(
        &mut self,
        buffer: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        process_offline(buffer, &self.curve_points(), self.filter_length, false, 0);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        let (_, spectrum) = build_filter(
            &self.curve_points(),
            self.filter_length,
            false,
            0,
            sample_rate,
        );
        self.rt_spectrum = spectrum;
        self.rt_tails = vec![vec![0.0; self.filter_length.saturating_sub(1)]; channels];
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        if self.rt_spectrum.is_empty() {
            self.rt_reset(44_100, block.len());
        }
        process_realtime_block(
            block,
            self.filter_length,
            &self.rt_spectrum,
            &mut self.rt_tails,
        );
    }

    fn rt_latency(&self) -> usize {
        self.filter_length.saturating_sub(1) / 2
    }
}

fn process_realtime_block(
    block: &mut [Vec<f32>],
    filter_length: usize,
    spectrum: &[Complex32],
    tails: &mut Vec<Vec<f32>>,
) {
    let length = filter_length.clamp(MIN_FILTER_LENGTH, MAX_FILTER_LENGTH);
    let tail_length = length - 1;
    let block_size = FFT_SIZE - tail_length;
    if tails.len() < block.len() {
        tails.resize_with(block.len(), || vec![0.0; tail_length]);
    }

    let mut planner = FftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(FFT_SIZE);
    let inverse = planner.plan_fft_inverse(FFT_SIZE);

    for (channel_index, channel) in block.iter_mut().enumerate() {
        if tails[channel_index].len() != tail_length {
            tails[channel_index].resize(tail_length, 0.0);
        }
        let mut offset = 0;
        while offset < channel.len() {
            let count = block_size.min(channel.len() - offset);
            let convolution = fft_convolution_block(
                &channel[offset..offset + count],
                spectrum,
                &forward,
                &inverse,
            );
            let old_tail = tails[channel_index].clone();

            for i in 0..count {
                channel[offset + i] =
                    convolution[i] + old_tail.get(i).copied().unwrap_or(0.0);
            }
            for i in 0..tail_length {
                let from_convolution = convolution.get(count + i).copied().unwrap_or(0.0);
                let from_old_tail = old_tail.get(count + i).copied().unwrap_or(0.0);
                tails[channel_index][i] = from_convolution + from_old_tail;
            }
            offset += count;
        }
    }
}

/// A selection of Audacity's built-in equalization curves.
pub fn factory_curves() -> Vec<(String, Vec<EqPoint>)> {
    vec![
        (
            "Bass Boost".to_string(),
            vec![(100.0, 9.0), (500.0, 0.0)],
        ),
        (
            "Treble Boost".to_string(),
            vec![(4_000.0, 0.0), (5_000.0, 9.0)],
        ),
        (
            "Telephone".to_string(),
            vec![
                (20.0, -94.087),
                (200.0, -14.254),
                (250.0, -7.243),
                (315.0, -2.245),
                (400.0, -0.414),
                (500.0, 0.0),
                (2_500.0, 0.0),
                (3_150.0, -0.874),
                (4_000.0, -3.992),
                (5_000.0, -9.993),
                (48_000.0, -88.117),
            ],
        ),
        (
            "AM Radio".to_string(),
            vec![
                (20.0, -63.67),
                (31.0, -33.219),
                (50.0, -3.01),
                (63.0, -0.106),
                (100.0, 0.0),
                (2_500.0, 0.0),
                (4_000.0, -0.614),
                (5_000.0, -8.059),
                (8_000.0, -39.981),
                (20_000.0, -103.651),
                (48_000.0, -164.485),
            ],
        ),
    ]
}

/// Returns one default instance of each effect in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(FilterCurveEq::default()),
        Box::new(GraphicEq::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{set_param, EffectContext};
    use au_core::AudioBuffer;

    fn sine_buffer(frames: usize, sample_rate: u32, frequency: f64) -> AudioBuffer {
        let mono: Vec<f32> = (0..frames)
            .map(|i| {
                (0.2 * (2.0 * std::f64::consts::PI * frequency * i as f64
                    / sample_rate as f64)
                    .sin()) as f32
            })
            .collect();
        AudioBuffer::from_channels(sample_rate, vec![mono.clone(), mono])
    }

    fn run(effect: &mut dyn BuiltinEffect, buffer: &mut AudioBuffer) {
        effect
            .process(buffer, &mut EffectContext::default())
            .expect("EQ processing should succeed");
    }

    #[test]
    fn defaults_empty_and_repeatability() {
        for mut effect in all() {
            let mut first = sine_buffer(128, 48_000, 440.0);
            let mut second = first.clone();
            run(effect.as_mut(), &mut first);
            run(effect.as_mut(), &mut second);
            assert_eq!(first.channels, second.channels);

            let mut empty = AudioBuffer::from_channels(48_000, vec![Vec::new(), Vec::new()]);
            run(effect.as_mut(), &mut empty);
            assert!(empty.channels.iter().all(Vec::is_empty));
        }
    }

    #[test]
    fn parameter_clamping() {
        let mut curve = FilterCurveEq::default();
        assert!(set_param(&mut curve, "FilterLength", -100.0));
        assert_eq!(curve.filter_length, MIN_FILTER_LENGTH);
        assert!(set_param(&mut curve, "InterpolationMethod", 99.0));
        assert_eq!(curve.interpolation_method, 2);

        let mut graphic = GraphicEq::default();
        assert!(set_param(&mut graphic, "FilterLength", 50_000.0));
        assert_eq!(graphic.filter_length, MAX_FILTER_LENGTH);
        assert!(set_param(&mut graphic, "f0", 100.0));
        assert_eq!(graphic.bands[0], 20.0);
        assert!(!set_param(&mut graphic, "not-a-parameter", 1.0));
    }

    #[test]
    fn flat_curve_is_unity_in_the_interior() {
        let mut effect = FilterCurveEq {
            filter_length: 21,
            ..FilterCurveEq::default()
        };
        let mut buffer = sine_buffer(2_048, 48_000, 1_000.0);
        let expected = buffer.channels[0][1_000];
        run(&mut effect, &mut buffer);
        assert!((buffer.channels[0][1_000] - expected).abs() < 1.0e-4);
    }

    #[test]
    fn bass_boost_increases_low_frequency_level() {
        let mut effect = FilterCurveEq {
            filter_length: 255,
            points: vec![
                (20.0, 0.0),
                (100.0, 9.0),
                (500.0, 0.0),
                (24_000.0, 0.0),
            ],
            ..FilterCurveEq::default()
        };
        let mut buffer = sine_buffer(32_768, 48_000, 100.0);
        let before = buffer.channels[0][16_000].abs();
        run(&mut effect, &mut buffer);
        let after = buffer.channels[0][16_000].abs();
        assert!(after > before * 1.4);
    }

    #[test]
    fn graphic_eq_flat_and_boost_sanity() {
        let mut effect = GraphicEq {
            filter_length: 21,
            ..GraphicEq::default()
        };
        let mut flat = sine_buffer(2_048, 48_000, 1_000.0);
        let expected = flat.channels[0][1_000];
        run(&mut effect, &mut flat);
        assert!((flat.channels[0][1_000] - expected).abs() < 1.0e-4);

        for index in 0..31 {
            effect.bands[index] = 6.0;
        }
        let mut boosted = sine_buffer(2_048, 48_000, 1_000.0);
        let before = boosted.channels[0][1_000].abs();
        run(&mut effect, &mut boosted);
        let after = boosted.channels[0][1_000].abs();
        assert!(after > before * 1.7);
    }

    #[test]
    fn curve_text_round_trip() {
        let mut effect = FilterCurveEq::default();
        assert!(effect.set_curve("f0=\"20\" v0=\"-3\" f1=\"1000\" v1=\"6\""));
        assert_eq!(effect.points, vec![(20.0, -3.0), (1_000.0, 6.0)]);
        assert!(effect.curve().contains("f1=\"1000\" v1=\"6\""));
    }
}
