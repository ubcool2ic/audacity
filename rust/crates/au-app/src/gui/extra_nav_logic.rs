//! Pure helpers modeling selected Audacity 3.7.3 behaviors.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelOp {
    CursorMove,
    Extend,
    Contract,
}

pub struct SeekCtx {
    pub t0: f64,
    pub t1: f64,
    /// Maximum of the tracks' end time and the screen end time.
    pub end: f64,
    pub pixels_per_sec: f64,
    pub h_offset: f64,
    /// Snap grid spacing in seconds (None = Snap-To Off).
    pub snap: Option<f64>,
}

fn offset_time_by_pixels(t: f64, px: f64, pixels_per_sec: f64, h_offset: f64) -> f64 {
    let pos = (0.5 + pixels_per_sec * (t - h_offset)).floor() as i64;
    h_offset + (pos as f64 + px) / pixels_per_sec
}

fn snap_to(grid: f64, t: f64) -> f64 {
    if grid > 0.0 { (t / grid).round() * grid } else { t }
}

/// GridMove: ProjectSnap::SingleStep to the next grid point in the direction of travel; if that
/// moves fewer than |step_px| pixels, move step_px pixels and snap instead.
fn grid_move(ctx: &SeekCtx, t: f64, step_px: f64) -> f64 {
    let target = offset_time_by_pixels(t, step_px, ctx.pixels_per_sec, ctx.h_offset);
    match ctx.snap {
        None => target,
        Some(grid) if grid > 0.0 => {
            let k = t / grid;
            let next = if step_px >= 0.0 { (k.floor() + 1.0) * grid } else { (k.ceil() - 1.0) * grid };
            if ((next - t) * ctx.pixels_per_sec).abs() >= step_px.abs() {
                next
            } else {
                snap_to(grid, target)
            }
        }
        Some(_) => target,
    }
}

fn set_t0_swapping(_t0: f64, t1: f64, new_t0: f64) -> (f64, f64) {
    if new_t0 > t1 {
        (t1, new_t0)
    } else {
        (new_t0, t1)
    }
}

fn set_t1_swapping(t0: f64, _t1: f64, new_t1: f64) -> (f64, f64) {
    if new_t1 < t0 {
        (new_t1, t0)
    } else {
        (t0, new_t1)
    }
}

/// Seeks a selection while audio is inactive, using pixel-based movement.
pub fn seek_inactive_pixels(ctx: &SeekCtx, step_px: f64, op: SelOp) -> (f64, f64) {
    let (t0, t1) = (ctx.t0, ctx.t1);

    if op == SelOp::CursorMove {
        if t0 == t1 {
            let new_t = grid_move(ctx, t0, step_px).max(0.0);
            return (new_t, new_t);
        }

        let new_t = if step_px < 0.0 {
            ctx.snap.map_or(t0, |grid| snap_to(grid, t0))
        } else {
            ctx.snap.map_or(t1, |grid| snap_to(grid, t1))
        };
        return (new_t, new_t);
    }

    let move_t0 = (op == SelOp::Contract && step_px > 0.0)
        || (op == SelOp::Extend && step_px < 0.0);
    let edge = if move_t0 { t0 } else { t1 };
    let mut new_t = grid_move(ctx, edge, step_px).max(0.0);
    new_t = new_t.min(ctx.end);

    if op == SelOp::Contract {
        new_t = if move_t0 {
            new_t.min(t1)
        } else {
            new_t.max(t0)
        };
    }

    if move_t0 {
        set_t0_swapping(t0, t1, new_t)
    } else {
        set_t1_swapping(t0, t1, new_t)
    }
}


/// Performs a boundary move, optionally using the play position as the new boundary.
pub fn boundary_move(
    t0: f64,
    t1: f64,
    end: f64,
    pps: f64,
    h_offset: f64,
    step_px: f64,
    play_pos: Option<f64>,
) -> (f64, f64) {
    let move_t0 = step_px < 0.0;

    if let Some(position) = play_pos {
        if move_t0 {
            if position > t1 {
                (position, position)
            } else {
                (position, t1)
            }
        } else if position < t0 {
            (position, t0)
        } else {
            (t0, position)
        }
    } else {
        let edge = if move_t0 { t0 } else { t1 };
        let new_t = offset_time_by_pixels(edge, step_px, pps, h_offset)
            .max(0.0)
            .min(end);
        if move_t0 {
            set_t0_swapping(t0, t1, new_t)
        } else {
            set_t1_swapping(t0, t1, new_t)
        }
    }
}

