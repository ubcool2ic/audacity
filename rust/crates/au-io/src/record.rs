// SPDX-License-Identifier: GPL-3.0-or-later

#![forbid(unsafe_code)]

use au_core::AudioBuffer;

/// The way a recorded take is placed on the timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordMode {
    /// Create a new track with the take beginning at the requested offset.
    NewTrack,
    /// Create a separate track with the take beginning at the requested offset.
    Overdub,
    /// Replace a portion of an existing track with a punch-and-roll take.
    PunchAndRoll {
        /// Timeline frame at which the replacement begins.
        punch_in: usize,
        /// Exclusive timeline frame at which the replacement ends.
        ///
        /// If absent, the replacement ends at the end of the take.
        punch_out: Option<usize>,
        /// Number of frames recorded before the punch-in point.
        preroll: usize,
    },
}

/// Applies a fixed recording latency offset to an audio buffer.
///
/// Positive latency drops that many frames from the start of the recording.
/// Negative latency prepends that many silent frames.
pub fn compensate_latency(recorded: &AudioBuffer, latency_frames: i64) -> AudioBuffer {
    let channels = if latency_frames >= 0 {
        let drop_frames = usize::try_from(latency_frames as u64).unwrap_or(usize::MAX);
        recorded
            .channels
            .iter()
            .map(|channel| channel.iter().skip(drop_frames).copied().collect())
            .collect()
    } else {
        let pad_frames = usize::try_from(latency_frames.unsigned_abs()).unwrap_or(usize::MAX);
        recorded
            .channels
            .iter()
            .map(|channel| {
                let mut adjusted = Vec::with_capacity(pad_frames.saturating_add(channel.len()));
                adjusted.resize(pad_frames, 0.0);
                adjusted.extend_from_slice(channel);
                adjusted
            })
            .collect()
    };

    AudioBuffer {
        sample_rate: recorded.sample_rate,
        channels,
    }
}

/// Places a take into a new track or replaces a region of an existing track.
///
/// For punch-and-roll recording, the take starts at
/// `punch_in.saturating_sub(preroll)`. Existing mono audio is duplicated to
/// each output channel when the take has more than one channel. Crossfades
/// occupy frames inside the replaced region.
pub fn place_take(
    existing: Option<&AudioBuffer>,
    take: &AudioBuffer,
    start: usize,
    mode: &RecordMode,
    crossfade: usize,
) -> AudioBuffer {
    match mode {
        RecordMode::NewTrack | RecordMode::Overdub => {
            let take_len = take.len();
            let output_len = start.saturating_add(take_len);
            let mut channels = vec![vec![0.0; output_len]; take.n_channels()];

            for (channel_index, output_channel) in channels.iter_mut().enumerate() {
                if let Some(input_channel) = take.channels.get(channel_index) {
                    let copy_len = take_len.min(input_channel.len());
                    let end = start.saturating_add(copy_len).min(output_len);
                    if start < end {
                        output_channel[start..end].copy_from_slice(&input_channel[..end - start]);
                    }
                }
            }

            AudioBuffer {
                sample_rate: take.sample_rate,
                channels,
            }
        }
        RecordMode::PunchAndRoll {
            punch_in,
            punch_out,
            preroll,
        } => {
            let take_origin = punch_in.saturating_sub(*preroll);
            let replacement_end = match punch_out {
                Some(end) => (*end).max(*punch_in),
                None => take_origin.saturating_add(take.len()).max(*punch_in),
            };
            let output_len = existing
                .map(AudioBuffer::len)
                .unwrap_or(0)
                .max(replacement_end);
            let output_channels = take.n_channels();
            let mut channels = vec![vec![0.0; output_len]; output_channels];

            if let Some(existing_buffer) = existing {
                for (channel_index, output_channel) in channels.iter_mut().enumerate() {
                    for (frame, output_sample) in output_channel.iter_mut().enumerate() {
                        *output_sample = existing_sample(existing_buffer, channel_index, frame);
                    }
                }
            }

            let replacement_len = replacement_end.saturating_sub(*punch_in);
            for frame in *punch_in..replacement_end.min(output_len) {
                let relative_frame = frame - *punch_in;
                let take_frame = frame.checked_sub(take_origin);
                let fade_in = if crossfade > 0 && relative_frame < crossfade {
                    linear_fade(relative_frame, crossfade)
                } else {
                    1.0
                };
                let from_end = replacement_end.saturating_sub(frame + 1);
                let fade_out = if crossfade > 0 && from_end < crossfade {
                    linear_fade(from_end, crossfade)
                } else {
                    1.0
                };
                let take_weight = fade_in.min(fade_out);

                for (channel_index, output_channel) in channels.iter_mut().enumerate() {
                    let old_sample = existing
                        .map(|buffer| existing_sample(buffer, channel_index, frame))
                        .unwrap_or(0.0);
                    let new_sample = take_frame
                        .and_then(|index| {
                            take.channels
                                .get(channel_index)
                                .and_then(|channel| channel.get(index))
                        })
                        .copied()
                        .unwrap_or(0.0);
                    output_channel[frame] =
                        old_sample * (1.0 - take_weight) + new_sample * take_weight;
                }
            }

            // Keep this calculation explicit so the exclusive replacement
            // interval remains well-defined even when its length is zero.
            let _ = replacement_len;

            AudioBuffer {
                sample_rate: take.sample_rate,
                channels,
            }
        }
    }
}

