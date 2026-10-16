// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Audacity (GPL-3.0-or-later).

use crate::{AudioBuffer, CoreError, Result, Sample};

/// A named audio clip positioned on a timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// The clip's name.
    pub name: String,
    /// Timeline frame of the first sample.
    pub start: usize,
    /// The clip's source audio.
    pub buffer: AudioBuffer,
    /// Time-stretch ratio; `1.0` means no stretching.
    pub stretch_ratio: f64,
    /// Hidden source audio before the visible buffer (`None` if nothing hidden).
    pub trim_left: Option<AudioBuffer>,
    /// Hidden source audio after the visible buffer (`None` if nothing hidden).
    pub trim_right: Option<AudioBuffer>,
}

impl Clip {
    /// Creates a clip at `start` with no time stretching.
    pub fn new(name: impl Into<String>, start: usize, buffer: AudioBuffer) -> Self {
        Self {
            name: name.into(),
            start,
            buffer,
            stretch_ratio: 1.0,
            trim_left: None,
            trim_right: None,
        }
    }

    /// Returns the number of hidden source samples before the visible buffer.
    pub fn hidden_left_len(&self) -> usize {
        self.trim_left.as_ref().map_or(0, AudioBuffer::len)
    }

    /// Returns the number of hidden source samples after the visible buffer.
    pub fn hidden_right_len(&self) -> usize {
        self.trim_right.as_ref().map_or(0, AudioBuffer::len)
    }

    /// Moves the clip's left edge to timeline `frame`, hiding or revealing audio.
    pub fn trim_left_to(&mut self, frame: usize) -> Result<()> {
        if frame == self.start {
            return Ok(());
        }
        if frame > self.start {
            let tf = frame - self.start;
            let src_count = if (self.stretch_ratio - 1.0).abs() <= f64::EPSILON {
                tf
            } else {
                (tf as f64 / self.stretch_ratio).round().max(0.0) as usize
            };
            let src_count = src_count.min(self.buffer.len().saturating_sub(1));
            if src_count > 0 {
                let (hide, keep) = split_buffer(&self.buffer, src_count);
                self.buffer = keep;
                self.trim_left = Some(match self.trim_left.take() {
                    Some(existing) => concat_buffers(&existing, &hide),
                    None => hide,
                });
                let timeline_shift = if (self.stretch_ratio - 1.0).abs() <= f64::EPSILON {
                    src_count
                } else {
                    rendered_len(src_count, self.stretch_ratio)
                };
                self.start = self.start.saturating_add(timeline_shift);
            }
        } else {
            let tf = self.start - frame;
            let src_count = if (self.stretch_ratio - 1.0).abs() <= f64::EPSILON {
                tf
            } else {
                (tf as f64 / self.stretch_ratio).round().max(0.0) as usize
            };
            let src_count = src_count.min(self.hidden_left_len());
            if src_count > 0 {
                let left_buf = self.trim_left.take().unwrap();
                let (remaining, reveal) = split_buffer(&left_buf, left_buf.len() - src_count);
                self.buffer = concat_buffers(&reveal, &self.buffer);
                self.trim_left = if remaining.is_empty() { None } else { Some(remaining) };
                let timeline_shift = if (self.stretch_ratio - 1.0).abs() <= f64::EPSILON {
                    src_count
                } else {
                    rendered_len(src_count, self.stretch_ratio)
                };
                self.start = self.start.saturating_sub(timeline_shift);
            }
        }
        Ok(())
    }

    /// Moves the clip's right edge to timeline `frame`, hiding or revealing audio.
    pub fn trim_right_to(&mut self, frame: usize) -> Result<()> {
        let current_end = self.end();
        if frame == current_end {
            return Ok(());
        }
        if frame < current_end {
            let tf = current_end - frame;
            let src_count = if (self.stretch_ratio - 1.0).abs() <= f64::EPSILON {
                tf
            } else {
                (tf as f64 / self.stretch_ratio).round().max(0.0) as usize
            };
            let src_count = src_count.min(self.buffer.len().saturating_sub(1));
            if src_count > 0 {
                let (keep, hide) = split_buffer(&self.buffer, self.buffer.len() - src_count);
                self.buffer = keep;
                self.trim_right = Some(match self.trim_right.take() {
                    Some(existing) => concat_buffers(&hide, &existing),
                    None => hide,
                });
            }
        } else {
            let tf = frame - current_end;
            let src_count = if (self.stretch_ratio - 1.0).abs() <= f64::EPSILON {
                tf
            } else {
                (tf as f64 / self.stretch_ratio).round().max(0.0) as usize
            };
            let src_count = src_count.min(self.hidden_right_len());
            if src_count > 0 {
                let right_buf = self.trim_right.take().unwrap();
                let (reveal, remaining) = split_buffer(&right_buf, src_count);
                self.buffer = concat_buffers(&self.buffer, &reveal);
                self.trim_right = if remaining.is_empty() { None } else { Some(remaining) };
            }
        }
        Ok(())
    }

    /// Returns the clip's rendered length in frames.
    pub fn len(&self) -> usize {
        rendered_len(self.buffer.len(), self.stretch_ratio)
    }

    /// Returns the exclusive ending frame of the clip.
    pub fn end(&self) -> usize {
        self.start.saturating_add(self.len())
    }

    /// Returns whether the clip contains `frame`.
    pub fn contains(&self, frame: usize) -> bool {
        self.start <= frame && frame < self.end()
    }

    /// Returns the clip's audio after applying its stretch ratio.
    pub fn rendered(&self) -> AudioBuffer {
        if self.stretch_ratio == 1.0 {
            self.buffer.clone()
        } else {
            time_stretch(&self.buffer, self.stretch_ratio)
        }
    }

    /// Splits the clip at an interior timeline frame.
    ///
    /// Stretching is baked into both visible and hidden parts of the returned clips.
    pub fn split_at(&self, frame: usize) -> Result<(Clip, Clip)> {
        if frame <= self.start || frame >= self.end() {
            return Err(CoreError::InvalidRange(self.start, frame));
        }

        let rendered = self.rendered();
        let split = frame - self.start;
        let left_buffer = rendered.slice(0, split)?;
        let right_buffer = rendered.slice(split, rendered.len())?;

        let baked_tl = self.trim_left.as_ref().map(|b| {
            if self.stretch_ratio == 1.0 {
                b.clone()
            } else {
                time_stretch(b, self.stretch_ratio)
            }
        });
        let baked_tr = self.trim_right.as_ref().map(|b| {
            if self.stretch_ratio == 1.0 {
                b.clone()
            } else {
                time_stretch(b, self.stretch_ratio)
            }
        });

        let mut left_clip = Clip::new(self.name.clone(), self.start, left_buffer.clone());
        left_clip.trim_left = baked_tl.clone();
        left_clip.trim_right = Some(match baked_tr.as_ref() {
            Some(tr) => concat_buffers(&right_buffer, tr),
            None => right_buffer.clone(),
        });

        let mut right_clip = Clip::new(format!("{}.1", self.name), frame, right_buffer);
        right_clip.trim_left = Some(match baked_tl.as_ref() {
            Some(tl) => concat_buffers(tl, &left_buffer),
            None => left_buffer,
        });
        right_clip.trim_right = baked_tr;

        Ok((left_clip, right_clip))
    }
}

/// A point in a volume envelope.
#[derive(Clone, Debug, PartialEq)]
pub struct EnvPoint {
    /// Time in seconds on the timeline.
    pub time: f64,
    /// Linear amplitude value.
    pub value: f64,
}

/// A volume envelope made of time/value points.
#[derive(Clone, Debug, PartialEq)]
pub struct Envelope {
    /// The envelope's sorted control points.
    pub points: Vec<EnvPoint>,
    /// Value used when the envelope has no points.
    pub default_value: f64,
}

impl Envelope {
    /// Creates an identity envelope with a default value of `1.0`.
    pub fn new() -> Self {
        Self {
            points: Vec::new(),
            default_value: 1.0,
        }
    }

