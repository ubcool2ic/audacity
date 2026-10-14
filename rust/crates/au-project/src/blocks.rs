// SPDX-License-Identifier: GPL-3.0-or-later

//! Sample encoding and summary generation for Audacity `.aup3` sample blocks.

/// The sample formats supported by Audacity sample blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// Signed 16-bit PCM.
    Int16,
    /// Signed 24-bit PCM stored in a 32-bit integer.
    Int24,
    /// 32-bit floating-point samples.
    Float,
}

impl SampleFormat {
    /// Returns Audacity's format code for this sample format.
    pub fn code(self) -> u32 {
        match self {
            Self::Int16 => 0x0002_0001,
            Self::Int24 => 0x0004_0001,
            Self::Float => 0x0004_000F,
        }
    }

    /// Returns the sample format for an Audacity format code, if recognized.
    pub fn from_code(code: u32) -> Option<Self> {
        match code {
            0x0002_0001 => Some(Self::Int16),
            0x0004_0001 => Some(Self::Int24),
            0x0004_000F => Some(Self::Float),
            _ => None,
        }
    }

    /// Returns the number of bytes used to store one sample.
    pub fn bytes_per_sample(self) -> usize {
        match self {
            Self::Int16 => 2,
            Self::Int24 | Self::Float => 4,
        }
    }

    /// Returns Audacity's default maximum number of samples per block.
    pub fn max_samples(self) -> usize {
        1_048_576 / self.bytes_per_sample()
    }
}

/// Encodes samples as little-endian bytes in the requested format.
///
/// Integer samples are rounded and clamped to the representable range.
pub fn encode_samples(samples: &[f32], fmt: SampleFormat) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * fmt.bytes_per_sample());

    match fmt {
        SampleFormat::Int16 => {
            for &sample in samples {
                let value = (sample * 32_768.0)
                    .round()
                    .clamp(-32_768.0, 32_767.0) as i16;
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        SampleFormat::Int24 => {
            for &sample in samples {
                let value = (sample * 8_388_608.0)
                    .round()
                    .clamp(-8_388_608.0, 8_388_607.0) as i32;
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        SampleFormat::Float => {
            for &sample in samples {
                bytes.extend_from_slice(&sample.to_le_bytes());
            }
        }
    }

    bytes
}

/// Decodes little-endian sample bytes, ignoring any trailing partial sample.
pub fn decode_samples(bytes: &[u8], fmt: SampleFormat) -> Vec<f32> {
    let sample_size = fmt.bytes_per_sample();
    let sample_count = bytes.len() / sample_size;
    let mut samples = Vec::with_capacity(sample_count);

    for chunk in bytes.chunks_exact(sample_size) {
        let sample = match fmt {
            SampleFormat::Int16 => {
                let value = i16::from_le_bytes([chunk[0], chunk[1]]);
                value as f32 / 32_768.0
            }
            SampleFormat::Int24 => {
                let value = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                value as f32 / 8_388_608.0
            }
            SampleFormat::Float => {
                f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]])
            }
        };
        samples.push(sample);
    }

    samples
}

/// Summary data for a block of samples.
///
/// `summary256` and `summary64k` store frames of three little-endian `f32`
/// values each: minimum, maximum, and RMS.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// Minimum sample value in the block.
    pub min: f32,
    /// Maximum sample value in the block.
    pub max: f32,
    /// RMS of all samples in the block.
    pub rms: f64,
    /// Summary frames for groups of 256 samples.
    pub summary256: Vec<u8>,
    /// Summary frames for groups of 65,536 samples.
    pub summary64k: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Frame {
    min: f32,
    max: f32,
    rms: f32,
}

fn append_frame(bytes: &mut Vec<u8>, frame: Frame) {
    bytes.extend_from_slice(&frame.min.to_le_bytes());
    bytes.extend_from_slice(&frame.max.to_le_bytes());
    bytes.extend_from_slice(&frame.rms.to_le_bytes());
}

