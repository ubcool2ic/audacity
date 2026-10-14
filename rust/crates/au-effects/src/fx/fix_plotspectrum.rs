// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Audacity-compatible Plot Spectrum analysis and text export.

use crate::core::*;
use au_core::AudioBuffer;

/// Algorithm labels used by Plot Spectrum.
pub const ALGORITHMS: &[&str] = &[
    "Spectrum",
    "Standard Autocorrelation",
    "Cuberoot Autocorrelation",
    "Enhanced Autocorrelation",
    "Cepstrum",
];

/// Window-function labels used by Plot Spectrum.
pub const FUNCTIONS: &[&str] = &[
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

/// FFT-size labels used by Plot Spectrum.
pub const SIZES: &[&str] = &[
    "128", "256", "512", "1024", "2048", "4096", "8192", "16384", "32768", "65536",
    "131072",
];

/// Frequency-axis labels used by Plot Spectrum.
pub const AXES: &[&str] = &["Linear frequency", "Log frequency"];

/// Calculated spectrum or lag data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpectrumData {
    /// Bin frequencies in Hz for Spectrum, or lag times in seconds otherwise.
    pub freq_or_lag: Vec<f32>,
    /// Processed values, in dB for Spectrum and linear units otherwise.
    pub values: Vec<f32>,
    /// Minimum calculated value.
    pub y_min: f32,
    /// Maximum calculated value.
    pub y_max: f32,
    /// Horizontal-axis label.
    pub x_label: String,
    /// Vertical-axis label.
    pub y_label: String,
    /// Sample rate used to calculate the data.
    pub rate: f64,
    /// FFT window size used to calculate the data.
    pub window_size: usize,
    /// Algorithm index used to calculate the data.
    pub algorithm: usize,
}

const MAX_ANALYSIS_SAMPLES: usize = 1_048_576;

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

    fn inverse_real_fft(&self, buffer: &mut [f32]) {
        let mut a = 2usize;
        let mut b = self.points * 2 - 2;
        let mut br1 = 1usize;

        while a < b {
            let sin = self.sin_table[self.bit_reversed[br1]];
            let cos = self.sin_table[self.bit_reversed[br1] + 1];

            let hrminus = buffer[a] - buffer[b];
            let hrplus = hrminus + buffer[b] * 2.0;
            let himinus = buffer[a + 1] - buffer[b + 1];
            let hiplus = himinus + buffer[b + 1] * 2.0;

            let v1 = sin * hrminus + cos * hiplus;
            let v2 = cos * hrminus - sin * hiplus;

            buffer[a] = (hrplus + v1) * 0.5;
            buffer[b] = buffer[a] - v1;
            buffer[a + 1] = (himinus - v2) * 0.5;
            buffer[b + 1] = buffer[a + 1] - himinus;

            a += 2;
            b -= 2;
            br1 += 1;
        }

        buffer[a + 1] = -buffer[a + 1];

        let v1 = 0.5f32 * (buffer[0] + buffer[1]);
        let v2 = 0.5f32 * (buffer[0] - buffer[1]);
        buffer[0] = v1;
        buffer[1] = v2;

        let mut butterflies_per_group = self.points / 2;
        let endptr1 = self.points * 2;

        while butterflies_per_group > 0 {
            let mut a = 0usize;
            let mut b = butterflies_per_group * 2;
            let mut sptr = 0usize;

            while a < endptr1 {
                let sin = self.sin_table[sptr];
                sptr += 1;
                let cos = self.sin_table[sptr];
                sptr += 1;
                let endptr2 = b;

                while a < endptr2 {
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
                b += butterflies_per_group * 2;
            }

            butterflies_per_group >>= 1;
        }
    }
}

fn real_fft(input: &[f32], real_out: &mut [f32], imag_out: &mut [f32]) {
    let size = input.len();
    let tables = RealFftTables::new(size);
    let mut buffer = input.to_vec();

    tables.real_fft(&mut buffer);

    for i in 1..(size / 2) {
        real_out[i] = buffer[tables.bit_reversed[i]];
        imag_out[i] = buffer[tables.bit_reversed[i] + 1];
    }

    real_out[0] = buffer[0];
    real_out[size / 2] = buffer[1];
    imag_out[0] = 0.0;
    imag_out[size / 2] = 0.0;

    for i in (size / 2 + 1)..size {
        real_out[i] = real_out[size - i];
        imag_out[i] = -imag_out[size - i];
    }
}