    /// Returns the interpolated amplitude value at time `t`.
    pub fn value_at(&self, t: f64) -> f64 {
        if self.points.is_empty() {
            return self.default_value;
        }
        if t <= self.points[0].time {
            return self.points[0].value;
        }
        if t >= self.points[self.points.len() - 1].time {
            return self.points[self.points.len() - 1].value;
        }

        for pair in self.points.windows(2) {
            let left = &pair[0];
            let right = &pair[1];
            if t >= left.time && t <= right.time {
                let span = right.time - left.time;
                if span <= 0.0 {
                    return right.value;
                }
                let fraction = (t - left.time) / span;
                return left.value + (right.value - left.value) * fraction;
            }
        }

        self.default_value
    }

    /// Inserts or replaces a point, returning its sorted index.
    ///
    /// Values are clamped to the Audacity volume envelope range `[0.0, 2.0]`.
    pub fn insert(&mut self, time: f64, value: f64) -> usize {
        let value = value.clamp(0.0, 2.0);
        if let Some(index) = self
            .points
            .iter()
            .position(|point| (point.time - time).abs() <= 1e-9)
        {
            self.points[index].value = value;
            return index;
        }

        let index = self
            .points
            .partition_point(|point| point.time.total_cmp(&time).is_lt());
        self.points.insert(index, EnvPoint { time, value });
        index
    }

    /// Removes and returns a point by index.
    pub fn remove(&mut self, idx: usize) -> Result<EnvPoint> {
        if idx >= self.points.len() {
            return Err(CoreError::InvalidRange(idx, idx.saturating_add(1)));
        }
        Ok(self.points.remove(idx))
    }

    /// Moves a point, re-sorts the envelope, and returns its new index.
    pub fn move_point(&mut self, idx: usize, time: f64, value: f64) -> Result<usize> {
        if idx >= self.points.len() {
            return Err(CoreError::InvalidRange(idx, idx.saturating_add(1)));
        }
        self.points.remove(idx);
        Ok(self.insert(time, value))
    }

    /// Applies the envelope to audio beginning at `start_frame`.
    pub fn apply(&self, buf: &mut AudioBuffer, start_frame: usize) {
        if buf.sample_rate == 0 {
            return;
        }
        let rate = f64::from(buf.sample_rate);
        for channel in &mut buf.channels {
            for (index, sample) in channel.iter_mut().enumerate() {
                let time = start_frame.saturating_add(index) as f64 / rate;
                *sample *= self.value_at(time) as Sample;
            }
        }
    }

    /// Shifts every point by `dt` seconds.
    pub fn shift(&mut self, dt: f64) {
        for point in &mut self.points {
            point.time += dt;
        }
        self.points.sort_by(|a, b| a.time.total_cmp(&b.time));
    }

    /// Returns whether this envelope is an identity envelope.
    pub fn is_identity(&self) -> bool {
        self.default_value == 1.0 && self.points.iter().all(|point| point.value == 1.0)
    }
}

impl Default for Envelope {
    fn default() -> Self {
        Self::new()
    }
}

/// Audio removed by a Cut while "Enable cut lines" is on; it can be
/// expanded back (Audacity's WaveClip cut lines).
#[derive(Clone, Debug, PartialEq)]
pub struct CutLine {
    /// Timeline frame where the cut line sits.
    pub at: usize,
    /// The removed audio (pre-envelope).
    pub audio: AudioBuffer,
}

/// A collection of clips forming a wave track.
#[derive(Clone, Debug, PartialEq)]
pub struct WaveClips {
    /// Sorted clips on the track.
    pub clips: Vec<Clip>,
    /// The track's volume envelope.
    pub envelope: Envelope,
    /// The track's sample rate.
    pub sample_rate: u32,
    /// The track's channel count.
    pub n_channels: usize,
    /// Cut lines left by cuts with "Enable cut lines" on.
    pub cut_lines: Vec<CutLine>,
    /// Audio of the first clip that sits before time 0 (Audacity lets a clip start at a negative
    /// time; that part is kept but never played or rendered).
    pub hidden_head: Option<HiddenHead>,
}

/// The part of the clip at frame 0 that lies before time 0.
#[derive(Clone, Debug, PartialEq)]
pub struct HiddenHead {
    /// Source audio before time 0, in order.
    pub audio: AudioBuffer,
}

fn concat_buffers(a: &AudioBuffer, b: &AudioBuffer) -> AudioBuffer {
    let channels = a.channels.iter().zip(&b.channels)
        .map(|(x, y)| x.iter().chain(y.iter()).copied().collect()).collect();
    AudioBuffer::from_channels(b.sample_rate, channels)
}

fn split_buffer(a: &AudioBuffer, at: usize) -> (AudioBuffer, AudioBuffer) {
    let (l, r) = a.channels.iter().map(|c| (c[..at].to_vec(), c[at..].to_vec())).unzip();
    (AudioBuffer::from_channels(a.sample_rate, l), AudioBuffer::from_channels(a.sample_rate, r))
}

impl WaveClips {
    /// The hidden head of clip 0, if it still belongs to it.
    pub fn hidden_head(&self) -> Option<&HiddenHead> {
        let head = self.hidden_head.as_ref()?;
        let first = self.clips.first()?;
        (first.start == 0).then_some(head)
    }

    /// A clip's start in timeline frames; negative when it begins before time 0.
    pub fn clip_start_frames(&self, idx: usize) -> i64 {
        let Some(clip) = self.clips.get(idx) else { return 0 };
        match (idx, self.hidden_head()) {
            (0, Some(head)) => -(rendered_len(head.audio.len(), clip.stretch_ratio) as i64),
            _ => clip.start as i64,
        }
    }

    /// Creates an empty wave track.
    pub fn new(rate: u32, n_ch: usize) -> Self {
        Self {
            clips: Vec::new(),
            envelope: Envelope::new(),
            sample_rate: rate,
            n_channels: n_ch,
            cut_lines: Vec::new(),
            hidden_head: None,
        }
    }

    /// Creates a wave track containing one clip at frame zero.
    pub fn from_buffer(name: impl Into<String>, buffer: AudioBuffer) -> Self {
        let mut track = Self::new(buffer.sample_rate, buffer.n_channels());
        track.clips.push(Clip::new(name, 0, buffer));
        track
    }

    /// Returns the greatest exclusive clip ending frame, or zero when empty.
    pub fn end(&self) -> usize {
        self.clips.iter().map(Clip::end).max().unwrap_or(0)
    }

    /// Returns the index of the clip containing `frame`.
    pub fn clip_at(&self, frame: usize) -> Option<usize> {
        self.clips.iter().position(|clip| clip.contains(frame))
    }

    /// Renders the complete track, including its envelope.
    pub fn render(&self) -> AudioBuffer {
        self.render_range(0, self.end())
            .unwrap_or_else(|_| empty_buffer(self.sample_rate, self.n_channels, 0))
    }

    /// Renders `[start, end)`, including the track envelope.
    pub fn render_range(&self, start: usize, end: usize) -> Result<AudioBuffer> {
        let mut rendered = self.render_pre_envelope_range(start, end)?;
        self.envelope.apply(&mut rendered, start);
        Ok(rendered)
    }

    /// Splits the clip containing `frame`; boundaries and gaps are unchanged.
    pub fn split(&mut self, frame: usize) -> Result<()> {
        let Some(index) = self.clip_at(frame) else {
            return Ok(());
        };
        let (left, right) = self.clips[index].split_at(frame)?;
        self.clips.splice(index..=index, [left, right]);
        Ok(())
    }

    /// Joins clips intersecting `[start, end)` into one clip, filling gaps with silence.
    pub fn join(&mut self, start: usize, end: usize) -> Result<()> {
        if start >= end {
            return Err(CoreError::InvalidRange(start, end));
        }

        let indices: Vec<usize> = self
            .clips
            .iter()
            .enumerate()
            .filter_map(|(index, clip)| {
                (clip.end() > start && clip.start < end).then_some(index)
            })
            .collect();
        let Some(&first_index) = indices.first() else {
            return Err(CoreError::InvalidRange(start, end));
        };

        let first_start = self.clips[indices[0]].start;
        let last_end = self.clips[*indices.last().unwrap()].end();
        let length = last_end.saturating_sub(first_start);
        let mut buffer = empty_buffer(self.sample_rate, self.n_channels, length);

        for &index in &indices {
            overlay_clip(&self.clips[index], &mut buffer, first_start);
        }

        let first_trim_left = self.clips[indices[0]].trim_left.clone();
        let last_trim_right = self.clips[*indices.last().unwrap()].trim_right.clone();

        let selected: Vec<bool> = (0..self.clips.len())
            .map(|index| indices.contains(&index))
            .collect();
        let mut joined = Vec::with_capacity(self.clips.len() - indices.len() + 1);
        for (index, clip) in self.clips.drain(..).enumerate() {
            if index == first_index {
                let mut new_clip = Clip::new(clip.name.clone(), first_start, buffer.clone());
                new_clip.trim_left = first_trim_left.clone();
                new_clip.trim_right = last_trim_right.clone();
                joined.push(new_clip);
            }
            if !selected[index] {
                joined.push(clip);
            }
        }
        self.clips = joined;
        self.sort_clips();
        Ok(())
    }

