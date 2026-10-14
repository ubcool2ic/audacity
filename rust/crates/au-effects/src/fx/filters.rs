// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! Audacity filter effects and reusable biquad filter implementation.

use crate::core::*;
use au_core::AudioBuffer;
use std::f64::consts::PI;

const DB_TO_LINEAR: fn(f64) -> f64 = |db| 10.0_f64.powf(db / 20.0);
const CLASSIC_TYPES: &[&str] = &["Butterworth", "Chebyshev Type I", "Chebyshev Type II"];
const CLASSIC_SUBTYPES: &[&str] = &["Lowpass", "Highpass"];
const ROLLOFFS: &[&str] = &["dB6", "dB12", "dB24", "dB36", "dB48"];
const SHELF_TYPES: &[&str] = &["Low-shelf", "High-shelf"];

/// A second-order IIR filter section using Audacity's direct-form-I calculation.
///
/// Coefficients and filter history are `f64`, while samples at each section's
/// input and output are `f32`, matching Audacity's `Biquad::ProcessOne`.
#[derive(Clone, Debug)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    prev_in: f64,
    prev_prev_in: f64,
    prev_out: f64,
    prev_prev_out: f64,
}

impl Default for Biquad {
    fn default() -> Self {
        Self::new()
    }
}

impl Biquad {
    /// Creates an identity filter section with zeroed history.
    pub fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            prev_in: 0.0,
            prev_prev_in: 0.0,
            prev_out: 0.0,
            prev_prev_out: 0.0,
        }
    }

    /// Clears this section's sample history.
    pub fn reset(&mut self) {
        self.prev_in = 0.0;
        self.prev_prev_in = 0.0;
        self.prev_out = 0.0;
        self.prev_prev_out = 0.0;
    }

    /// Sets normalized numerator coefficients `b0`, `b1`, `b2` and denominator
    /// coefficients `a1`, `a2` (with `a0` assumed to be one).
    pub fn set_coefficients(
        &mut self,
        b0: f64,
        b1: f64,
        b2: f64,
        a1: f64,
        a2: f64,
    ) {
        self.b0 = b0;
        self.b1 = b1;
        self.b2 = b2;
        self.a1 = a1;
        self.a2 = a2;
    }

    /// Processes one sample, retaining the same double-precision state used by
    /// Audacity's `Biquad::ProcessOne`.
    pub fn process_one(&mut self, input: f32) -> f32 {
        let output = f64::from(input) * self.b0
            + self.prev_in * self.b1
            + self.prev_prev_in * self.b2
            - self.prev_out * self.a1
            - self.prev_prev_out * self.a2;

        self.prev_prev_in = self.prev_in;
        self.prev_in = f64::from(input);
        self.prev_prev_out = self.prev_out;
        self.prev_out = output;
        output as f32
    }

    /// Calculates Butterworth low-pass or high-pass sections.
    ///
    /// `subtype` is zero for low-pass and one for high-pass.
    pub fn calc_butterworth_filter(
        order: usize,
        nyquist: f64,
        cutoff: f64,
        subtype: usize,
    ) -> Vec<Self> {
        let order = order.clamp(1, 10);
        let mut sections = vec![Self::new(); (order + 1) / 2];
        let mut norm = cutoff / nyquist;
        if norm >= 0.9999 {
            norm = 0.9999_f32 as f64;
        }
        let c = (PI * norm / 2.0).tan();
        let mut dc_pole_dist = 1.0;
        let mut zx = 0.0;
        let mut zy = 0.0;

        if order % 2 == 0 {
            for pair in 0..order / 2 {
                let sx = c * (PI - (pair as f64 + 0.5) * PI / order as f64).cos();
                let sy = c * (PI - (pair as f64 + 0.5) * PI / order as f64).sin();
                bilin_transform(sx, sy, &mut zx, &mut zy);
                sections[pair].b0 = 1.0;
                sections[pair].b1 = if subtype == 0 { 2.0 } else { -2.0 };
                sections[pair].b2 = 1.0;
                sections[pair].a1 = -2.0 * zx;
                sections[pair].a2 = zx * zx + zy * zy;
                dc_pole_dist *= if subtype == 0 {
                    calc_2d_dist_sqr(1.0, 0.0, zx, zy)
                } else {
                    calc_2d_dist_sqr(-1.0, 0.0, zx, zy)
                };
            }
        } else {
            let sx = -c;
            let sy = 0.0;
            bilin_transform(sx, sy, &mut zx, &mut zy);
            sections[0].b0 = 1.0;
            sections[0].b1 = if subtype == 0 { 1.0 } else { -1.0 };
            sections[0].b2 = 0.0;
            sections[0].a1 = -zx;
            sections[0].a2 = 0.0;
            dc_pole_dist = if subtype == 0 { 1.0 - zx } else { zx + 1.0 };

            for pair in 1..=order / 2 {
                let sx = c * (PI - pair as f64 * PI / order as f64).cos();
                let sy = c * (PI - pair as f64 * PI / order as f64).sin();
                bilin_transform(sx, sy, &mut zx, &mut zy);
                sections[pair].b0 = 1.0;
                sections[pair].b1 = if subtype == 0 { 2.0 } else { -2.0 };
                sections[pair].b2 = 1.0;
                sections[pair].a1 = -2.0 * zx;
                sections[pair].a2 = zx * zx + zy * zy;
                dc_pole_dist *= if subtype == 0 {
                    calc_2d_dist_sqr(1.0, 0.0, zx, zy)
                } else {
                    calc_2d_dist_sqr(-1.0, 0.0, zx, zy)
                };
            }
        }

        let scale = dc_pole_dist / (1_u32 << order) as f64;
        sections[0].b0 *= scale;
        sections[0].b1 *= scale;
        sections[0].b2 *= scale;
        sections
    }

    /// Calculates Chebyshev Type I low-pass or high-pass sections.
    ///
    /// `subtype` is zero for low-pass and one for high-pass.
    pub fn calc_chebyshev_type1_filter(
        order: usize,
        nyquist: f64,
        cutoff: f64,
        ripple: f64,
        subtype: usize,
    ) -> Vec<Self> {
        let order = order.clamp(1, 10);
        let mut sections = vec![Self::new(); (order + 1) / 2];
        let mut norm = cutoff / nyquist;
        if norm >= 0.9999 {
            norm = 0.9999_f32 as f64;
        }
        let c = (PI * norm / 2.0).tan();
        let mut dc_pole_dist;
        let mut zx = 0.0;
        let mut zy = 0.0;
        let mut zero_x;
        let beta = (norm * PI).cos();

        let eps = (10.0_f64.powf(ripple.max(0.001) / 10.0) - 1.0).sqrt();
        let a = (1.0 / eps + (1.0 / (eps * eps) + 1.0).sqrt()).ln() / order as f64;

        for pair in 0..order / 2 {
            let sx = -c * a.sinh() * ((2 * pair + 1) as f64 * PI / (2 * order) as f64).sin();
            let sy = c * a.cosh() * ((2 * pair + 1) as f64 * PI / (2 * order) as f64).cos();
            bilin_transform(sx, sy, &mut zx, &mut zy);
            if subtype == 0 {
                zero_x = -1.0;
                dc_pole_dist = calc_2d_dist_sqr(1.0, 0.0, zx, zy) / 4.0;
            } else {
                let (tx, ty) = complex_div(
                    beta - zx,
                    -zy,
                    1.0 - beta * zx,
                    -beta * zy,
                );
                zx = tx;
                zy = ty;
                zero_x = 1.0;
                dc_pole_dist = calc_2d_dist_sqr(-1.0, 0.0, zx, zy) / 4.0;
            }
            sections[pair].b0 = dc_pole_dist;
            sections[pair].b1 = -2.0 * zero_x * dc_pole_dist;
            sections[pair].b2 = dc_pole_dist;
            sections[pair].a1 = -2.0 * zx;
            sections[pair].a2 = zx * zx + zy * zy;
        }

        if order % 2 == 0 {
            let temp = DB_TO_LINEAR(-ripple.max(0.001));
            sections[0].b0 *= temp;
            sections[0].b1 *= temp;
            sections[0].b2 *= temp;
        } else {
            let sx = -c * a.sinh();
            let sy = 0.0;
            bilin_transform(sx, sy, &mut zx, &mut zy);
            if subtype == 0 {
                zero_x = -1.0;
                dc_pole_dist = calc_2d_dist_sqr(1.0, 0.0, zx, zy).sqrt() / 2.0;
            } else {
                let (tx, ty) = complex_div(
                    beta - zx,
                    -zy,
                    1.0 - beta * zx,
                    -beta * zy,
                );
                zx = tx;
                zy = ty;
                zero_x = 1.0;
                dc_pole_dist = calc_2d_dist_sqr(-1.0, 0.0, zx, zy).sqrt() / 2.0;
            }
            let index = (order - 1) / 2;
            sections[index].b0 = dc_pole_dist;
            sections[index].b1 = -zero_x * dc_pole_dist;
            sections[index].b2 = 0.0;
            sections[index].a1 = -zx;
            sections[index].a2 = 0.0;
        }
        sections
    }

    /// Calculates Chebyshev Type II low-pass or high-pass sections.
    ///
    /// `subtype` is zero for low-pass and one for high-pass.
    pub fn calc_chebyshev_type2_filter(
        order: usize,
        nyquist: f64,
        cutoff: f64,
        ripple: f64,
        subtype: usize,
    ) -> Vec<Self> {
        let order = order.clamp(1, 10);
        let mut sections = vec![Self::new(); (order + 1) / 2];
        let mut norm = cutoff / nyquist;
        if norm >= 0.9999 {
            norm = 0.9999_f32 as f64;
        }
        let c = (PI * norm / 2.0).tan();
        let beta = (norm * PI).cos();
        let eps = DB_TO_LINEAR(-ripple.max(0.001));
        let a = (1.0 / eps + (1.0 / (eps * eps) + 1.0).sqrt()).ln() / order as f64;

        for pair in 0..order / 2 {
            let angle = (2 * pair + 1) as f64 * PI / (2 * order) as f64;
            let (sx, sy) = complex_div(
                c,
                0.0,
                -a.sinh() * angle.sin(),
                a.cosh() * angle.cos(),
            );
            let (mut zx, mut zy) = (0.0, 0.0);
            bilin_transform(sx, sy, &mut zx, &mut zy);

            let zero_angle = (2 * pair + 1) as f64 * PI / (2 * order) as f64;
            let (mut zero_x, mut zero_y) =
                (0.0, c / zero_angle.cos());
            bilin_transform(0.0, zero_y, &mut zero_x, &mut zero_y);

            let dc_pole_dist = if subtype == 0 {
                calc_2d_dist_sqr(1.0, 0.0, zx, zy)
                    / calc_2d_dist_sqr(1.0, 0.0, zero_x, zero_y)
            } else {
                let (tx, ty) = complex_div(
                    beta - zx,
                    -zy,
                    1.0 - beta * zx,
                    -beta * zy,
                );
                zx = tx;
                zy = ty;
                let (tx, ty) = complex_div(
                    beta - zero_x,
                    -zero_y,
                    1.0 - beta * zero_x,
                    -beta * zero_y,
                );
                zero_x = tx;
                zero_y = ty;
                calc_2d_dist_sqr(-1.0, 0.0, zx, zy)
                    / calc_2d_dist_sqr(-1.0, 0.0, zero_x, zero_y)
            };

            sections[pair].b0 = dc_pole_dist;
            sections[pair].b1 = -2.0 * zero_x * dc_pole_dist;
            sections[pair].b2 = (zero_x * zero_x + zero_y * zero_y) * dc_pole_dist;
            sections[pair].a1 = -2.0 * zx;
            sections[pair].a2 = zx * zx + zy * zy;
        }

        if order % 2 == 1 {
            let pair = (order - 1) / 2;
            let angle = (2 * pair + 1) as f64 * PI / (2 * order) as f64;
            let (sx, sy) = complex_div(
                c,
                0.0,
                -a.sinh() * angle.sin(),
                a.cosh() * angle.cos(),
            );
            let (mut zx, mut zy) = (0.0, 0.0);
            bilin_transform(sx, sy, &mut zx, &mut zy);
            let mut zero_x = -1.0;
            let dc_pole_dist;
            if subtype == 0 {
                dc_pole_dist = calc_2d_dist_sqr(1.0, 0.0, zx, zy).sqrt() / 2.0;
            } else {
                let (tx, ty) = complex_div(
                    beta - zx,
                    -zy,
                    1.0 - beta * zx,
                    -zy,
                );
                zx = tx;
                zy = ty;
                zero_x = 1.0;
                dc_pole_dist = calc_2d_dist_sqr(-1.0, 0.0, zx, zy).sqrt() / 2.0;
            }
            sections[pair].b0 = dc_pole_dist;
            sections[pair].b1 = -zero_x * dc_pole_dist;
            sections[pair].b2 = 0.0;
            sections[pair].a1 = -zx;
            sections[pair].a2 = 0.0;
        }
        sections
    }
}