fn power_spectrum(input: &[f32], output: &mut [f32]) {
    let size = input.len();
    let tables = RealFftTables::new(size);
    let mut buffer = input.to_vec();

    tables.real_fft(&mut buffer);

    for i in 1..(size / 2) {
        let real = buffer[tables.bit_reversed[i]];
        let imag = buffer[tables.bit_reversed[i] + 1];
        output[i] = (real * real) + (imag * imag);
    }

    output[0] = buffer[0] * buffer[0];
    output[size / 2] = buffer[1] * buffer[1];
}

fn inverse_real_fft(real_in: &[f32], real_out: &mut [f32]) {
    let size = real_in.len();
    let tables = RealFftTables::new(size);
    let mut buffer = vec![0.0f32; size];

    for i in 0..(size / 2) {
        buffer[2 * i] = real_in[i];
        buffer[2 * i + 1] = 0.0;
    }
    buffer[1] = real_in[size / 2];

    tables.inverse_real_fft(&mut buffer);

    for i in 0..tables.points {
        real_out[i * 2] = buffer[tables.bit_reversed[i]];
        real_out[i * 2 + 1] = buffer[tables.bit_reversed[i] + 1];
    }
}

fn window_values(which: usize, size: usize) -> Vec<f32> {
    let mut window = vec![1.0f32; size];
    if size == 0 {
        return window;
    }

    let extra_sample = matches!(which, 2..=5);
    let n = if extra_sample { size - 1 } else { size };

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
        2 | 3 | 4 | 5 => {
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

            if extra_sample {
                let value = if which == 2 { 0.08 } else { 0.0 };
                window[n] = (window[n] as f64 * value) as f32;
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

    window
}

fn cubic_interpolate(y0: f32, y1: f32, y2: f32, y3: f32, x: f32) -> f32 {
    let a = (y0 as f64 / -6.0 + y1 as f64 / 2.0 - y2 as f64 / 2.0 + y3 as f64 / 6.0)
        as f32;
    let b = (y0 as f64 - 5.0 * y1 as f64 / 2.0 + 2.0 * y2 as f64 - y3 as f64 / 2.0)
        as f32;
    let c = (-11.0 * y0 as f64 / 6.0 + 3.0 * y1 as f64 - 3.0 * y2 as f64 / 2.0
        + y3 as f64 / 3.0) as f32;
    let d = y0;

    let xx = x * x;
    let xxx = xx * x;
    a * xxx + b * xx + c * x + d
}

fn cubic_maximize(y0: f32, y1: f32, y2: f32, y3: f32) -> (f32, f32) {
    let a = (y0 as f64 / -6.0 + y1 as f64 / 2.0 - y2 as f64 / 2.0 + y3 as f64 / 6.0)
        as f32;
    let b = (y0 as f64 - 5.0 * y1 as f64 / 2.0 + 2.0 * y2 as f64 - y3 as f64 / 2.0)
        as f32;
    let c = (-11.0 * y0 as f64 / 6.0 + 3.0 * y1 as f64 - 3.0 * y2 as f64 / 2.0
        + y3 as f64 / 3.0) as f32;
    let d = y0;

    let da = 3.0 * a;
    let db = 2.0 * b;
    let dc = c;
    let discriminant = db * db - 4.0 * da * dc;

    if discriminant < 0.0 {
        return (-1.0, 0.0);
    }

    let root = discriminant.sqrt();
    let x1 = (-db + root) / (2.0 * da);
    let x2 = (-db - root) / (2.0 * da);
    let dda = 2.0 * da;
    let ddb = db;

    let x = if dda * x1 + ddb < 0.0 { x1 } else { x2 };
    let value = a * x * x * x + b * x * x + c * x + d;
    (x, value)
}

fn selected_mono_samples(buf: &AudioBuffer) -> Vec<f32> {
    let Some(first) = buf.channels.first() else {
        return Vec::new();
    };

    let length = buf.channels.iter().map(Vec::len).min().unwrap_or(0);
    let length = length.min(MAX_ANALYSIS_SAMPLES);
    let mut samples = first[..length].to_vec();

    for channel in buf.channels.iter().skip(1) {
        for (sample, &value) in samples.iter_mut().zip(channel.iter()).take(length) {
            *sample += value;
        }
    }

    samples
}

/// Plot Spectrum analysis effect.
#[derive(Clone, Debug)]
pub struct FixPlotSpectrum {
    /// Algorithm choice index.
    pub algorithm: f64,
    /// Window-function choice index.
    pub function: f64,
    /// FFT-size choice index.
    pub size: f64,
    /// Frequency-axis choice index.
    pub axis: f64,
}

impl Default for FixPlotSpectrum {
    fn default() -> Self {
        Self {
            algorithm: 0.0,
            function: 3.0,
            size: 6.0,
            axis: 1.0,
        }
    }
}

impl FixPlotSpectrum {
    fn window_size(&self) -> usize {
        let index = (self.size.round() as usize).min(SIZES.len() - 1);
        SIZES[index].parse().unwrap_or(8192)
    }

    /// Calculates Plot Spectrum values from an audio buffer.
    pub fn analyse(&self, buf: &AudioBuffer) -> SpectrumData {
        let size = self.window_size();
        let rate = buf.sample_rate as f64;
        let algorithm = (self.algorithm.round() as usize).min(ALGORITHMS.len() - 1);
        let function = (self.function.round() as usize).min(FUNCTIONS.len() - 1);
        let samples = selected_mono_samples(buf);

        if size < 32 || samples.len() < size || rate <= 0.0 {
            return SpectrumData {
                rate,
                window_size: size,
                algorithm,
                ..SpectrumData::default()
            };
        }

        let half = size / 2;
        let window = window_values(function, size);
        let mut window_sum = 0.0f64;
        for &value in &window {
            window_sum += value as f64;
        }

        if window_sum > 0.0 {
            window_sum = 4.0 / (window_sum * window_sum);
        } else {
            window_sum = 1.0;
        }

        let mut processed = vec![0.0f32; size];
        let mut input = vec![0.0f32; size];
        let mut out = vec![0.0f32; size];
        let mut out2 = vec![0.0f32; size];

        let mut start = 0usize;
        let mut windows = 0usize;

        while start + size <= samples.len() {
            for i in 0..size {
                input[i] = window[i] * samples[start + i];
            }

            match algorithm {
                0 => {
                    power_spectrum(&input, &mut out);
                    for i in 0..half {
                        processed[i] += out[i];
                    }
                }
                1..=3 => {
                    real_fft(&input, &mut out, &mut out2);

                    for i in 0..size {
                        input[i] = (out[i] * out[i]) + (out2[i] * out2[i]);
                    }

                    if algorithm == 1 {
                        for value in &mut input {
                            *value = value.sqrt();
                        }
                    } else {
                        let exponent = 1.0f32 / 3.0f32;
                        for value in &mut input {
                            *value = value.powf(exponent);
                        }
                    }

                    real_fft(&input, &mut out, &mut out2);
                    for i in 0..half {
                        processed[i] += out[i];
                    }
                }
                4 => {
                    real_fft(&input, &mut out, &mut out2);

                    let min_power = (1.0e-20f64 * size as f64 * size as f64) as f32;
                    for i in 0..size {
                        let power = (out[i] * out[i]) + (out2[i] * out2[i]);
                        input[i] = if power < min_power {
                            min_power.ln()
                        } else {
                            power.ln()
                        };
                    }

                    inverse_real_fft(&input, &mut out);
                    for i in 0..half {
                        processed[i] += out[i];
                    }
                }
                _ => unreachable!(),
            }

            start += half;
            windows += 1;
        }

        let mut y_min = 1_000_000.0f32;
        let mut y_max = -1_000_000.0f32;

        match algorithm {
            0 => {
                let scale = window_sum / windows as f64;
                for value in processed.iter_mut().take(half) {
                    *value = (10.0 * (*value as f64 * scale).log10()) as f32;
                    if *value > y_max {
                        y_max = *value;
                    } else if *value < y_min {
                        y_min = *value;
                    }
                }
            }
            1 | 2 => {
                for value in processed.iter_mut().take(half) {
                    *value /= windows as f32;
                }

                y_min = processed[0];
                y_max = processed[0];
                for &value in processed.iter().take(half).skip(1) {
                    if value > y_max {
                        y_max = value;
                    } else if value < y_min {
                        y_min = value;
                    }
                }
            }
            3 => {
                for value in processed.iter_mut().take(half) {
                    *value /= windows as f32;
                }

                for value in processed.iter_mut().take(half) {
                    if *value < 0.0 {
                        *value = 0.0;
                    }
                }

                let mut temp = vec![0.0f32; size];
                temp[..half].copy_from_slice(&processed[..half]);

                for i in 0..half {
                    if i % 2 == 0 {
                        processed[i] -= temp[i / 2];
                    } else {
                        processed[i] -= (temp[i / 2] + temp[i / 2 + 1]) / 2.0;
                    }
                }

                for value in processed.iter_mut().take(half) {
                    if *value < 0.0 {
                        *value = 0.0;
                    }
                }

                y_min = processed[0];
                y_max = processed[0];
                for &value in processed.iter().take(half).skip(1) {
                    if value > y_max {
                        y_max = value;
                    } else if value < y_min {
                        y_min = value;
                    }
                }
            }
            4 => {
                for value in processed.iter_mut().take(half) {
                    *value /= windows as f32;
                }

                let ignore = 4usize;
                y_min = processed[ignore];
                y_max = processed[ignore];
                for i in (ignore + 1)..(half - ignore) {
                    let value = processed[i];
                    if value > y_max {
                        y_max = value;
                    } else if value < y_min {
                        y_min = value;
                    }
                }
            }
            _ => unreachable!(),
        }

        let values = processed[..half].to_vec();
        let freq_or_lag = (0..half)
            .map(|i| {
                if algorithm == 0 {
                    (i as f64 * rate / size as f64) as f32
                } else {
                    (i as f64 / rate) as f32
                }
            })
            .collect();

        SpectrumData {
            freq_or_lag,
            values,
            y_min,
            y_max,
            x_label: if algorithm == 0 { "Hz" } else { "s" }.to_string(),
            y_label: if algorithm == 0 {
                "dB".to_string()
            } else {
                String::new()
            },
            rate,
            window_size: size,
            algorithm,
        }
    }

    /// Formats calculated values in the same columns and order as Plot Spectrum export.
    pub fn export_text(&self, data: &SpectrumData, rate: f64) -> String {
        let mut output = String::new();

        if data.algorithm == 0 {
            output.push_str("Frequency (Hz)\tLevel (dB)\n");
            for i in 1..data.values.len() {
                let frequency = i as f64 * rate / data.window_size as f64;
                output.push_str(&format!("{frequency:.6}\t{:.6}\n", data.values[i]));
            }
        } else {
            output.push_str("Lag (seconds)\tFrequency (Hz)\tLevel\n");
            for i in 1..data.values.len() {
                let lag = i as f64 / rate;
                let frequency = rate / i as f64;
                output.push_str(&format!(
                    "{lag:.6}\t{frequency:.6}\t{:.6}\n",
                    data.values[i]
                ));
            }
        }

        output
    }

    /// Finds the nearest local peak using SpectrumAnalyst's cubic maximization.
    pub fn find_peak(&self, data: &SpectrumData, x: f32) -> (f32, f32) {
        let values = &data.values;
        let mut best_peak = 0.0f32;
        let mut best_value = 0.0f32;

        if values.len() > 1 {
            let mut up = values[1] > values[0];
            let mut best_dist = 1_000_000.0f32;

            for bin in 3..values.len().saturating_sub(1) {
                let now_up = values[bin] > values[bin - 1];
                if !now_up && up {
                    let left = bin - 2;
                    let (offset, value_at_max) = cubic_maximize(
                        values[left],
                        values[left + 1],
                        values[left + 2],
                        values[left + 3],
                    );
                    let max = left as f32 + offset;
                    let this_peak = if data.algorithm == 0 {
                        (max as f64 * data.rate / data.window_size as f64) as f32
                    } else {
                        (max as f64 / data.rate) as f32
                    };

                    let distance = (this_peak - x).abs();
                    if distance < best_dist {
                        best_peak = this_peak;
                        best_dist = distance;
                        best_value = value_at_max;
                        if this_peak > x {
                            break;
                        }
                    }
                }
                up = now_up;
            }
        }

        (best_peak, best_value)
    }
}

impl BuiltinEffect for FixPlotSpectrum {
    fn id(&self) -> &'static str {
        "PlotSpectrum"
    }

    fn name(&self) -> &'static str {
        "Plot Spectrum"
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Analyze
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice("Algorithm", "Algorithm", ALGORITHMS, 0),
            ParamDef::choice("Function", "Function", FUNCTIONS, 3),
            ParamDef::choice("Size", "Size", SIZES, 6),
            ParamDef::choice("Axis", "Axis", AXES, 1),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Algorithm" => Some(self.algorithm),
            "Function" => Some(self.function),
            "Size" => Some(self.size),
            "Axis" => Some(self.axis),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let definition = match key {
            "Algorithm" => ParamDef::choice("Algorithm", "Algorithm", ALGORITHMS, 0),
            "Function" => ParamDef::choice("Function", "Function", FUNCTIONS, 3),
            "Size" => ParamDef::choice("Size", "Size", SIZES, 6),
            "Axis" => ParamDef::choice("Axis", "Axis", AXES, 1),
            _ => return false,
        };

        let value = definition.clamp(value);
        match key {
            "Algorithm" => self.algorithm = value,
            "Function" => self.function = value,
            "Size" => self.size = value,
            "Axis" => self.axis = value,
            _ => return false,
        }
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let data = self.analyse(buf);
        ctx.report = self.export_text(&data, buf.sample_rate as f64);
        Ok(())
    }
}

