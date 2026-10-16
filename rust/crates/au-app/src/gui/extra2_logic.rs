//! Extra-menu items 68-89 of docs/SPLIT_extra.md (Cursor, Track, Scriptables).
//! Utilities porting the specified Audacity 3.7.3 track-control behavior.

/// Returns the collapsed cursor time after a jump with audio inactive.
///
/// A point cursor moves by `step` and is clamped to zero; a range collapses to
/// its left or right endpoint depending on the direction of the step.
pub fn cursor_jump(
    t0: f64,
    t1: f64,
    step: f64,
    snap: Option<&dyn Fn(f64) -> f64>,
) -> f64 {
    if t0 == t1 {
        0.0_f64.max(t0 + step)
    } else if step < 0.0 {
        snap.map(|f| f(t0)).unwrap_or(t0)
    } else {
        snap.map(|f| f(t1)).unwrap_or(t1)
    }
}

/// Moves a pan value by slider steps, clamping the input and output to `[-1, 1]`.
pub fn pan_step(pan: f32, steps: f32) -> f32 {
    (pan.clamp(-1.0, 1.0) + steps * 0.1).clamp(-1.0, 1.0)
}

/// Moves a linear gain value by decibel slider steps.
///
/// The gain is converted to decibels, clamped to `[-36, 36]`, adjusted by
/// `steps`, clamped again, and converted back to linear gain.
pub fn gain_step(gain: f32, steps: f32) -> f32 {
    let mut db = if gain <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * gain.log10()
    };
    db = db.clamp(-36.0, 36.0);
    db = (db + steps).clamp(-36.0, 36.0);
    10.0_f32.powf(db / 20.0)
}

/// Toggles a track's mute state and, in simple mode, updates solo states.
pub fn track_mute(mutes: &mut [bool], solos: &mut [bool], idx: usize, simple: bool) {
    if idx >= mutes.len() {
        return;
    }

    mutes[idx] = !mutes[idx];

    if simple {
        let n = mutes.len();
        let playing = mutes.iter().filter(|&&mute| !mute).count();
        for (i, solo) in solos.iter_mut().enumerate() {
            *solo = playing == 1 && n > 1 && !mutes[i];
        }
    }
}

/// Toggles a track's solo state, applying simple or exclusive solo behavior.
pub fn track_solo(
    mutes: &mut [bool],
    solos: &mut [bool],
    idx: usize,
    simple: bool,
    exclusive: bool,
) {
    if idx >= solos.len() || (simple && idx >= mutes.len()) {
        return;
    }

    let was = solos[idx];
    let solo_multiple = !simple ^ exclusive;

    if solo_multiple {
        solos[idx] = !was;
    } else {
        for i in 0..solos.len() {
            if i == idx {
                solos[i] = !was;
                if simple {
                    mutes[i] = false;
                }
            } else {
                solos[i] = false;
                if simple {
                    mutes[i] = !was;
                }
            }
        }
    }
}

/// Describes a track's destination when moving it within a track list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveChoice {
    /// Move the track one position upward.
    Up,
    /// Move the track one position downward.
    Down,
    /// Move the track to the first position.
    Top,
    /// Move the track to the last position.
    Bottom,
}

/// Returns the destination index for a track move, or `None` if the move is
/// invalid or would leave the track in its current position.
pub fn move_track(n: usize, idx: usize, choice: MoveChoice) -> Option<usize> {
    if idx >= n {
        return None;
    }

    let to = match choice {
        MoveChoice::Up if idx > 0 => idx - 1,
        MoveChoice::Down if idx + 1 < n => idx + 1,
        MoveChoice::Top if idx > 0 => 0,
        MoveChoice::Bottom if idx + 1 < n => n - 1,
        _ => return None,
    };
    Some(to)
}

/// Removes the element at `idx` and inserts it at `to`.
///
/// Does nothing if either index is out of range.
pub fn apply_move<T>(v: &mut Vec<T>, idx: usize, to: usize) {
    if idx >= v.len() || to >= v.len() {
        return;
    }
    let item = v.remove(idx);
    v.insert(to, item);
}

/// Returns the focused track index after removing a track.
///
/// Focus moves to the next track when one exists, otherwise to the previous
/// track. The returned index refers to the list after removal.
pub fn focus_after_remove(n_before: usize, removed: usize) -> Option<usize> {
    if n_before == 0 || removed >= n_before || n_before == 1 {
        None
    } else if removed < n_before - 1 {
        Some(removed)
    } else {
        Some(removed - 1)
    }
}

/// Counts sample pairs whose absolute difference exceeds `threshold`.
///
/// Samples beyond either channel's length are treated as zero.
pub fn compare_audio(
    a: &[Vec<f32>],
    b: &[Vec<f32>],
    start: usize,
    end: usize,
    threshold: f32,
) -> usize {
    let mut count = 0;
    for (channel_a, channel_b) in a.iter().zip(b.iter()) {
        for frame in start..end {
            let x = channel_a.get(frame).copied().unwrap_or(0.0);
            let y = channel_b.get(frame).copied().unwrap_or(0.0);
            // CompareSample: fabs on doubles, compared with the float threshold promoted to double.
            if (f64::from(x) - f64::from(y)).abs() > f64::from(threshold) {
                count += 1;
            }
        }
    }
    count
}

