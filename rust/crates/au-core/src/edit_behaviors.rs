//! Audacity-style editing behavior preferences and editing operations.

use crate::{
    frames_to_secs, AudioBuffer, CoreError, ProjectState, Result, Selection,
};
use crate::timeline::{Clip, CutLine, LabelTrack, WaveClips};

/// Preferences controlling cut, delete, paste, and label editing behavior.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditBehavior {
    /// Whether cuts inside a single clip leave expandable cut lines.
    pub enable_cut_lines: bool,
    /// Whether clips after an edit move to make room or close a gap.
    pub edit_clips_can_move: bool,
    /// Whether pasted audio is kept as a separate clip.
    pub paste_as_new_clips: bool,
    /// Whether labels touching selection boundaries are retained.
    pub retain_labels: bool,
}

impl Default for EditBehavior {
    fn default() -> Self {
        Self {
            enable_cut_lines: false,
            edit_clips_can_move: true,
            paste_as_new_clips: false,
            retain_labels: false,
        }
    }
}

/// The relation of an edit region to a label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelRelation {
    /// The region is before the label.
    BeforeLabel,
    /// The region is after the label.
    AfterLabel,
    /// The region surrounds the label.
    SurroundsLabel,
    /// The region is within the label.
    WithinLabel,
    /// The region begins within the label.
    BeginsInLabel,
    /// The region ends within the label.
    EndsInLabel,
}

/// Classifies the relation between a label and an edit region.
///
/// The arguments are label start and end times, followed by region start and
/// end times. The branch ordering and boundary comparisons match Audacity's
/// `LabelStruct::RegionRelation`.
pub fn label_region_relation(
    label_start: f64,
    label_end: f64,
    reg_t0: f64,
    reg_t1: f64,
    retain: bool,
) -> LabelRelation {
    if retain {
        if reg_t0 < label_start && reg_t1 > label_end {
            LabelRelation::SurroundsLabel
        } else if reg_t1 < label_start {
            LabelRelation::BeforeLabel
        } else if reg_t0 > label_end {
            LabelRelation::AfterLabel
        } else if reg_t0 >= label_start
            && reg_t0 <= label_end
            && reg_t1 >= label_start
            && reg_t1 <= label_end
        {
            LabelRelation::WithinLabel
        } else if reg_t0 >= label_start && reg_t0 <= label_end {
            LabelRelation::BeginsInLabel
        } else {
            LabelRelation::EndsInLabel
        }
    } else if reg_t0 <= label_start && reg_t1 >= label_end {
        LabelRelation::SurroundsLabel
    } else if reg_t1 <= label_start {
        LabelRelation::BeforeLabel
    } else if reg_t0 >= label_end {
        LabelRelation::AfterLabel
    } else if reg_t0 > label_start
        && reg_t0 < label_end
        && reg_t1 > label_start
        && reg_t1 < label_end
    {
        LabelRelation::WithinLabel
    } else if reg_t0 > label_start && reg_t0 < label_end {
        LabelRelation::BeginsInLabel
    } else {
        LabelRelation::EndsInLabel
    }
}

/// Clears a time range from a label track using Audacity's label-retention
/// rules.
pub fn clear_labels(lt: &mut LabelTrack, b: f64, e: f64, retain: bool) {
    let duration = e - b;
    let mut index = 0;
    while index < lt.labels.len() {
        let relation = {
            let label = &lt.labels[index];
            label_region_relation(label.start, label.end, b, e, retain)
        };

        match relation {
            LabelRelation::BeforeLabel => {
                lt.labels[index].start -= duration;
                lt.labels[index].end -= duration;
            }
            LabelRelation::SurroundsLabel => {
                lt.labels.remove(index);
                continue;
            }
            LabelRelation::EndsInLabel => {
                lt.labels[index].start = b;
                lt.labels[index].end -= duration;
            }
            LabelRelation::BeginsInLabel => {
                lt.labels[index].end = b;
            }
            LabelRelation::WithinLabel => {
                lt.labels[index].end -= duration;
            }
            LabelRelation::AfterLabel => {}
        }
        index += 1;
    }
    lt.labels
        .sort_by(|left, right| left.start.total_cmp(&right.start));
}

