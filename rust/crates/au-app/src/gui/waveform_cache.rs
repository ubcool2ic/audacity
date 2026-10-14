//! Thread-pool-backed waveform summaries for efficient clip drawing.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use au_core::timeline::Clip;
use au_core::AudioBuffer;
use au_runtime::compute::{waveform_summary, Exec, Summary};
use au_runtime::pool::{CancelToken, JobHandle};
use au_runtime::{JobError, Runtime};
use eframe::egui;

/// Audacity's first summary level: 256 samples per entry.
pub const LEVEL1: usize = 256;

/// Audacity's second summary level: 65,536 samples per entry.
pub const LEVEL2: usize = 65_536;

/// Identifies clip audio using its channel-0 pointer, lengths, channel count,
/// stretch ratio, and a fingerprint of 32 evenly spaced samples per channel.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WaveKey {
    channel0_ptr: usize,
    lengths: Vec<usize>,
    channels: usize,
    stretch_ratio_bits: u64,
    fingerprint: u64,
}

impl WaveKey {
    /// Creates a key for a clip without rendering or stretching its audio.
    pub fn of(clip: &Clip) -> Self {
        let channels = &clip.buffer.channels;
        let lengths: Vec<usize> = channels.iter().map(Vec::len).collect();
        let channel0_ptr = channels
            .first()
            .map_or(0, |channel| channel.as_ptr() as usize);

        let mut fingerprint = 0xcbf2_9ce4_8422_2325_u64;
        for channel in channels {
            if channel.is_empty() {
                continue;
            }

            for sample_index in 0..32 {
                let index = if channel.len() == 1 {
                    0
                } else {
                    (((channel.len() - 1) as u128 * sample_index as u128) / 31) as usize
                };
                for byte in channel[index].to_bits().to_le_bytes() {
                    fingerprint ^= u64::from(byte);
                    fingerprint = fingerprint.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }

        Self {
            channel0_ptr,
            lengths,
            channels: channels.len(),
            stretch_ratio_bits: clip.stretch_ratio.to_bits(),
            fingerprint,
        }
    }
}

/// Per-channel waveform summaries, retaining stretched audio when needed so
/// drawing never has to stretch the clip again.
pub struct WaveSummaries {
    /// Number of frames in the summarized audio.
    pub len: usize,
    /// Per-channel summaries at 256 samples per entry.
    pub level1: Vec<Vec<Summary>>,
    /// Per-channel summaries at 65,536 samples per entry.
    pub level2: Vec<Vec<Summary>>,
    /// Stretched audio, present only when the clip's stretch ratio is not 1.
    pub stretched: Option<AudioBuffer>,
}

impl WaveSummaries {
    /// Computes both summary levels, checking for cancellation between
    /// channels.
    pub fn compute(
        clip_buffer: &AudioBuffer,
        stretch_ratio: f64,
        cancel: &CancelToken,
    ) -> Result<Self, JobError> {
        cancel.check()?;

        let stretched = if stretch_ratio != 1.0 {
            Some(
                Clip {
                    name: String::new(),
                    start: 0,
                    buffer: clip_buffer.clone(),
                    stretch_ratio,
                }
                .rendered(),
            )
        } else {
            None
        };

        cancel.check()?;
        let audio = stretched.as_ref().unwrap_or(clip_buffer);
        let len = audio.len();
        let mut level1 = Vec::with_capacity(audio.n_channels());
        let mut level2 = Vec::with_capacity(audio.n_channels());

        for samples in &audio.channels {
            cancel.check()?;
            level1.push(waveform_summary(Exec::Parallel, samples, LEVEL1));
            level2.push(waveform_summary(Exec::Parallel, samples, LEVEL2));
        }

        Ok(Self {
            len,
            level1,
            level2,
            stretched,
        })
    }

    /// Returns the raw samples used for drawing at close zoom.
    ///
    /// Stretched audio is preferred; otherwise the clip's original channel is
    /// returned.
    pub fn raw<'a>(&'a self, clip_buffer: &'a AudioBuffer, channel: usize) -> Option<&'a [f32]> {
        self.stretched
            .as_ref()
            .and_then(|audio| audio.channels.get(channel))
            .or_else(|| clip_buffer.channels.get(channel))
            .map(Vec::as_slice)
    }

    /// Returns the exact minimum and maximum of samples in `[start, end)`.
    ///
    /// The range is clamped to the summarized audio. For sufficiently large
    /// ranges, whole blocks come from the coarsest suitable summary level and
    /// partial edge blocks are scanned from `raw`. Small ranges are scanned
    /// directly. Returns `None` for an empty range or when required raw edge
    /// samples are unavailable.
    pub fn min_max(
        &self,
        raw: Option<&[f32]>,
        channel: usize,
        start: usize,
        end: usize,
    ) -> Option<(f32, f32)> {
        let raw_len = raw.map_or(self.len, <[f32]>::len);
        let end = end.min(self.len).min(raw_len);
        let start = start.min(end);
        if start >= end {
            return None;
        }

        let samples = end - start;
        let summaries = self
            .level2
            .get(channel)
            .filter(|_| LEVEL2 <= samples / 2)
            .map(|summaries| (summaries, LEVEL2))
            .or_else(|| {
                self.level1
                    .get(channel)
                    .filter(|_| LEVEL1 <= samples / 2)
                    .map(|summaries| (summaries, LEVEL1))
            });

        let Some((summaries, block)) = summaries else {
            return raw.and_then(|samples| min_max_slice(samples, start, end));
        };

        let first_full = if start % block == 0 {
            start
        } else {
            (start / block).saturating_add(1).saturating_mul(block)
        };
        let full_end = end / block * block;

        if first_full > full_end {
            return raw.and_then(|samples| min_max_slice(samples, start, end));
        }

        let summary_start = first_full / block;
        let summary_end = full_end / block;
        if summary_end > summaries.len() {
            return raw.and_then(|samples| min_max_slice(samples, start, end));
        }

        let left_end = first_full.min(end);
        let right_start = full_end.max(start);
        let has_edges = start < left_end || right_start < end;
        if has_edges && raw.is_none() {
            return None;
        }

        let mut result = None;
        if start < left_end {
            include_min_max(
                &mut result,
                min_max_slice(raw?, start, left_end)?,
            );
        }

        for summary in &summaries[summary_start..summary_end] {
            include_min_max(&mut result, (summary.min, summary.max));
        }

        if right_start < end {
            include_min_max(
                &mut result,
                min_max_slice(raw?, right_start, end)?,
            );
        }

        result
    }
}

fn min_max_slice(samples: &[f32], start: usize, end: usize) -> Option<(f32, f32)> {
    let samples = samples.get(start..end)?;
    let first = *samples.first()?;
    Some(samples.iter().skip(1).fold(
        (first, first),
        |(low, high), &sample| (low.min(sample), high.max(sample)),
    ))
}

fn include_min_max(result: &mut Option<(f32, f32)>, next: (f32, f32)) {
    *result = Some(match *result {
        Some((low, high)) => (low.min(next.0), high.max(next.1)),
        None => next,
    });
}

/// LRU cache of waveform summaries computed on a dedicated two-thread runtime.
pub struct WaveformCache {
    rt: Runtime,
    entries: HashMap<WaveKey, (Arc<WaveSummaries>, u64)>,
    pending: HashMap<WaveKey, JobHandle<WaveSummaries>>,
    pending_last_used: HashMap<WaveKey, u64>,
    frame: u64,
}

impl WaveformCache {
    /// Creates a cache backed by a dedicated two-thread runtime.
    pub fn new() -> Self {
        Self {
            rt: Runtime::new(2, 64),
            entries: HashMap::new(),
            pending: HashMap::new(),
            pending_last_used: HashMap::new(),
            frame: 0,
        }
    }

    /// Returns ready summaries or queues a job for the clip.
    ///
    /// While a job is pending, requests a repaint after 30 milliseconds.
    pub fn get(&mut self, clip: &Clip, ctx: &egui::Context) -> Option<Arc<WaveSummaries>> {
        let key = WaveKey::of(clip);

        if let Some((summaries, last_used)) = self.entries.get_mut(&key) {
            *last_used = self.frame;
            return Some(Arc::clone(summaries));
        }

        if self.pending.contains_key(&key) {
            self.pending_last_used.insert(key, self.frame);
            ctx.request_repaint_after(Duration::from_millis(30));
            return None;
        }

        let buffer = clip.buffer.clone();
        let stretch_ratio = clip.stretch_ratio;
        let units = buffer
            .len()
            .saturating_mul(buffer.n_channels())
            .max(1);
        let total_units = u64::try_from(units).unwrap_or(u64::MAX);

        let handle = self.rt.submit(total_units, move |ctx| {
            ctx.install(|| WaveSummaries::compute(&buffer, stretch_ratio, &ctx.cancel))
        });

        self.pending.insert(key.clone(), handle);
        self.pending_last_used.insert(key, self.frame);
        ctx.request_repaint_after(Duration::from_millis(30));
        None
    }

    /// Polls jobs, evicts entries unused for 300 frames, and enforces the
    /// 64-entry cache limit.
    pub fn end_frame(&mut self) {
        self.frame = self.frame.saturating_add(1);

        let finished: Vec<(WaveKey, Result<WaveSummaries, JobError>)> = self
            .pending
            .iter()
            .filter_map(|(key, handle)| handle.try_result().map(|result| (key.clone(), result)))
            .collect();

        for (key, result) in finished {
            self.pending.remove(&key);
            let last_used = self
                .pending_last_used
                .remove(&key)
                .unwrap_or(self.frame);

            if self.frame.saturating_sub(last_used) < 300 {
                if let Ok(summaries) = result {
                    self.entries.insert(
                        key,
                        (Arc::new(summaries), last_used),
                    );
                }
            }
        }

        self.entries
            .retain(|_, (_, last_used)| self.frame.saturating_sub(*last_used) < 300);

        let stale_pending: Vec<WaveKey> = self
            .pending_last_used
            .iter()
            .filter_map(|(key, last_used)| {
                (self.frame.saturating_sub(*last_used) >= 300).then(|| key.clone())
            })
            .collect();

        for key in stale_pending {
            if let Some(handle) = self.pending.remove(&key) {
                handle.cancel.cancel();
            }
            self.pending_last_used.remove(&key);
        }

        while self.entries.len() > 64 {
            let Some(least_recently_used) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, last_used))| *last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.entries.remove(&least_recently_used);
        }
    }

