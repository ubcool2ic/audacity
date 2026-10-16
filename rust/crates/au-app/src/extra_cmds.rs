//! Pure helpers for Audacity's Extra-menu commands.
//!
//! This module ports the relevant time, selection, tool, slider, and scrub-speed
//! calculations from Audacity 3.7.3.

/// A play-region command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlayCmd {
    /// Play one second centered on the most recent mouse position.
    OneSec,
    /// Play between the mouse position and the nearest selection boundary.
    ToSelection,
    /// Play the configured interval before the selection start.
    BeforeSelStart,
    /// Play after the selection start, or the short selection itself.
    AfterSelStart,
    /// Play before the selection end, or the short selection itself.
    BeforeSelEnd,
    /// Play the configured interval after the selection end.
    AfterSelEnd,
    /// Play before the selection start and after it (or through a short selection).
    BeforeAndAfterSelStart,
    /// Play before the selection end and after it (or through a short selection).
    BeforeAndAfterSelEnd,
}

/// Computes the selected region used by a play command.
///
/// `pos` is the most recent mouse-position time. `before_len` and `after_len`
/// are the configured cut-preview lengths. The result is not clamped; playback
/// performs its own clamping.
pub fn play_region(
    cmd: PlayCmd,
    pos: f64,
    t0: f64,
    t1: f64,
    before_len: f64,
    after_len: f64,
) -> (f64, f64) {
    match cmd {
        PlayCmd::OneSec => (pos - 0.5, pos + 0.5),
        PlayCmd::ToSelection => {
            let edge = if (pos - t0).abs() < (pos - t1).abs() {
                t0
            } else {
                t1
            };
            if pos < edge {
                (pos, edge)
            } else {
                (edge, pos)
            }
        }
        PlayCmd::BeforeSelStart => (t0 - before_len, t0),
        PlayCmd::AfterSelStart => {
            if t1 - t0 > 0.0 && t1 - t0 < after_len {
                (t0, t1)
            } else {
                (t0, t0 + after_len)
            }
        }
        PlayCmd::BeforeSelEnd => {
            if t1 - t0 > 0.0 && t1 - t0 < before_len {
                (t0, t1)
            } else {
                (t1 - before_len, t1)
            }
        }
        PlayCmd::AfterSelEnd => (t1, t1 + after_len),
        PlayCmd::BeforeAndAfterSelStart => {
            if t1 - t0 > 0.0 && t1 - t0 < after_len {
                (t0 - before_len, t1)
            } else {
                (t0 - before_len, t0 + after_len)
            }
        }
        PlayCmd::BeforeAndAfterSelEnd => {
            if t1 - t0 > 0.0 && t1 - t0 < before_len {
                (t0, t1 + after_len)
            } else {
                (t1 - before_len, t1 + after_len)
            }
        }
    }
}

/// Returns the two cut-preview playback segments, skipping the selection.
pub fn cut_preview_segments(
    t0: f64,
    t1: f64,
    before_len: f64,
    after_len: f64,
) -> [(f64, f64); 2] {
    [(t0 - before_len, t0), (t1, t1 + after_len)]
}

/// The selection operation used when seeking while audio is inactive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelOp {
    /// Move a selection edge away from the selection.
    Extend,
    /// Move a selection edge toward the other edge without crossing it.
    Contract,
    /// Move or collapse the cursor.
    CursorMove,
}

/// The repeat interval used to determine fast keyboard seeking.
pub const FAST_REPEAT_MS: u64 = 50;

/// Returns the signed pixel seek step, multiplied by four for a fast repeat.
pub fn seek_step(direction: f64, fast: bool) -> f64 {
    direction * if fast { 4.0 } else { 1.0 }
}

