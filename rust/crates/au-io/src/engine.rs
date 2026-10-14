// SPDX-License-Identifier: GPL-3.0-or-later

//! Realtime-safe mixing, lock-free engine control, and simple audio backends.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use au_core::AudioBuffer;

/// Gain, pan, mute, and solo settings for a track.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackParams {
    /// Linear track gain.
    pub gain: f32,
    /// Pan or stereo balance, clamped to the range `-1.0..=1.0` while mixing.
    pub pan: f32,
    /// Whether this track is muted.
    pub mute: bool,
    /// Whether this track is soloed.
    pub solo: bool,
}

impl Default for TrackParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
        }
    }
}

/// An audio buffer placed at a timeline offset with track-specific settings.
pub struct MixTrack {
    /// The track's audio data.
    pub audio: Arc<AudioBuffer>,
    /// Timeline frame at which this track starts.
    pub offset: usize,
    /// Gain, pan, mute, and solo settings.
    pub params: TrackParams,
}

/// Peak and RMS levels for a stereo signal.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterReading {
    /// Peak absolute level for the left and right channels.
    pub peak: [f32; 2],
    /// Root-mean-square level for the left and right channels.
    pub rms: [f32; 2],
}

#[derive(Clone, Copy, Debug, Default)]
struct MeterAccumulator {
    peak: [f32; 2],
    sum_squares: [f64; 2],
}

impl MeterAccumulator {
    fn add(&mut self, left: f32, right: f32) {
        let values = [left, right];
        for (channel, value) in values.into_iter().enumerate() {
            let magnitude = value.abs();
            if magnitude > self.peak[channel] {
                self.peak[channel] = magnitude;
            }
            self.sum_squares[channel] += f64::from(value) * f64::from(value);
        }
    }

    fn reading(self, frames: usize) -> MeterReading {
        if frames == 0 {
            return MeterReading::default();
        }

        let divisor = frames as f64;
        MeterReading {
            peak: self.peak,
            rms: [
                (self.sum_squares[0] / divisor).sqrt() as f32,
                (self.sum_squares[1] / divisor).sqrt() as f32,
            ],
        }
    }
}

/// A preloaded, allocation-free audio mixer.
pub struct Mixer {
    tracks: Vec<MixTrack>,
    master_gain: f32,
    position: f64,
    speed: f64,
    loop_range: Option<(usize, usize)>,
    end: usize,
    track_meters: Vec<MeterReading>,
    track_accumulators: Vec<MeterAccumulator>,
    master_meter: MeterReading,
    clip_count: u64,
    track_fx: Vec<Option<au_effects::stack::EffectStack>>,
    master_fx: Option<au_effects::stack::EffectStack>,
    scratch: Vec<Vec<f32>>,
    mix_scratch: Vec<Vec<f32>>,
    positions: Vec<f64>,
}

impl Mixer {
    /// Creates a mixer from tracks and a master gain.
    pub fn new(tracks: Vec<MixTrack>, master_gain: f32) -> Self {
        let end = tracks.iter().fold(0usize, |latest, track| {
            latest.max(track.offset.saturating_add(track.audio.len()))
        });
        let track_count = tracks.len();

        Self {
            tracks,
            master_gain,
            position: 0.0,
            speed: 1.0,
            loop_range: None,
            end,
            track_meters: vec![MeterReading::default(); track_count],
            track_accumulators: vec![MeterAccumulator::default(); track_count],
            master_meter: MeterReading::default(),
            clip_count: 0,
            track_fx: (0..track_count).map(|_| None).collect(),
            master_fx: None,
            scratch: Vec::new(),
            mix_scratch: Vec::new(),
            positions: Vec::new(),
        }
    }

    /// Installs (or clears) a track's realtime effect stack. The stack is reset
    /// for the track's sample rate and channel count.
    pub fn set_track_effects(&mut self, index: usize, stack: Option<au_effects::stack::EffectStack>) {
        if let (Some(slot), Some(track)) = (self.track_fx.get_mut(index), self.tracks.get(index)) {
            *slot = stack.map(|mut s| { s.reset(track.audio.sample_rate, track.audio.n_channels().max(1)); s });
        }
    }

    /// Installs (or clears) the master realtime effect stack (processes the stereo/mono mix).
    pub fn set_master_effects(&mut self, stack: Option<au_effects::stack::EffectStack>) {
        let rate = self.tracks.first().map(|t| t.audio.sample_rate).unwrap_or(44_100);
        self.master_fx = stack.map(|mut s| { s.reset(rate, 2); s });
    }