/// Calculates block-wide statistics and Audacity's 256-sample and
/// 65,536-sample summary frames.
pub fn summarize(samples: &[f32]) -> Summary {
    let n = samples.len();

    if n == 0 {
        return Summary {
            min: 0.0,
            max: 0.0,
            rms: 0.0,
            summary256: Vec::new(),
            summary64k: Vec::new(),
        };
    }

    let frames64k = n / 65_536 + usize::from(n % 65_536 != 0);
    let frames256 = frames64k * 256;
    let sum_len = n / 256 + usize::from(n % 256 != 0);

    let mut summary_frames = Vec::with_capacity(frames256);
    let mut total_squares = 0.0f64;
    let mut fraction = 0.0f64;
    let mut summaries = 256usize;

    for i in 0..sum_len {
        let offset = i * 256;
        let mut min = samples[offset];
        let mut max = min;
        let mut sumsq = min * min;
        let mut jcount = 256usize;

        if jcount > n - offset {
            jcount = n - offset;
            fraction = 1.0 - (jcount as f64 / 256.0);
        }

        for &sample in &samples[offset + 1..offset + jcount] {
            sumsq += sample * sample;
            if sample < min {
                min = sample;
            } else if sample > max {
                max = sample;
            }
        }

        total_squares += sumsq as f64;
        summary_frames.push(Frame {
            min,
            max,
            rms: (sumsq / jcount as f32).sqrt(),
        });
    }

    for _ in sum_len..frames256 {
        summaries -= 1;
        summary_frames.push(Frame {
            min: f32::MAX,
            max: -f32::MAX,
            rms: 0.0,
        });
    }

    let rms = (total_squares / n as f64).sqrt();
    let mut summary64k_frames = Vec::with_capacity(frames64k);

    for i in 0..frames64k {
        let offset = i * 256;
        let first = summary_frames[offset];
        let mut min = first.min;
        let mut max = first.max;
        let mut sumsq = first.rms * first.rms;

        for frame in &summary_frames[offset + 1..offset + 256] {
            if frame.min < min {
                min = frame.min;
            }
            if frame.max > max {
                max = frame.max;
            }
            sumsq += frame.rms * frame.rms;
        }

        let denominator = if i < frames64k - 1 {
            256.0
        } else {
            summaries as f64 - fraction
        };

        summary64k_frames.push(Frame {
            min,
            max,
            rms: ((sumsq as f64) / denominator).sqrt() as f32,
        });
    }

    let mut min = summary64k_frames[0].min;
    let mut max = summary64k_frames[0].max;
    for frame in &summary64k_frames[1..] {
        if frame.min < min {
            min = frame.min;
        }
        if frame.max > max {
            max = frame.max;
        }
    }

    let mut summary256 = Vec::with_capacity(frames256 * 12);
    for &frame in &summary_frames {
        append_frame(&mut summary256, frame);
    }

    let mut summary64k = Vec::with_capacity(frames64k * 12);
    for &frame in &summary64k_frames {
        append_frame(&mut summary64k, frame);
    }

    Summary {
        min,
        max,
        rms,
        summary256,
        summary64k,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_codes_round_trip() {
        for (format, code) in [
            (SampleFormat::Int16, 0x0002_0001),
            (SampleFormat::Int24, 0x0004_0001),
            (SampleFormat::Float, 0x0004_000F),
        ] {
            assert_eq!(format.code(), code);
            assert_eq!(SampleFormat::from_code(code), Some(format));
        }
        assert_eq!(SampleFormat::from_code(0), None);
    }

    #[test]
    fn integer_encode_decode_is_within_one_lsb() {
        let samples = [-1.0, -0.75, -0.1, 0.0, 0.1, 0.75, 1.0];

        for (format, lsb) in [
            (SampleFormat::Int16, 1.0 / 32_768.0),
            (SampleFormat::Int24, 1.0 / 8_388_608.0),
        ] {
            let encoded = encode_samples(&samples, format);
            let decoded = decode_samples(&encoded, format);
            for (&original, &actual) in samples.iter().zip(&decoded) {
                assert!((original - actual).abs() <= lsb);
            }
        }
    }

    #[test]
    fn integer_encoding_clamps() {
        let samples = [-2.0, 2.0];

        let int16 = encode_samples(&samples, SampleFormat::Int16);
        assert_eq!(i16::from_le_bytes([int16[0], int16[1]]), i16::MIN);
        assert_eq!(i16::from_le_bytes([int16[2], int16[3]]), i16::MAX);

        let int24 = encode_samples(&samples, SampleFormat::Int24);
        assert_eq!(
            i32::from_le_bytes([int24[0], int24[1], int24[2], int24[3]]),
            -8_388_608
        );
        assert_eq!(
            i32::from_le_bytes([int24[4], int24[5], int24[6], int24[7]]),
            8_388_607
        );
    }

    #[test]
    fn summary_sizes_match_audacity_frames() {
        let summary_44100 = summarize(&vec![0.0; 44_100]);
        assert_eq!(summary_44100.summary256.len(), 3_072);
        assert_eq!(summary_44100.summary64k.len(), 12);

        let summary_96000 = summarize(&vec![0.0; 96_000]);
        assert_eq!(summary_96000.summary256.len(), 6_144);
        assert_eq!(summary_96000.summary64k.len(), 24);
    }

    #[test]
    fn summary_statistics_match_known_signal() {
        let summary = summarize(&[1.0, -1.0, 0.5, -0.5]);
        assert_eq!(summary.min, -1.0);
        assert_eq!(summary.max, 1.0);
        assert!((summary.rms - 0.625f64.sqrt()).abs() < 1e-12);
    }
}