impl crate::Editor {
    /// Cuts the selected audio, placing it on the clipboard.
    ///
    /// When cut lines are enabled, a cut wholly inside one clip leaves an
    /// expandable cut line. A single undo entry is recorded.
    pub fn cut_with(&mut self, sel: Selection, b: &EditBehavior) -> Result<()> {
        let sel = sel.normalized();
        let mut state = self.state().clone();
        let track = state
            .tracks
            .get(sel.track)
            .ok_or(CoreError::TrackNotFound(sel.track))?;

        let removed = render_pre_envelope_range(&track.clips, sel.start, sel.end)?;
        let move_clips = state.sync_lock || b.edit_clips_can_move;
        let add_cut_line = b.enable_cut_lines
            && sel.start < sel.end
            && contains_range_in_one_clip(&track.clips, sel.start, sel.end);

        clear_wave_range(
            &mut state.tracks[sel.track].clips,
            sel.start,
            sel.end,
            move_clips,
            add_cut_line.then(|| removed.clone()),
        )?;

        ripple_remove_with_labels(&mut state, sel, b.retain_labels);
        self.history.push("Cut", state);
        self.clipboard = Some(removed);
        Ok(())
    }

    /// Deletes the selected audio without changing the clipboard.
    ///
    /// A single undo entry is recorded.
    pub fn delete_with(&mut self, sel: Selection, b: &EditBehavior) -> Result<()> {
        let sel = sel.normalized();
        let mut state = self.state().clone();
        let track = state
            .tracks
            .get(sel.track)
            .ok_or(CoreError::TrackNotFound(sel.track))?;

        // Validate the range before editing, just as for cut.
        let _ = render_pre_envelope_range(&track.clips, sel.start, sel.end)?;
        let move_clips = state.sync_lock || b.edit_clips_can_move;
        clear_wave_range(
            &mut state.tracks[sel.track].clips,
            sel.start,
            sel.end,
            move_clips,
            None,
        )?;

        ripple_remove_with_labels(&mut state, sel, b.retain_labels);
        self.history.push("Delete", state);
        Ok(())
    }

    /// Pastes the clipboard at a frame position.
    ///
    /// Returns successfully without an undo entry if the clipboard is empty.
    /// A paste that would overlap existing clips while clips cannot move fails
    /// with [`CoreError::NoRoom`] and leaves the project unchanged.
    pub fn paste_with(
        &mut self,
        track: usize,
        at: usize,
        b: &EditBehavior,
    ) -> Result<()> {
        let Some(clipboard) = self.clipboard.as_ref() else {
            return Ok(());
        };
        let clipboard = clipboard.clone();
        let mut state = self.state().clone();
        let target = state
            .tracks
            .get_mut(track)
            .ok_or(CoreError::TrackNotFound(track))?;
        let move_clips = state.sync_lock || b.edit_clips_can_move;

        paste_wave(
            &mut target.clips,
            at,
            &clipboard,
            move_clips,
            b.paste_as_new_clips,
        )?;

        let len = clipboard.len();
        ripple_insert_with_state(&mut state, track, at, len);
        self.history.push("Paste", state);
        Ok(())
    }

    /// Expands the cut line at `at`, reinserting its audio.
    ///
    /// Returns `false` and records no undo entry when there is no matching cut
    /// line. A single undo entry is recorded on success.
    pub fn expand_cut_line(&mut self, track: usize, at: usize) -> Result<bool> {
        let mut state = self.state().clone();
        let clips = &mut state
            .tracks
            .get_mut(track)
            .ok_or(CoreError::TrackNotFound(track))?
            .clips;

        let Some((line_index, line)) = find_cut_line(clips, at) else {
            return Ok(false);
        };
        let audio = line.audio.clone();
        let move_clips = state.sync_lock || true;

        let mut updated = clips.clone();
        updated.cut_lines.remove(line_index);

        if move_clips {
            updated.insert(at, &audio)?;
            for cut_line in &mut updated.cut_lines {
                if cut_line.at >= at {
                    cut_line.at = cut_line.at.saturating_add(audio.len());
                }
            }
        } else {
            insert_without_moving(&mut updated, at, &audio)?;
        }

        *clips = updated;
        self.history.push("Expand", state);
        Ok(true)
    }