    /// Moves a clip, clamping its start to zero and rejecting overlaps.
    ///
    /// Returns the clip's index after sorting.
    /// Moves a clip, clamping its start to zero and rejecting overlaps.
    ///
    /// Returns the clip's index after sorting.
    pub fn move_clip(&mut self, idx: usize, new_start: i64) -> Result<usize> {
        if idx >= self.clips.len() {
            return Err(CoreError::InvalidRange(idx, idx.saturating_add(1)));
        }
        // Audacity allows clips to start before 0. The part before 0 is kept in `hidden_head`.
        if new_start < 0 || (idx == 0 && self.hidden_head().is_some()) {
            let head = if idx == 0 { self.hidden_head().cloned() } else { None };
            let clip = &self.clips[idx];
            let ratio = clip.stretch_ratio;
            let full = match &head { Some(h) => concat_buffers(&h.audio, &clip.buffer), None => clip.buffer.clone() };
            let total_frames = rendered_len(full.len(), ratio) as i64;
            let new_start = new_start.max(1 - total_frames); // keep at least one sample at or after 0
            let (start, hidden, visible) = if new_start < 0 {
                let frames_before_zero = (-new_start) as f64;
                let head_samples = if (ratio - 1.0).abs() <= f64::EPSILON {
                    (-new_start) as usize
                } else {
                    (frames_before_zero / ratio).round() as usize
                };
                let head_samples = head_samples.clamp(1, full.len().saturating_sub(1));
                let (h, v) = split_buffer(&full, head_samples);
                (0usize, Some(h), v)
            } else {
                (new_start as usize, None, full)
            };
            let visible_frames = rendered_len(visible.len(), ratio);
            if !self.can_place(idx, start, visible_frames) {
                return Err(CoreError::InvalidRange(start, start.saturating_add(visible_frames)));
            }
            self.clips[idx].buffer = visible;
            self.clips[idx].start = start;
            self.hidden_head = hidden.map(|audio| HiddenHead { audio });
            self.sort_clips();
            return self.clips.iter().position(|clip| clip.start == start)
                .ok_or(CoreError::InvalidRange(start, start.saturating_add(visible_frames)));
        }
        let start = new_start.max(0) as usize;
        let len = self.clips[idx].len();
        if !self.can_place(idx, start, len) {
            return Err(CoreError::InvalidRange(start, start.saturating_add(len)));
        }

        if (idx == 0 && start > 0) || (idx != 0 && start == 0) {
            self.hidden_head = None;
        }
        self.clips[idx].start = start;
        self.sort_clips();
        self.clips
            .iter()
            .position(|clip| clip.start == start)
            .ok_or(CoreError::InvalidRange(start, start.saturating_add(len)))
    }

    /// Trims a clip's left edge to timeline `frame`, rejecting overlaps.
    pub fn trim_clip_left(&mut self, idx: usize, frame: usize) -> Result<()> {
        if idx >= self.clips.len() {
            return Err(CoreError::InvalidRange(idx, idx.saturating_add(1)));
        }
        let mut modified = self.clips[idx].clone();
        modified.trim_left_to(frame)?;
        if !self.can_place(idx, modified.start, modified.len()) {
            return Err(CoreError::InvalidRange(modified.start, modified.end()));
        }
        if idx == 0 && modified.start > 0 {
            self.hidden_head = None;
        }
        self.clips[idx] = modified;
        self.sort_clips();
        Ok(())
    }

    /// Trims a clip's right edge to timeline `frame`, rejecting overlaps.
    pub fn trim_clip_right(&mut self, idx: usize, frame: usize) -> Result<()> {
        if idx >= self.clips.len() {
            return Err(CoreError::InvalidRange(idx, idx.saturating_add(1)));
        }
        let mut modified = self.clips[idx].clone();
        modified.trim_right_to(frame)?;
        if !self.can_place(idx, modified.start, modified.len()) {
            return Err(CoreError::InvalidRange(modified.start, modified.end()));
        }
        self.clips[idx] = modified;
        self.sort_clips();
        Ok(())
    }

    /// Returns whether the clip at `idx` can occupy `[start, start + len)` without overlap.
    pub fn can_place(&self, idx: usize, start: usize, len: usize) -> bool {
        if idx >= self.clips.len() {
            return false;
        }
        let end = start.saturating_add(len);
        self.clips.iter().enumerate().all(|(other_idx, clip)| {
            other_idx == idx || end <= clip.start || start >= clip.end()
        })
    }

    /// Deletes `[start, end)` and returns its pre-envelope audio.
    ///
    /// Clips are trimmed or shortened as needed, and clips after the range shift left.
    pub fn remove_range(&mut self, start: usize, end: usize) -> Result<AudioBuffer> {
        if start > end {
            return Err(CoreError::InvalidRange(start, end));
        }
        let removed_audio = self.render_pre_envelope_range(start, end)?;
        if start == end {
            return Ok(removed_audio);
        }
        let amount = end - start;
        let clear_hidden_head = start == 0
            && self.clips.first().is_some_and(|clip| clip.start == 0 && end >= clip.end());
        let mut updated = Vec::with_capacity(self.clips.len());

        for clip in self.clips.drain(..) {
            let clip_end = clip.end();
            if clip_end <= start {
                updated.push(clip);
                continue;
            }
            if clip.start >= end {
                let mut shifted = clip;
                shifted.start = shifted.start.saturating_sub(amount);
                updated.push(shifted);
                continue;
            }

            let rendered = clip.rendered();
            let left_len = start.saturating_sub(clip.start).min(rendered.len());
            let right_start = end.saturating_sub(clip.start).min(rendered.len());
            let mut channels = Vec::with_capacity(rendered.n_channels());

            for channel in &rendered.channels {
                let mut samples = Vec::with_capacity(
                    left_len.saturating_add(rendered.len().saturating_sub(right_start)),
                );
                samples.extend_from_slice(&channel[..left_len.min(channel.len())]);
                if right_start < channel.len() {
                    samples.extend_from_slice(&channel[right_start..]);
                }
                channels.push(samples);
            }

            if channels.first().is_some_and(|channel| !channel.is_empty()) {
                let new_start = if clip.start >= start {
                    start
                } else {
                    clip.start
                };
                updated.push(Clip::new(
                    clip.name,
                    new_start,
                    AudioBuffer::from_channels(rendered.sample_rate, channels),
                ));
            }
        }

        self.clips = updated;
        if clear_hidden_head {
            self.hidden_head = None;
        }
        self.sort_clips();
        self.remove_envelope_range(start, end);
        Ok(removed_audio)
    }

    /// Inserts audio at `at`, inserting into a clip at its interior or exclusive end.
    ///
    /// In a gap, a new clip named `"Pasted"` is created. Clips starting at or after
    /// the insertion point are shifted right, except for the insertion target.
    pub fn insert(&mut self, at: usize, buffer: &AudioBuffer) -> Result<()> {
        if buffer.sample_rate != self.sample_rate {
            return Err(CoreError::RateMismatch);
        }
        if buffer.n_channels() != self.n_channels {
            return Err(CoreError::ChannelMismatch);
        }
        let length = buffer.len();
        if length == 0 {
            return Ok(());
        }

        let target = self
            .clips
            .iter()
            .position(|clip| clip.contains(at) || clip.end() == at);

        for (index, clip) in self.clips.iter_mut().enumerate() {
            if Some(index) != target && clip.start >= at {
                clip.start = clip.start.saturating_add(length);
            }
        }

        if let Some(index) = target {
            let mut rendered = self.clips[index].rendered();
            let local_at = at.saturating_sub(self.clips[index].start);
            rendered.insert(local_at, buffer)?;
            self.clips[index].buffer = rendered;
            self.clips[index].stretch_ratio = 1.0;
        } else {
            self.clips
                .push(Clip::new("Pasted", at, buffer.clone()));
        }
        self.sort_clips();
        Ok(())
    }

