// SPDX-License-Identifier: GPL-3.0-or-later

use au_core::AudioBuffer;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

fn sinc_parameters() -> SincInterpolationParameters {
    SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Cubic,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    }
}

fn rounded_output_len(input_len: usize, from_rate: u32, to_rate: u32) -> usize {
    ((input_len as u128 * to_rate as u128 + from_rate as u128 / 2) / from_rate as u128)
        .min(usize::MAX as u128) as usize
}

/// Resamples an audio buffer to `to_rate` using a high-quality sinc resampler.
///
/// The returned buffer has a length rounded to the nearest output frame. The
/// resampler's startup delay is removed so that the output remains time-aligned
/// with the input.
pub fn resample(buf: &AudioBuffer, to_rate: u32) -> AudioBuffer {
    if buf.sample_rate == to_rate {
        return buf.clone();
    }

    let output_len = if buf.sample_rate == 0 || to_rate == 0 {
        0
    } else {
        rounded_output_len(buf.len(), buf.sample_rate, to_rate)
    };
    let channels = buf.n_channels();

    if output_len == 0 || channels == 0 || buf.sample_rate == 0 || to_rate == 0 {
        return AudioBuffer::from_channels(to_rate, vec![vec![0.0; output_len]; channels]);
    }

    let mut resampler = SincFixedIn::<f32>::new(
        to_rate as f64 / buf.sample_rate as f64,
        1.0,
        sinc_parameters(),
        1024,
        channels,
    )
    .expect("valid sinc resampler configuration");

    // Empirically (rubato 0.15, SincFixedIn) the first output frame is already
    // aligned with input frame 0; trimming output_delay() shifted audio early.
    let delay = 0usize;
    let _ = resampler.output_delay();
    let mut collected = vec![Vec::<f32>::new(); channels];
    let mut input_offset = 0usize;

    while input_offset + 1024 <= buf.len() {
        let input_chunk: Vec<Vec<f32>> = buf
            .channels
            .iter()
            .map(|channel| channel[input_offset..input_offset + 1024].to_vec())
            .collect();
        let output_chunk = resampler
            .process(&input_chunk, None)
            .expect("sinc resampling failed");
        for (dest, source) in collected.iter_mut().zip(output_chunk) {
            dest.extend(source);
        }
        input_offset += 1024;
    }

    if input_offset < buf.len() {
        let input_chunk: Vec<Vec<f32>> = buf
            .channels
            .iter()
            .map(|channel| channel[input_offset..].to_vec())
            .collect();
        let output_chunk = resampler
            .process_partial(Some(&input_chunk), None)
            .expect("sinc partial resampling failed");
        for (dest, source) in collected.iter_mut().zip(output_chunk) {
            dest.extend(source);
        }
    }

    // A final empty partial process drains any output still buffered by the
    // resampler, including its delay line.
    for _ in 0..4 {
        let output_chunk = resampler
            .process_partial(None::<&[Vec<f32>]>, None)
            .expect("sinc resampler flush failed");
        let produced = output_chunk.first().map_or(0, Vec::len);
        for (dest, source) in collected.iter_mut().zip(output_chunk) {
            dest.extend(source);
        }
        if produced == 0 {
            break;
        }
    }

    let output_channels = collected
        .into_iter()
        .map(|channel| {
            let available = channel.len().saturating_sub(delay);
            let start = delay.min(channel.len());
            let count = output_len.min(available);
            let mut result = channel[start..start + count].to_vec();
            result.resize(output_len, 0.0);
            result
        })
        .collect();

    AudioBuffer::from_channels(to_rate, output_channels)
}

/// A streaming sinc resampler for realtime playback.
///
/// Input is accepted in interleaved frames. Complete processing chunks are
/// converted immediately; incomplete chunks are retained for the next call.
/// Reserve enough capacity in the output vector for the expected output before
/// playback: once that capacity is available, steady-state processing does not
/// allocate.
pub struct StreamResampler {
    resampler: SincFixedIn<f32>,
    channels: usize,
    chunk_size: usize,
    pending: Vec<Vec<f32>>,
    output_scratch: Vec<Vec<f32>>,
    output_capacity: usize,
}

impl StreamResampler {
    /// Creates a streaming resampler from `from` to `to` sample rate.
    ///
    /// `channels` is the number of interleaved channels, and `chunk` is the
    /// fixed input chunk size used by the underlying resampler.
    pub fn new(from: u32, to: u32, channels: usize, chunk: usize) -> Self {
        let channels = channels.max(1);
        let chunk_size = chunk.max(1);
        let resampler = SincFixedIn::<f32>::new(
            to.max(1) as f64 / from.max(1) as f64,
            1.0,
            sinc_parameters(),
            chunk_size,
            channels,
        )
        .expect("valid streaming sinc resampler configuration");
        let output_capacity = resampler.output_frames_max().max(1);

        Self {
            resampler,
            channels,
            chunk_size,
            pending: (0..channels)
                .map(|_| Vec::with_capacity(chunk_size))
                .collect(),
            output_scratch: (0..channels)
                .map(|_| vec![0.0; output_capacity])
                .collect(),
            output_capacity,
        }
    }