    /// Removes the cut line at `at` without reinserting its audio.
    ///
    /// Returns `false` and records no undo entry when there is no matching cut
    /// line. A single undo entry is recorded on success.
    pub fn remove_cut_line(&mut self, track: usize, at: usize) -> Result<bool> {
        let mut state = self.state().clone();
        let clips = &mut state
            .tracks
            .get_mut(track)
            .ok_or(CoreError::TrackNotFound(track))?
            .clips;

        let Some((line_index, _)) = find_cut_line(clips, at) else {
            return Ok(false);
        };
        clips.cut_lines.remove(line_index);
        self.history.push("Remove cut line", state);
        Ok(true)
    }
}

fn find_cut_line(clips: &WaveClips, at: usize) -> Option<(usize, CutLine)> {
    clips
        .cut_lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.at.abs_diff(at) == 0)
        .map(|(index, line)| (index, line.clone()))
}

fn contains_range_in_one_clip(clips: &WaveClips, start: usize, end: usize) -> bool {
    clips
        .clips
        .iter()
        .filter(|clip| clip.start <= start && clip.end() >= end)
        .count()
        == 1
}

fn render_pre_envelope_range(
    clips: &WaveClips,
    start: usize,
    end: usize,
) -> Result<AudioBuffer> {
    if start > end {
        return Err(CoreError::InvalidRange(start, end));
    }

    let mut output = AudioBuffer::from_channels(
        clips.sample_rate,
        (0..clips.n_channels)
            .map(|_| vec![0.0; end - start])
            .collect(),
    );
    for clip in &clips.clips {
        let rendered = clip.rendered();
        let overlap_start = start.max(clip.start);
        let overlap_end = end.min(clip.end());
        for frame in overlap_start..overlap_end {
            let source_frame = frame - clip.start;
            let dest_frame = frame - start;
            for channel in 0..output.n_channels() {
                if let (Some(source), Some(dest)) = (
                    rendered.channels.get(channel).and_then(|c| c.get(source_frame)),
                    output.channels.get_mut(channel).and_then(|c| c.get_mut(dest_frame)),
                ) {
                    *dest = *source;
                }
            }
        }
    }
    Ok(output)
}