    /// Applies an edit to pre-envelope audio in `[start, end)` and replaces that range.
    pub fn apply_range(
        &mut self,
        start: usize,
        end: usize,
        f: impl FnOnce(&mut AudioBuffer),
    ) -> Result<()> {
        if start > end {
            return Err(CoreError::InvalidRange(start, end));
        }
        let mut edited = self.render_pre_envelope_range(start, end)?;
        f(&mut edited);
        if edited.sample_rate != self.sample_rate {
            return Err(CoreError::RateMismatch);
        }
        if edited.n_channels() != self.n_channels {
            return Err(CoreError::ChannelMismatch);
        }
        let saved_head = self.hidden_head().cloned();
        self.remove_range(start, end)?;
        self.insert(start, &edited)?;
        if self.clips.first().is_some_and(|clip| clip.start == 0) {
            if saved_head.is_some() {
                self.hidden_head = saved_head;
            }
        }
        Ok(())
    }

    /// Sets a clip's time-stretch ratio, rejecting nonpositive or overlapping results.
    pub fn stretch_clip(&mut self, idx: usize, ratio: f64) -> Result<()> {
        if idx >= self.clips.len() {
            return Err(CoreError::InvalidRange(idx, idx.saturating_add(1)));
        }
        if !ratio.is_finite() || ratio <= 0.0 {
            return Err(CoreError::InvalidRange(0, 0));
        }

        let start = self.clips[idx].start;
        let len = rendered_len(self.clips[idx].buffer.len(), ratio);
        if !self.can_place(idx, start, len) {
            return Err(CoreError::InvalidRange(start, start.saturating_add(len)));
        }
        self.clips[idx].stretch_ratio = ratio;
        self.sort_clips();
        Ok(())
    }

    /// Finds the nearest rendered zero crossing within `radius` frames of `frame`.
    pub fn zero_crossings_near(&self, frame: usize, radius: usize) -> Option<usize> {
        let rendered = self.render();
        if rendered.channels.is_empty() || rendered.len() == 0 {
            return None;
        }

        let lower = frame.saturating_sub(radius).max(1);
        let upper = frame.saturating_add(radius).min(rendered.len().saturating_sub(1));
        if lower > upper {
            return None;
        }

        let sum_at = |at: usize| -> f64 {
            rendered
                .channels
                .iter()
                .filter_map(|channel| channel.get(at))
                .map(|sample| f64::from(*sample))
                .sum()
        };

        (lower..=upper)
            .filter(|&at| {
                let before = sum_at(at - 1);
                let after = sum_at(at);
                before == 0.0 || after == 0.0 || before.is_sign_positive() != after.is_sign_positive()
            })
            .min_by_key(|&at| at.abs_diff(frame))
    }

    fn render_pre_envelope_range(&self, start: usize, end: usize) -> Result<AudioBuffer> {
        if start > end {
            return Err(CoreError::InvalidRange(start, end));
        }
        let mut output = empty_buffer(self.sample_rate, self.n_channels, end - start);
        for clip in &self.clips {
            overlay_clip(&clip, &mut output, start);
        }
        Ok(output)
    }

    fn remove_envelope_range(&mut self, start: usize, end: usize) {
        if start == end || self.sample_rate == 0 {
            return;
        }
        let start_time = start as f64 / f64::from(self.sample_rate);
        let end_time = end as f64 / f64::from(self.sample_rate);
        let duration = end_time - start_time;
        self.envelope.points.retain(|point| {
            point.time < start_time || point.time >= end_time
        });
        for point in &mut self.envelope.points {
            if point.time >= end_time {
                point.time -= duration;
            }
        }
    }

    pub(crate) fn sort_clips(&mut self) {
        self.clips.sort_by_key(|clip| clip.start);
    }
}