fn complex_div(nr: f64, ni: f64, dr: f64, di: f64) -> (f64, f64) {
    let denom = dr * dr + di * di;
    ((nr * dr + ni * di) / denom, (ni * dr - nr * di) / denom)
}

fn bilin_transform(sx: f64, sy: f64, zx: &mut f64, zy: &mut f64) {
    let denom = (1.0 - sx) * (1.0 - sx) + sy * sy;
    *zx = (1.0 - sx * sx - sy * sy) / denom;
    *zy = 2.0 * sy / denom;
}

// Biquad.cpp declares this helper as returning float, so preserve its rounding.
fn calc_2d_dist_sqr(x1: f64, y1: f64, x2: f64, y2: f64) -> f64 {
    ((x1 - x2) * (x1 - x2) + (y1 - y2) * (y1 - y2)) as f32 as f64
}

/// Bass and Treble tone control effect.
#[derive(Clone, Debug)]
pub struct BassAndTreble {
    /// Bass shelf adjustment in decibels.
    pub bass: f64,
    /// Treble shelf adjustment in decibels.
    pub treble: f64,
    /// Overall output gain in decibels.
    pub gain: f64,
    /// Whether the user interface links the two tone sliders.
    pub link_sliders: bool,
    states: Vec<BassTrebleState>,
}