fn existing_sample(buffer: &AudioBuffer, output_channel: usize, frame: usize) -> f32 {
    if buffer.channels.is_empty() {
        return 0.0;
    }

    let source_channel = if buffer.channels.len() == 1 {
        0
    } else {
        output_channel.min(buffer.channels.len() - 1)
    };

    buffer.channels[source_channel]
        .get(frame)
        .copied()
        .unwrap_or(0.0)
}

fn linear_fade(position: usize, frames: usize) -> f32 {
    if frames <= 1 {
        1.0
    } else {
        position as f32 / (frames - 1) as f32
    }
}

/// A deterministic recording window expressed in frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimerRecord {
    /// Frame at which recording begins.
    pub start_at: u64,
    /// Number of frames for which recording is active.
    pub duration: u64,
}

/// The state of a timer-driven recording window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerState {
    /// The recording window has not started.
    Waiting,
    /// The recording window is active.
    Recording,
    /// The recording window has ended.
    Done,
}

impl TimerRecord {
    /// Returns the timer state at the given frame.
    ///
    /// The start frame is included and the end frame is excluded.
    pub fn state_at(&self, now: u64) -> TimerState {
        if now < self.start_at {
            TimerState::Waiting
        } else if now < self.start_at.saturating_add(self.duration) {
            TimerState::Recording
        } else {
            TimerState::Done
        }
    }

    /// Returns the sub-range of a block that falls inside the recording window.
    ///
    /// The returned range indexes the block, whose first frame is
    /// `block_start`.
    pub fn frames_to_capture(
        &self,
        block_start: u64,
        block_len: usize,
    ) -> std::ops::Range<usize> {
        let block_len_u64 = u64::try_from(block_len).unwrap_or(u64::MAX);
        let block_end = block_start.saturating_add(block_len_u64);
        let window_end = self.start_at.saturating_add(self.duration);

        if window_end <= block_start {
            return 0..0;
        }
        if self.start_at >= block_end {
            return block_len..block_len;
        }

        let intersection_start = block_start.max(self.start_at);
        let intersection_end = block_end.min(window_end);
        if intersection_end <= intersection_start {
            let offset = usize::try_from(intersection_start.saturating_sub(block_start))
                .unwrap_or(block_len)
                .min(block_len);
            return offset..offset;
        }

        let start = usize::try_from(intersection_start - block_start)
            .unwrap_or(block_len)
            .min(block_len);
        let end = usize::try_from(intersection_end - block_start)
            .unwrap_or(block_len)
            .min(block_len);

        start..end.max(start)
    }
}

/// Configuration for sound-activated recording.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundActivation {
    /// Activation threshold in decibels relative to full scale.
    pub threshold_db: f32,
    /// Number of consecutive below-threshold frames to retain before pausing.
    pub hold_frames: usize,
}

/// The current state of sound-activated recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaState {
    /// Input is not currently being captured.
    Paused,
    /// Input is currently being captured.
    Recording,
}

/// A frame-based sound-activation state machine.
#[derive(Clone, Debug)]
pub struct SoundActivated {
    cfg: SoundActivation,
    state: SaState,
    below_count: usize,
}

impl SoundActivated {
    /// Creates a sound-activated recorder, initially paused.
    pub fn new(cfg: SoundActivation) -> Self {
        Self {
            cfg,
            state: SaState::Paused,
            below_count: 0,
        }
    }

    /// Current gate state.
    pub fn state(&self) -> SaState {
        self.state
    }