fn clear_wave_range(
    clips: &mut WaveClips,
    start: usize,
    end: usize,
    move_clips: bool,
    cut_audio: Option<AudioBuffer>,
) -> Result<AudioBuffer> {
    if start > end {
        return Err(CoreError::InvalidRange(start, end));
    }

    let removed = render_pre_envelope_range(clips, start, end)?;
    if start == end {
        return Ok(removed);
    }

    let amount = end - start;
    let mut cut_lines = Vec::with_capacity(clips.cut_lines.len() + usize::from(cut_audio.is_some()));
    for mut line in clips.cut_lines.drain(..) {
        // Audacity removes cut lines at both selection boundaries too.
        if line.at >= start && line.at <= end {
            continue;
        }
        if move_clips && line.at > end {
            line.at = line.at.saturating_sub(amount);
        }
        cut_lines.push(line);
    }

    if move_clips {
        clips.remove_range(start, end)?;
    } else {
        let clear_hidden_head = start == 0
            && clips.clips.first().is_some_and(|clip| clip.start == 0 && end >= clip.end());
        let mut updated = Vec::with_capacity(clips.clips.len());
        for clip in clips.clips.drain(..) {
            let clip_end = clip.end();
            if clip_end <= start || clip.start >= end {
                updated.push(clip);
                continue;
            }

            let rendered = clip.rendered();
            let left_end = start.saturating_sub(clip.start).min(rendered.len());
            let right_start = end.saturating_sub(clip.start).min(rendered.len());
            let mut channels = Vec::with_capacity(rendered.n_channels());
            for channel in &rendered.channels {
                let mut samples = Vec::with_capacity(
                    left_end.saturating_add(channel.len().saturating_sub(right_start)),
                );
                samples.extend_from_slice(&channel[..left_end.min(channel.len())]);
                if right_start < channel.len() {
                    samples.extend_from_slice(&channel[right_start..]);
                }
                channels.push(samples);
            }

            if channels.first().is_some_and(|channel| !channel.is_empty()) {
                updated.push(Clip::new(
                    clip.name,
                    clip.start,
                    AudioBuffer::from_channels(rendered.sample_rate, channels),
                ));
            }
        }
        clips.clips = updated;
        if clear_hidden_head {
            clips.hidden_head = None;
        }
        clips.clips.sort_by_key(|clip| clip.start);

        let duration = amount as f64 / f64::from(clips.sample_rate.max(1));
        let begin = start as f64 / f64::from(clips.sample_rate.max(1));
        let finish = end as f64 / f64::from(clips.sample_rate.max(1));
        clips
            .envelope
            .points
            .retain(|point| point.time < begin || point.time >= finish);
        let _ = duration;
    }

    if let Some(audio) = cut_audio {
        cut_lines.push(CutLine { at: start, audio });
    }
    cut_lines.sort_by_key(|line| line.at);
    clips.cut_lines = cut_lines;
    Ok(removed)
}

fn ripple_remove_with_labels(
    state: &mut ProjectState,
    sel: Selection,
    retain_labels: bool,
) {
    if !state.sync_lock {
        return;
    }

    let rate = state.tracks[sel.track].sample_rate();
    let (a, b) = (
        frames_to_secs(sel.start, rate),
        frames_to_secs(sel.end, rate),
    );

    for (index, track) in state.tracks.iter_mut().enumerate() {
        if index != sel.track {
            let end = sel.end.min(track.len().max(sel.start));
            if sel.start < end {
                let _ = track.clips.remove_range(sel.start, end);
            }
        }
    }
    for labels in &mut state.labels {
        clear_labels(labels, a, b, retain_labels);
    }
}

fn ripple_insert_with_state(state: &mut ProjectState, track: usize, at: usize, len: usize) {
    if !state.sync_lock || len == 0 {
        return;
    }

    let rate = state.tracks[track].sample_rate();
    for (index, other) in state.tracks.iter_mut().enumerate() {
        if index != track && at < other.len() {
            let silence = AudioBuffer::from_channels(
                other.sample_rate(),
                (0..other.clips.n_channels)
                    .map(|_| vec![0.0; len])
                    .collect(),
            );
            let _ = other.clips.insert(at, &silence);
        }
    }
    for labels in &mut state.labels {
        labels.insert_space(
            frames_to_secs(at, rate),
            frames_to_secs(len, rate),
        );
    }
}