#[derive(Clone, Debug)]
struct BassTrebleState {
    samplerate: f64,
    slope: f64,
    hz_bass: f64,
    hz_treble: f64,
    bass_coeffs: [f64; 6],
    treble_coeffs: [f64; 6],
    bass_x1: f64,
    bass_x2: f64,
    bass_y1: f64,
    bass_y2: f64,
    treble_x1: f64,
    treble_x2: f64,
    treble_y1: f64,
    treble_y2: f64,
}

impl BassTrebleState {
    fn new(sample_rate: u32) -> Self {
        Self {
            samplerate: f64::from(sample_rate as f32),
            slope: 0.4_f32 as f64,
            hz_bass: 250.0,
            hz_treble: 4000.0,
            bass_coeffs: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            treble_coeffs: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            bass_x1: 0.0,
            bass_x2: 0.0,
            bass_y1: 0.0,
            bass_y2: 0.0,
            treble_x1: 0.0,
            treble_x2: 0.0,
            treble_y1: 0.0,
            treble_y2: 0.0,
        }
    }

    fn coefficients(hz: f64, slope: f64, gain: f64, samplerate: f64, bass: bool) -> [f64; 6] {
        let w = 2.0 * PI * hz / samplerate;
        let a = (10.0_f64.ln() * gain / 40.0).exp();
        let b = (((a * a + 1.0) / slope) - (a - 1.0).powi(2)).sqrt();
        let (a0, a1, a2, b0, b1, b2);
        if bass {
            b0 = a * ((a + 1.0) - (a - 1.0) * w.cos() + b * w.sin());
            b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * w.cos());
            b2 = a * ((a + 1.0) - (a - 1.0) * w.cos() - b * w.sin());
            a0 = (a + 1.0) + (a - 1.0) * w.cos() + b * w.sin();
            a1 = -2.0 * ((a - 1.0) + (a + 1.0) * w.cos());
            a2 = (a + 1.0) + (a - 1.0) * w.cos() - b * w.sin();
        } else {
            b0 = a * ((a + 1.0) + (a - 1.0) * w.cos() + b * w.sin());
            b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * w.cos());
            b2 = a * ((a + 1.0) + (a - 1.0) * w.cos() - b * w.sin());
            a0 = (a + 1.0) - (a - 1.0) * w.cos() + b * w.sin();
            a1 = 2.0 * ((a - 1.0) - (a + 1.0) * w.cos());
            a2 = (a + 1.0) - (a - 1.0) * w.cos() - b * w.sin();
        }
        [a0, a1, a2, b0, b1, b2]
    }

    fn filter(&mut self, input: f32) -> f32 {
        let [a0, a1, a2, b0, b1, b2] = self.bass_coeffs;
        let mut output = ((b0 * f64::from(input) + b1 * self.bass_x1 + b2 * self.bass_x2
            - a1 * self.bass_y1
            - a2 * self.bass_y2)
            / a0) as f32;
        self.bass_x2 = self.bass_x1;
        self.bass_x1 = f64::from(input);
        self.bass_y2 = self.bass_y1;
        self.bass_y1 = f64::from(output);

        let [a0, a1, a2, b0, b1, b2] = self.treble_coeffs;
        let treble_input = output;
        output = ((b0 * f64::from(treble_input) + b1 * self.treble_x1
            + b2 * self.treble_x2
            - a1 * self.treble_y1
            - a2 * self.treble_y2)
            / a0) as f32;
        self.treble_x2 = self.treble_x1;
        self.treble_x1 = f64::from(treble_input);
        self.treble_y2 = self.treble_y1;
        self.treble_y1 = f64::from(output);
        output
    }
}