/// Formats a comparison count, duration, and threshold into the result message.
pub fn compare_audio_message(count: usize, rate: u32, threshold: f32) -> String {
    let secs = count as f64 / rate as f64;
    format!(
        "{count}\n{secs:.4}\nFinished comparison: {count} samples ({secs:.3} seconds) exceeded the error threshold of {threshold:.6}."
    )
}


/// Index that the element originally at `i` has after moving `from` to `to`
/// (copied from BizyBot's extra_logic.rs).
pub fn moved_index(i: usize, from: usize, to: usize) -> usize {
    if i == from {
        to
    } else if from < to && i > from && i <= to {
        i - 1
    } else if to < from && i >= to && i < from {
        i + 1
    } else {
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_jump() {
        assert_eq!(cursor_jump(1.0, 1.0, -2.0, None), 0.0);
        assert_eq!(cursor_jump(1.0, 1.0, 2.0, None), 3.0);
        let snap = |t: f64| t + 0.5;
        assert_eq!(cursor_jump(1.0, 3.0, -1.0, Some(&snap)), 1.5);
        assert_eq!(cursor_jump(1.0, 3.0, 1.0, Some(&snap)), 3.5);
    }

    #[test]
    fn test_pan_step() {
        assert!((pan_step(0.0, 2.0) - 0.2).abs() < 1e-6);
        assert_eq!(pan_step(2.0, 0.0), 1.0);
        assert_eq!(pan_step(-1.0, -1.0), -1.0);
    }

    #[test]
    fn test_gain_step() {
        assert!((gain_step(1.0, 6.0) - 10.0_f32.powf(6.0 / 20.0)).abs() < 1e-6);
        assert!((gain_step(0.0, 0.0) - 10.0_f32.powf(-36.0 / 20.0)).abs() < 1e-6);
        assert!((gain_step(100.0, 0.0) - 10.0_f32.powf(36.0 / 20.0)).abs() < 1e-5);
    }

    #[test]
    fn test_track_mute() {
        let mut mutes = vec![false, true, true];
        let mut solos = vec![false; 3];
        track_mute(&mut mutes, &mut solos, 0, true);
        assert_eq!(mutes, vec![true, true, true]);
        assert_eq!(solos, vec![false; 3]);

        let before = mutes.clone();
        track_mute(&mut mutes, &mut solos, 9, true);
        assert_eq!(mutes, before);
    }

    #[test]
    fn test_track_solo() {
        let mut mutes = vec![false, false, false];
        let mut solos = vec![false, false, false];
        track_solo(&mut mutes, &mut solos, 1, true, false);
        assert_eq!(solos, vec![false, true, false]);
        assert_eq!(mutes, vec![true, false, true]);

        track_solo(&mut mutes, &mut solos, 1, false, true);
        assert_eq!(solos, vec![false, false, false]);

        let before = solos.clone();
        track_solo(&mut mutes, &mut solos, 8, false, true);
        assert_eq!(solos, before);
    }

    #[test]
    fn test_move_track() {
        assert_eq!(move_track(4, 2, MoveChoice::Up), Some(1));
        assert_eq!(move_track(4, 2, MoveChoice::Down), Some(3));
        assert_eq!(move_track(4, 2, MoveChoice::Top), Some(0));
        assert_eq!(move_track(4, 1, MoveChoice::Bottom), Some(3));
        assert_eq!(move_track(4, 0, MoveChoice::Up), None);
        assert_eq!(move_track(4, 3, MoveChoice::Down), None);
        assert_eq!(move_track(4, 0, MoveChoice::Top), None);
        assert_eq!(move_track(4, 3, MoveChoice::Bottom), None);
        assert_eq!(move_track(0, 0, MoveChoice::Up), None);
    }

    #[test]
    fn test_apply_move() {
        let mut v = vec![1, 2, 3, 4];
        apply_move(&mut v, 1, 3);
        assert_eq!(v, vec![1, 3, 4, 2]);
        apply_move(&mut v, 9, 0);
        assert_eq!(v, vec![1, 3, 4, 2]);
        apply_move(&mut v, 0, 9);
        assert_eq!(v, vec![1, 3, 4, 2]);
    }

    #[test]
    fn test_focus_after_remove() {
        assert_eq!(focus_after_remove(1, 0), None);
        assert_eq!(focus_after_remove(4, 1), Some(1));
        assert_eq!(focus_after_remove(4, 3), Some(2));
        assert_eq!(focus_after_remove(0, 0), None);
        assert_eq!(focus_after_remove(3, 3), None);
    }

    #[test]
    fn test_compare_audio() {
        let a = vec![vec![1.0, 0.0], vec![0.0, 2.0]];
        let b = vec![vec![0.0], vec![0.0, 2.0]];
        assert_eq!(compare_audio(&a, &b, 0, 3, 0.5), 1);
        assert_eq!(compare_audio(&a, &b, 2, 1, 0.0), 0);
    }

    #[test]
    fn test_compare_audio_message() {
        assert_eq!(
            compare_audio_message(48000, 48000, 0.001),
            "48000\n1.0000\nFinished comparison: 48000 samples (1.000 seconds) exceeded the error threshold of 0.001000."
        );
    }
}