    fn has_effects(&self) -> bool {
        self.master_fx.is_some() || self.track_fx.iter().any(|s| s.is_some())
    }

    /// Block path used when realtime effects are installed: each track is read
    /// into a scratch block, run through its stack (pre-fader, like Audacity),
    /// then gain/pan-mixed; the master stack processes the mix.
    fn process_with_effects(&mut self, out: &mut [f32], out_channels: usize) -> bool {
        let frames = out.len() / out_channels;
        self.positions.clear();
        for _ in 0..frames {
            self.normalize_loop_position();
            if !self.is_playable() { break; }
            self.positions.push(self.position);
            self.position += self.speed;
        }
        let n = self.positions.len();
        if self.mix_scratch.len() != 2 { self.mix_scratch = vec![Vec::new(), Vec::new()]; }
        for m in &mut self.mix_scratch { m.clear(); m.resize(n, 0.0); }
        let any_solo = self.tracks.iter().any(|track| track.params.solo);
        for index in 0..self.tracks.len() {
            let track = &self.tracks[index];
            let params = track.params;
            let audible = !params.mute && (!any_solo || params.solo);
            let ch = track.audio.n_channels();
            let mut acc = MeterAccumulator::default();
            if audible && ch > 0 {
                if self.scratch.len() < ch { self.scratch.resize(ch, Vec::new()); }
                for c in 0..ch {
                    let buf = &mut self.scratch[c];
                    buf.clear();
                    buf.extend(self.positions.iter().map(|&p| interpolated_sample(track, c, p - track.offset as f64)));
                }
                if let Some(Some(stack)) = self.track_fx.get_mut(index) {
                    stack.process_block(&mut self.scratch[..ch]);
                }
                let track = &self.tracks[index];
                for f in 0..n {
                    let (l, r) = if ch == 1 {
                        let sample = self.scratch[0][f] * params.gain;
                        let angle = (params.pan.clamp(-1.0, 1.0) + 1.0) * (std::f32::consts::PI * 0.25);
                        (sample * angle.cos() * std::f32::consts::SQRT_2, sample * angle.sin() * std::f32::consts::SQRT_2)
                    } else {
                        let pan = params.pan.clamp(-1.0, 1.0);
                        let (mut l, mut r) = (self.scratch[0][f] * params.gain, self.scratch[1][f] * params.gain);
                        if pan < 0.0 { r *= 1.0 + pan } else if pan > 0.0 { l *= 1.0 - pan }
                        (l, r)
                    };
                    let _ = track;
                    acc.add(l, r);
                    self.mix_scratch[0][f] += l;
                    self.mix_scratch[1][f] += r;
                }
            }
            self.track_meters[index] = acc.reading(frames);
        }
        for m in &mut self.mix_scratch { for v in m.iter_mut() { *v *= self.master_gain; } }
        if let Some(stack) = self.master_fx.as_mut() { stack.process_block(&mut self.mix_scratch); }
        let mut master = MeterAccumulator::default();
        for f in 0..n {
            let (rl, rr) = (self.mix_scratch[0][f], self.mix_scratch[1][f]);
            if out_channels == 1 {
                let (mono, clipped) = clamp_output_sample((rl + rr) * 0.5);
                self.clip_count = self.clip_count.saturating_add(u64::from(clipped));
                out[f] = mono; master.add(mono, mono);
            } else {
                let (l, cl) = clamp_output_sample(rl);
                let (r, cr) = clamp_output_sample(rr);
                self.clip_count = self.clip_count.saturating_add(u64::from(cl)).saturating_add(u64::from(cr));
                out[f * out_channels] = l; out[f * out_channels + 1] = r;
                master.add(l, r);
            }
        }
        self.master_meter = master.reading(frames);
        self.is_playable()
    }

    /// Sets the timeline position in frames.
    pub fn set_position(&mut self, frame: usize) {
        self.position = frame as f64;
    }

    /// Returns the current timeline position in frames.
    pub fn position(&self) -> f64 {
        self.position
    }