impl Default for BassAndTreble {
    fn default() -> Self {
        Self {
            bass: 0.0,
            treble: 0.0,
            gain: 0.0,
            link_sliders: false,
            states: Vec::new(),
        }
    }
}

impl BuiltinEffect for BassAndTreble {
    fn id(&self) -> &'static str {
        "BassAndTreble"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Bass", "Bass", -30.0, 30.0, 0.0),
            ParamDef::float("Treble", "Treble", -30.0, 30.0, 0.0),
            ParamDef::float("Gain", "Gain", -30.0, 30.0, 0.0),
            ParamDef::boolean("Link Sliders", "Link Sliders", false),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Bass" => Some(self.bass),
            "Treble" => Some(self.treble),
            "Gain" => Some(self.gain),
            "Link Sliders" => Some(if self.link_sliders { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Bass" => self.bass = value,
            "Treble" => self.treble = value,
            "Gain" => self.gain = value,
            "Link Sliders" => self.link_sliders = value >= 0.5,
            _ => return false,
        }
        true
    }

    fn process(&mut self, buf: &mut AudioBuffer, _ctx: &mut EffectContext) -> Result<(), EffectError> {
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        let sample_rate = sample_rate.max(1);
        self.states = (0..channels)
            .map(|_| {
                let mut state = BassTrebleState::new(sample_rate);
                state.bass_coeffs =
                    BassTrebleState::coefficients(state.hz_bass, state.slope, self.bass, state.samplerate, true);
                state.treble_coeffs = BassTrebleState::coefficients(
                    state.hz_treble,
                    state.slope,
                    self.treble,
                    state.samplerate,
                    false,
                );
                state
            })
            .collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        let gain = DB_TO_LINEAR(self.gain);
        for (channel, state) in block.iter_mut().zip(&mut self.states) {
            for sample in channel {
                *sample = state.filter(*sample) * gain as f32;
            }
        }
    }
}

/// Classic Butterworth and Chebyshev IIR filters.
#[derive(Clone, Debug)]
pub struct ClassicFilters {
    /// Filter design: Butterworth, Chebyshev Type I, or Chebyshev Type II.
    pub filter_type: usize,
    /// Filter subtype: low-pass or high-pass.
    pub subtype: usize,
    /// Filter order, from one to ten.
    pub order: usize,
    /// Cutoff frequency in hertz.
    pub cutoff: f64,
    /// Chebyshev Type I passband ripple in decibels.
    pub passband_ripple: f64,
    /// Chebyshev Type II stopband ripple in decibels.
    pub stopband_ripple: f64,
    channels: Vec<Vec<Biquad>>,
}