/// Performs pitch-preserving waveform-similarity overlap-add time stretching.
///
/// The implementation uses WSOLA with a Hann window, a 50% synthesis overlap,
/// and a short cross-correlation search. Inputs shorter than one analysis frame
/// use linear interpolation instead.
pub fn time_stretch(buf: &AudioBuffer, ratio: f64) -> AudioBuffer {
    if !ratio.is_finite() || ratio <= 0.0 {
        return empty_buffer(buf.sample_rate, buf.n_channels(), 0);
    }
    if ratio == 1.0 {
        return buf.clone();
    }

    let output_len = rendered_len(buf.len(), ratio);
    if output_len == 0 || buf.n_channels() == 0 {
        return empty_buffer(buf.sample_rate, buf.n_channels(), output_len);
    }

    let frame_len = ((f64::from(buf.sample_rate.max(1)) * 1024.0 / 44_100.0).round() as usize)
        .max(32);
    if buf.len() < frame_len {
        return linear_stretch(buf, output_len);
    }

    let synthesis_hop = (frame_len / 2).max(1);
    let tolerance = ((f64::from(buf.sample_rate.max(1)) * 0.01).round() as usize).max(1);
    let last_candidate = buf.len() - frame_len;
    let window: Vec<f64> = (0..frame_len)
        .map(|index| {
            let phase = std::f64::consts::PI * (index as f64 + 0.5) / frame_len as f64;
            phase.sin()
        })
        .collect();

    let mut output = vec![vec![0.0_f64; output_len]; buf.n_channels()];
    let mut weights = vec![0.0_f64; output_len];
    let mut output_pos = 0usize;
    let mut _previous_candidate = 0usize;
    let mut first = true;

    while output_pos < output_len {
        let expected = (output_pos as f64 / ratio).round() as usize;
        let candidate = if first {
            0
        } else {
            let low = expected.saturating_sub(tolerance);
            let high = expected.saturating_add(tolerance).min(last_candidate);
            let overlap = synthesis_hop.min(output_len - output_pos);
            let mut best = expected.min(last_candidate);
            let mut best_score = f64::NEG_INFINITY;

            for candidate in low.min(last_candidate)..=high.max(low.min(last_candidate)) {
                let mut dot = 0.0;
                let mut left_energy = 0.0;
                let mut right_energy = 0.0;
                for offset in 0..overlap {
                    let out_index = output_pos + offset;
                    let weight = weights[out_index];
                    if weight <= 0.0 {
                        continue;
                    }
                    let left: f64 = output
                        .iter()
                        .map(|channel| channel[out_index] / weight)
                        .sum();
                    let right: f64 = buf
                        .channels
                        .iter()
                        .filter_map(|channel| channel.get(candidate + offset))
                        .map(|sample| f64::from(*sample))
                        .sum();
                    dot += left * right;
                    left_energy += left * left;
                    right_energy += right * right;
                }
                let score = if left_energy > 0.0 && right_energy > 0.0 {
                    dot / (left_energy * right_energy).sqrt()
                } else {
                    -((candidate as isize - expected as isize).unsigned_abs() as f64)
                };
                if score > best_score {
                    best_score = score;
                    best = candidate;
                }
            }
            best
        };

        for offset in 0..frame_len {
            let out_index = output_pos.saturating_add(offset);
            if out_index >= output_len {
                break;
            }
            let weight = window[offset];
            weights[out_index] += weight;
            for (channel_index, channel) in buf.channels.iter().enumerate() {
                if let Some(sample) = channel.get(candidate + offset) {
                    output[channel_index][out_index] += f64::from(*sample) * weight;
                }
            }
        }

        _previous_candidate = candidate;
        first = false;
        output_pos = output_pos.saturating_add(synthesis_hop);
    }

    let channels = output
        .into_iter()
        .map(|channel| {
            channel
                .into_iter()
                .enumerate()
                .map(|(index, sample)| {
                    if weights[index] > 0.0 {
                        (sample / weights[index]) as Sample
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect();
    AudioBuffer::from_channels(buf.sample_rate, channels)
}

/// A timeline label, either a point or a region.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// Label start time in seconds.
    pub start: f64,
    /// Label end time in seconds.
    pub end: f64,
    /// Label text.
    pub text: String,
}

impl Label {
    /// Returns whether this label describes a nonempty region.
    pub fn is_region(&self) -> bool {
        self.end > self.start
    }
}

/// A sorted collection of timeline labels.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelTrack {
    /// The label track's name.
    pub name: String,
    /// Labels sorted by start time.
    pub labels: Vec<Label>,
}

impl LabelTrack {
    /// Creates an empty label track.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            labels: Vec::new(),
        }
    }

    /// Adds a label, normalizing its times, and returns its sorted index.
    pub fn add(&mut self, start: f64, end: f64, text: impl Into<String>) -> usize {
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let label = Label {
            start,
            end,
            text: text.into(),
        };
        let index = self
            .labels
            .partition_point(|existing| existing.start.total_cmp(&start).is_le());
        self.labels.insert(index, label);
        index
    }

    /// Replaces the text of a label.
    pub fn edit_text(&mut self, idx: usize, text: impl Into<String>) -> Result<()> {
        let Some(label) = self.labels.get_mut(idx) else {
            return Err(CoreError::InvalidRange(idx, idx.saturating_add(1)));
        };
        label.text = text.into();
        Ok(())
    }

    /// Sets a label's times, normalizes them, and returns its new sorted index.
    pub fn set_times(&mut self, idx: usize, start: f64, end: f64) -> Result<usize> {
        if idx >= self.labels.len() {
            return Err(CoreError::InvalidRange(idx, idx.saturating_add(1)));
        }
        let mut label = self.labels.remove(idx);
        (label.start, label.end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let new_index = self
            .labels
            .partition_point(|existing| existing.start.total_cmp(&label.start).is_le());
        self.labels.insert(new_index, label);
        Ok(new_index)
    }

    /// Removes and returns a label by index.
    pub fn remove(&mut self, idx: usize) -> Result<Label> {
        if idx >= self.labels.len() {
            return Err(CoreError::InvalidRange(idx, idx.saturating_add(1)));
        }
        Ok(self.labels.remove(idx))
    }

    /// Shifts labels beginning at or after `t` by `dt` seconds.
    pub fn shift_after(&mut self, t: f64, dt: f64) {
        for label in &mut self.labels {
            if label.start >= t {
                label.start += dt;
                label.end += dt;
            }
        }
        self.sort_labels();
    }

    /// Removes a timeline range from labels, shrinking regions that span it.
    pub fn remove_range(&mut self, start: f64, end: f64) {
        if start >= end {
            return;
        }
        let duration = end - start;
        self.labels.retain(|label| !(label.start >= start && label.end <= end));
        for label in &mut self.labels {
            if label.start >= end {
                label.start -= duration;
                label.end -= duration;
            } else if label.start < start && label.end > start {
                if label.end <= end {
                    label.end = start;
                } else {
                    label.end -= duration;
                }
            } else if label.start >= start && label.start < end {
                label.start = start;
                label.end -= duration;
            }
        }
        self.sort_labels();
    }

    /// Inserts timeline space, shifting later labels and expanding spanning regions.
    pub fn insert_space(&mut self, at: f64, dur: f64) {
        if dur <= 0.0 {
            return;
        }
        for label in &mut self.labels {
            if label.start >= at {
                label.start += dur;
                label.end += dur;
            } else if label.start < at && label.end > at {
                label.end += dur;
            }
        }
        self.sort_labels();
    }

    /// Exports labels in Audacity tab-separated text format.
    pub fn to_audacity_txt(&self) -> String {
        let mut output = String::new();
        for label in &self.labels {
            output.push_str(&format!(
                "{:.6}\t{:.6}\t{}\n",
                label.start, label.end, label.text
            ));
        }
        output
    }

    /// Parses Audacity tab-separated label text.
    ///
    /// Spectral-frequency lines beginning with a backslash and blank lines are skipped.
    pub fn from_audacity_txt(name: impl Into<String>, text: &str) -> Result<LabelTrack> {
        let mut track = LabelTrack::new(name);
        for (line_index, original) in text.lines().enumerate() {
            let line = original.trim_end_matches('\r');
            let trimmed = line.trim();
            if trimmed.is_empty() || line.starts_with('\\') {
                continue;
            }
            let line_no = line_index + 1;
            let fields: Vec<&str> = line.split('\t').collect();

            if fields.len() == 1 {
                let time = fields[0]
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| CoreError::InvalidRange(line_no, line_no))?;
                track.add(time, time, "");
            } else if fields.len() == 2 {
                let start = fields[0]
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| CoreError::InvalidRange(line_no, line_no))?;
                let end = fields[1]
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| CoreError::InvalidRange(line_no, line_no))?;
                track.add(start, end, "");
            } else if fields.len() == 3 {
                let start = fields[0]
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| CoreError::InvalidRange(line_no, line_no))?;
                let end = fields[1]
                    .trim()
                    .parse::<f64>()
                    .map_err(|_| CoreError::InvalidRange(line_no, line_no))?;
                track.add(start, end, fields[2]);
            } else {
                return Err(CoreError::InvalidRange(line_no, line_no));
            }
        }
        Ok(track)
    }

    fn sort_labels(&mut self) {
        self.labels
            .sort_by(|left, right| left.start.total_cmp(&right.start));
    }
}

/// The snapping behavior used when positioning timeline selections.
#[derive(Clone, Debug, PartialEq)]
pub enum SnapMode {
    /// Do not snap.
    Off,
    /// Snap to multiples of the given interval in seconds.
    Grid {
        /// Grid interval in seconds.
        secs: f64,
    },
    /// Snap to sample boundaries.
    Samples,
    /// Snap to nearby zero crossings.
    ZeroCrossing,
    /// Snap to label starts or ends.
    Labels,
}

/// A helper that snaps times to a selected timeline feature.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapper {
    /// The active snapping mode.
    pub mode: SnapMode,
    /// Maximum snap distance in screen pixels.
    pub pixel_tolerance: f64,
}

impl Snapper {
    /// Snaps `t` using the configured mode and timeline context.
    pub fn snap(
        &self,
        t: f64,
        zoom: &ZoomInfo,
        rate: u32,
        labels: &[LabelTrack],
        clips: Option<&WaveClips>,
    ) -> f64 {
        match self.mode {
            SnapMode::Off => t,
            SnapMode::Grid { secs } if secs.is_finite() && secs > 0.0 => {
                (t / secs).round() * secs
            }
            SnapMode::Grid { .. } => t,
            SnapMode::Samples if rate > 0 => (t * f64::from(rate)).round() / f64::from(rate),
            SnapMode::Samples => t,
            SnapMode::Labels => {
                let Some(pixels_per_sec) = valid_pixels_per_sec(zoom.pixels_per_sec) else {
                    return t;
                };
                labels
                    .iter()
                    .flat_map(|track| track.labels.iter())
                    .flat_map(|label| [label.start, label.end])
                    .filter(|candidate| {
                        (candidate - t).abs() * pixels_per_sec <= self.pixel_tolerance
                    })
                    .min_by(|left, right| {
                        (left - t)
                            .abs()
                            .total_cmp(&(right - t).abs())
                    })
                    .unwrap_or(t)
            }
            SnapMode::ZeroCrossing => {
                let Some(clips) = clips else {
                    return t;
                };
                if clips.sample_rate == 0 {
                    return t;
                }
                let Some(pixels_per_sec) = valid_pixels_per_sec(zoom.pixels_per_sec) else {
                    return t;
                };
                let rate = f64::from(clips.sample_rate);
                let frame = (t * rate).round().max(0.0) as usize;
                let radius = (self.pixel_tolerance / pixels_per_sec * rate)
                    .ceil()
                    .max(0.0) as usize;
                clips
                    .zero_crossings_near(frame, radius)
                    .map(|crossing| crossing as f64 / rate)
                    .unwrap_or(t)
            }
        }
    }
}

/// Horizontal timeline coordinate mapping and zoom state.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoomInfo {
    /// Time in seconds at horizontal coordinate zero.
    pub h_offset: f64,
    /// Horizontal pixels per second.
    pub pixels_per_sec: f64,
}

