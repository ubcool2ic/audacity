#![allow(dead_code)] // public API used by golden tests
// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Audacity-compatible spectrogram column calculation.

use crate::spectrogram_view::{FreqScale, ViewSettings};

/// Audacity spectrogram window-function labels, in Audacity's choice order.
pub const WINDOW_TYPES: &[&str] = &[
    "Rectangular window",
    "Bartlett window",
    "Hamming window",
    "Hann window",
    "Blackman window",
    "Blackman-Harris window",
    "Welch window",
    "Gaussian(a=2.5) window",
    "Gaussian(a=3.5) window",
    "Gaussian(a=4.5) window",
];

/// Audacity spectrogram frequency-scale labels.
pub const SCALE_TYPES: &[&str] = &[
    "Linear",
    "Logarithmic",
    "Mel",
    "Bark",
    "ERB",
    "Period",
];

/// Audacity spectrogram algorithm labels.
pub const ALGORITHMS: &[&str] = &["Frequencies", "Reassignment", "Pitch (EAC)"];

const ALGORITHM_FREQUENCIES: usize = 0;
const ALGORITHM_REASSIGNMENT: usize = 1;
const ALGORITHM_PITCH_EAC: usize = 2;
const DB_FLOOR: f32 = -160.0;

/// Settings used by Audacity's spectrum cache to calculate spectrogram columns.
///
/// Choice fields are indices into [`WINDOW_TYPES`], [`SCALE_TYPES`], and
/// [`ALGORITHMS`], respectively.
#[derive(Clone, Debug, PartialEq)]
pub struct AudSpectrogramSettings {
    /// Minimum frequency bound in hertz.
    pub min_freq: i32,
    /// Maximum frequency bound in hertz.
    pub max_freq: i32,
    /// Display range in decibels.
    pub range: i32,
    /// Display gain in decibels.
    pub gain: i32,
    /// Frequency-dependent gain in dB/decade.
    pub frequency_gain: i32,
    /// Window-function choice index.
    pub window_type: usize,
    /// FFT window size in samples.
    pub window_size: usize,
    /// Zero-padding factor.
    pub zero_padding_factor: usize,
    /// Frequency-scale choice index.
    pub scale_type: usize,
    /// Algorithm choice index.
    pub algorithm: usize,
}

impl Default for AudSpectrogramSettings {
    fn default() -> Self {
        Self {
            min_freq: 0,
            max_freq: 20_000,
            range: 80,
            gain: 20,
            frequency_gain: 0,
            window_type: 3,
            window_size: 2048,
            zero_padding_factor: 2,
            scale_type: 2,
            algorithm: ALGORITHM_FREQUENCIES,
        }
    }
}

impl AudSpectrogramSettings {
    fn normalized(&self) -> NormalizedSettings {
        let window_size = normalize_power_of_two(self.window_size, 8, 32_768);
        let max_padding = (32_768 / window_size).max(1);
        let zero_padding_factor =
            normalize_power_of_two(self.zero_padding_factor, 1, max_padding);
        let algorithm = self.algorithm.min(ALGORITHM_PITCH_EAC);
        let zero_padding_factor = if algorithm == ALGORITHM_PITCH_EAC {
            1
        } else {
            zero_padding_factor
        };

        NormalizedSettings {
            window_size,
            zero_padding_factor,
            window_type: self.window_type.min(WINDOW_TYPES.len() - 1),
            algorithm,
            frequency_gain: self.frequency_gain.clamp(0, 60),
        }
    }
}

#[derive(Clone, Copy)]
struct NormalizedSettings {
    window_size: usize,
    zero_padding_factor: usize,
    window_type: usize,
    algorithm: usize,
    frequency_gain: i32,
}

fn normalize_power_of_two(value: usize, minimum: usize, maximum: usize) -> usize {
    let maximum = maximum.max(minimum);
    let value = value.clamp(minimum, maximum);
    let lower = 1usize << (usize::BITS - 1 - value.leading_zeros());
    lower.clamp(minimum, maximum)
}

struct RealFftTables {
    points: usize,
    bit_reversed: Vec<usize>,
    sin_table: Vec<f32>,
}

impl RealFftTables {
    fn new(fft_len: usize) -> Self {
        let points = fft_len / 2;
        let mut bit_reversed = vec![0usize; points];

        for (i, value) in bit_reversed.iter_mut().enumerate() {
            let mut temp = 0usize;
            let mut mask = points / 2;
            while mask > 0 {
                temp = (temp >> 1) + if i & mask != 0 { points } else { 0 };
                mask >>= 1;
            }
            *value = temp;
        }

        let mut sin_table = vec![0.0f32; 2 * points];
        for i in 0..points {
            let angle = 2.0 * std::f64::consts::PI * i as f64 / (2 * points) as f64;
            sin_table[bit_reversed[i]] = (-angle.sin()) as f32;
            sin_table[bit_reversed[i] + 1] = (-angle.cos()) as f32;
        }

        Self {
            points,
            bit_reversed,
            sin_table,
        }
    }

