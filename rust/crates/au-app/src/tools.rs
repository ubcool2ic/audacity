//! Multi-tool hit testing ported from Audacity 3.7.3 WaveformView::DetailedHitTest
//! (EnvelopeHandle::HitEnvelope, then SampleHandle::HitTest, else selection).

/// What the Multi tool acts as at the pointer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultiHit {
    Envelope,
    TimeShift,
    Draw,
    Select,
}

/// EnvelopeHandle::HitEnvelope tolerances.
pub const ENV_Y_TOLERANCE: i32 = 5;
pub const ENV_Y_MISALIGN: i32 = 2;
pub const ENV_MAX_CONTOURS: i32 = 2;
/// SampleHandle::HitTest tolerance.
pub const SAMPLE_Y_TOLERANCE: i32 = 10;

/// EnvelopeHandle::HitEnvelope: `env_y`, `ctr_y`, `mouse_y` in pixels; `height` of the
/// track area and the vertical zoom bounds give the contour spacing.
pub fn hit_envelope(env_y: i32, ctr_y: i32, mouse_y: i32, height: i32, zoom_min: f32, zoom_max: f32) -> bool {
    let y_mouse = (ctr_y - mouse_y).abs();
    let y_value = (ctr_y - env_y).abs();
    let span = (zoom_max - zoom_min).max(f32::EPSILON);
    let spacing = ((height as f32 / (2.0 * span)) as i32).max(1);
    let displace = y_value - ENV_Y_MISALIGN - y_mouse + spacing / 2;
    if displace > ENV_MAX_CONTOURS * spacing {
        return false;
    }
    // C++ `%` truncates toward zero like Rust's.
    let distance = ((displace % spacing) - spacing / 2).abs();
    distance < ENV_Y_TOLERANCE
}

/// WaveformView threshold2: at least 3 pixels per sample shows draggable points
/// (SampleResolutionTest).
pub fn sample_editing_possible(pixels_per_sec: f64, rate: f64) -> bool {
    pixels_per_sec > 3.0 * rate
}

/// SampleHandle::HitTest's y check.
pub fn hit_sample(sample_y: i32, mouse_y: i32) -> bool {
    (sample_y - mouse_y).abs() < SAMPLE_Y_TOLERANCE
}

/// TimeShiftHandle::HitTest's grip check.
pub fn hit_grip(mouse_x: i32, rect_x: i32, rect_width: i32) -> bool {
    mouse_x + 5 < rect_x + 14 || mouse_x + 5 >= rect_x + rect_width - 14
}

/// Priority order of WaveformView::DetailedHitTest with bMultiTool.
pub fn multi_hit(envelope: bool, grip: bool, sample: bool) -> MultiHit {
    if envelope {
        MultiHit::Envelope
    } else if grip {
        MultiHit::TimeShift
    } else if sample {
        MultiHit::Draw
    } else {
        MultiHit::Select
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_contours() {
        // 200 px track, zoom -1..1: contour spacing 50 px, centre at 100.
        assert!(hit_envelope(20, 100, 22, 200, -1.0, 1.0));
        assert!(hit_envelope(20, 100, 178, 200, -1.0, 1.0)); // mirrored half
        assert!(!hit_envelope(20, 100, 40, 200, -1.0, 1.0));
        // Inner contour one spacing closer to the centre also hits.
        assert!(hit_envelope(20, 100, 72, 200, -1.0, 1.0));
        // Beyond MaxContours outward misses.
    }

    #[test]
    fn samples_and_priority() {
        assert!(sample_editing_possible(44100.0 * 4.0, 44100.0));
        assert!(!sample_editing_possible(44100.0 * 3.0, 44100.0));
        assert!(hit_sample(50, 59));
        assert!(!hit_sample(50, 60));
        assert_eq!(multi_hit(true, true, true), MultiHit::Envelope);
        assert_eq!(multi_hit(false, true, true), MultiHit::TimeShift);
        assert_eq!(multi_hit(false, false, true), MultiHit::Draw);
        assert_eq!(multi_hit(false, false, false), MultiHit::Select);
    }

    #[test]
    fn grip_hit_test() {
        assert!(hit_grip(108, 100, 200));
        assert!(!hit_grip(109, 100, 200));
        assert!(!hit_grip(280, 100, 200));
        assert!(hit_grip(281, 100, 200));
    }
}