    /// Processes interleaved input and appends captured frames to `out`.
    ///
    /// The peak absolute sample across each frame's channels is compared with
    /// the configured threshold. Incomplete trailing samples are ignored.
    /// With zero channels, no frames are processed.
    pub fn feed(&mut self, block: &[f32], channels: usize, out: &mut Vec<f32>) -> SaState {
        if channels == 0 {
            return self.state;
        }

        let threshold = 10.0_f32.powf(self.cfg.threshold_db / 20.0);
        for frame in block.chunks_exact(channels) {
            let peak = frame
                .iter()
                .map(|sample| sample.abs())
                .fold(0.0_f32, f32::max);
            let above_threshold = peak >= threshold;

            match self.state {
                SaState::Paused => {
                    if above_threshold {
                        self.state = SaState::Recording;
                        self.below_count = 0;
                        out.extend_from_slice(frame);
                    }
                }
                SaState::Recording => {
                    out.extend_from_slice(frame);
                    if above_threshold {
                        self.below_count = 0;
                    } else {
                        self.below_count = self.below_count.saturating_add(1);
                        if self.below_count >= self.cfg.hold_frames {
                            self.state = SaState::Paused;
                            self.below_count = 0;
                        }
                    }
                }
            }
        }

        self.state
    }
}

/// A device-independent recording session that collects interleaved samples.
#[derive(Clone, Debug)]
pub struct RecordSession {
    mode: RecordMode,
    channels: usize,
    sample_rate: u32,
    latency: i64,
    captured: Vec<f32>,
}

impl RecordSession {
    /// Creates a recording session.
    pub fn new(
        mode: RecordMode,
        channels: usize,
        sample_rate: u32,
        latency: i64,
    ) -> Self {
        Self {
            mode,
            channels,
            sample_rate,
            latency,
            captured: Vec::new(),
        }
    }

    /// Frames captured so far.
    pub fn captured_frames(&self) -> usize {
        self.captured.len() / self.channels.max(1)
    }

    /// Appends interleaved samples captured by the caller.
    pub fn push_interleaved(&mut self, data: &[f32]) {
        self.captured.extend_from_slice(data);
    }