/// Returns the fast-navigation multiplier.
pub fn fast_multiplier(elapsed_ms_since_last: Option<u128>) -> f64 {
    if matches!(elapsed_ms_since_last, Some(elapsed) if elapsed < 50) {
        4.0
    } else {
        1.0
    }
}

/// Finds the next label according to Audacity's label-track navigation rules.
pub fn find_next_label(t0s: &[f64], cur_t0: f64, last: &mut i32) -> i32 {
    let mut i = -1;
    if !t0s.is_empty() {
        let len = t0s.len();
        let previous = *last;
        if previous >= 0
            && (previous as usize) + 1 < len
            && cur_t0 == t0s[previous as usize]
            && cur_t0 == t0s[previous as usize + 1]
        {
            i = previous + 1;
        } else {
            i = 0;
            if cur_t0 < t0s[len - 1] {
                while (i as usize) < len && t0s[i as usize] <= cur_t0 {
                    i += 1;
                }
            }
        }
    }
    *last = i;
    i
}

/// Finds the previous label according to Audacity's label-track navigation rules.
pub fn find_prev_label(t0s: &[f64], cur_t0: f64, last: &mut i32) -> i32 {
    let mut i = -1;
    if !t0s.is_empty() {
        let len = t0s.len();
        let previous = *last;
        if previous > 0
            && (previous as usize) < len
            && cur_t0 == t0s[previous as usize]
            && cur_t0 == t0s[previous as usize - 1]
        {
            i = previous - 1;
        } else {
            i = len as i32 - 1;
            if cur_t0 > t0s[0] {
                while i >= 0 && t0s[i as usize] >= cur_t0 {
                    i -= 1;
                }
            }
        }
    }
    *last = i;
    i
}









/// Finds the previous track for non-shift navigation.
pub fn prev_track(focus: Option<usize>, n: usize, circular: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    match focus {
        None => Some(n - 1),
        Some(index) if index >= n => None,
        Some(0) => Some(if circular { n - 1 } else { 0 }),
        Some(index) => Some(index - 1),
    }
}

/// Finds the next track for non-shift navigation.
pub fn next_track(focus: Option<usize>, n: usize, circular: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    match focus {
        None => Some(0),
        Some(index) if index >= n => None,
        Some(index) if index + 1 < n => Some(index + 1),
        Some(_) => Some(if circular { 0 } else { n - 1 }),
    }
}