    /// Sets playback speed. Negative speeds play in reverse.
    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed;
    }

    /// Sets the loop range, whose end frame is exclusive.
    ///
    /// An empty or reversed range disables looping.
    pub fn set_loop(&mut self, loop_range: Option<(usize, usize)>) {
        self.loop_range = loop_range.filter(|(start, end)| start < end);
    }

    /// Replaces the settings for the track at `index`, if it exists.
    pub fn set_track_params(&mut self, index: usize, params: TrackParams) {
        if let Some(track) = self.tracks.get_mut(index) {
            track.params = params;
        }
    }

    /// Sets the master output gain.
    pub fn set_master_gain(&mut self, gain: f32) {
        self.master_gain = gain;
    }

    /// Returns the number of output samples clipped since this mixer was created.
    pub fn clip_count(&self) -> u64 {
        self.clip_count
    }

    /// Returns the master meter reading from the most recent processing block.
    pub fn master_meter(&self) -> MeterReading {
        self.master_meter
    }

    /// Returns a track's meter reading from the most recent processing block.
    ///
    /// An invalid track index returns a zero reading.
    pub fn track_meter(&self, index: usize) -> MeterReading {
        self.track_meters
            .get(index)
            .copied()
            .unwrap_or_default()
    }

    /// Mixes into interleaved output without allocating.
    ///
    /// Returns `false` when playback has reached its end. Any unfilled output
    /// samples are set to zero. One output channel receives the average of the
    /// stereo mix; with two or more channels, the first two receive left and
    /// right and additional channels remain silent.
    pub fn process(&mut self, out: &mut [f32], out_channels: usize) -> bool {
        out.fill(0.0);

        for accumulator in &mut self.track_accumulators {
            *accumulator = MeterAccumulator::default();
        }
        self.track_meters.fill(MeterReading::default());
        self.master_meter = MeterReading::default();

        if out_channels == 0 {
            return self.is_playable();
        }

        if self.has_effects() {
            return self.process_with_effects(out, out_channels);
        }
        let frames = out.len() / out_channels;
        let mut master_accumulator = MeterAccumulator::default();
        let any_solo = self.tracks.iter().any(|track| track.params.solo);

        for frame in 0..frames {
            self.normalize_loop_position();

            if !self.is_playable() {
                break;
            }

            let mut mix_left = 0.0f32;
            let mut mix_right = 0.0f32;

            for (index, track) in self.tracks.iter().enumerate() {
                let params = track.params;
                let audible = !params.mute && (!any_solo || params.solo);

                let (left, right) = if audible {
                    route_track_sample(track, self.position, params)
                } else {
                    (0.0, 0.0)
                };

                self.track_accumulators[index].add(left, right);
                mix_left += left;
                mix_right += right;
            }

            let raw_left = mix_left * self.master_gain;
            let raw_right = mix_right * self.master_gain;

            if out_channels == 1 {
                let raw_mono = (raw_left + raw_right) * 0.5;
                let (mono, clipped) = clamp_output_sample(raw_mono);
                if clipped {
                    self.clip_count = self.clip_count.saturating_add(1);
                }
                out[frame] = mono;
                master_accumulator.add(mono, mono);
            } else {
                let (left, clipped_left) = clamp_output_sample(raw_left);
                let (right, clipped_right) = clamp_output_sample(raw_right);
                self.clip_count = self
                    .clip_count
                    .saturating_add(u64::from(clipped_left))
                    .saturating_add(u64::from(clipped_right));

                let base = frame * out_channels;
                out[base] = left;
                out[base + 1] = right;
                master_accumulator.add(left, right);
            }

            self.position += self.speed;
        }

        for (meter, accumulator) in self
            .track_meters
            .iter_mut()
            .zip(self.track_accumulators.iter().copied())
        {
            *meter = accumulator.reading(frames);
        }
        self.master_meter = master_accumulator.reading(frames);

        self.is_playable()
    }

    fn valid_loop(&self) -> Option<(usize, usize)> {
        self.loop_range
            .filter(|(start, end)| start < end)
    }

    fn normalize_loop_position(&mut self) {
        let Some((start, end)) = self.valid_loop() else {
            return;
        };

        let start_f = start as f64;
        let end_f = end as f64;
        let length = end_f - start_f;

        if self.speed >= 0.0 && self.position >= end_f {
            self.position = start_f + (self.position - start_f).rem_euclid(length);
        } else if self.speed < 0.0 && self.position < start_f {
            self.position = start_f + (self.position - start_f).rem_euclid(length);
        }
    }

    fn is_playable(&self) -> bool {
        if self.valid_loop().is_some() {
            return true;
        }
        if self.end == 0 {
            return false;
        }

        if self.speed < 0.0 {
            self.position >= 0.0
        } else {
            self.position < self.end as f64
        }
    }

    fn clear_meters(&mut self) {
        self.master_meter = MeterReading::default();
        self.track_meters.fill(MeterReading::default());
        for accumulator in &mut self.track_accumulators {
            *accumulator = MeterAccumulator::default();
        }
    }
}