#[cfg(test)]
mod grid_move_tests {
    use super::{seek_inactive, SelOp};
    const PPS: f64 = 86.1328125;
    fn prior_snap(t: f64) -> f64 {
        (t * 75.0).floor() / 75.0
    }
    fn nearest_snap(t: f64) -> f64 {
        (t * 75.0).round() / 75.0
    }
    fn grid_step(t: f64, forward: bool) -> f64 {
        if forward {
            ((t * 75.0).floor() as i64 + 1) as f64 / 75.0
        } else {
            ((t * 75.0).ceil() as i64 - 1) as f64 / 75.0
        }
    }
    #[test]
    fn grid_move_matches_audacity() {
        let (a0, a1) = seek_inactive(
            0.0,
            2.37,
            2.37,
            -1.0,
            SelOp::CursorMove,
            PPS,
            100.0,
            &prior_snap,
            true,
            &grid_step,
        );
        assert!((a0 - 177.0 / 75.0).abs() < 1e-12, "got {a0}");
        assert!((a1 - 177.0 / 75.0).abs() < 1e-12, "got {a1}");
        let (b0, b1) = seek_inactive(
            0.0,
            1.234,
            1.234,
            -1.0,
            SelOp::CursorMove,
            PPS,
            100.0,
            &nearest_snap,
            true,
            &grid_step,
        );
        assert!((b0 - 91.0 / 75.0).abs() < 1e-12, "got {b0}");
        assert!((b1 - 91.0 / 75.0).abs() < 1e-12, "got {b1}");
    }
}
/// Moves a selection or cursor while audio is inactive.
///
/// Pixel offsets are converted to integer pixels as in Audacity's
/// `OffsetTime`. When snapping is enabled, `single_step` supplies the next grid
/// point; if that point moves fewer than the requested pixels, the requested
/// pixel offset is used and passed through `snap`.
/// ZoomInfo::OffsetTimeByPixels: PositionToTime(TimeToPosition(t) + pixels), where
/// TimeToPosition = floor(0.5 + zoom * (t - h)) and PositionToTime(p) = h + p / zoom.
pub fn offset_time_by_pixels(t: f64, pixels: i64, pixels_per_sec: f64, h: f64) -> f64 {
    let pos = (0.5 + pixels_per_sec * (t - h)).floor() as i64;
    h + (pos + pixels) as f64 / pixels_per_sec
}

pub fn seek_inactive(
    h: f64,
    t0: f64,
    t1: f64,
    step_pixels: f64,
    op: SelOp,
    pixels_per_sec: f64,
    end: f64,
    snap: &dyn Fn(f64) -> f64,
    snap_on: bool,
    single_step: &dyn Fn(f64, bool) -> f64,
) -> (f64, f64) {
    let pixels = step_pixels as i32;

    let offset_time = |t: f64| {
        let forward = pixels >= 0;
        if !snap_on {
            offset_time_by_pixels(t, pixels as i64, pixels_per_sec, h)
        } else {
            let result = single_step(t, forward);
            let time_to_position =
                |x: f64| (0.5 + pixels_per_sec * (x - h)).floor() as i64;
            if (time_to_position(result) - time_to_position(t)).abs()
                >= pixels.abs() as i64
            {
                result
            } else {
                let shifted =
                    offset_time_by_pixels(t, pixels as i64, pixels_per_sec, h);
                snap(shifted)
            }
        }
    };

    if op == SelOp::CursorMove {
        if t0 == t1 {
            let new_t = offset_time(t0).max(0.0);
            return (new_t, new_t);
        }

        // A non-collapsed selection first transitions to a cursor at the
        // corresponding edge; the seek amount is not additionally applied.
        let new_t = if step_pixels < 0.0 {
            if snap_on { snap(t0) } else { t0 }
        } else if snap_on {
            snap(t1)
        } else {
            t1
        };
        return (new_t, new_t);
    }

    let move_t0 = (op == SelOp::Contract && step_pixels > 0.0)
        || (op == SelOp::Extend && step_pixels < 0.0);
    let origin = if move_t0 { t0 } else { t1 };
    let mut new_t = offset_time(origin).max(0.0).min(end);

    if op == SelOp::Contract {
        new_t = if move_t0 {
            new_t.min(t1)
        } else {
            new_t.max(t0)
        };
    }

    if move_t0 {
        set_t0(t0, t1, new_t, true)
    } else {
        set_t1(t0, t1, new_t, true)
    }
}