fn paste_wave(
    clips: &mut WaveClips,
    at: usize,
    audio: &AudioBuffer,
    move_clips: bool,
    paste_as_new_clip: bool,
) -> Result<()> {
    if audio.sample_rate != clips.sample_rate {
        return Err(CoreError::RateMismatch);
    }
    if audio.n_channels() != clips.n_channels {
        return Err(CoreError::ChannelMismatch);
    }
    let len = audio.len();
    if len == 0 {
        return Ok(());
    }

    if move_clips {
        if paste_as_new_clip {
            clips.split(at)?;
            for clip in &mut clips.clips {
                if clip.start >= at {
                    clip.start = clip.start.saturating_add(len);
                }
            }
            clips
                .clips
                .push(Clip::new("Pasted", at, audio.clone()));
            clips.clips.sort_by_key(|clip| clip.start);
            for line in &mut clips.cut_lines {
                if line.at >= at {
                    line.at = line.at.saturating_add(len);
                }
            }
            return Ok(());
        }

        // Audacity's single-clip merge path applies only when the paste point
        // splits a clip, not when it is at its beginning or end.
        if let Some(index) = clips
            .clips
            .iter()
            .position(|clip| at > clip.start && at < clip.end())
        {
            for (other_index, clip) in clips.clips.iter_mut().enumerate() {
                if other_index != index && clip.start >= at {
                    clip.start = clip.start.saturating_add(len);
                }
            }
            let clip = &mut clips.clips[index];
            let mut rendered = clip.rendered();
            rendered.insert(at - clip.start, audio)?;
            clip.buffer = rendered;
            clip.stretch_ratio = 1.0;
        } else {
            for clip in &mut clips.clips {
                if clip.start >= at {
                    clip.start = clip.start.saturating_add(len);
                }
            }
            clips
                .clips
                .push(Clip::new("Pasted", at, audio.clone()));
        }
        for line in &mut clips.cut_lines {
            if line.at >= at {
                line.at = line.at.saturating_add(len);
            }
        }
        clips.clips.sort_by_key(|clip| clip.start);
        return Ok(());
    }

    if paste_as_new_clip {
        let end = at.saturating_add(len);
        if clips
            .clips
            .iter()
            .any(|clip| at < clip.end() && end > clip.start)
        {
            return Err(CoreError::NoRoom);
        }
        clips
            .clips
            .push(Clip::new("Pasted", at, audio.clone()));
        clips.clips.sort_by_key(|clip| clip.start);
        return Ok(());
    }

    if let Some(index) = clips
        .clips
        .iter()
        .position(|clip| clip.start <= at && at < clip.end())
    {
        let old_end = clips.clips[index].end();
        let new_end = old_end.saturating_add(len);
        if clips.clips.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && other.start >= clips.clips[index].start
                && other.start < new_end
                && other.end() > old_end
        }) {
            return Err(CoreError::NoRoom);
        }
        let clip = &mut clips.clips[index];
        let mut rendered = clip.rendered();
        rendered.insert(at - clip.start, audio)?;
        clip.buffer = rendered;
        clip.stretch_ratio = 1.0;
    } else if let Some(index) = clips
        .clips
        .iter()
        .position(|clip| clip.end() == at)
    {
        let new_end = at.saturating_add(len);
        if clips.clips.iter().enumerate().any(|(other_index, other)| {
            other_index != index && other.start >= at && other.start < new_end
        }) {
            return Err(CoreError::NoRoom);
        }
        let clip = &mut clips.clips[index];
        let mut rendered = clip.rendered();
        rendered.insert(rendered.len(), audio)?;
        clip.buffer = rendered;
        clip.stretch_ratio = 1.0;
    } else {
        let end = at.saturating_add(len);
        if clips
            .clips
            .iter()
            .any(|clip| at < clip.end() && end > clip.start)
        {
            return Err(CoreError::NoRoom);
        }
        clips
            .clips
            .push(Clip::new("Pasted", at, audio.clone()));
    }
    clips.clips.sort_by_key(|clip| clip.start);
    Ok(())
}