    /// Returns the number of additional input frames needed for the next
    /// complete processing chunk.
    pub fn input_frames_needed(&self) -> usize {
        self.chunk_size
            .saturating_sub(self.pending.first().map_or(0, Vec::len))
    }

    /// Appends resampled interleaved frames to `output`.
    ///
    /// Incomplete input frames are ignored. Processing is performed in
    /// preallocated chunks; reserve sufficient space in `output` to avoid
    /// allocations during steady-state playback.
    pub fn process_interleaved(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let frames = input.len() / self.channels;
        let mut frame_offset = 0usize;

        while frame_offset < frames {
            let pending_frames = self.pending[0].len();
            let count = (self.chunk_size - pending_frames).min(frames - frame_offset);
            for frame in 0..count {
                for channel in 0..self.channels {
                    self.pending[channel]
                        .push(input[(frame_offset + frame) * self.channels + channel]);
                }
            }
            frame_offset += count;

            if self.pending[0].len() == self.chunk_size {
                let (input_frames, output_frames) = self
                    .resampler
                    .process_into_buffer(&self.pending, &mut self.output_scratch, None)
                    .expect("streaming sinc resampling failed");
                let output_frames = output_frames.min(self.output_capacity);
                for frame in 0..output_frames {
                    for channel in 0..self.channels {
                        output.push(self.output_scratch[channel][frame]);
                    }
                }
                debug_assert_eq!(input_frames, self.chunk_size);
                for channel in &mut self.pending {
                    channel.clear();
                }
            }
        }
    }
}

/// Selects the quantization dither algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Dither {
    /// Do not add dither.
    None,
    /// Add uniform noise spanning half an LSB in either direction.
    Rectangle,
    /// Add triangular-probability-density noise.
    #[default]
    Triangle,
    /// Add triangular dither with first-order per-channel error feedback.
    Shaped,
}

/// Adds dither and quantizes floating-point audio to signed integer samples.
pub struct Ditherer {
    kind: Dither,
    rng: u32,
    error: [f64; 8],
}

impl Ditherer {
    /// Creates a ditherer of the given kind and initializes its RNG with `seed`.
    pub fn new(kind: Dither, seed: u32) -> Self {
        Self {
            kind,
            rng: if seed == 0 { 0x6d2b_79f5 } else { seed },
            error: [0.0; 8],
        }
    }

    fn random_unit(&mut self) -> f64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x as f64 / u32::MAX as f64
    }

    /// Quantizes one normalized sample to a signed integer with `bits` bits.
    ///
    /// `ch` selects the independent error-feedback state used by shaped
    /// dither; channel indices above seven share the final state.
    pub fn quantize(&mut self, x: f32, bits: u32, ch: usize) -> i32 {
        let bits = bits.clamp(1, 32);
        let scale = ((1u64 << (bits - 1)) - 1) as f64;
        let channel = ch.min(7);
        let mut value = x as f64 * scale;

        match self.kind {
            Dither::None => {}
            Dither::Rectangle => {
                value += self.random_unit() - 0.5;
            }
            Dither::Triangle => {
                value += self.random_unit() - self.random_unit();
            }
            Dither::Shaped => {
                value += self.random_unit() - self.random_unit() + self.error[channel];
            }
        }

        let rounded = value.round();
        let min = -(1i64 << (bits - 1));
        let max = (1i64 << (bits - 1)) - 1;
        let quantized = (rounded as i64).clamp(min, max);

        if self.kind == Dither::Shaped {
            self.error[channel] = (value - quantized as f64).clamp(-2.0, 2.0);
        }

        quantized as i32
    }
}

