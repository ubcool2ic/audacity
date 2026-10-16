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
            let grid_time = single_step(t, forward);
            if ((grid_time * pixels_per_sec) - (t * pixels_per_sec)).abs()
                >= pixels.abs() as f64
            {
                grid_time
            } else {
                snap(t + pixels as f64 / pixels_per_sec)
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

/// Returns whether keyboard scrubbing can start at the selection start.
pub fn keyboard_scrub_can_start(t0: f64, end_time: f64, backwards: bool) -> bool {
    if backwards {
        t0 > 0.0 && t0 <= end_time
    } else {
        t0 >= 0.0 && t0 < end_time
    }
}

/// Returns the desired time shift for a one-pixel clip move.
pub fn clip_shift_desired(t0: f64, right: bool, pixels_per_sec: f64) -> f64 {
    let desired_t0 = t0 + (if right { 1.0 } else { -1.0 }) / pixels_per_sec;
    desired_t0 - t0
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
        close(clip_shift_desired(2.0, true, 100.0), 0.01);
        close(clip_shift_desired(2.0, false, 100.0), -0.01);
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