impl Default for ClassicFilters {
    fn default() -> Self {
        Self {
            filter_type: 0,
            subtype: 0,
            order: 1,
            cutoff: 1000.0,
            passband_ripple: 1.0,
            stopband_ripple: 30.0,
            channels: Vec::new(),
        }
    }
}

impl ClassicFilters {
    fn sections(&self, sample_rate: u32) -> Vec<Biquad> {
        let nyquist = f64::from(sample_rate.max(1)) / 2.0;
        let cutoff = self.cutoff.max(1.0);
        match self.filter_type.min(2) {
            0 => Biquad::calc_butterworth_filter(
                self.order,
                nyquist,
                cutoff,
                self.subtype.min(1),
            ),
            1 => Biquad::calc_chebyshev_type1_filter(
                self.order,
                nyquist,
                cutoff,
                self.passband_ripple,
                self.subtype.min(1),
            ),
            _ => Biquad::calc_chebyshev_type2_filter(
                self.order,
                nyquist,
                cutoff,
                self.stopband_ripple,
                self.subtype.min(1),
            ),
        }
    }
}

impl BuiltinEffect for ClassicFilters {
    fn id(&self) -> &'static str {
        "ClassicFilters"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice("FilterType", "Filter type", CLASSIC_TYPES, 0),
            ParamDef::choice("FilterSubtype", "Filter subtype", CLASSIC_SUBTYPES, 0),
            ParamDef::int("Order", "Order", 1.0, 10.0, 1.0),
            ParamDef::float("Cutoff", "Cutoff", 1.0, f32::MAX as f64, 1000.0),
            ParamDef::float("PassbandRipple", "Passband ripple", 0.0, 100.0, 1.0),
            ParamDef::float("StopbandRipple", "Stopband ripple", 0.0, 100.0, 30.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "FilterType" => Some(self.filter_type as f64),
            "FilterSubtype" => Some(self.subtype as f64),
            "Order" => Some(self.order as f64),
            "Cutoff" => Some(self.cutoff),
            "PassbandRipple" => Some(self.passband_ripple),
            "StopbandRipple" => Some(self.stopband_ripple),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "FilterType" => self.filter_type = value.round().clamp(0.0, 2.0) as usize,
            "FilterSubtype" => self.subtype = value.round().clamp(0.0, 1.0) as usize,
            "Order" => self.order = value.round().clamp(1.0, 10.0) as usize,
            "Cutoff" => self.cutoff = value,
            "PassbandRipple" => self.passband_ripple = value,
            "StopbandRipple" => self.stopband_ripple = value,
            _ => return false,
        }
        true
    }

    fn process(&mut self, buf: &mut AudioBuffer, _ctx: &mut EffectContext) -> Result<(), EffectError> {
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        let sections = self.sections(sample_rate);
        self.channels = (0..channels).map(|_| sections.clone()).collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        for (samples, sections) in block.iter_mut().zip(&mut self.channels) {
            for section in sections {
                for sample in samples.iter_mut() {
                    *sample = section.process_one(*sample);
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
struct OnePoleState {
    previous_input: f32,
    previous_output: f32,
}

impl OnePoleState {
    fn new() -> Self {
        Self {
            previous_input: 0.0,
            previous_output: 0.0,
        }
    }

    fn highpass(&mut self, input: f32, coefficient: f32) -> f32 {
        let output = coefficient * (self.previous_output + input - self.previous_input);
        self.previous_input = input;
        self.previous_output = output;
        output
    }

    fn lowpass(&mut self, input: f32, coefficient: f32) -> f32 {
        let output = (1.0 - coefficient) * input + coefficient * self.previous_output;
        self.previous_output = output;
        output
    }
}

#[derive(Clone, Debug)]
struct CascadeState {
    one_pole: OnePoleState,
    sections: Vec<Biquad>,
}

impl CascadeState {
    fn new(sample_rate: u32, frequency: f64, rolloff: usize, highpass: bool) -> Self {
        let sample_rate_f64 = f64::from(sample_rate.max(1));
        let sections_count = rolloff;
        let sections = (0..sections_count)
            .map(|_| rbj_filter(sample_rate_f64, frequency, 0.7071, highpass))
            .collect();
        Self {
            one_pole: OnePoleState::new(),
            sections,
        }
    }

    fn process(&mut self, samples: &mut [f32], coefficient: f32, highpass: bool) {
        if self.sections.is_empty() {
            for sample in samples {
                *sample = if highpass {
                    self.one_pole.highpass(*sample, coefficient)
                } else {
                    self.one_pole.lowpass(*sample, coefficient)
                };
            }
        } else {
            for section in &mut self.sections {
                for sample in samples.iter_mut() {
                    *sample = section.process_one(*sample);
                }
            }
        }
    }
}

fn rbj_filter(sample_rate: f64, frequency: f64, q: f64, highpass: bool) -> Biquad {
    let frequency = frequency.clamp(0.000_001, sample_rate * 0.499_999);
    let omega = 2.0 * PI * frequency / sample_rate;
    let cosine = omega.cos();
    let alpha = omega.sin() / (2.0 * q);
    let a0 = 1.0 + alpha;
    let (b0, b1, b2) = if highpass {
        ((1.0 + cosine) / 2.0, -(1.0 + cosine), (1.0 + cosine) / 2.0)
    } else {
        ((1.0 - cosine) / 2.0, 1.0 - cosine, (1.0 - cosine) / 2.0)
    };
    let mut filter = Biquad::new();
    filter.set_coefficients(
        b0 / a0,
        b1 / a0,
        b2 / a0,
        -2.0 * cosine / a0,
        (1.0 - alpha) / a0,
    );
    filter
}

/// Nyquist High-Pass Filter.
#[derive(Clone, Debug)]
pub struct HighPassFilter {
    /// Cutoff frequency in hertz.
    pub frequency: f64,
    /// Roll-off selection: dB6, dB12, dB24, dB36, or dB48.
    pub rolloff: usize,
    states: Vec<CascadeState>,
    one_pole_coefficient: f32,
}

impl Default for HighPassFilter {
    fn default() -> Self {
        Self {
            frequency: 1000.0,
            rolloff: 0,
            states: Vec::new(),
            one_pole_coefficient: 0.0,
        }
    }
}

impl BuiltinEffect for HighPassFilter {
    fn id(&self) -> &'static str {
        "High-passFilter"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Frequency", "Frequency (Hz)", 0.0, f64::MAX, 1000.0),
            ParamDef::choice("Rolloff", "Roll-off (dB per octave)", ROLLOFFS, 0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Frequency" => Some(self.frequency),
            "Rolloff" => Some(self.rolloff as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Frequency" => self.frequency = value,
            "Rolloff" => self.rolloff = value.round().clamp(0.0, 4.0) as usize,
            _ => return false,
        }
        true
    }

    fn process(&mut self, buf: &mut AudioBuffer, _ctx: &mut EffectContext) -> Result<(), EffectError> {
        if self.frequency < 0.1 || self.frequency >= f64::from(buf.sample_rate) / 2.0 {
            return Err(EffectError::Invalid(
                "High-pass frequency must be at least 0.1 Hz and below Nyquist.".to_string(),
            ));
        }
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        let sr = sample_rate.max(1);
        self.one_pole_coefficient =
            (-2.0 * PI * self.frequency / f64::from(sr)).exp() as f32;
        let count = self.rolloff;
        self.states = (0..channels)
            .map(|_| CascadeState::new(sr, self.frequency, count, true))
            .collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        for (samples, state) in block.iter_mut().zip(&mut self.states) {
            state.process(samples, self.one_pole_coefficient, true);
        }
    }
}

/// Nyquist Low-Pass Filter.
#[derive(Clone, Debug)]
pub struct LowPassFilter {
    /// Cutoff frequency in hertz.
    pub frequency: f64,
    /// Roll-off selection: dB6, dB12, dB24, dB36, or dB48.
    pub rolloff: usize,
    states: Vec<CascadeState>,
    one_pole_coefficient: f32,
}

impl Default for LowPassFilter {
    fn default() -> Self {
        Self {
            frequency: 1000.0,
            rolloff: 0,
            states: Vec::new(),
            one_pole_coefficient: 0.0,
        }
    }
}

impl BuiltinEffect for LowPassFilter {
    fn id(&self) -> &'static str {
        "Low-passFilter"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Frequency", "Frequency (Hz)", 0.0, f64::MAX, 1000.0),
            ParamDef::choice("Rolloff", "Roll-off (dB per octave)", ROLLOFFS, 0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Frequency" => Some(self.frequency),
            "Rolloff" => Some(self.rolloff as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Frequency" => self.frequency = value,
            "Rolloff" => self.rolloff = value.round().clamp(0.0, 4.0) as usize,
            _ => return false,
        }
        true
    }

    fn process(&mut self, buf: &mut AudioBuffer, _ctx: &mut EffectContext) -> Result<(), EffectError> {
        if self.frequency < 0.1 || self.frequency >= f64::from(buf.sample_rate) / 2.0 {
            return Err(EffectError::Invalid(
                "Low-pass frequency must be at least 0.1 Hz and below Nyquist.".to_string(),
            ));
        }
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        let sr = sample_rate.max(1);
        self.one_pole_coefficient =
            (-2.0 * PI * self.frequency / f64::from(sr)).exp() as f32;
        self.states = (0..channels)
            .map(|_| CascadeState::new(sr, self.frequency, self.rolloff, false))
            .collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        for (samples, state) in block.iter_mut().zip(&mut self.states) {
            state.process(samples, self.one_pole_coefficient, false);
        }
    }
}

/// Nyquist Notch Filter.
#[derive(Clone, Debug)]
pub struct NotchFilter {
    /// Center frequency in hertz.
    pub frequency: f64,
    /// Notch quality factor; higher values produce a narrower notch.
    pub q: f64,
    states: Vec<Biquad>,
}

impl Default for NotchFilter {
    fn default() -> Self {
        Self {
            frequency: 60.0,
            q: 1.0,
            states: Vec::new(),
        }
    }
}

impl BuiltinEffect for NotchFilter {
    fn id(&self) -> &'static str {
        "NotchFilter"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Frequency", "Frequency (Hz)", 0.0, f64::MAX, 60.0),
            ParamDef::float("Q", "Q", 0.1, 1000.0, 1.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Frequency" => Some(self.frequency),
            "Q" => Some(self.q),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Frequency" => self.frequency = value,
            "Q" => self.q = value,
            _ => return false,
        }
        true
    }

    fn process(&mut self, buf: &mut AudioBuffer, _ctx: &mut EffectContext) -> Result<(), EffectError> {
        if self.frequency < 0.1 || self.frequency >= f64::from(buf.sample_rate) / 2.0 {
            return Err(EffectError::Invalid(
                "Notch frequency must be at least 0.1 Hz and below Nyquist.".to_string(),
            ));
        }
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        let sr = f64::from(sample_rate.max(1));
        let frequency = self.frequency.clamp(0.000_001, sr * 0.499_999);
        let omega = 2.0 * PI * frequency / sr;
        let alpha = omega.sin() / (2.0 * self.q.max(0.1));
        let a0 = 1.0 + alpha;
        let cosine = omega.cos();
        let mut section = Biquad::new();
        section.set_coefficients(
            1.0 / a0,
            -2.0 * cosine / a0,
            1.0 / a0,
            -2.0 * cosine / a0,
            (1.0 - alpha) / a0,
        );
        self.states = (0..channels).map(|_| section.clone()).collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        for (samples, state) in block.iter_mut().zip(&mut self.states) {
            for sample in samples {
                *sample = state.process_one(*sample);
            }
        }
    }
}

/// Nyquist low-shelf and high-shelf filter.
#[derive(Clone, Debug)]
pub struct ShelfFilter {
    /// Shelf type: zero is low-shelf and one is high-shelf.
    pub filter_type: usize,
    /// Shelf corner frequency in hertz.
    pub hz: usize,
    /// Shelf gain in decibels.
    pub gain: i32,
    states: Vec<Biquad>,
}

impl Default for ShelfFilter {
    fn default() -> Self {
        Self {
            filter_type: 0,
            hz: 1000,
            gain: -6,
            states: Vec::new(),
        }
    }
}

impl ShelfFilter {
    fn shelf_section(&self, sample_rate: u32) -> Biquad {
        let sr = f64::from(sample_rate.max(1));
        let frequency = (self.hz as f64).clamp(0.000_001, sr * 0.499_999);
        let a = 10.0_f64.powf(f64::from(self.gain) / 40.0);
        let omega = 2.0 * PI * frequency / sr;
        let cosine = omega.cos();
        let sine = omega.sin();
        // Nyquist's EQ shelf filters use the RBJ shelf form with S = 1.
        let alpha = sine / 2.0 * ((a + 1.0 / a) * 0.0 + 2.0).sqrt();
        let beta = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = if self.filter_type == 0 {
            (
                a * ((a + 1.0) - (a - 1.0) * cosine + beta),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cosine),
                a * ((a + 1.0) - (a - 1.0) * cosine - beta),
                (a + 1.0) + (a - 1.0) * cosine + beta,
                -2.0 * ((a - 1.0) + (a + 1.0) * cosine),
                (a + 1.0) + (a - 1.0) * cosine - beta,
            )
        } else {
            (
                a * ((a + 1.0) + (a - 1.0) * cosine + beta),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cosine),
                a * ((a + 1.0) + (a - 1.0) * cosine - beta),
                (a + 1.0) - (a - 1.0) * cosine + beta,
                2.0 * ((a - 1.0) - (a + 1.0) * cosine),
                (a + 1.0) - (a - 1.0) * cosine - beta,
            )
        };
        let mut section = Biquad::new();
        section.set_coefficients(b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0);
        section
    }
}

impl BuiltinEffect for ShelfFilter {
    fn id(&self) -> &'static str {
        "ShelfFilter"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice("Type", "Filter type", SHELF_TYPES, 0),
            ParamDef::int("Hz", "Frequency (Hz)", 10.0, 10000.0, 1000.0),
            ParamDef::int("Gain", "Amount (dB)", -72.0, 72.0, -6.0),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Type" => Some(self.filter_type as f64),
            "Hz" => Some(self.hz as f64),
            "Gain" => Some(self.gain as f64),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Type" => self.filter_type = value.round().clamp(0.0, 1.0) as usize,
            "Hz" => self.hz = value.round().clamp(10.0, 10000.0) as usize,
            "Gain" => self.gain = value.round().clamp(-72.0, 72.0) as i32,
            _ => return false,
        }
        true
    }

    fn process(&mut self, buf: &mut AudioBuffer, _ctx: &mut EffectContext) -> Result<(), EffectError> {
        if self.hz as f64 > f64::from(buf.sample_rate) / 2.0 {
            return Err(EffectError::Invalid(
                "Shelf frequency is too high for the selected sample rate.".to_string(),
            ));
        }
        process_via_realtime(self, buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        let section = self.shelf_section(sample_rate);
        self.states = (0..channels).map(|_| section.clone()).collect();
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        for (samples, state) in block.iter_mut().zip(&mut self.states) {
            for sample in samples {
                *sample = state.process_one(*sample);
            }
        }
    }
}