    /// Deinterleaves, latency-compensates, and places the captured take.
    pub fn finish(
        self,
        existing: Option<&AudioBuffer>,
        start: usize,
        crossfade: usize,
    ) -> AudioBuffer {
        let mut channels = vec![Vec::new(); self.channels];

        if self.channels > 0 {
            let frame_count = self.captured.len() / self.channels;
            for channel in &mut channels {
                channel.reserve(frame_count);
            }
            for frame in self.captured.chunks_exact(self.channels) {
                for (channel_index, sample) in frame.iter().enumerate() {
                    channels[channel_index].push(*sample);
                }
            }
        }

        let recorded = AudioBuffer {
            sample_rate: self.sample_rate,
            channels,
        };
        let compensated = compensate_latency(&recorded, self.latency);
        place_take(existing, &compensated, start, &self.mode, crossfade)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(sample_rate: u32, channels: Vec<Vec<f32>>) -> AudioBuffer {
        AudioBuffer {
            sample_rate,
            channels,
        }
    }

    #[test]
    fn latency_positive_drops() {
        let input = buffer(48_000, vec![vec![1.0, 2.0, 3.0, 4.0]]);
        let output = compensate_latency(&input, 2);
        assert_eq!(output.channels, vec![vec![3.0, 4.0]]);
    }

    #[test]
    fn latency_negative_pads() {
        let input = buffer(48_000, vec![vec![1.0, 2.0]]);
        let output = compensate_latency(&input, -2);
        assert_eq!(output.channels, vec![vec![0.0, 0.0, 1.0, 2.0]]);
    }

    #[test]
    fn new_track_place_offset() {
        let take = buffer(44_100, vec![vec![1.0, 2.0]]);
        let output = place_take(None, &take, 3, &RecordMode::NewTrack, 0);
        assert_eq!(
            output.channels,
            vec![vec![0.0, 0.0, 0.0, 1.0, 2.0]]
        );
    }

    #[test]
    fn punch_boundaries_exact() {
        let existing = buffer(
            48_000,
            vec![(0..100).map(|frame| frame as f32).collect()],
        );
        let take = buffer(48_000, vec![vec![9.0; 100]]);
        let mode = RecordMode::PunchAndRoll {
            punch_in: 30,
            punch_out: Some(60),
            preroll: 10,
        };
        let output = place_take(Some(&existing), &take, 0, &mode, 0);
        for frame in 0..100 {
            if (30..60).contains(&frame) {
                assert_eq!(output.channels[0][frame], 9.0);
            } else {
                assert_eq!(output.channels[0][frame], frame as f32);
            }
        }
    }

    #[test]
    fn punch_crossfade_monotonic() {
        let existing = buffer(48_000, vec![vec![0.0; 12]]);
        let take = buffer(48_000, vec![vec![1.0; 12]]);
        let mode = RecordMode::PunchAndRoll {
            punch_in: 2,
            punch_out: Some(10),
            preroll: 0,
        };
        let output = place_take(Some(&existing), &take, 0, &mode, 3);
        assert!(output.channels[0][2] <= output.channels[0][3]);
        assert!(output.channels[0][3] <= output.channels[0][4]);
        assert!(output.channels[0][4] <= output.channels[0][5]);
        assert!(output.channels[0][7] >= output.channels[0][8]);
        assert!(output.channels[0][8] >= output.channels[0][9]);
    }

    #[test]
    fn punch_open_end_extends() {
        let existing = buffer(48_000, vec![vec![2.0; 5]]);
        let take = buffer(48_000, vec![vec![9.0; 6]]);
        let mode = RecordMode::PunchAndRoll {
            punch_in: 3,
            punch_out: None,
            preroll: 1,
        };
        let output = place_take(Some(&existing), &take, 0, &mode, 0);
        assert_eq!(output.len(), 8);
        assert_eq!(&output.channels[0][3..8], &[9.0, 9.0, 9.0, 9.0, 9.0]);
    }

    #[test]
    fn punch_mono_existing_stereo_take() {
        let existing = buffer(48_000, vec![vec![1.0, 2.0, 3.0, 4.0]]);
        let take = buffer(48_000, vec![vec![9.0; 4], vec![8.0; 4]]);
        let mode = RecordMode::PunchAndRoll {
            punch_in: 1,
            punch_out: Some(3),
            preroll: 0,
        };
        let output = place_take(Some(&existing), &take, 0, &mode, 0);
        assert_eq!(output.n_channels(), 2);
        assert_eq!(output.channels[0], vec![1.0, 9.0, 9.0, 4.0]);
        assert_eq!(output.channels[1], vec![1.0, 8.0, 8.0, 4.0]);
    }

    #[test]
    fn timer_state_transitions() {
        let timer = TimerRecord {
            start_at: 10,
            duration: 5,
        };
        assert_eq!(timer.state_at(9), TimerState::Waiting);
        assert_eq!(timer.state_at(10), TimerState::Recording);
        assert_eq!(timer.state_at(12), TimerState::Recording);
        assert_eq!(timer.state_at(15), TimerState::Done);
    }

    #[test]
    fn timer_block_range() {
        let timer = TimerRecord {
            start_at: 8,
            duration: 4,
        };
        assert_eq!(timer.frames_to_capture(5, 10), 3..7);
    }

    #[test]
    fn sound_activation_starts_on_threshold() {
        let mut activated = SoundActivated::new(SoundActivation {
            threshold_db: 0.0,
            hold_frames: 2,
        });
        let mut output = Vec::new();
        let state = activated.feed(&[0.2, 0.99, 1.0], 1, &mut output);
        assert_eq!(state, SaState::Recording);
        assert_eq!(output, vec![1.0]);
    }

    #[test]
    fn sound_activation_hold_then_pause() {
        let mut activated = SoundActivated::new(SoundActivation {
            threshold_db: -6.0,
            hold_frames: 2,
        });
        let mut output = Vec::new();
        let state = activated.feed(&[1.0, 0.1, 0.1], 1, &mut output);
        assert_eq!(state, SaState::Paused);
        assert_eq!(output, vec![1.0, 0.1, 0.1]);

        let state = activated.feed(&[0.1], 1, &mut output);
        assert_eq!(state, SaState::Paused);
        assert_eq!(output, vec![1.0, 0.1, 0.1]);
    }

    #[test]
    fn sound_activation_retriggers() {
        let mut activated = SoundActivated::new(SoundActivation {
            threshold_db: -6.0,
            hold_frames: 1,
        });
        let mut output = Vec::new();

        assert_eq!(
            activated.feed(&[1.0, 0.1], 1, &mut output),
            SaState::Paused
        );
        assert_eq!(
            activated.feed(&[0.1, 1.0], 1, &mut output),
            SaState::Recording
        );
        assert_eq!(output, vec![1.0, 0.1, 1.0]);
    }

    #[test]
    fn session_finish_compensates_latency() {
        let mut session = RecordSession::new(RecordMode::NewTrack, 1, 48_000, 1);
        session.push_interleaved(&[1.0, 2.0, 3.0]);
        let output = session.finish(None, 0, 0);
        assert_eq!(output.channels, vec![vec![2.0, 3.0]]);
    }
}