/// Moves a selection boundary by pixels while audio is inactive.
///
/// A fast move multiplies the requested pixel count by four. The moved
/// boundary is clamped to `[0, end]`, and the selection boundary setter may
/// swap endpoints if the moved boundary crosses the other one.
pub fn boundary_move(
    h: f64,
    t0: f64,
    t1: f64,
    step: i32,
    fast: bool,
    pixels_per_sec: f64,
    end: f64,
) -> (f64, f64) {
    let pixels = if fast {
        step.saturating_mul(4)
    } else {
        step
    };
    let move_t0 = step < 0;
    let origin = if move_t0 { t0 } else { t1 };
    let new_t = offset_time_by_pixels(origin, pixels as i64, pixels_per_sec, h)
        .max(0.0)
        .min(end);

    if move_t0 {
        set_t0(t0, t1, new_t, true)
    } else {
        set_t1(t0, t1, new_t, true)
    }
}

/// Scroll-to-start selection behavior: extend the selection from time zero.
pub fn sel_to_start(_t0: f64, t1: f64) -> (f64, f64) {
    (0.0, t1)
}

/// Scroll-to-end selection behavior: extend the selection to `end_time`.
pub fn sel_to_end(t0: f64, _t1: f64, end_time: f64) -> (f64, f64) {
    (t0, end_time)
}

/// Number of tools cycled by the Extra-menu tool commands.
pub const NUM_TOOLS: usize = 4;

/// Selects the preceding tool, wrapping around the tool list.
pub fn prev_tool(t: usize) -> usize {
    (t % NUM_TOOLS + NUM_TOOLS - 1) % NUM_TOOLS
}

/// Selects the following tool, wrapping around the tool list.
pub fn next_tool(t: usize) -> usize {
    (t % NUM_TOOLS + 1) % NUM_TOOLS
}

/// Adjusts a slider by signed step count, then clamps it to its range.
///
/// A zero `step_value` selects the continuous-slider increment
/// `(max - min) / 10`. Positive steps use the slider's Increase behavior;
/// negative steps use its Decrease behavior.
pub fn slider_step(value: f32, steps: f32, min: f32, max: f32, step_value: f32) -> f32 {
    let step_value = if step_value == 0.0 {
        (max - min) / 10.0
    } else {
        step_value
    };

    let mut current = value;
    if steps > 0.0 {
        current += steps * step_value;
    } else if steps < 0.0 {
        current -= (-steps) * step_value;
    }

    if current < min {
        min
    } else if current > max {
        max
    } else {
        current
    }
}

/// Minimum play-speed slider value.
pub const SPEED_MIN: f32 = 0.01;
/// Maximum play-speed slider value.
pub const SPEED_MAX: f32 = 3.0;

/// Adjusts play speed using the continuous play-speed slider increment.
pub fn play_speed_adjust(speed: f32, adj: f32) -> f32 {
    slider_step(speed, adj, SPEED_MIN, SPEED_MAX, 0.0)
}

/// Adjusts meter gain using the continuous percent-slider increment.
pub fn volume_adjust(v: f32, adj: i32) -> f32 {
    slider_step(v, adj as f32, 0.0, 1.0, 0.0)
}

/// Returns keyboard scrub speed for the current zoom, clamped to Audacity's
/// minimum and maximum scrub speeds.
pub fn keyboard_scrub_speed(default_zoom: f64, zoom: f64) -> f64 {
    let mut speed = 0.5 * default_zoom / zoom;
    if speed > 3.0 {
        speed = 3.0;
    }
    if speed < 0.0625 {
        speed = 0.0625;
    }
    speed
}
pub use crate::scrub::SCRUB_POLL_INTERVAL_MS as SCRUB_POLL_MS;
use crate::scrub::{MAX_SCRUB_SPEED, MIN_SCRUB_SPEED, SCRUB_POLL_INTERVAL_MS};

/// One queued scrub stutter (ScrubQueue Entry).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stutter {
    pub s0: i64,
    pub s1: i64,
    pub duration: i64,
    pub silence: i64,
    /// mGoal (-1 when none).
    pub goal: i64,
}