/// Returns one default instance of every filter effect in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(BassAndTreble::default()),
        Box::new(ClassicFilters::default()),
        Box::new(HighPassFilter::default()),
        Box::new(LowPassFilter::default()),
        Box::new(NotchFilter::default()),
        Box::new(ShelfFilter::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_buffer(sample_rate: u32, frames: usize, frequency: f64) -> AudioBuffer {
        let channels = (0..2)
            .map(|channel| {
                (0..frames)
                    .map(|index| {
                        let phase = 2.0 * PI * frequency * index as f64
                            / f64::from(sample_rate)
                            + channel as f64 * 0.2;
                        phase.sin() as f32
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        AudioBuffer::from_channels(sample_rate, channels)
    }

    fn process(effect: &mut dyn BuiltinEffect, buffer: &mut AudioBuffer) {
        effect
            .process(buffer, &mut EffectContext::default())
            .expect("effect processing should succeed");
    }

    fn rms(channel: &[f32]) -> f64 {
        (channel.iter().map(|sample| f64::from(*sample).powi(2)).sum::<f64>()
            / channel.len().max(1) as f64)
            .sqrt()
    }

    #[test]
    fn defaults_process_stereo_empty_repeatably_and_clamp_parameters() {
        for mut effect in all() {
            let mut first = sine_buffer(44100, 512, 440.0);
            process(effect.as_mut(), &mut first);
            assert_eq!(first.channels.len(), 2);
            assert!(first.channels.iter().flatten().all(|sample| sample.is_finite()));

            let mut empty = AudioBuffer::from_channels(44100, vec![Vec::new(), Vec::new()]);
            process(effect.as_mut(), &mut empty);
            assert_eq!(empty.len(), 0);

            let mut left = sine_buffer(44100, 512, 440.0);
            let mut right = left.clone();
            process(effect.as_mut(), &mut left);
            process(effect.as_mut(), &mut right);
            assert_eq!(left.channels, right.channels, "{}", effect.id());

            let definitions = effect.params();
            for definition in definitions {
                assert!(set_param(effect.as_mut(), definition.key, definition.max + 1.0));
                assert_eq!(
                    effect.get(definition.key),
                    Some(definition.clamp(definition.max + 1.0)),
                    "{}: {}",
                    effect.id(),
                    definition.key
                );
                assert!(set_param(effect.as_mut(), definition.key, definition.min - 1.0));
                assert_eq!(
                    effect.get(definition.key),
                    Some(definition.clamp(definition.min - 1.0)),
                    "{}: {}",
                    effect.id(),
                    definition.key
                );
            }
        }
    }

    #[test]
    fn bass_treble_gain_increases_low_frequency_level() {
        let mut effect = BassAndTreble::default();
        effect.bass = 12.0;
        let mut buffer = sine_buffer(44100, 12000, 100.0);
        let before = rms(&buffer.channels[0][1000..]);
        process(&mut effect, &mut buffer);
        let after = rms(&buffer.channels[0][1000..]);
        assert!(after > before * 1.5);
    }

    #[test]
    fn classic_lowpass_attenuates_high_frequencies() {
        let mut effect = ClassicFilters::default();
        effect.order = 4;
        effect.cutoff = 500.0;
        let mut buffer = sine_buffer(44100, 12000, 6000.0);
        let before = rms(&buffer.channels[0][1000..]);
        process(&mut effect, &mut buffer);
        let after = rms(&buffer.channels[0][1000..]);
        assert!(after < before * 0.2);
    }

    #[test]
    fn highpass_attenuates_low_frequencies() {
        let mut effect = HighPassFilter::default();
        effect.frequency = 1000.0;
        effect.rolloff = 1;
        let mut buffer = sine_buffer(44100, 12000, 100.0);
        let before = rms(&buffer.channels[0][1000..]);
        process(&mut effect, &mut buffer);
        let after = rms(&buffer.channels[0][1000..]);
        assert!(after < before * 0.25);
    }

    #[test]
    fn lowpass_attenuates_high_frequencies() {
        let mut effect = LowPassFilter::default();
        effect.frequency = 1000.0;
        effect.rolloff = 1;
        let mut buffer = sine_buffer(44100, 12000, 8000.0);
        let before = rms(&buffer.channels[0][1000..]);
        process(&mut effect, &mut buffer);
        let after = rms(&buffer.channels[0][1000..]);
        assert!(after < before * 0.2);
    }

    #[test]
    fn notch_reduces_its_center_frequency() {
        let mut effect = NotchFilter::default();
        effect.frequency = 1000.0;
        effect.q = 1.0;
        let mut buffer = sine_buffer(8000, 16000, 1000.0);
        let before = rms(&buffer.channels[0][2000..]);
        process(&mut effect, &mut buffer);
        let after = rms(&buffer.channels[0][2000..]);
        assert!(after < before * 0.05);
    }

    #[test]
    fn low_shelf_boosts_low_frequencies() {
        let mut effect = ShelfFilter::default();
        effect.filter_type = 0;
        effect.hz = 1000;
        effect.gain = 12;
        let mut buffer = sine_buffer(44100, 12000, 100.0);
        let before = rms(&buffer.channels[0][1000..]);
        process(&mut effect, &mut buffer);
        let after = rms(&buffer.channels[0][1000..]);
        assert!(after > before * 2.0);
    }
}