    fn real_fft(&self, buffer: &mut [f32]) {
        let mut butterflies_per_group = self.points / 2;
        let endptr1 = self.points * 2;

        while butterflies_per_group > 0 {
            let mut a = 0usize;
            let mut b = butterflies_per_group * 2;
            let mut sptr = 0usize;

            while a < endptr1 {
                let sin = self.sin_table[sptr];
                let cos = self.sin_table[sptr + 1];
                let endptr2 = b;

                while a < endptr2 {
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
                b += butterflies_per_group * 2;
                sptr += 2;
            }

            butterflies_per_group >>= 1;
        }

        let mut br1 = 1usize;
        let mut br2 = self.points - 1;

        while br1 < br2 {
            let sin = self.sin_table[self.bit_reversed[br1]];
            let cos = self.sin_table[self.bit_reversed[br1] + 1];
            let a = self.bit_reversed[br1];
            let b = self.bit_reversed[br2];

            let hrminus = buffer[a] - buffer[b];
            let hrplus = hrminus + buffer[b] * 2.0;
            let himinus = buffer[a + 1] - buffer[b + 1];
            let hiplus = himinus + buffer[b + 1] * 2.0;

            let v1 = sin * hrminus - cos * hiplus;
            let v2 = cos * hrminus + sin * hiplus;

            buffer[a] = (hrplus + v1) * 0.5;
            buffer[b] = buffer[a] - v1;
            buffer[a + 1] = (himinus + v2) * 0.5;
            buffer[b + 1] = buffer[a + 1] - himinus;

            br1 += 1;
            br2 -= 1;
        }

        let center = self.bit_reversed[br1] + 1;
        buffer[center] = -buffer[center];

        let v1 = buffer[0] - buffer[1];
        buffer[0] += buffer[1];
        buffer[1] = v1;
    }
}

fn window_values(which: usize, size: usize, extra_sample: bool) -> Vec<f32> {
    let mut window = vec![1.0f32; size];
    if size == 0 {
        return window;
    }

    let has_extra = extra_sample && size > 1;
    let n = if has_extra { size - 1 } else { size };

    match which {
        0 => {}
        1 => {
            let n_pairs = (n as isize - 1) / 2;
            let denom = n as f32 / 2.0;
            window[0] = 0.0;
            for ii in 1..=n_pairs.max(0) as usize {
                let value = ii as f32 / denom;
                window[ii] *= value;
                window[n - ii] *= value;
            }
        }
        2..=5 => {
            let multiplier = 2.0 * std::f64::consts::PI / n as f64;
            let multiplier2 = 2.0 * multiplier;
            let multiplier3 = 3.0 * multiplier;

            for ii in 0..n {
                let i = ii as f64;
                let value = match which {
                    2 => 0.54 - 0.46 * (i * multiplier).cos(),
                    3 => 0.5 - 0.5 * (i * multiplier).cos(),
                    4 => {
                        0.42 - 0.5 * (i * multiplier).cos()
                            + 0.08 * (i * multiplier2).cos()
                    }
                    _ => {
                        0.35875 - 0.48829 * (i * multiplier).cos()
                            + 0.14128 * (i * multiplier2).cos()
                            - 0.01168 * (i * multiplier3).cos()
                    }
                };
                window[ii] = (window[ii] as f64 * value) as f32;
            }
        }
        6 => {
            let n_float = n as f32;
            for (ii, value) in window.iter_mut().take(n).enumerate() {
                let i_over_n = ii as f32 / n_float;
                *value *= 4.0 * i_over_n * (1.0 - i_over_n);
            }
        }
        7..=9 => {
            let a = match which {
                7 => -2.0 * 2.5 * 2.5,
                8 => -2.0 * 3.5 * 3.5,
                _ => -2.0 * 4.5 * 4.5,
            };
            let n_float = n as f32;
            for (ii, value) in window.iter_mut().take(n).enumerate() {
                let i_over_n = ii as f32 / n_float;
                let exponent =
                    a * (0.25 + (i_over_n * i_over_n) as f64 - i_over_n as f64);
                *value = (*value as f64 * exponent.exp()) as f32;
            }
        }
        _ => {}
    }

    if has_extra && which != 0 {
        let value = match which {
            2 => 0.08,
            7 => (-2.0f64 * 2.5 * 2.5 * 0.25).exp(),
            8 => (-2.0f64 * 3.5 * 3.5 * 0.25).exp(),
            9 => (-2.0f64 * 4.5 * 4.5 * 0.25).exp(),
            _ => 0.0,
        };
        window[n] = (window[n] as f64 * value) as f32;
    }
    window
}

fn derivative_window_values(which: usize, size: usize, extra_sample: bool) -> Vec<f32> {
    let mut result = vec![0.0f32; size];
    if size == 0 {
        return result;
    }

    let has_extra = extra_sample && size > 1;
    let n = if has_extra { size - 1 } else { size };
    let nf = n as f64;

    match which {
        0 => {}
        1 => {
            let denom = n as f32 / 2.0;
            for (i, value) in result.iter_mut().take(n).enumerate() {
                if i < n / 2 {
                    *value = 1.0 / denom;
                } else if i > n / 2 || n % 2 == 0 {
                    *value = -1.0 / denom;
                }
            }
        }
        2..=5 => {
            let multiplier = 2.0 * std::f64::consts::PI / nf;
            let multiplier2 = 2.0 * multiplier;
            let multiplier3 = 3.0 * multiplier;

            for i in 0..n {
                let x = i as f64;
                let derivative = match which {
                    2 => 0.46 * multiplier * (x * multiplier).sin(),
                    3 => 0.5 * multiplier * (x * multiplier).sin(),
                    4 => {
                        0.5 * multiplier * (x * multiplier).sin()
                            - 0.16 * multiplier2 * (x * multiplier2).sin()
                    }
                    _ => {
                        0.48829 * multiplier * (x * multiplier).sin()
                            - 0.28256 * multiplier2 * (x * multiplier2).sin()
                            + 0.03504 * multiplier3 * (x * multiplier3).sin()
                    }
                };
                result[i] = derivative as f32;
            }
        }
        6 => {
            let n_float = n as f32;
            for (i, value) in result.iter_mut().take(n).enumerate() {
                let i_over_n = i as f32 / n_float;
                *value = 4.0 * (1.0 - 2.0 * i_over_n) / n_float;
            }
        }
        7..=9 => {
            let a = match which {
                7 => -2.0 * 2.5 * 2.5,
                8 => -2.0 * 3.5 * 3.5,
                _ => -2.0 * 4.5 * 4.5,
            };
            let n_float = n as f32;
            for (i, value) in result.iter_mut().take(n).enumerate() {
                let i_over_n = i as f32 / n_float;
                let exponent =
                    a * (0.25 + (i_over_n * i_over_n) as f64 - i_over_n as f64);
                let window_value = exponent.exp();
                *value = (window_value * (a * (2.0 * i_over_n as f64 - 1.0) / nf)) as f32;
            }
        }
        _ => {}
    }

    result
}

struct CachedWindows {
    fft_len: usize,
    points: usize,
    padding: usize,
    window: Vec<f32>,
    t_window: Vec<f32>,
    d_window: Vec<f32>,
    tables: RealFftTables,
}

impl CachedWindows {
    fn new(settings: NormalizedSettings) -> Self {
        let fft_len = settings.window_size * settings.zero_padding_factor;
        let padding = (settings.window_size * (settings.zero_padding_factor - 1)) / 2;
        let effective_size = settings.window_size + usize::from(padding > 0);
        let extra = padding > 0;

        let mut window = vec![0.0f32; fft_len];
        let raw = window_values(settings.window_type, effective_size, extra);
        let mut sum = 0.0f64;
        for &value in &raw {
            sum += value as f64;
        }
        let scale = if sum > 0.0 { 2.0 / sum } else { 1.0 };

        for (i, &value) in raw.iter().enumerate() {
            window[padding + i] = (value as f64 * scale) as f32;
        }

        let mut t_window = Vec::new();
        let mut d_window = Vec::new();

        if settings.algorithm == ALGORITHM_REASSIGNMENT {
            t_window = window.clone();
            for (i, value) in t_window
                .iter_mut()
                .enumerate()
                .skip(padding)
                .take(effective_size)
            {
                let multiplier = i as isize - padding as isize - (effective_size / 2) as isize;
                *value *= multiplier as f32;
            }

            let derivative = derivative_window_values(
                settings.window_type,
                effective_size,
                extra,
            );
            d_window = vec![0.0f32; fft_len];
            for (i, &value) in derivative.iter().enumerate() {
                d_window[padding + i] = (value as f64 * scale) as f32;
            }
        }

        Self {
            fft_len,
            points: fft_len / 2,
            padding,
            window,
            t_window,
            d_window,
            tables: RealFftTables::new(fft_len),
        }
    }