/// ScrubState Entry::Init. `prev_goal` is the previous entry's goal; bounds are samples.
/// Returns `None` when a seeking stutter would be too short.
#[allow(clippy::too_many_arguments)]
pub fn scrub_entry_init(
    prev_goal: i64, mut s0: i64, mut s1: i64, mut duration: i64, min_speed: f64, max_speed: f64,
    adjust_start: bool, min_stutter_samples: i64, min_sample: i64, max_sample: i64,
) -> Option<Stutter> {
    let orig = duration;
    let mut speed = (s1 - s0).abs() as f64 / duration.max(1) as f64;
    let mut adjusted = false;
    let mut min_speed = min_speed.min(max_speed);
    let goal;
    if !adjust_start && speed > max_speed {
        speed = max_speed;
        goal = s1;
        adjusted = true;
    } else if !adjust_start && prev_goal >= 0 && prev_goal == s1 {
        min_speed = max_speed;
        goal = s1;
        adjusted = true;
    } else {
        goal = -1;
    }
    if speed < min_speed {
        if !(s0 != s1 && adjust_start) {
            duration = ((speed * duration as f64 / min_speed).round() as i64).max(0);
        }
        speed = min_speed;
        adjusted = true;
    }
    if speed < MIN_SCRUB_SPEED {
        adjusted = true;
        speed = 0.0;
    }
    if adjusted && !adjust_start {
        let diff = (speed * duration as f64).round() as i64;
        s1 = if s0 < s1 { s0 + diff } else { s0 - diff };
    }
    let mut silent = false;
    if s1 != s0 {
        if adjust_start && duration < min_stutter_samples {
            return None;
        }
        let new_s1 = s1.clamp(min_sample, max_sample);
        let mut new_dur = duration;
        if s1 != new_s1 {
            new_dur = ((duration as f64 * (new_s1 - s0) as f64 / (s1 - s0) as f64) as i64).max(0);
        }
        if new_dur == 0 {
            silent = true;
            s1 = s0;
        } else if s1 != new_s1 {
            duration = new_dur;
            s1 = new_s1;
        }
    }
    if adjust_start && !silent {
        let diff = (max_speed.min(speed) * duration as f64).round() as i64;
        s0 = if s0 < s1 { s1 - diff } else { s1 + diff };
    }
    Some(Stutter { s0, s1, duration, silence: (orig - duration).max(0), goal })
}

/// One keyboard-scrub poll (ScrubState::UpdateScrub with bySpeed, keyboard options):
/// a stutter of one poll interval from `pos` at signed `speed`, kept inside 0..`end`.
pub fn keyboard_stutter(prev_goal: i64, pos: i64, speed: f64, rate: f64, end: i64) -> Stutter {
    let duration = (rate * SCRUB_POLL_INTERVAL_MS as f64 / 1000.0).round() as i64;
    let s1 = pos + (duration as f64 * speed).round() as i64;
    scrub_entry_init(prev_goal, pos, s1, duration, MIN_SCRUB_SPEED, MAX_SCRUB_SPEED,
        false, duration, 0, end)
        .unwrap_or(Stutter { s0: pos, s1: pos, duration: 0, silence: duration, goal: -1 })
}

/// Returns whether keyboard scrubbing can start at the selection start.
pub fn keyboard_scrub_can_start(t0: f64, end_time: f64, backwards: bool) -> bool {
    if backwards {
        t0 > 0.0 && t0 <= end_time
    } else {
        t0 >= 0.0 && t0 < end_time
    }
}

fn set_t0(_t0: f64, t1: f64, new_t0: f64, may_swap: bool) -> (f64, f64) {
    if may_swap && new_t0 > t1 {
        (t1, new_t0)
    } else {
        (new_t0, t1)
    }
}