/// Applies the selection changes for shift-navigation between focused and target tracks.
pub fn shift_nav(sel: &mut Vec<bool>, focus: usize, target: usize) {
    if focus >= sel.len() || target >= sel.len() {
        return;
    }

    let focus_selected = sel[focus];
    let target_selected = sel[target];

    match (focus_selected, target_selected) {
        (true, true) => sel[focus] = false,
        (true, false) => sel[target] = true,
        (false, true) => sel[target] = false,
        (false, false) => sel[focus] = true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seek_inactive_pixels_covers_cursor_extend_contract_and_snap() {
        let ctx = SeekCtx {
            t0: 2.0,
            t1: 4.0,
            end: 10.0,
            pixels_per_sec: 10.0,
            h_offset: 0.0,
            snap: None,
        };
        assert_eq!(
            seek_inactive_pixels(&ctx, -1.0, SelOp::CursorMove),
            (2.0, 2.0)
        );
        assert_eq!(
            seek_inactive_pixels(&ctx, 1.0, SelOp::CursorMove),
            (4.0, 4.0)
        );

        let point = SeekCtx {
            t0: 0.1,
            t1: 0.1,
            ..ctx
        };
        assert_eq!(
            seek_inactive_pixels(&point, -2.0, SelOp::CursorMove),
            (0.0, 0.0)
        );

        assert_eq!(seek_inactive_pixels(&ctx, -10.0, SelOp::Extend), (1.0, 4.0));
        assert_eq!(seek_inactive_pixels(&ctx, 10.0, SelOp::Extend), (2.0, 5.0));
        assert_eq!(seek_inactive_pixels(&ctx, 10.0, SelOp::Contract), (3.0, 4.0));
        assert_eq!(seek_inactive_pixels(&ctx, -10.0, SelOp::Contract), (2.0, 3.0));

        let snapping = SeekCtx {
            t0: 1.0,
            t1: 2.0,
            snap: Some(0.5),
            ..ctx
        };
        // First snap lands on the original time, so the move is tried once more.
        assert_eq!(
            seek_inactive_pixels(&snapping, 1.0, SelOp::Extend),
            (1.0, 2.5)
        );
        assert_eq!(
            seek_inactive_pixels(&snapping, -1.0, SelOp::CursorMove),
            (1.0, 1.0)
        );
    }

    #[test]
    fn offset_time_by_pixels_uses_whole_pixel_positions() {
        let pps = 44100.0 / 512.0;
        assert!((offset_time_by_pixels(2.0, 1.0, pps, 0.0) - 173.0 / pps).abs() < 1e-12);
        assert!((offset_time_by_pixels(2.0, -1.0, pps, 0.0) - 171.0 / pps).abs() < 1e-12);
    }


    #[test]
    fn boundary_move_covers_play_position_and_pixel_moves() {
        assert_eq!(boundary_move(2.0, 4.0, 10.0, 10.0, 0.0, -10.0, None), (1.0, 4.0));
        assert_eq!(boundary_move(2.0, 4.0, 3.0, 10.0, 0.0, 10.0, None), (2.0, 3.0));
        assert_eq!(boundary_move(2.0, 4.0, 10.0, 10.0, 0.0, -1.0, Some(5.0)), (5.0, 5.0));
        assert_eq!(boundary_move(2.0, 4.0, 10.0, 10.0, 0.0, 1.0, Some(1.0)), (1.0, 2.0));
    }

    #[test]
    fn fast_multiplier_respects_fifty_millisecond_boundary() {
        assert_eq!(fast_multiplier(None), 1.0);
        assert_eq!(fast_multiplier(Some(0)), 4.0);
        assert_eq!(fast_multiplier(Some(49)), 4.0);
        assert_eq!(fast_multiplier(Some(50)), 1.0);
    }

    #[test]
    fn find_next_label_matches_order_and_duplicate_rules() {
        let labels = [1.0, 2.0, 2.0, 4.0];
        let mut last = -1;
        assert_eq!(find_next_label(&labels, 0.0, &mut last), 0);
        assert_eq!(find_next_label(&labels, 2.0, &mut last), 3);
        last = 1;
        assert_eq!(find_next_label(&labels, 2.0, &mut last), 2);
        assert_eq!(find_next_label(&[], 0.0, &mut last), -1);
        assert_eq!(last, -1);
    }

    #[test]
    fn find_prev_label_matches_order_and_duplicate_rules() {
        let labels = [1.0, 2.0, 2.0, 4.0];
        let mut last = -1;
        assert_eq!(find_prev_label(&labels, 5.0, &mut last), 3);
        assert_eq!(find_prev_label(&labels, 2.0, &mut last), 0);
        last = 2;
        assert_eq!(find_prev_label(&labels, 2.0, &mut last), 1);
        assert_eq!(find_prev_label(&[], 0.0, &mut last), -1);
        assert_eq!(last, -1);
    }








    #[test]
    fn track_navigation_handles_focus_edges_and_circularity() {
        assert_eq!(prev_track(None, 3, false), Some(2));
        assert_eq!(prev_track(Some(0), 3, false), Some(0));
        assert_eq!(prev_track(Some(0), 3, true), Some(2));
        assert_eq!(prev_track(Some(2), 3, false), Some(1));

        assert_eq!(next_track(None, 3, false), Some(0));
        assert_eq!(next_track(Some(2), 3, false), Some(2));
        assert_eq!(next_track(Some(2), 3, true), Some(0));
        assert_eq!(next_track(Some(0), 3, false), Some(1));

        assert_eq!(prev_track(None, 0, false), None);
        assert_eq!(next_track(None, 0, false), None);
        assert_eq!(prev_track(Some(3), 3, false), None);
        assert_eq!(next_track(Some(3), 3, false), None);
    }

    #[test]
    fn shift_nav_implements_all_four_selection_cases() {
        let mut both = vec![true, true];
        shift_nav(&mut both, 0, 1);
        assert_eq!(both, vec![false, true]);

        let mut focus_only = vec![true, false];
        shift_nav(&mut focus_only, 0, 1);
        assert_eq!(focus_only, vec![true, true]);

        let mut target_only = vec![false, true];
        shift_nav(&mut target_only, 0, 1);
        assert_eq!(target_only, vec![false, false]);

        let mut neither = vec![false, false];
        shift_nav(&mut neither, 0, 1);
        assert_eq!(neither, vec![true, false]);

        shift_nav(&mut neither, 5, 1);
        assert_eq!(neither, vec![true, false]);
    }
}