    fn input_buffer(&self, mono: &[f32], center: i64, window_size: usize) -> Vec<f32> {
        let mut buffer = vec![0.0f32; self.fft_len];
        let start = center - (window_size / 2) as i64;

        for i in 0..window_size {
            let sample_index = start + i as i64;
            if sample_index >= 0 {
                if let Some(&sample) = mono.get(sample_index as usize) {
                    buffer[self.padding + i] = sample;
                }
            }
        }

        buffer
    }
}

fn compute_standard_spectrum(
    buffer: &mut [f32],
    windows: &CachedWindows,
    output: &mut [f32],
) {
    for i in 0..windows.fft_len {
        buffer[i] *= windows.window[i];
    }
    windows.tables.real_fft(buffer);

    let power = buffer[0] * buffer[0];
    output[0] = if power <= 0.0 {
        DB_FLOOR
    } else {
        10.0 * power.log10()
    };

    for i in 1..windows.points {
        let index = windows.tables.bit_reversed[i];
        let re = buffer[index];
        let im = buffer[index + 1];
        let power = re * re + im * im;
        output[i] = if power <= 0.0 {
            DB_FLOOR
        } else {
            10.0 * power.log10()
        };
    }
}

fn compute_eac(
    mono: &[f32],
    center: i64,
    settings: NormalizedSettings,
    output: &mut [f32],
) {
    // CalculateOneSpectrum's window extraction (zero-filled at clip edges),
    // then lib-fft ComputeSpectrum(width = windowSize, autocorrelation = true).
    let ws = settings.window_size;
    let mut data = vec![0.0f32; ws];
    let start = center - (ws >> 1) as i64;
    for (i, d) in data.iter_mut().enumerate() {
        let k = start + i as i64;
        if k >= 0 && (k as usize) < mono.len() {
            *d = mono[k as usize];
        }
    }
    let tables = RealFftTables::new(ws);
    // RealFFT (FFT.cpp): RealFFTf then unpack with symmetric upper half.
    let real_fft = |input: &[f32], re: &mut [f32], im: &mut [f32]| {
        let mut p = input.to_vec();
        tables.real_fft(&mut p);
        for i in 1..ws / 2 {
            re[i] = p[tables.bit_reversed[i]];
            im[i] = p[tables.bit_reversed[i] + 1];
        }
        re[0] = p[0];
        re[ws / 2] = p[1];
        im[0] = 0.0;
        im[ws / 2] = 0.0;
        for i in ws / 2 + 1..ws {
            re[i] = re[ws - i];
            im[i] = -im[ws - i];
        }
    };
    let half = ws / 2;
    let mut processed = vec![0.0f32; ws];
    // WindowFunc: extra sample only for Hamming/Hann/Blackman/Blackman-Harris.
    let extra = matches!(settings.window_type, 2..=5);
    let window = window_values(settings.window_type, ws, extra);
    let mut input: Vec<f32> = data.iter().zip(&window).map(|(d, w)| d * w).collect();
    let mut out = vec![0.0f32; ws];
    let mut out2 = vec![0.0f32; ws];
    real_fft(&input, &mut out, &mut out2);
    for i in 0..ws {
        input[i] = out[i] * out[i] + out2[i] * out2[i];
    }
    for v in input.iter_mut() {
        *v = v.powf(1.0f32 / 3.0f32);
    }
    real_fft(&input, &mut out, &mut out2);
    for i in 0..half {
        processed[i] += out[i];
    }
    for i in 0..half {
        if processed[i] < 0.0 {
            processed[i] = 0.0;
        }
        out[i] = processed[i];
        if i % 2 == 0 {
            processed[i] -= out[i / 2];
        } else {
            processed[i] -= (out[i / 2] + out[i / 2 + 1]) / 2.0;
        }
        if processed[i] < 0.0 {
            processed[i] = 0.0;
        }
    }
    for i in 0..half {
        input[i] = processed[i] / (ws / 4) as f32;
    }
    for i in 0..half {
        processed[half - 1 - i] = input[i];
    }
    output[..half].copy_from_slice(&processed[..half]);
}

fn gain_factors(fft_len: usize, sample_rate: u32, frequency_gain: i32) -> Vec<f32> {
    if frequency_gain <= 0 {
        return Vec::new();
    }

    let factor = (sample_rate as f64 / fft_len as f64) / 1000.0;
    let half = fft_len / 2;
    let mut result = Vec::with_capacity(half);
    result.push((frequency_gain as f64 * factor.log10()) as f32);
    for x in 1..half {
        result.push((frequency_gain as f64 * (factor * x as f64).log10()) as f32);
    }
    result
}

fn reassign_one(
    mono: &[f32],
    center: i64,
    source_column: isize,
    pixels_per_second: f64,
    sample_rate: u32,
    settings: NormalizedSettings,
    windows: &CachedWindows,
    output: &mut [f32],
    columns: usize,
) -> bool {
    if center < 0 || center as usize >= mono.len() {
        return false;
    }

    let mut base = windows.input_buffer(mono, center, settings.window_size);
    let mut derivative = base.clone();
    let mut time_weighted = base.clone();

    for i in 0..windows.fft_len {
        base[i] *= windows.window[i];
        derivative[i] *= windows.d_window[i];
        time_weighted[i] *= windows.t_window[i];
    }

    windows.tables.real_fft(&mut base);
    windows.tables.real_fft(&mut derivative);
    windows.tables.real_fft(&mut time_weighted);

    let mut contributed = false;
    let epsilon = 1.0e-16f64;
    for ii in 0..windows.points {
        let index = windows.tables.bit_reversed[ii];
        let denom_re = base[index];
        let denom_im = if ii == 0 { 0.0 } else { base[index + 1] };
        let power = denom_re as f64 * denom_re as f64 + denom_im as f64 * denom_im as f64;
        if power < epsilon {
            continue;
        }

        let num_re = derivative[index];
        let num_im = if ii == 0 {
            0.0
        } else {
            derivative[index + 1]
        };
        let quot_im =
            (-(num_re as f64) * denom_im as f64 + num_im as f64 * denom_re as f64) / power;
        let multiplier =
            -(windows.fft_len as f64 / (2.0f64 * std::f64::consts::PI));
        let freq_correction = multiplier * quot_im;
        let bin = (ii as f64 + freq_correction + 0.5) as isize;

        if bin < 0 || bin >= windows.points as isize {
            continue;
        }

        let num_re = time_weighted[index];
        let num_im = if ii == 0 {
            0.0
        } else {
            time_weighted[index + 1]
        };
        let time_correction =
            (num_re as f64 * denom_re as f64 + num_im as f64 * denom_im as f64) / power;
        let corrected_column = (0.5
            + source_column as f64
            + time_correction * pixels_per_second / sample_rate as f64)
            .floor() as isize;

        if corrected_column >= 0 && corrected_column < columns as isize {
            let n_bins = windows.points;
            let offset = corrected_column as usize * n_bins + bin as usize;
            output[offset] += power as f32;
            contributed = true;
        }
    }

    contributed
}

fn compute_columns_internal(
    mono: &[f32],
    sample_rate: u32,
    settings: &AudSpectrogramSettings,
    first_sample_positions: &[i64],
    pixels_per_second: Option<f64>,
    cancel: &au_runtime::pool::CancelToken,
) -> Option<Vec<Vec<f32>>> {
    if first_sample_positions.is_empty() || sample_rate == 0 {
        return Some(Vec::new());
    }

    let normalized = settings.normalized();
    let windows = CachedWindows::new(normalized);
    let n_bins = windows.points;
    let columns = first_sample_positions.len();
    let mut result = vec![vec![DB_FLOOR; n_bins]; columns];

    if normalized.algorithm == ALGORITHM_REASSIGNMENT {
        let pps = pixels_per_second.unwrap_or_else(|| {
            let spp = if columns > 1 {
                let first = first_sample_positions[1] - first_sample_positions[0];
                let last = first_sample_positions[columns - 1]
                    - first_sample_positions[columns - 2];
                ((first + last) as f64 / 2.0).abs().max(1.0)
            } else {
                1.0
            };
            sample_rate as f64 / spp
        });
        let limit = ((0.5
            + windows.fft_len as f64 * pps / sample_rate as f64) as usize)
            .min(100);
        let mut powers = vec![0.0f32; columns * n_bins];

        for (column, &center) in first_sample_positions.iter().enumerate() {
            if cancel.check().is_err() {
                return None;
            }
            reassign_one(
                mono,
                center,
                column as isize,
                pps,
                sample_rate,
                normalized,
                &windows,
                &mut powers,
                columns,
            );
        }

        let step = if columns > 1 {
            let mut total = 0i64;
            for pair in first_sample_positions.windows(2) {
                total += pair[1] - pair[0];
            }
            total as f64 / (columns - 1) as f64
        } else {
            sample_rate as f64 / pps
        };

        for side in 0..2 {
            let mut source_column = if side == 0 { 0isize } else { columns as isize - 1 };
            for _ in 0..limit {
                source_column += if side == 0 { -1 } else { 1 };
                let center = if side == 0 {
                    first_sample_positions[0] as f64 + source_column as f64 * step
                } else {
                    first_sample_positions[columns - 1] as f64
                        + (source_column - (columns as isize - 1)) as f64 * step
                }
                .round() as i64;

                if !reassign_one(
                    mono,
                    center,
                    source_column,
                    pps,
                    sample_rate,
                    normalized,
                    &windows,
                    &mut powers,
                    columns,
                ) {
                    break;
                }
            }
        }

        let gains = gain_factors(windows.fft_len, sample_rate, normalized.frequency_gain);
        for column in 0..columns {
            if cancel.check().is_err() {
                return None;
            }
            for bin in 0..n_bins {
                let power = powers[column * n_bins + bin];
                let mut value = if power <= 0.0 {
                    DB_FLOOR
                } else {
                    10.0 * power.log10()
                };
                if !gains.is_empty() {
                    value += gains[bin];
                }
                result[column][bin] = value;
            }
        }

        return Some(result);
    }

    let mut scratch = vec![0.0f32; windows.fft_len];
    for (column, &center) in first_sample_positions.iter().enumerate() {
        if cancel.check().is_err() {
            return None;
        }

        if center < 0 || center as usize >= mono.len() {
            // SpecCache: out-of-clip columns are filled with 0.0.
            result[column].iter_mut().for_each(|v| *v = 0.0);
            continue;
        }

        if normalized.algorithm == ALGORITHM_PITCH_EAC {
            compute_eac(mono, center, normalized, &mut result[column]);
        } else {
            // CalculateOneSpectrum: the scratch buffer persists across columns
            // and is only partly rewritten, so with zero padding the window's
            // extra sample multiplies whatever the previous FFT left there.
            let ws = normalized.window_size;
            let n = mono.len() as i64;
            let mut from = center - (ws >> 1) as i64;
            let mut my_len = ws as i64;
            let mut copy = windows.padding > 0;
            let mut adj = windows.padding;
            if from < 0 {
                for _ in from..0 {
                    scratch[adj] = 0.0;
                    adj += 1;
                }
                my_len += from;
                from = 0;
                copy = true;
            }
            if from + my_len >= n {
                let newlen = n - from;
                for ii in newlen..my_len {
                    scratch[adj + ii as usize] = 0.0;
                }
                my_len = newlen;
                copy = true;
            }
            if copy {
                if my_len > 0 {
                    let (f, l) = (from as usize, my_len as usize);
                    scratch[adj..adj + l].copy_from_slice(&mono[f..f + l]);
                }
                compute_standard_spectrum(&mut scratch, &windows, &mut result[column]);
            } else {
                let mut buffer = mono[from as usize..from as usize + ws].to_vec();
                buffer.resize(windows.fft_len, 0.0);
                compute_standard_spectrum(&mut buffer, &windows, &mut result[column]);
            }

            if normalized.frequency_gain > 0 {
                let gains = gain_factors(
                    windows.fft_len,
                    sample_rate,
                    normalized.frequency_gain,
                );
                for (value, gain) in result[column].iter_mut().zip(gains) {
                    *value += gain;
                }
            }
        }
    }

    Some(result)
}

/// Compute Audacity-compatible dB-valued spectrum columns at exact sample positions.
///
/// The returned vector is column-major. Each column contains `window_size *
/// zero_padding_factor / 2` bins, except for Pitch (EAC), which follows Audacity
/// and disables zero padding. Values have not been mapped through display gain,
/// range, or frequency scale.
pub fn compute_columns(
    mono: &[f32],
    sample_rate: u32,
    settings: &AudSpectrogramSettings,
    first_sample_positions: &[i64],
    cancel: &au_runtime::pool::CancelToken,
) -> Option<Vec<Vec<f32>>> {
    compute_columns_internal(
        mono,
        sample_rate,
        settings,
        first_sample_positions,
        None,
        cancel,
    )
}

/// Map pixel columns to sample positions using Audacity's pixel-to-time sampling.
///
/// `t1` is the right edge of the visible time interval, so the sample spacing is
/// `(t1 - t0) / columns`. Positions are rounded to the nearest sample, including
/// Audacity's half-sample bias when filling its spectrum-cache position array.
pub fn column_positions(
    t0: f64,
    t1: f64,
    columns: usize,
    sample_rate: u32,
) -> Vec<i64> {
    if columns == 0 || sample_rate == 0 {
        return Vec::new();
    }

    let step = (t1 - t0) / columns as f64;
    (0..columns)
        .map(|column| {
            let sample = (t0 + column as f64 * step) * sample_rate as f64;
            (sample + 0.5).floor() as i64
        })
        .collect()
}

/// Compute spectrogram columns for the existing spectrogram-view callback.
///
/// Audacity settings are used, with the FFT window size taken from
/// [`ViewSettings`]. Display gain, range, and frequency mapping are applied by
/// the view after this function returns.
pub fn audacity_columns(
    mono: &[f32],
    sample_rate: u32,
    view: &ViewSettings,
    t0: f64,
    t1: f64,
    columns: usize,
    cancel: &au_runtime::pool::CancelToken,
) -> Option<Vec<Vec<f32>>> {
    let mut settings = AudSpectrogramSettings::default();
    settings.window_size = view.window_size;
    settings.scale_type = match view.scale {
        FreqScale::Linear => 0,
        FreqScale::Logarithmic => 1,
        FreqScale::Mel => 2,
    };

    let positions = column_positions(t0, t1, columns, sample_rate);
    let pixels_per_second = if t1 != t0 && columns > 0 {
        columns as f64 / (t1 - t0).abs()
    } else {
        sample_rate as f64
    };

    compute_columns_internal(
        mono,
        sample_rate,
        &settings,
        &positions,
        Some(pixels_per_second),
        cancel,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_with_cancel<T: Send + 'static>(
        run: impl FnOnce(&au_runtime::pool::CancelToken) -> T + Send + 'static,
    ) -> T {
        let runtime = au_runtime::Runtime::new(1, 1);
        let handle = runtime.submit(0, move |ctx| Ok(run(&ctx.cancel)));
        loop {
            if let Some(result) = handle.try_result() {
                return result.expect("spectrogram test job failed");
            }
            std::thread::yield_now();
        }
    }

    #[test]
    fn one_kilohertz_sine_peaks_near_expected_bin() {
        let sample_rate = 44_100u32;
        let mono: Vec<f32> = (0..8192)
            .map(|i| {
                (2.0 * std::f32::consts::PI * 1_000.0 * i as f32
                    / sample_rate as f32)
                    .sin()
            })
            .collect();
        let settings = AudSpectrogramSettings::default();

        let columns = run_with_cancel(move |cancel| {
            compute_columns(&mono, sample_rate, &settings, &[4096], cancel)
        })
        .expect("calculation was cancelled");

        let peak = columns[0]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, _)| index)
            .unwrap();
        let expected = 1_000.0 / (sample_rate as f32 / 4096.0);
        assert!((peak as f32 - expected).abs() <= 1.0);
    }

    #[test]
    fn hann_window_has_a_zero_edge_and_normalized_center() {
        let settings = AudSpectrogramSettings::default().normalized();
        let windows = CachedWindows::new(settings);
        assert_eq!(windows.window[windows.padding], 0.0);
        let center = windows.padding + settings.window_size / 2;
        assert!((windows.window[center] - 4.0 / settings.window_size as f32).abs() < 1.0e-6);
    }

    #[test]
    fn zero_padding_doubles_the_bin_count() {
        let sample_rate = 44_100u32;
        let mono = vec![0.0f32; 8192];
        let mut settings = AudSpectrogramSettings::default();
        settings.zero_padding_factor = 1;

        let without_padding = run_with_cancel({
            let mono = mono.clone();
            let settings = settings.clone();
            move |cancel| compute_columns(&mono, sample_rate, &settings, &[4096], cancel)
        })
        .expect("calculation was cancelled");

        settings.zero_padding_factor = 2;
        let with_padding = run_with_cancel(move |cancel| {
            compute_columns(&mono, sample_rate, &settings, &[4096], cancel)
        })
        .expect("calculation was cancelled");

        assert_eq!(with_padding[0].len(), without_padding[0].len() * 2);
    }

    #[test]
    fn eac_tone_has_a_peak_near_its_period() {
        let sample_rate = 44_100u32;
        let mono: Vec<f32> = (0..8192)
            .map(|i| {
                (2.0 * std::f32::consts::PI * 220.0 * i as f32
                    / sample_rate as f32)
                    .sin()
            })
            .collect();
        let mut settings = AudSpectrogramSettings::default();
        settings.algorithm = ALGORITHM_PITCH_EAC;

        let columns = run_with_cancel(move |cancel| {
            compute_columns(&mono, sample_rate, &settings, &[4096], cancel)
        })
        .expect("calculation was cancelled");

        let peak = columns[0]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, _)| index)
            .unwrap();
        // ComputeSpectrum reverses the lag axis: index = half - 1 - lag.
        let half = columns[0].len() as f32;
        let expected_index = half - 1.0 - sample_rate as f32 / 220.0;
        assert!((peak as f32 - expected_index).abs() <= 4.0, "{peak} vs {expected_index}");
    }

    #[test]
    fn audacity_callback_handles_empty_and_repeated_calls() {
        let view = ViewSettings::default();
        let mono = vec![0.0f32; 4096];
        let first = run_with_cancel({
            let mono = mono.clone();
            move |cancel| audacity_columns(&mono, 44_100, &view, 0.0, 0.1, 4, cancel)
        })
        .expect("calculation was cancelled");
        let second = run_with_cancel({
            let mono = mono.clone();
            move |cancel| audacity_columns(&mono, 44_100, &view, 0.0, 0.1, 4, cancel)
        })
        .expect("calculation was cancelled");
        assert_eq!(first, second);

        let empty = run_with_cancel(move |cancel| {
            audacity_columns(&[], 44_100, &view, 0.0, 0.1, 0, cancel)
        })
        .expect("calculation was cancelled");
        assert!(empty.is_empty());
    }

    #[test]
    fn pixel_positions_include_audacity_half_sample_bias() {
        assert_eq!(column_positions(0.0, 0.01, 3, 100), vec![0, 0, 1]);
    }
}