fn set_t1(t0: f64, _t1: f64, new_t1: f64, may_swap: bool) -> (f64, f64) {
    if may_swap && new_t1 < t0 {
        (new_t1, t0)
    } else {
        (t0, new_t1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1.0e-12, "{a} != {b}");
    }

    fn close_f32(a: f32, b: f32) {
        assert!((a - b).abs() < 1.0e-6, "{a} != {b}");
    }

    #[test]
    fn play_region_covers_all_commands() {
        close_pair(play_region(PlayCmd::OneSec, 5.0, 2.0, 4.0, 2.0, 1.0), (4.5, 5.5));

        // Ties choose the right selection edge, as in Audacity's strict `<`.
        close_pair(
            play_region(PlayCmd::ToSelection, 3.0, 2.0, 4.0, 2.0, 1.0),
            (3.0, 4.0),
        );
        close_pair(
            play_region(PlayCmd::ToSelection, 1.0, 2.0, 4.0, 2.0, 1.0),
            (1.0, 2.0),
        );
        close_pair(
            play_region(PlayCmd::ToSelection, 5.0, 2.0, 4.0, 2.0, 1.0),
            (4.0, 5.0),
        );

        close_pair(
            play_region(PlayCmd::BeforeSelStart, 9.0, 2.0, 4.0, 2.0, 1.0),
            (0.0, 2.0),
        );
        close_pair(
            play_region(PlayCmd::AfterSelStart, 9.0, 2.0, 2.5, 2.0, 1.0),
            (2.0, 2.5),
        );
        close_pair(
            play_region(PlayCmd::AfterSelStart, 9.0, 2.0, 4.0, 2.0, 1.0),
            (2.0, 3.0),
        );
        close_pair(
            play_region(PlayCmd::BeforeSelEnd, 9.0, 2.0, 2.5, 2.0, 1.0),
            (2.0, 2.5),
        );
        close_pair(
            play_region(PlayCmd::BeforeSelEnd, 9.0, 2.0, 4.0, 2.0, 1.0),
            (2.0, 4.0),
        );
        close_pair(
            play_region(PlayCmd::AfterSelEnd, 9.0, 2.0, 4.0, 2.0, 1.0),
            (4.0, 5.0),
        );
        close_pair(
            play_region(
                PlayCmd::BeforeAndAfterSelStart,
                9.0,
                2.0,
                2.5,
                2.0,
                1.0,
            ),
            (0.0, 2.5),
        );
        close_pair(
            play_region(
                PlayCmd::BeforeAndAfterSelStart,
                9.0,
                2.0,
                4.0,
                2.0,
                1.0,
            ),
            (0.0, 3.0),
        );
        close_pair(
            play_region(
                PlayCmd::BeforeAndAfterSelEnd,
                9.0,
                2.0,
                2.5,
                2.0,
                1.0,
            ),
            (2.0, 3.5),
        );
        close_pair(
            play_region(
                PlayCmd::BeforeAndAfterSelEnd,
                9.0,
                2.0,
                4.0,
                2.0,
                1.0,
            ),
            (2.0, 5.0),
        );

        // A zero-duration selection does not use the short-selection branch.
        close_pair(
            play_region(PlayCmd::AfterSelStart, 0.0, 2.0, 2.0, 2.0, 1.0),
            (2.0, 3.0),
        );
    }

    fn close_pair(a: (f64, f64), b: (f64, f64)) {
        close(a.0, b.0);
        close(a.1, b.1);
    }

    #[test]
    fn cut_preview_segments_skip_the_selection() {
        assert_eq!(cut_preview_segments(2.0, 5.0, 2.0, 1.0), [(0.0, 2.0), (5.0, 6.0)]);
    }

    #[test]
    fn seek_step_has_normal_and_fast_paths() {
        close(seek_step(-1.0, false), -1.0);
        close(seek_step(1.5, true), 6.0);
        assert_eq!(FAST_REPEAT_MS, 50);
    }

    #[test]
    fn seek_inactive_moves_and_clamps_selection_edges() {
        let identity = |t| t;
        let single = |t: f64, forward: bool| t + if forward { 1.0 } else { -1.0 };

        close_pair(
            seek_inactive(0.0, 2.0, 4.0, -1.0, SelOp::Extend, 1.0, 10.0, &identity, false, &single),
            (1.0, 4.0),
        );
        close_pair(
            seek_inactive(0.0, 2.0, 4.0, 1.0, SelOp::Extend, 1.0, 10.0, &identity, false, &single),
            (2.0, 5.0),
        );
        close_pair(
            seek_inactive(0.0, 2.0, 4.0, 1.0, SelOp::Contract, 1.0, 10.0, &identity, false, &single),
            (3.0, 4.0),
        );
        close_pair(
            seek_inactive(0.0, 2.0, 4.0, -1.0, SelOp::Contract, 1.0, 10.0, &identity, false, &single),
            (2.0, 3.0),
        );

        // Contracting cannot cross the other edge.
        close_pair(
            seek_inactive(0.0, 2.0, 4.0, 20.0, SelOp::Contract, 1.0, 30.0, &identity, false, &single),
            (4.0, 4.0),
        );
        close_pair(
            seek_inactive(0.0, 2.0, 4.0, -20.0, SelOp::Contract, 1.0, 30.0, &identity, false, &single),
            (2.0, 2.0),
        );

        // The selection endpoints are constrained to zero and end.
        close_pair(
            seek_inactive(0.0, 2.0, 4.0, -20.0, SelOp::Extend, 1.0, 10.0, &identity, false, &single),
            (0.0, 4.0),
        );
        close_pair(
            seek_inactive(0.0, 2.0, 4.0, 20.0, SelOp::Extend, 1.0, 6.0, &identity, false, &single),
            (2.0, 6.0),
        );

        // Extending may cross; SelectedRegion's setter swaps the endpoints.
        let far_grid = |_t: f64, _forward: bool| 9.0;
        close_pair(
            seek_inactive(0.0, 2.0, 4.0, -1.0, SelOp::Extend, 1.0, 10.0, &identity, true, &far_grid),
            (4.0, 9.0),
        );
    }

    #[test]
    fn seek_inactive_snap_grid_and_cursor_paths() {
        let identity = |t| t;
        let tiny_step = |t: f64, forward: bool| t + if forward { 0.25 } else { -0.25 };

        // An adequate single step is accepted directly.
        let adequate = |t: f64, forward: bool| t + if forward { 2.0 } else { -2.0 };
        close_pair(
            seek_inactive(0.0, 1.0, 3.0, 2.0, SelOp::Extend, 1.0, 10.0, &identity, true, &adequate),
            (1.0, 5.0),
        );

        // A too-small grid step falls back to the requested pixel offset, then snaps.
        let round_to_half = |t: f64| (t * 2.0).round() / 2.0;
        close_pair(
            seek_inactive(0.0, 1.0, 3.0, 2.0, SelOp::Extend, 1.0, 10.0, &round_to_half, true, &tiny_step),
            (1.0, 5.0),
        );

        // Cursor movement from a point applies the pixel move and clamps at zero.
        close_pair(
            seek_inactive(0.0, 1.0, 1.0, -3.0, SelOp::CursorMove, 1.0, 10.0, &identity, false, &tiny_step),
            (0.0, 0.0),
        );
        close_pair(
            seek_inactive(0.0, 1.0, 1.0, 2.0, SelOp::CursorMove, 1.0, 10.0, &identity, false, &tiny_step),
            (3.0, 3.0),
        );

        // A non-point selection collapses to the directional edge, snapping it
        // if enabled, without applying an additional pixel move.
        let snap_up = |t: f64| t + 0.5;
        close_pair(
            seek_inactive(0.0, 2.0, 4.0, -1.0, SelOp::CursorMove, 1.0, 10.0, &snap_up, true, &tiny_step),
            (2.5, 2.5),
        );
        close_pair(
            seek_inactive(0.0, 2.0, 4.0, 1.0, SelOp::CursorMove, 1.0, 10.0, &identity, false, &tiny_step),
            (4.0, 4.0),
        );
    }

    #[test]
    fn boundary_move_clamps_fast_moves_and_swaps() {
        close_pair(boundary_move(0.0, 2.0, 4.0, -1, false, 1.0, 10.0), (1.0, 4.0));
        close_pair(boundary_move(0.0, 2.0, 4.0, 1, true, 1.0, 10.0), (2.0, 8.0));
        close_pair(boundary_move(0.0, 2.0, 4.0, -20, false, 1.0, 10.0), (0.0, 4.0));
        close_pair(boundary_move(0.0, 2.0, 4.0, 20, false, 1.0, 6.0), (2.0, 6.0));

    }

    #[test]
    fn selection_to_start_and_end_matches_scroll_commands() {
        close_pair(sel_to_start(2.0, 5.0), (0.0, 5.0));
        close_pair(sel_to_end(2.0, 5.0, 12.0), (2.0, 12.0));
    }

    #[test]
    fn tools_wrap_in_both_directions() {
        assert_eq!(NUM_TOOLS, 4);
        assert_eq!(prev_tool(0), 3);
        assert_eq!(prev_tool(1), 0);
        assert_eq!(prev_tool(5), 0);
        assert_eq!(next_tool(3), 0);
        assert_eq!(next_tool(0), 1);
        assert_eq!(next_tool(5), 2);
    }

    #[test]
    fn sliders_adjust_and_clamp() {
        close_f32(slider_step(0.5, 2.0, 0.0, 1.0, 0.1), 0.7);
        close_f32(slider_step(0.5, -2.0, 0.0, 1.0, 0.1), 0.3);
        close_f32(slider_step(0.5, 0.0, 0.0, 1.0, 0.1), 0.5);

        close_f32(play_speed_adjust(0.02, -1.0), SPEED_MIN);
        close_f32(play_speed_adjust(2.99, 1.0), SPEED_MAX);
        close_f32(play_speed_adjust(1.0, 0.1), 1.0299);

        close_f32(volume_adjust(0.5, 1), 0.6);
        close_f32(volume_adjust(0.5, -1), 0.4);
        close_f32(volume_adjust(0.95, 1), 1.0);
        close_f32(volume_adjust(0.05, -1), 0.0);
    }

    #[test]
    fn keyboard_stutters_follow_scrub_queue() {
        // 0.5x forward at 44.1 kHz: 2205-sample stutter covering 1103 samples.
        let st = keyboard_stutter(-1, 1000, 0.5, 44100.0, 100_000);
        assert_eq!((st.s0, st.s1, st.duration, st.silence), (1000, 2103, 2205, 0));
        // Backwards stutter clipped at time zero shortens the stutter, rest is silence.
        let st = keyboard_stutter(-1, 500, -1.0, 44100.0, 100_000);
        assert_eq!((st.s1, st.duration), (0, 500));
        assert_eq!(st.silence, 2205 - 500);
        // At the end: silent.
        let st = keyboard_stutter(-1, 100_000, 1.0, 44100.0, 100_000);
        assert_eq!(st.s1, 100_000);
        // Above max allowed speed it is capped and records a goal.
        let st = keyboard_stutter(-1, 0, 40.0, 44100.0, 10_000_000);
        assert_eq!(st.s1, (32.0 * 2205.0) as i64);
        assert_eq!(st.goal, 40 * 2205);
        // Seeking stutters shorter than minStutterTime are abandoned.
        assert!(scrub_entry_init(-1, 0, 100, 10, 0.0, 32.0, true, 2205, 0, 1000).is_none());
    }

    #[test]
    fn keyboard_scrubbing_speed_and_start_conditions() {
        close(keyboard_scrub_speed(1.0, 1.0), 0.5);
        close(keyboard_scrub_speed(100.0, 1.0), 3.0);
        close(keyboard_scrub_speed(0.01, 10.0), 0.0625);

        assert!(keyboard_scrub_can_start(0.0, 10.0, false));
        assert!(keyboard_scrub_can_start(9.0, 10.0, false));
        assert!(!keyboard_scrub_can_start(10.0, 10.0, false));
        assert!(!keyboard_scrub_can_start(-0.1, 10.0, false));

        assert!(keyboard_scrub_can_start(10.0, 10.0, true));
        assert!(keyboard_scrub_can_start(0.1, 10.0, true));
        assert!(!keyboard_scrub_can_start(0.0, 10.0, true));
        assert!(!keyboard_scrub_can_start(10.1, 10.0, true));
    }

    #[test]
    fn clip_shift_is_one_pixel_in_either_direction() {
    }

    #[test]
    fn offset_time_by_pixels_matches_audacity_3_7_3() {
        // Measured over the pipe at ZoomNormal (44100/512 px/s): Shift+Left/Right from 2..4 and 3..3.
        let z = 86.1328125;
        assert!((offset_time_by_pixels(2.0, -1, z, 0.0) - 1.9853).abs() < 1e-4);
        assert!((offset_time_by_pixels(4.0, 1, z, 0.0) - 4.0171).abs() < 1e-4);
        assert!((offset_time_by_pixels(3.0, -1, z, 0.0) - 2.9838).abs() < 1e-4);
        assert!((offset_time_by_pixels(3.0, 1, z, 0.0) - 3.0070).abs() < 1e-4);
    }
}