impl Default for ZoomInfo {
    fn default() -> Self {
        Self {
            h_offset: 0.0,
            pixels_per_sec: 86.1328125,
        }
    }
}

impl ZoomInfo {
    /// Converts a time in seconds to a horizontal coordinate.
    pub fn time_to_x(&self, time: f64) -> f64 {
        (time - self.h_offset) * self.pixels_per_sec
    }

    /// Converts a horizontal coordinate to time in seconds.
    pub fn x_to_time(&self, x: f64) -> f64 {
        let pixels_per_sec = valid_pixels_per_sec(self.pixels_per_sec).unwrap_or(1.0);
        self.h_offset + x / pixels_per_sec
    }

    /// Converts a timeline frame to a horizontal coordinate.
    pub fn frame_to_x(&self, frame: usize, rate: u32) -> f64 {
        if rate == 0 {
            return self.time_to_x(0.0);
        }
        self.time_to_x(frame as f64 / f64::from(rate))
    }

    /// Converts a horizontal coordinate to a rounded, nonnegative frame.
    pub fn x_to_frame(&self, x: f64, rate: u32) -> usize {
        if rate == 0 {
            return 0;
        }
        let frame = (self.x_to_time(x) * f64::from(rate)).round();
        if !frame.is_finite() || frame <= 0.0 {
            0
        } else {
            frame.min(usize::MAX as f64) as usize
        }
    }

    /// Changes zoom around `anchor_x`, preserving the time at that coordinate.
    pub fn zoom_by(&mut self, factor: f64, anchor_x: f64) {
        if !factor.is_finite() || factor <= 0.0 {
            return;
        }
        let anchor_time = self.x_to_time(anchor_x);
        self.pixels_per_sec =
            (self.pixels_per_sec * factor).clamp(0.001, 6_000_000.0);
        self.h_offset = anchor_time - anchor_x / self.pixels_per_sec;
    }

    /// Doubles zoom, anchored at horizontal coordinate zero.
    pub fn zoom_in(&mut self) {
        self.zoom_by(2.0, 0.0);
    }

    /// Halves zoom, anchored at horizontal coordinate zero.
    pub fn zoom_out(&mut self) {
        self.zoom_by(0.5, 0.0);
    }

    /// Fits `duration` seconds into `width_px` pixels.
    pub fn fit(&mut self, duration: f64, width_px: f64) {
        self.h_offset = 0.0;
        if duration > 0.0 && width_px > 0.0 {
            self.pixels_per_sec = (width_px / duration).clamp(0.001, 6_000_000.0);
        }
    }
}

/// A timeline containing wave and label tracks.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Timeline {
    /// Wave tracks in the project.
    pub wave: Vec<WaveClips>,
    /// Label tracks in the project.
    pub labels: Vec<LabelTrack>,
    /// Whether edits are synchronized across tracks.
    pub sync_lock: bool,
    /// Timeline horizontal zoom and offset.
    pub zoom: ZoomInfo,
}

impl Timeline {
    /// Deletes a frame range from a wave track, synchronizing other tracks when enabled.
    pub fn delete_range(
        &mut self,
        track: usize,
        start_frame: usize,
        end_frame: usize,
        rate: u32,
    ) -> Result<AudioBuffer> {
        if track >= self.wave.len() {
            return Err(CoreError::TrackNotFound(track));
        }
        if rate == 0 {
            return Err(CoreError::InvalidRange(start_frame, end_frame));
        }

        let start_seconds = start_frame as f64 / f64::from(rate);
        let end_seconds = end_frame as f64 / f64::from(rate);
        let removed = self.wave[track].remove_range(start_frame, end_frame)?;

        if self.sync_lock {
            for (index, wave) in self.wave.iter_mut().enumerate() {
                if index == track {
                    continue;
                }
                let other_start = (start_seconds * f64::from(wave.sample_rate)).round() as usize;
                let other_end = (end_seconds * f64::from(wave.sample_rate)).round() as usize;
                wave.remove_range(other_start, other_end)?;
            }
            for labels in &mut self.labels {
                labels.remove_range(start_seconds, end_seconds);
            }
        }
        Ok(removed)
    }

    /// Inserts silence at a frame position, synchronizing other tracks when enabled.
    pub fn insert_silence(
        &mut self,
        track: usize,
        at: usize,
        frames: usize,
        rate: u32,
    ) -> Result<()> {
        if track >= self.wave.len() {
            return Err(CoreError::TrackNotFound(track));
        }
        if rate == 0 {
            return Err(CoreError::InvalidRange(at, at));
        }
        if self.wave[track].sample_rate != rate {
            return Err(CoreError::RateMismatch);
        }

        let seconds = at as f64 / f64::from(rate);
        let duration = frames as f64 / f64::from(rate);
        let silence = empty_buffer(
            rate,
            self.wave[track].n_channels,
            frames,
        );
        self.wave[track].insert(at, &silence)?;

        if self.sync_lock {
            for (index, wave) in self.wave.iter_mut().enumerate() {
                if index == track {
                    continue;
                }
                let other_at = (seconds * f64::from(wave.sample_rate)).round() as usize;
                let other_frames = (duration * f64::from(wave.sample_rate)).round() as usize;
                let other_silence =
                    empty_buffer(wave.sample_rate, wave.n_channels, other_frames);
                wave.insert(other_at, &other_silence)?;
            }
            for labels in &mut self.labels {
                labels.insert_space(seconds, duration);
            }
        }
        Ok(())
    }
}

fn rendered_len(source_len: usize, ratio: f64) -> usize {
    if !ratio.is_finite() || ratio <= 0.0 {
        return 0;
    }
    let length = (source_len as f64 * ratio).round();
    if !length.is_finite() || length <= 0.0 {
        0
    } else {
        length.min(usize::MAX as f64) as usize
    }
}

fn empty_buffer(rate: u32, n_channels: usize, frames: usize) -> AudioBuffer {
    AudioBuffer::from_channels(
        rate,
        (0..n_channels).map(|_| vec![0.0; frames]).collect(),
    )
}

fn linear_stretch(buf: &AudioBuffer, output_len: usize) -> AudioBuffer {
    let input_len = buf.len();
    let channels = buf
        .channels
        .iter()
        .map(|channel| {
            if input_len == 0 || channel.is_empty() || output_len == 0 {
                return vec![0.0; output_len];
            }
            (0..output_len)
                .map(|index| {
                    let position = if output_len <= 1 {
                        0.0
                    } else {
                        index as f64 * input_len.saturating_sub(1) as f64
                            / output_len.saturating_sub(1) as f64
                    };
                    let left = position.floor() as usize;
                    let right = (left + 1).min(channel.len() - 1);
                    let fraction = position - left as f64;
                    (f64::from(channel[left]) * (1.0 - fraction)
                        + f64::from(channel[right]) * fraction) as Sample
                })
                .collect()
        })
        .collect();
    AudioBuffer::from_channels(buf.sample_rate, channels)
}

fn overlay_clip(clip: &Clip, output: &mut AudioBuffer, output_start: usize) {
    let rendered = clip.rendered();
    let output_end = output_start.saturating_add(output.len());
    let overlap_start = clip.start.max(output_start);
    let overlap_end = clip.end().min(output_end);
    if overlap_start >= overlap_end {
        return;
    }

    for channel_index in 0..output.n_channels() {
        let Some(source) = rendered.channels.get(channel_index) else {
            continue;
        };
        let Some(destination) = output.channels.get_mut(channel_index) else {
            continue;
        };
        for frame in overlap_start..overlap_end {
            let source_index = frame - clip.start;
            let destination_index = frame - output_start;
            if let (Some(source_sample), Some(destination_sample)) =
                (source.get(source_index), destination.get_mut(destination_index))
            {
                *destination_sample = *source_sample;
            }
        }
    }
}