    /// Returns the number of ready cache entries.
    #[allow(dead_code)] // diagnostics and tests
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns the number of currently pending jobs.
    #[allow(dead_code)] // diagnostics and tests
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
}

impl Default for WaveformCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Instant;

    fn summaries_for(samples: &[f32]) -> WaveSummaries {
        WaveSummaries {
            len: samples.len(),
            level1: vec![waveform_summary(Exec::Parallel, samples, LEVEL1)],
            level2: vec![waveform_summary(Exec::Parallel, samples, LEVEL2)],
            stretched: None,
        }
    }

    fn check_ranges(samples: &[f32]) {
        let summaries = summaries_for(samples);
        let mut ranges = vec![
            (0, samples.len()),
            (0, LEVEL2 * 2),
            (LEVEL2, LEVEL2 * 2),
            (LEVEL1, LEVEL1 * 10),
            (LEVEL1 + 1, LEVEL1 * 9 + 3),
            (LEVEL2 - 1, LEVEL2 + 1),
        ];

        for block in [LEVEL1, LEVEL2] {
            for multiple in 0..=2 {
                let start = multiple * block;
                let end = (start + block * 3).min(samples.len());
                if start < end {
                    ranges.push((start, end));
                }
                let unaligned_start = start.saturating_add(1);
                let unaligned_end = end.saturating_sub(1);
                if unaligned_start < unaligned_end {
                    ranges.push((unaligned_start, unaligned_end));
                }
            }
        }

        let mut state = 0x1234_5678_u64;
        for _ in 0..800 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let start = (state as usize) % samples.len();
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let end = start + 1 + (state as usize) % (samples.len() - start);
            ranges.push((start, end));
        }

        for (start, end) in ranges {
            let actual = summaries.min_max(Some(samples), 0, start, end);
            let expected = min_max_slice(samples, start, end);
            assert_eq!(actual, expected, "range [{start}, {end})");
        }
    }

    #[test]
    fn min_max_matches_brute_force_for_ramp_and_sine_ranges() {
        let length = LEVEL2 * 2 + 17;
        let ramp: Vec<f32> = (0..length).map(|index| index as f32 / length as f32).collect();
        let sine: Vec<f32> = (0..length)
            .map(|index| (index as f32 * 0.013).sin())
            .collect();

        check_ranges(&ramp);
        check_ranges(&sine);
    }

    #[test]
    fn wave_key_changes_when_a_sample_changes() {
        let mut clip = Clip {
            name: String::from("test"),
            start: 0,
            buffer: AudioBuffer {
                sample_rate: 48_000,
                channels: vec![vec![0.0; 1024]],
            },
            stretch_ratio: 1.0,
        };

        let before = WaveKey::of(&clip);
        clip.buffer.channels[0][0] = 0.5;
        let after = WaveKey::of(&clip);
        assert_ne!(before, after);
    }

    #[test]
    fn cache_completes_and_stretched_length_matches_clip() {
        let clip = Clip {
            name: String::from("stretched"),
            start: 0,
            buffer: AudioBuffer {
                sample_rate: 48_000,
                channels: vec![
                    (0..4_800)
                        .map(|index| (index as f32 * 0.017).sin())
                        .collect(),
                ],
            },
            stretch_ratio: 2.0,
        };
        let context = egui::Context::default();
        let mut cache = WaveformCache::new();
        let deadline = Instant::now() + Duration::from_secs(5);

        let summaries = loop {
            if let Some(summaries) = cache.get(&clip, &context) {
                break summaries;
            }

            cache.end_frame();
            assert!(
                Instant::now() < deadline,
                "waveform summary job did not complete within five seconds"
            );
            thread::sleep(Duration::from_millis(5));
        };

        let expected_len = clip.len();
        let tolerance = (expected_len / 50).max(1);
        assert!(summaries.len.abs_diff(expected_len) <= tolerance);
        assert!(summaries.stretched.is_some());
    }
}