fn route_track_sample(track: &MixTrack, position: f64, params: TrackParams) -> (f32, f32) {
    let channels = track.audio.n_channels();
    if channels == 0 {
        return (0.0, 0.0);
    }

    let local_position = position - track.offset as f64;
    let gain = params.gain;

    if channels == 1 {
        let sample = interpolated_sample(track, 0, local_position) * gain;
        let pan = params.pan.clamp(-1.0, 1.0);
        let angle = (pan + 1.0) * (std::f32::consts::PI * 0.25);
        let normalization = std::f32::consts::SQRT_2;
        (
            sample * angle.cos() * normalization,
            sample * angle.sin() * normalization,
        )
    } else {
        let mut left = interpolated_sample(track, 0, local_position) * gain;
        let mut right = interpolated_sample(track, 1, local_position) * gain;
        let pan = params.pan.clamp(-1.0, 1.0);

        if pan < 0.0 {
            right *= 1.0 + pan;
        } else if pan > 0.0 {
            left *= 1.0 - pan;
        }

        (left, right)
    }
}

fn interpolated_sample(track: &MixTrack, channel: usize, local_position: f64) -> f32 {
    let length = track.audio.len();
    if length == 0 || local_position <= -1.0 || local_position >= length as f64 {
        return 0.0;
    }

    let base_index = local_position.floor() as i64;
    let fraction = (local_position - base_index as f64) as f32;
    let first = sample_at_index(track, channel, base_index);
    let second = sample_at_index(track, channel, base_index + 1);

    first + (second - first) * fraction
}

fn sample_at_index(track: &MixTrack, channel: usize, index: i64) -> f32 {
    if index < 0 {
        return 0.0;
    }

    track
        .audio
        .channels
        .get(channel)
        .and_then(|samples| samples.get(index as usize))
        .copied()
        .unwrap_or(0.0)
}

fn clamp_output_sample(sample: f32) -> (f32, bool) {
    if sample.is_nan() {
        (0.0, false)
    } else if sample > 1.0 {
        (1.0, true)
    } else if sample < -1.0 {
        (-1.0, true)
    } else {
        (sample, false)
    }
}

/// Renders all tracks to an in-memory audio buffer.
pub fn render_offline(
    tracks: Vec<MixTrack>,
    master_gain: f32,
    out_channels: usize,
    block: usize,
) -> AudioBuffer {
    let sample_rate = tracks
        .first()
        .map(|track| track.audio.sample_rate)
        .unwrap_or(44_100);
    let mut mixer = Mixer::new(tracks, master_gain);
    let total_frames = mixer.end;
    let block_size = block.max(1);

    let mut channels: Vec<Vec<f32>> = (0..out_channels)
        .map(|_| Vec::with_capacity(total_frames))
        .collect();
    let mut interleaved = vec![0.0; block_size.saturating_mul(out_channels)];
    let mut rendered = 0usize;

    while rendered < total_frames {
        let frames = (total_frames - rendered).min(block_size);
        let sample_count = frames.saturating_mul(out_channels);
        mixer.process(&mut interleaved[..sample_count], out_channels);

        for frame in 0..frames {
            for channel in 0..out_channels {
                channels[channel].push(interleaved[frame * out_channels + channel]);
            }
        }
        rendered += frames;
    }

    AudioBuffer {
        sample_rate,
        channels,
    }
}

/// Offline render (export / mixdown) with realtime effect stacks applied
/// non-destructively: each track is rendered through its stack with latency
/// compensation, then mixed, then the master stack processes the mix.
pub fn render_offline_with_effects(
    tracks: Vec<MixTrack>,
    track_fx: Vec<Option<au_effects::stack::EffectStack>>,
    master_fx: Option<au_effects::stack::EffectStack>,
    master_gain: f32,
    out_channels: usize,
    block: usize,
) -> AudioBuffer {
    let tracks: Vec<MixTrack> = tracks.into_iter().zip(track_fx.into_iter().chain(std::iter::repeat_with(|| None)))
        .map(|(mut t, fx)| {
            if let Some(mut stack) = fx {
                let mut audio = (*t.audio).clone();
                stack.render(&mut audio);
                t.audio = Arc::new(audio);
            }
            t
        }).collect();
    let mut out = render_offline(tracks, master_gain, out_channels, block);
    if let Some(mut stack) = master_fx { stack.render(&mut out); }
    out
}