fn valid_pixels_per_sec(pixels_per_sec: f64) -> Option<f64> {
    (pixels_per_sec.is_finite() && pixels_per_sec > 0.0).then_some(pixels_per_sec)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mono(rate: u32, samples: &[f32]) -> AudioBuffer {
        AudioBuffer::from_channels(rate, vec![samples.to_vec()])
    }

    fn clip_before_zero() -> WaveClips {
        let mut track = WaveClips::from_buffer("clip", mono(10, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]));
        track.move_clip(0, -3).unwrap();
        track
    }

    #[test]
    fn timeline_split_join_and_sample_accurate_split() {
        let mut track = WaveClips::from_buffer("clip", mono(44_100, &[1.0, 2.0, 3.0]));
        track.split(1).unwrap();
        assert_eq!(track.clips.len(), 2);
        assert_eq!(track.clips[0].len(), 1);
        assert_eq!(track.clips[1].len(), 2);
        assert_eq!(track.clips[1].name, "clip.1");
        track.join(0, 3).unwrap();
        assert_eq!(track.clips.len(), 1);
        assert_eq!(track.clips[0].buffer.channels[0], vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn timeline_move_rejects_overlap() {
        let mut track = WaveClips::new(44_100, 1);
        track.clips.push(Clip::new("a", 0, mono(44_100, &[1.0; 4])));
        track.clips.push(Clip::new("b", 10, mono(44_100, &[2.0; 4])));
        assert!(track.move_clip(0, 8).is_err());
        assert_eq!(track.move_clip(0, 5).unwrap(), 0);
    }

    #[test]
    fn timeline_gaps_render_silence() {
        let mut track = WaveClips::new(44_100, 1);
        track.clips.push(Clip::new("a", 0, mono(44_100, &[1.0, 2.0])));
        track.clips.push(Clip::new("b", 4, mono(44_100, &[3.0])));
        assert_eq!(track.render().channels[0], vec![1.0, 2.0, 0.0, 0.0, 3.0]);
    }

    #[test]
    fn timeline_remove_range_trims_and_shifts() {
        let mut track = WaveClips::new(44_100, 1);
        track.clips.push(Clip::new("a", 0, mono(44_100, &[0.0, 1.0, 2.0, 3.0, 4.0])));
        track.clips.push(Clip::new("b", 8, mono(44_100, &[8.0, 9.0])));
        let removed = track.remove_range(1, 3).unwrap();
        assert_eq!(removed.channels[0], vec![1.0, 2.0]);
        assert_eq!(track.clips.len(), 2);
        assert_eq!(track.clips[0].buffer.channels[0], vec![0.0, 3.0, 4.0]);
        assert_eq!(track.clips[1].start, 6);
    }

    #[test]
    fn timeline_paste_into_clip_and_gap() {
        let mut track = WaveClips::from_buffer("a", mono(44_100, &[1.0, 2.0, 3.0, 4.0]));
        track.insert(2, &mono(44_100, &[8.0, 9.0])).unwrap();
        assert_eq!(track.clips.len(), 1);
        assert_eq!(
            track.clips[0].buffer.channels[0],
            vec![1.0, 2.0, 8.0, 9.0, 3.0, 4.0]
        );
        track.insert(10, &mono(44_100, &[7.0])).unwrap();
        assert!(track.clips.iter().any(|clip| clip.name == "Pasted"));
    }

    #[test]
    fn timeline_apply_range_keeps_single_clip() {
        let mut track = WaveClips::from_buffer("a", mono(44_100, &[1.0, 2.0, 3.0, 4.0]));
        track
            .apply_range(1, 3, |buffer| {
                for sample in &mut buffer.channels[0] {
                    *sample *= 2.0;
                }
            })
            .unwrap();
        assert_eq!(track.clips.len(), 1);
        assert_eq!(track.clips[0].buffer.channels[0], vec![1.0, 4.0, 6.0, 4.0]);
    }

    #[test]
    fn timeline_envelope_interpolation_and_apply() {
        let mut envelope = Envelope::new();
        envelope.insert(0.0, 0.0);
        envelope.insert(2.0, 2.0);
        assert_eq!(envelope.value_at(1.0), 1.0);
        let mut buffer = mono(1, &[1.0, 1.0, 1.0]);
        envelope.apply(&mut buffer, 0);
        assert_eq!(buffer.channels[0], vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn timeline_labels_txt_round_trip_and_spectral_skip() {
        let text = "0.5\t1.0\tregion\n\\freq\tignored\n2.0\n";
        let track = LabelTrack::from_audacity_txt("Labels", text).unwrap();
        assert_eq!(track.labels.len(), 2);
        let output = track.to_audacity_txt();
        let parsed = LabelTrack::from_audacity_txt("Labels", &output).unwrap();
        assert_eq!(parsed.labels, track.labels);
    }

    #[test]
    fn timeline_labels_txt_reports_bad_line() {
        let error = LabelTrack::from_audacity_txt("Labels", "0.0\tbad").unwrap_err();
        assert_eq!(error, CoreError::InvalidRange(1, 1));
    }

    #[test]
    fn timeline_label_remove_range_and_insert_space() {
        let mut labels = LabelTrack::new("Labels");
        labels.add(1.0, 1.0, "point");
        labels.add(2.0, 8.0, "region");
        labels.add(9.0, 9.0, "later");
        labels.remove_range(4.0, 6.0);
        assert_eq!(labels.labels.len(), 3);
        assert_eq!(labels.labels[1].end, 6.0);
        labels.insert_space(3.0, 2.0);
        assert_eq!(labels.labels[1].end, 8.0);
        assert_eq!(labels.labels[2].start, 9.0);
    }

    #[test]
    fn timeline_snap_grid_samples_and_labels() {
        let zoom = ZoomInfo {
            h_offset: 0.0,
            pixels_per_sec: 100.0,
        };
        let labels = vec![{
            let mut track = LabelTrack::new("Labels");
            track.add(1.0, 1.0, "point");
            track
        }];
        let snapper = Snapper {
            mode: SnapMode::Grid { secs: 0.5 },
            pixel_tolerance: 8.0,
        };
        assert_eq!(snapper.snap(0.74, &zoom, 10, &[], None), 0.5);

        let snapper = Snapper {
            mode: SnapMode::Samples,
            pixel_tolerance: 8.0,
        };
        assert_eq!(snapper.snap(0.26, &zoom, 10, &[], None), 0.3);

        let snapper = Snapper {
            mode: SnapMode::Labels,
            pixel_tolerance: 8.0,
        };
        assert_eq!(snapper.snap(0.95, &zoom, 10, &labels, None), 1.0);
    }

    #[test]
    fn timeline_snap_zero_crossing() {
        let rate = 1000;
        let samples: Vec<f32> = (0..100)
            .map(|index| if index < 50 { 1.0 } else { -1.0 })
            .collect();
        let track = WaveClips::from_buffer("audio", mono(rate, &samples));
        let snapper = Snapper {
            mode: SnapMode::ZeroCrossing,
            pixel_tolerance: 10.0,
        };
        let zoom = ZoomInfo {
            h_offset: 0.0,
            pixels_per_sec: 100.0,
        };
        assert_eq!(
            snapper.snap(0.048, &zoom, rate, &[], Some(&track)),
            0.05
        );
    }

    #[test]
    fn timeline_zoom_coordinate_round_trips_multiple_rates() {
        let zoom = ZoomInfo::default();
        for rate in [44_100, 48_000, 96_000] {
            let frame = rate as usize * 3 + 123;
            let x = zoom.frame_to_x(frame, rate);
            assert_eq!(zoom.x_to_frame(x, rate), frame);
        }
    }

    #[test]
    fn timeline_zoom_by_preserves_anchor() {
        let mut zoom = ZoomInfo {
            h_offset: 2.5,
            pixels_per_sec: 100.0,
        };
        let anchor_x = 250.0;
        let anchor_time = zoom.x_to_time(anchor_x);
        zoom.zoom_by(3.0, anchor_x);
        assert!((zoom.x_to_time(anchor_x) - anchor_time).abs() < 1e-12);
    }

    #[test]
    fn timeline_time_stretch_length_and_pitch() {
        let rate = 44_100;
        let samples: Vec<f32> = (0..rate)
            .map(|index| (2.0 * std::f32::consts::PI * 440.0 * index as f32 / rate as f32).sin())
            .collect();
        let stretched = time_stretch(&mono(rate, &samples), 1.2);
        assert_eq!(stretched.len(), 52_920);

        let channel = &stretched.channels[0];
        let crossings = channel
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        let duration = stretched.len() as f64 / f64::from(rate);
        let frequency = crossings as f64 / duration;
        assert!((frequency - 440.0).abs() / 440.0 < 0.05);
    }

    #[test]
    fn timeline_sync_lock_delete_shifts_labels_and_other_tracks() {
        let mut timeline = Timeline::default();
        timeline
            .wave
            .push(WaveClips::from_buffer("a", mono(10, &[1.0; 10])));
        timeline
            .wave
            .push(WaveClips::from_buffer("b", mono(10, &[2.0; 10])));
        timeline.labels.push({
            let mut labels = LabelTrack::new("Labels");
            labels.add(0.8, 0.8, "later");
            labels
        });
        timeline.sync_lock = true;

        timeline.delete_range(0, 2, 4, 10).unwrap();
        assert_eq!(timeline.wave[1].end(), 8);
        assert!((timeline.labels[0].labels[0].start - 0.6).abs() < 1e-9);
    }

    #[test]
    fn clip_moves_before_zero_and_back_like_audacity() {
        let buf = AudioBuffer::from_channels(100, vec![(0..10).map(|i| i as f32).collect()]);
        let mut t = WaveClips::from_buffer("c", buf.clone());
        t.move_clip(0, -3).unwrap();
        assert_eq!(t.clip_start_frames(0), -3);
        assert_eq!(t.clips[0].start, 0);
        assert_eq!(t.clips[0].buffer.channels[0], vec![3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        t.move_clip(0, -1).unwrap();
        assert_eq!(t.clip_start_frames(0), -1);
        t.move_clip(0, 2).unwrap();
        assert_eq!(t.clip_start_frames(0), 2);
        assert_eq!(t.clips[0].buffer, buf);
        assert!(t.hidden_head().is_none());
        t.move_clip(0, -50).unwrap();
        assert_eq!(t.clip_start_frames(0), -9);
    }

    #[test]
    fn hidden_head_survives_split_and_move_back() {
        let mut track = clip_before_zero();
        track.split(4).unwrap();
        track.move_clip(0, -3).unwrap();
        assert_eq!(track.clip_start_frames(0), -3);
        assert_eq!(track.hidden_head().unwrap().audio.channels[0], vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn hidden_head_survives_partial_remove_range() {
        let mut track = clip_before_zero();
        track.remove_range(2, 4).unwrap();
        assert_eq!(track.clip_start_frames(0), -3);
    }

    #[test]
    fn removing_whole_visible_clip_clears_hidden_head() {
        let mut track = clip_before_zero();
        let end = track.end();
        track.remove_range(0, end).unwrap();
        assert!(track.hidden_head().is_none());
        assert!(track.hidden_head.is_none());
    }

    #[test]
    fn apply_range_over_whole_visible_clip_keeps_hidden_head() {
        let mut track = clip_before_zero();
        let end = track.end();
        track.apply_range(0, end, |_| {}).unwrap();
        assert_eq!(track.clip_start_frames(0), -3);
        assert_eq!(track.hidden_head().unwrap().audio.channels[0], vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn insert_into_visible_clip_keeps_hidden_head() {
        let mut track = clip_before_zero();
        track.insert(2, &mono(10, &[10.0])).unwrap();
        assert_eq!(track.clip_start_frames(0), -3);
        assert_eq!(track.hidden_head().unwrap().audio.channels[0], vec![0.0, 1.0, 2.0]);
    }

    #[test]
    fn clip_trim_left_and_right_hide_and_reveal_with_limits() {
        let buf = mono(10, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        let mut clip = Clip::new("c", 10, buf);
        assert_eq!(clip.start, 10);
        assert_eq!(clip.end(), 20);

        clip.trim_left_to(13).unwrap();
        assert_eq!(clip.start, 13);
        assert_eq!(clip.end(), 20);
        assert_eq!(clip.hidden_left_len(), 3);
        assert_eq!(clip.buffer.channels[0], vec![3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        assert_eq!(clip.trim_left.as_ref().unwrap().channels[0], vec![0.0, 1.0, 2.0]);

        clip.trim_left_to(25).unwrap();
        assert_eq!(clip.buffer.len(), 1);
        assert_eq!(clip.buffer.channels[0], vec![9.0]);
        assert_eq!(clip.hidden_left_len(), 9);
        assert_eq!(clip.start, 19);

        clip.trim_left_to(10).unwrap();
        assert_eq!(clip.start, 10);
        assert_eq!(clip.end(), 20);
        assert_eq!(clip.hidden_left_len(), 0);
        assert!(clip.trim_left.is_none());
        assert_eq!(clip.buffer.channels[0], vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);

        clip.trim_left_to(5).unwrap();
        assert_eq!(clip.start, 10);
        assert_eq!(clip.buffer.len(), 10);

        clip.trim_right_to(17).unwrap();
        assert_eq!(clip.start, 10);
        assert_eq!(clip.end(), 17);
        assert_eq!(clip.hidden_right_len(), 3);
        assert_eq!(clip.buffer.channels[0], vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(clip.trim_right.as_ref().unwrap().channels[0], vec![7.0, 8.0, 9.0]);

        clip.trim_right_to(5).unwrap();
        assert_eq!(clip.buffer.len(), 1);
        assert_eq!(clip.buffer.channels[0], vec![0.0]);
        assert_eq!(clip.hidden_right_len(), 9);
        assert_eq!(clip.end(), 11);

        clip.trim_right_to(20).unwrap();
        assert_eq!(clip.end(), 20);
        assert_eq!(clip.hidden_right_len(), 0);
        assert!(clip.trim_right.is_none());
        assert_eq!(clip.buffer.channels[0], vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
    }

    #[test]
    fn split_keeps_hidden_audio_and_trim_restores_original() {
        let buf = mono(10, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        let clip = Clip::new("c", 0, buf.clone());
        let (mut left, mut right) = clip.split_at(4).unwrap();
        assert_eq!(left.buffer.channels[0], vec![0.0, 1.0, 2.0, 3.0]);
        assert_eq!(right.buffer.channels[0], vec![4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);

        left.trim_right_to(10).unwrap();
        assert_eq!(left.buffer.channels[0], buf.channels[0]);
        assert_eq!(left.end(), 10);

        right.trim_left_to(0).unwrap();
        assert_eq!(right.buffer.channels[0], buf.channels[0]);
        assert_eq!(right.start, 0);
    }

    #[test]
    fn editor_trim_hides_outside_selection_and_dragging_edges_restores() {
        let buf = mono(10, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        let track = crate::Track::new("t", buf.clone());
        let mut editor = crate::Editor::new(crate::ProjectState {
            tracks: vec![track],
            sample_rate: 10,
            ..Default::default()
        });

        assert!(editor.trim(crate::Selection { track: 0, start: 5, end: 5 }).is_err());

        editor.trim(crate::Selection { track: 0, start: 2, end: 7 }).unwrap();
        {
            let t = &editor.state().tracks[0];
            assert_eq!(t.clips.clips.len(), 1);
            let c = &t.clips.clips[0];
            assert_eq!(c.start, 2);
            assert_eq!(c.end(), 7);
            assert_eq!(c.buffer.channels[0], vec![2.0, 3.0, 4.0, 5.0, 6.0]);
            assert_eq!(c.hidden_left_len(), 2);
            assert_eq!(c.hidden_right_len(), 3);
        }

        editor.trim_clip_edge(0, 0, true, 0).unwrap();
        editor.trim_clip_edge(0, 0, false, 10).unwrap();
        {
            let t = &editor.state().tracks[0];
            let c = &t.clips.clips[0];
            assert_eq!(c.start, 0);
            assert_eq!(c.end(), 10);
            assert_eq!(c.buffer.channels[0], buf.channels[0]);
            assert_eq!(c.hidden_left_len(), 0);
            assert_eq!(c.hidden_right_len(), 0);
        }
    }

    #[test]
    fn stretched_clip_moves_before_zero() {
        let buf = mono(100, &vec![1.0; 200]);
        let mut track = WaveClips::from_buffer("c", buf);
        track.stretch_clip(0, 2.0).unwrap();
        assert_eq!(track.clips[0].len(), 400);

        track.move_clip(0, -100).unwrap();
        let start = track.clip_start_frames(0);
        assert!((start - (-100)).abs() <= 1, "expected -100 ±1, got {start}");
        assert_eq!(track.clips[0].start, 0);
        assert!(track.hidden_head().is_some());
    }
}