fn insert_without_moving(
    clips: &mut WaveClips,
    at: usize,
    audio: &AudioBuffer,
) -> Result<()> {
    if audio.sample_rate != clips.sample_rate {
        return Err(CoreError::RateMismatch);
    }
    if audio.n_channels() != clips.n_channels {
        return Err(CoreError::ChannelMismatch);
    }
    let len = audio.len();
    if len == 0 {
        return Ok(());
    }

    let target = clips
        .clips
        .iter()
        .position(|clip| clip.start <= at && at <= clip.end());
    if let Some(index) = target {
        let clip_start = clips.clips[index].start;
        let old_end = clips.clips[index].end();
        let new_end = old_end.saturating_add(len);
        if clips.clips.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && other.start > clip_start
                && other.start < new_end
        }) {
            return Err(CoreError::NoRoom);
        }
        let clip = &mut clips.clips[index];
        let mut rendered = clip.rendered();
        rendered.insert(at - clip.start, audio)?;
        clip.buffer = rendered;
        clip.stretch_ratio = 1.0;
    } else {
        let end = at.saturating_add(len);
        if clips
            .clips
            .iter()
            .any(|clip| at < clip.end() && end > clip.start)
        {
            return Err(CoreError::NoRoom);
        }
        clips
            .clips
            .push(Clip::new("Expanded", at, audio.clone()));
    }
    clips.clips.sort_by_key(|clip| clip.start);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProjectState, Track};

    fn mono(samples: &[f32]) -> AudioBuffer {
        AudioBuffer::from_channels(10, vec![samples.to_vec()])
    }

    fn editor_with_clips(clips: WaveClips) -> crate::Editor {
        crate::Editor::new(ProjectState {
            tracks: vec![Track {
                name: "track".into(),
                clips,
                gain: 1.0,
                pan: 0.0,
                mute: false,
                solo: false,
                int_bits: None,
            }],
            sample_rate: 10,
            ..Default::default()
        })
    }

    #[test]
    fn cut_line_is_created_expanded_and_removed() {
        let mut editor = editor_with_clips(WaveClips::from_buffer("clip", mono(&[1.0, 2.0, 3.0, 4.0, 5.0])));
        let behavior = EditBehavior {
            enable_cut_lines: true,
            ..Default::default()
        };

        editor
            .cut_with(
                Selection {
                    track: 0,
                    start: 1,
                    end: 3,
                },
                &behavior,
            )
            .unwrap();
        assert_eq!(editor.state().tracks[0].clips.cut_lines.len(), 1);
        assert_eq!(editor.state().tracks[0].render().channels[0], vec![1.0, 4.0, 5.0]);

        assert!(editor.expand_cut_line(0, 1).unwrap());
        assert_eq!(editor.state().tracks[0].clips.cut_lines.len(), 0);
        assert_eq!(editor.state().tracks[0].render().channels[0], vec![1.0, 2.0, 3.0, 4.0, 5.0]);

        editor
            .cut_with(
                Selection {
                    track: 0,
                    start: 1,
                    end: 3,
                },
                &behavior,
            )
            .unwrap();
        assert!(editor.remove_cut_line(0, 1).unwrap());
        assert!(editor.state().tracks[0].clips.cut_lines.is_empty());
    }

    #[test]
    fn no_move_clear_keeps_later_clip_position() {
        let mut clips = WaveClips::new(10, 1);
        clips.clips.push(Clip::new("a", 0, mono(&[1.0, 2.0, 3.0, 4.0])));
        clips.clips.push(Clip::new("b", 8, mono(&[8.0, 9.0])));
        let mut editor = editor_with_clips(clips);
        let behavior = EditBehavior {
            edit_clips_can_move: false,
            ..Default::default()
        };

        editor
            .delete_with(
                Selection {
                    track: 0,
                    start: 1,
                    end: 3,
                },
                &behavior,
            )
            .unwrap();
        assert_eq!(editor.state().tracks[0].clips.clips[1].start, 8);
        assert_eq!(editor.state().tracks[0].clips.clips[0].buffer.channels[0], vec![1.0, 4.0]);
    }

    #[test]
    fn no_move_paste_fails_without_changing_project() {
        let mut clips = WaveClips::new(10, 1);
        clips.clips.push(Clip::new("a", 0, mono(&[1.0, 2.0])));
        clips.clips.push(Clip::new("b", 3, mono(&[3.0, 4.0])));
        let mut editor = editor_with_clips(clips);
        editor.clipboard = Some(mono(&[9.0, 8.0]));
        let before = editor.state().clone();
        let behavior = EditBehavior {
            edit_clips_can_move: false,
            ..Default::default()
        };

        assert_eq!(editor.paste_with(0, 1, &behavior), Err(CoreError::NoRoom));
        assert_eq!(editor.state(), &before);
    }

    #[test]
    fn paste_as_new_clip_keeps_clip_separate() {
        let mut editor = editor_with_clips(WaveClips::from_buffer("clip", mono(&[1.0, 2.0, 3.0, 4.0])));
        editor.clipboard = Some(mono(&[9.0, 8.0]));
        let behavior = EditBehavior {
            paste_as_new_clips: true,
            ..Default::default()
        };

        editor.paste_with(0, 2, &behavior).unwrap();
        assert_eq!(editor.state().tracks[0].clips.clips.len(), 3);
        assert_eq!(editor.state().tracks[0].clips.clips[1].name, "Pasted");
    }

    #[test]
    fn label_relation_branches_match_both_retention_modes() {
        let label = (2.0, 4.0);
        let cases = [
            ((0.0, 1.0), LabelRelation::BeforeLabel, LabelRelation::BeforeLabel),
            ((5.0, 6.0), LabelRelation::AfterLabel, LabelRelation::AfterLabel),
            ((1.0, 5.0), LabelRelation::SurroundsLabel, LabelRelation::SurroundsLabel),
            ((2.5, 3.5), LabelRelation::WithinLabel, LabelRelation::WithinLabel),
            ((3.0, 5.0), LabelRelation::BeginsInLabel, LabelRelation::BeginsInLabel),
            ((1.0, 3.0), LabelRelation::EndsInLabel, LabelRelation::EndsInLabel),
            // Boundary cases differ between retain modes.
            ((2.0, 4.0), LabelRelation::WithinLabel, LabelRelation::SurroundsLabel),
            ((1.0, 2.0), LabelRelation::EndsInLabel, LabelRelation::BeforeLabel),
            ((4.0, 5.0), LabelRelation::BeginsInLabel, LabelRelation::AfterLabel),
        ];

        for ((region_start, region_end), retain_relation, discard_relation) in cases {
            assert_eq!(
                label_region_relation(label.0, label.1, region_start, region_end, true),
                retain_relation
            );
            assert_eq!(
                label_region_relation(label.0, label.1, region_start, region_end, false),
                discard_relation
            );
        }
    }

    #[test]
    fn clear_labels_applies_relation_edits() {
        let mut labels = LabelTrack::new("Labels");
        labels.add(0.0, 0.0, "before");
        labels.add(4.0, 5.0, "surrounded");
        labels.add(3.0, 7.0, "ends");
        labels.add(5.0, 8.0, "after");

        clear_labels(&mut labels, 3.0, 6.0, false);
        assert!(labels.labels.iter().all(|label| label.text != "surrounded"));
        let ends = labels.labels.iter().find(|label| label.text == "ends").unwrap();
        assert_eq!((ends.start, ends.end), (3.0, 4.0));
    }

    #[test]
    fn cut_delete_and_paste_record_expected_undo_descriptions() {
        let mut editor = editor_with_clips(WaveClips::from_buffer("clip", mono(&[1.0, 2.0, 3.0])));
        editor
            .delete_with(
                Selection {
                    track: 0,
                    start: 0,
                    end: 1,
                },
                &EditBehavior::default(),
            )
            .unwrap();
        assert_eq!(editor.history.undo_description(), Some("Delete"));

        editor.clipboard = Some(mono(&[7.0]));
        editor.paste_with(0, 0, &EditBehavior::default()).unwrap();
        assert_eq!(editor.history.undo_description(), Some("Paste"));
    }
}

#[cfg(test)]
mod paste_regression {
    use super::*;
    use crate::{AudioBuffer, Editor, ProjectState, Selection, Track};

    #[test]
    fn paste_at_clip_start_without_moving_grows_the_clip() {
        let mut state = ProjectState::default();
        state.sample_rate = 100;
        state.tracks.push(Track::new("a", AudioBuffer::from_channels(100, vec![vec![0.1; 150]])));
        let mut editor = Editor::new(state);
        editor.copy(Selection { track: 0, start: 0, end: 150 }).unwrap();
        editor.paste_with(0, 0, &EditBehavior::default()).unwrap();
        assert_eq!(editor.state().tracks[0].len(), 300);
    }
}