/// Atomically shared master meters and playback position.
pub struct SharedMeters {
    master_peak_left: AtomicU32,
    master_peak_right: AtomicU32,
    master_rms_left: AtomicU32,
    master_rms_right: AtomicU32,
    position: AtomicU64,
}

impl SharedMeters {
    /// Creates meters initialized to zero.
    pub fn new() -> Self {
        Self {
            master_peak_left: AtomicU32::new(0.0f32.to_bits()),
            master_peak_right: AtomicU32::new(0.0f32.to_bits()),
            master_rms_left: AtomicU32::new(0.0f32.to_bits()),
            master_rms_right: AtomicU32::new(0.0f32.to_bits()),
            position: AtomicU64::new(0.0f64.to_bits()),
        }
    }

    /// Stores a master meter reading.
    pub fn store(&self, reading: &MeterReading) {
        self.master_peak_left
            .store(reading.peak[0].to_bits(), Ordering::Relaxed);
        self.master_peak_right
            .store(reading.peak[1].to_bits(), Ordering::Relaxed);
        self.master_rms_left
            .store(reading.rms[0].to_bits(), Ordering::Relaxed);
        self.master_rms_right
            .store(reading.rms[1].to_bits(), Ordering::Relaxed);
    }

    /// Loads the most recently stored master meter reading.
    pub fn load(&self) -> MeterReading {
        MeterReading {
            peak: [
                f32::from_bits(self.master_peak_left.load(Ordering::Relaxed)),
                f32::from_bits(self.master_peak_right.load(Ordering::Relaxed)),
            ],
            rms: [
                f32::from_bits(self.master_rms_left.load(Ordering::Relaxed)),
                f32::from_bits(self.master_rms_right.load(Ordering::Relaxed)),
            ],
        }
    }

    /// Returns the most recently stored playback position in frames.
    pub fn position(&self) -> f64 {
        f64::from_bits(self.position.load(Ordering::Relaxed))
    }

    fn store_position(&self, position: f64) {
        self.position
            .store(position.to_bits(), Ordering::Relaxed);
    }
}

impl Default for SharedMeters {
    fn default() -> Self {
        Self::new()
    }
}

/// Commands sent from the control thread to the audio callback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EngineCommand {
    /// Start playback.
    Play,
    /// Pause playback without changing the position.
    Pause,
    /// Stop playback and return to frame zero.
    Stop,
    /// Seek to a timeline frame.
    Seek(usize),
    /// Set playback speed.
    SetSpeed(f64),
    /// Set the loop range, whose end frame is exclusive.
    SetLoop(Option<(usize, usize)>),
    /// Set a track's parameters.
    SetTrack(usize, TrackParams),
    /// Set master gain.
    SetMasterGain(f32),
}

/// Control-thread handle for a lock-free audio engine.
pub struct EngineHandle {
    tx: rtrb::Producer<EngineCommand>,
    /// Shared playback meters and position.
    pub meters: Arc<SharedMeters>,
    playing: Arc<AtomicBool>,
}

impl EngineHandle {
    /// Attempts to enqueue a command, returning `false` if the queue is full.
    pub fn send(&mut self, command: EngineCommand) -> bool {
        self.tx.push(command).is_ok()
    }

    /// Returns whether playback is currently active.
    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    /// Returns the latest playback position in frames.
    pub fn position(&self) -> f64 {
        self.meters.position()
    }
}

/// Audio-thread side of the lock-free engine.
pub struct AudioCallback {
    mixer: Mixer,
    rx: rtrb::Consumer<EngineCommand>,
    meters: Arc<SharedMeters>,
    playing: Arc<AtomicBool>,
}