/// Returns the Plot Spectrum analyzer as a built-in effect.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(FixPlotSpectrum::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::set_param;

    fn stereo_sine() -> AudioBuffer {
        let rate = 44_100u32;
        let channels = (0..2)
            .map(|channel| {
                (0..4096)
                    .map(|i| {
                        let phase = 2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32;
                        phase.sin() * if channel == 0 { 0.5 } else { 0.25 }
                    })
                    .collect()
            })
            .collect();
        AudioBuffer::from_channels(rate, channels)
    }

    #[test]
    fn default_runs_on_stereo_and_does_not_change_audio() {
        let mut buf = stereo_sine();
        let original = buf.channels.clone();
        let mut effect = FixPlotSpectrum::default();
        let mut ctx = EffectContext::default();
        effect.process(&mut buf, &mut ctx).unwrap();
        assert_eq!(buf.channels, original);
        assert!(ctx.report.starts_with("Frequency (Hz)\tLevel (dB)\n"));
    }

    #[test]
    fn empty_buffer_is_safe() {
        let mut buf = AudioBuffer::from_channels(44_100, vec![Vec::new(), Vec::new()]);
        let mut effect = FixPlotSpectrum::default();
        let mut ctx = EffectContext::default();
        effect.process(&mut buf, &mut ctx).unwrap();
        assert_eq!(ctx.report, "Frequency (Hz)\tLevel (dB)\n");
    }

    #[test]
    fn analysis_is_repeatable() {
        let buf = stereo_sine();
        let effect = FixPlotSpectrum::default();
        assert_eq!(effect.analyse(&buf), effect.analyse(&buf));
    }

    #[test]
    fn parameters_clamp() {
        let mut effect = FixPlotSpectrum::default();
        assert!(set_param(&mut effect, "Algorithm", 99.0));
        assert_eq!(effect.algorithm, (ALGORITHMS.len() - 1) as f64);
        assert!(set_param(&mut effect, "Size", -1.0));
        assert_eq!(effect.size, 0.0);
        assert!(!set_param(&mut effect, "unknown", 1.0));
    }
}