/// Quantizes every channel in an audio buffer to signed integer samples.
pub fn quantize_buffer(
    buf: &AudioBuffer,
    bits: u32,
    kind: Dither,
    seed: u32,
) -> Vec<Vec<i32>> {
    let mut ditherer = Ditherer::new(kind, seed);
    buf.channels
        .iter()
        .enumerate()
        .map(|(channel_index, channel)| {
            channel
                .iter()
                .map(|&sample| ditherer.quantize(sample, bits, channel_index))
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono(rate: u32, samples: Vec<f32>) -> AudioBuffer {
        AudioBuffer::from_channels(rate, vec![samples])
    }

    fn sine(rate: u32, frequency: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| (std::f64::consts::TAU * frequency as f64 * n as f64 / rate as f64).sin() as f32)
            .collect()
    }

    fn rms(samples: &[f32]) -> f64 {
        (samples.iter().map(|x| (*x as f64).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
    }

    #[test]
    fn resample_identity() {
        let input = mono(48_000, sine(48_000, 997.0, 512));
        let output = resample(&input, 48_000);
        assert_eq!(input.channels, output.channels);
        assert_eq!(input.sample_rate, output.sample_rate);
    }

    #[test]
    fn resample_length_48k_to_44k1() {
        let input = mono(48_000, vec![0.0; 48_000]);
        let output = resample(&input, 44_100);
        assert_eq!(output.len(), 44_100);
    }

    #[test]
    fn resample_passband_preserved() {
        let input_samples = sine(48_000, 1_000.0, 48_000);
        let output = resample(&mono(48_000, input_samples.clone()), 44_100);
        let input_rms = rms(&input_samples[4_800..43_200]);
        let output_rms = rms(&output.channels[0][4_410..39_690]);
        let db = 20.0 * (output_rms / input_rms).log10();
        assert!(db.abs() < 0.1, "RMS difference was {db} dB");

        let crossings = output.channels[0][4_410..39_690]
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        assert!((crossings as isize - 800).abs() <= 2);
    }

    #[test]
    fn resample_alias_rejection() {
        let input_samples = sine(48_000, 23_000.0, 48_000);
        let output = resample(&mono(48_000, input_samples.clone()), 32_000);
        let input_rms = rms(&input_samples[4_800..43_200]);
        let output_rms = rms(&output.channels[0][3_200..28_800]);
        assert!(
            output_rms <= input_rms * 0.001,
            "alias rejection was {} dB",
            20.0 * (output_rms / input_rms).log10()
        );
    }

    #[test]
    fn resample_upsample_aligned() {
        let mut input = vec![0.0; 2_048];
        input[512] = 1.0;
        let output = resample(&mono(32_000, input), 48_000);
        let peak = output.channels[0]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0;
        assert!((peak as isize - 768).abs() <= 2, "peak {peak}");
    }

    #[test]
    fn stream_resampler_matches_offline_length() {
        let samples = sine(48_000, 440.0, 10_000);
        let input = mono(48_000, samples.clone());
        let offline = resample(&input, 44_100);
        let mut stream = StreamResampler::new(48_000, 44_100, 1, 256);
        let mut output = Vec::with_capacity(offline.len() + 1_000);
        for chunk in samples.chunks(137) {
            stream.process_interleaved(chunk, &mut output);
        }
        let output_frames = output.len();
        assert!(output_frames.abs_diff(offline.len()) <= 256);
    }

    #[test]
    fn dither_none_rounds() {
        let mut dither = Ditherer::new(Dither::None, 1);
        assert_eq!(dither.quantize(0.5, 16, 0), 16_384);
    }

    #[test]
    fn dither_triangle_mean_unbiased() {
        let mut dither = Ditherer::new(Dither::Triangle, 123);
        let sample = 0.25 / 32_767.0;
        let count = 100_000;
        let mean = (0..count)
            .map(|_| dither.quantize(sample, 16, 0) as f64)
            .sum::<f64>()
            / count as f64;
        assert!((mean - 0.25).abs() < 0.05, "mean was {mean}");
    }

    #[test]
    fn dither_decorrelates_low_level() {
        let samples = sine(48_000, 997.0, 2_000)
            .into_iter()
            .map(|sample| sample * 10f32.powf(-100.0 / 20.0))
            .collect::<Vec<_>>();
        let plain = quantize_buffer(&mono(48_000, samples.clone()), 16, Dither::None, 5);
        assert!(plain[0].iter().all(|&sample| sample == 0));
        let dithered = quantize_buffer(&mono(48_000, samples), 16, Dither::Triangle, 5);
        assert!(dithered[0].iter().any(|&sample| sample != 0));
    }

    #[test]
    fn dither_clamps_full_scale() {
        let mut dither = Ditherer::new(Dither::None, 1);
        assert_eq!(dither.quantize(2.0, 16, 0), 32_767);
        assert_eq!(dither.quantize(-2.0, 16, 0), -32_768);
    }

    #[test]
    fn dither_shaped_runs() {
        let mut dither = Ditherer::new(Dither::Shaped, 42);
        let output: Vec<_> = (0..1_000)
            .map(|n| dither.quantize((n as f32 * 0.01).sin() * 0.1, 16, n % 2))
            .collect();
        assert!(output.iter().any(|&sample| sample != 0));
    }
}