impl AudioCallback {
    /// Drains pending commands and renders one interleaved output block.
    pub fn render(&mut self, out: &mut [f32], channels: usize) {
        while let Ok(command) = self.rx.pop() {
            match command {
                EngineCommand::Play => {
                    self.playing.store(true, Ordering::Relaxed);
                }
                EngineCommand::Pause => {
                    self.playing.store(false, Ordering::Relaxed);
                }
                EngineCommand::Stop => {
                    self.mixer.set_position(0);
                    self.playing.store(false, Ordering::Relaxed);
                }
                EngineCommand::Seek(frame) => self.mixer.set_position(frame),
                EngineCommand::SetSpeed(speed) => self.mixer.set_speed(speed),
                EngineCommand::SetLoop(loop_range) => self.mixer.set_loop(loop_range),
                EngineCommand::SetTrack(index, params) => {
                    self.mixer.set_track_params(index, params);
                }
                EngineCommand::SetMasterGain(gain) => self.mixer.set_master_gain(gain),
            }
        }

        if self.playing.load(Ordering::Relaxed) {
            if !self.mixer.process(out, channels) {
                self.playing.store(false, Ordering::Relaxed);
            }
        } else {
            out.fill(0.0);
            self.mixer.clear_meters();
        }

        self.meters.store(&self.mixer.master_meter());
        self.meters.store_position(self.mixer.position());
    }
}

/// Creates a lock-free engine handle and its audio callback.
pub fn engine(mixer: Mixer) -> (EngineHandle, AudioCallback) {
    let (tx, rx) = rtrb::RingBuffer::<EngineCommand>::new(256);
    let meters = Arc::new(SharedMeters::new());
    let playing = Arc::new(AtomicBool::new(false));

    (
        EngineHandle {
            tx,
            meters: Arc::clone(&meters),
            playing: Arc::clone(&playing),
        },
        AudioCallback {
            mixer,
            rx,
            meters,
            playing,
        },
    )
}

/// Output backend interface for an audio callback.
pub trait OutputBackend {
    /// Returns the backend sample rate.
    fn sample_rate(&self) -> u32;

    /// Returns the backend output channel count.
    fn channels(&self) -> usize;

    /// Runs the callback on the backend.
    fn run(&mut self, callback: AudioCallback) -> Result<(), String>;
}

/// A headless output backend useful for tests and offline pumping.
pub struct NullOutput {
    /// Output sample rate.
    pub sample_rate: u32,
    /// Number of output channels.
    pub channels: usize,
    /// Number of frames rendered by each pump block.
    pub block: usize,
    /// Interleaved samples captured by [`NullOutput::pump`].
    pub captured: Vec<f32>,
}

impl NullOutput {
    /// Renders `blocks` blocks and appends their samples to `captured`.
    pub fn pump(&mut self, callback: &mut AudioCallback, blocks: usize) {
        let samples_per_block = self.block.saturating_mul(self.channels);
        for _ in 0..blocks {
            let old_length = self.captured.len();
            let new_length = old_length.saturating_add(samples_per_block);
            self.captured.resize(new_length, 0.0);
            callback.render(&mut self.captured[old_length..new_length], self.channels);
        }
    }
}

impl OutputBackend for NullOutput {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channels(&self) -> usize {
        self.channels
    }

    fn run(&mut self, _callback: AudioCallback) -> Result<(), String> {
        Ok(())
    }
}

/// Audio input source interface.
pub trait InputSource {
    /// Reads interleaved samples into `out` and returns the number of data-backed frames.
    fn read(&mut self, out: &mut [f32], channels: usize) -> usize;
}

/// A headless input source that yields stored samples and then silence.
pub struct NullInput {
    /// Interleaved samples to provide.
    pub data: Vec<f32>,
    /// Current sample position within `data`.
    pub pos: usize,
}

impl InputSource for NullInput {
    fn read(&mut self, out: &mut [f32], channels: usize) -> usize {
        out.fill(0.0);
        if channels == 0 || self.pos >= self.data.len() {
            return 0;
        }

        let copied = out.len().min(self.data.len() - self.pos);
        out[..copied].copy_from_slice(&self.data[self.pos..self.pos + copied]);
        self.pos += copied;
        copied / channels
    }
}