/// Golden comparison against Audacity's own lib-fft.so via tools/specref
/// (built by tools/specref/build.sh). Skipped when the reference is absent.
#[cfg(test)]
mod golden {
    use super::*;

    fn reference(sig: &[f32], s: &AudSpectrogramSettings, pos: &[i64]) -> Option<Vec<f32>> {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../tools/specref");
        let exe = format!("{root}/spec_ref");
        if !std::path::Path::new(&exe).exists() {
            let ok = std::process::Command::new("sh").arg(format!("{root}/build.sh")).status().ok()?;
            if !ok.success() {
                return None;
            }
        }
        let dir = std::env::temp_dir().join(format!("specref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let input = dir.join("in.f32");
        let positions = dir.join("pos.txt");
        let out = dir.join("out.f32");
        std::fs::write(&input, sig.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>()).ok()?;
        std::fs::write(&positions, pos.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("\n")).ok()?;
        let zp = if s.algorithm == 2 { 1 } else { s.zero_padding_factor };
        let st = std::process::Command::new(exe)
            .args([
                input.to_str()?, "44100", &s.window_size.to_string(), &zp.to_string(),
                &s.window_type.to_string(), &s.frequency_gain.to_string(), &s.algorithm.to_string(),
                positions.to_str()?, out.to_str()?,
            ])
            .output()
            .ok()?;
        if !st.status.success() {
            return None;
        }
        let b = std::fs::read(&out).ok()?;
        Some(b.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
    }

    #[test]
    fn matches_audacity_lib_fft() {
        let n = 30_000;
        let mut seed = 12345u32;
        let sig: Vec<f32> = (0..n)
            .map(|i| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                let noise = (seed >> 9) as f32 / (1u32 << 23) as f32 - 0.5;
                (0.5 * (i as f32 * 0.0712).sin() + 0.2 * (i as f32 * 0.31).sin() + 0.05 * noise) as f32
            })
            .collect();
        let pos: Vec<i64> = vec![0, 17, 500, 1023, 1024, 4096, 15_000, 29_000, 29_999];
        let cancel = au_runtime::pool::CancelToken::default();
        let mut cases = Vec::new();
        for (wt, ws, zp, fg, alg) in [
            (3, 2048, 2, 0, 0), (3, 1024, 1, 0, 0), (0, 256, 4, 0, 0), (2, 512, 2, 10, 0),
            (5, 4096, 1, 0, 0), (7, 2048, 2, 0, 0), (9, 1024, 2, 0, 0), (3, 2048, 1, 0, 2),
        ] {
            let s = AudSpectrogramSettings {
                window_type: wt, window_size: ws, zero_padding_factor: zp, frequency_gain: fg,
                algorithm: alg, ..Default::default()
            };
            let Some(r) = reference(&sig, &s, &pos) else {
                eprintln!("spec_ref unavailable; skipping");
                return;
            };
            let ours = compute_columns(&sig, 44100, &s, &pos, &cancel).unwrap();
            let flat: Vec<f32> = ours.into_iter().flatten().collect();
            assert_eq!(flat.len(), r.len(), "bins for {wt} {ws} {zp} {alg}");
            let maxd = flat.iter().zip(&r).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            let exact = flat.iter().zip(&r).filter(|(a, b)| a.to_bits() == b.to_bits()).count();
            cases.push(format!("w{wt} n{ws} zp{zp} fg{fg} alg{alg}: max|d|={maxd:e} exact {exact}/{}", r.len()));
            assert!(maxd <= 1e-3, "{}", cases.last().unwrap());
        }
        eprintln!("{}", cases.join("\n"));
        assert!(cases.len() == 8);
    }
}