#[cfg(test)]
mod tests {
    fn assert_close(a: &[f32], b: &[f32], scale: f32) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x - y * scale).abs() < 1e-5, "{a:?} vs {b:?}*{scale}");
        }
    }

    use super::*;

    fn audio(channels: Vec<Vec<f32>>) -> Arc<AudioBuffer> {
        Arc::new(AudioBuffer {
            sample_rate: 44_100,
            channels,
        })
    }

    fn mono_track(samples: Vec<f32>, offset: usize, params: TrackParams) -> MixTrack {
        MixTrack {
            audio: audio(vec![samples]),
            offset,
            params,
        }
    }

    fn close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "actual {actual}, expected {expected}, tolerance {tolerance}"
        );
    }

    #[test]
    fn mixer_gain_and_master() {
        let mut params = TrackParams::default();
        params.gain = 0.5;
        let track = mono_track(vec![0.8], 0, params);
        let mut mixer = Mixer::new(vec![track], 0.5);
        let mut out = [0.0; 2];
        mixer.process(&mut out, 2);
        close(out[0], 0.2, 1e-6);
        close(out[1], 0.2, 1e-6);
    }

    #[test]
    fn mixer_pan_constant_power_center_unity() {
        let track = mono_track(vec![0.4], 0, TrackParams::default());
        let mut mixer = Mixer::new(vec![track], 1.0);
        let mut out = [0.0; 2];
        mixer.process(&mut out, 2);
        close(out[0], 0.4, 1e-6);
        close(out[1], 0.4, 1e-6);
    }

    #[test]
    fn mixer_stereo_balance() {
        let track = MixTrack {
            audio: audio(vec![vec![1.0], vec![1.0]]),
            offset: 0,
            params: TrackParams {
                pan: -0.5,
                ..TrackParams::default()
            },
        };
        let mut mixer = Mixer::new(vec![track], 1.0);
        let mut out = [0.0; 2];
        mixer.process(&mut out, 2);
        close(out[0], 1.0, 1e-6);
        close(out[1], 0.5, 1e-6);
    }

    #[test]
    fn mixer_mute() {
        let track = mono_track(
            vec![1.0],
            0,
            TrackParams {
                mute: true,
                ..TrackParams::default()
            },
        );
        let mut mixer = Mixer::new(vec![track], 1.0);
        let mut out = [1.0; 2];
        mixer.process(&mut out, 2);
        assert_eq!(out, [0.0, 0.0]);
    }

    #[test]
    fn mixer_solo_overrides() {
        let first = mono_track(
            vec![0.25],
            0,
            TrackParams {
                solo: true,
                ..TrackParams::default()
            },
        );
        let second = mono_track(vec![0.5], 0, TrackParams::default());
        let mut mixer = Mixer::new(vec![first, second], 1.0);
        let mut out = [0.0; 2];
        mixer.process(&mut out, 2);
        close(out[0], 0.25, 1e-6);
        close(out[1], 0.25, 1e-6);
    }

    #[test]
    fn mixer_clipping_clamped_and_counted() {
        let track = mono_track(vec![2.0], 0, TrackParams::default());
        let mut mixer = Mixer::new(vec![track], 1.0);
        let mut out = [0.0];
        mixer.process(&mut out, 1);
        assert_eq!(out, [1.0]);
        assert_eq!(mixer.clip_count(), 1);
    }

    #[test]
    fn mixer_offset_track_starts_late() {
        let track = mono_track(vec![1.0, 2.0], 2, TrackParams::default());
        let mut mixer = Mixer::new(vec![track], 0.25);
        let mut out = [0.0; 4];
        mixer.process(&mut out, 1);
        assert_close(&out, &[0.0, 0.0, 1.0, 2.0], 0.25);
    }

    #[test]
    fn mixer_loop_wraps_sample_accurate() {
        let track = mono_track(vec![0.0, 1.0, 2.0, 3.0, 4.0], 0, TrackParams::default());
        let mut mixer = Mixer::new(vec![track], 0.25);
        mixer.set_loop(Some((1, 4)));
        mixer.set_position(1);
        let mut out = [0.0; 5];
        mixer.process(&mut out, 1);
        assert_close(&out, &[1.0, 2.0, 3.0, 1.0, 2.0], 0.25);
    }

    #[test]
    fn mixer_speed_half_interpolates() {
        let track = mono_track(vec![0.0, 1.0, 2.0], 0, TrackParams::default());
        let mut mixer = Mixer::new(vec![track], 0.25);
        mixer.set_speed(0.5);
        let mut out = [0.0; 5];
        mixer.process(&mut out, 1);
        assert_close(&out, &[0.0, 0.5, 1.0, 1.5, 2.0], 0.25);
    }

    #[test]
    fn mixer_reverse_speed() {
        let track = mono_track(vec![0.0, 1.0, 2.0, 3.0], 0, TrackParams::default());
        let mut mixer = Mixer::new(vec![track], 0.25);
        mixer.set_position(2);
        mixer.set_speed(-1.0);
        let mut out = [0.0; 4];
        assert!(!mixer.process(&mut out, 1));
        assert_close(&out, &[2.0, 1.0, 0.0, 0.0], 0.25);
    }

    #[test]
    fn mixer_meters_peak_rms() {
        let track = mono_track(
            vec![0.0, 0.5, 0.0, -0.5],
            0,
            TrackParams::default(),
        );
        let mut mixer = Mixer::new(vec![track], 1.0);
        let mut out = [0.0; 8];
        mixer.process(&mut out, 2);
        let meter = mixer.master_meter();
        close(meter.peak[0], 0.5, 1e-2);
        close(meter.rms[0], 0.3536, 1e-2);
    }

    #[test]
    fn mixer_offline_matches_reference() {
        let first = mono_track(vec![0.1, 0.2, 0.3], 0, TrackParams::default());
        let second = mono_track(vec![0.4, 0.5], 1, TrackParams::default());
        let rendered = render_offline(vec![first, second], 1.0, 1, 2);
        assert_eq!(rendered.channels.len(), 1);
        assert_eq!(rendered.channels[0].len(), 3);
        close(rendered.channels[0][0], 0.1, 1e-6);
        close(rendered.channels[0][1], 0.6, 1e-6);
        close(rendered.channels[0][2], 0.8, 1e-6);
    }

    #[test]
    fn engine_commands_play_seek_stop_via_null_output() {
        let track = mono_track(vec![0.25, 0.5, 0.75, 1.0], 0, TrackParams::default());
        let mixer = Mixer::new(vec![track], 1.0);
        let (mut handle, mut callback) = engine(mixer);
        let mut output = NullOutput {
            sample_rate: 44_100,
            channels: 1,
            block: 2,
            captured: Vec::new(),
        };

        assert!(handle.send(EngineCommand::Play));
        output.pump(&mut callback, 1);
        assert!(output.captured.iter().zip([0.25f32, 0.5]).all(|(a, b)| (a - b).abs() < 1e-6));
        assert_eq!(handle.position(), 2.0);
        assert!(handle.is_playing());

        assert!(handle.send(EngineCommand::Seek(1)));
        assert!(handle.send(EngineCommand::Play));
        output.pump(&mut callback, 1);
        assert!(output.captured[2..].iter().zip([0.5f32, 0.75]).all(|(a, b)| (a - b).abs() < 1e-6));

        assert!(handle.send(EngineCommand::Stop));
        output.pump(&mut callback, 1);
        assert_eq!(handle.position(), 0.0);
        assert!(!handle.is_playing());
        assert_eq!(&output.captured[4..], &[0.0, 0.0]);
    }

    #[test]
    fn engine_silence_when_paused() {
        let track = mono_track(vec![1.0, 1.0], 0, TrackParams::default());
        let (mut _handle, mut callback) = engine(Mixer::new(vec![track], 1.0));
        let mut out = [1.0; 4];
        callback.render(&mut out, 2);
        assert_eq!(out, [0.0; 4]);
    }

    #[test]
    fn engine_meters_shared() {
        let track = mono_track(vec![0.5, 0.5], 0, TrackParams::default());
        let (mut handle, mut callback) = engine(Mixer::new(vec![track], 1.0));
        assert!(handle.send(EngineCommand::Play));
        let mut out = [0.0; 4];
        callback.render(&mut out, 2);
        let meter = handle.meters.load();
        close(meter.peak[0], 0.5, 1e-6);
        close(meter.rms[1], 0.5, 1e-6);
        assert_eq!(handle.position(), 2.0);
    }

    #[test]
    fn null_input_yields_data_then_zero() {
        let mut input = NullInput {
            data: vec![1.0, 2.0, 3.0],
            pos: 0,
        };
        let mut first = [0.0; 2];
        assert_eq!(input.read(&mut first, 1), 2);
        assert_eq!(first, [1.0, 2.0]);

        let mut second = [9.0; 3];
        assert_eq!(input.read(&mut second, 1), 1);
        assert_eq!(second, [3.0, 0.0, 0.0]);
    }
}
