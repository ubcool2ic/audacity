//! Waveform geometry helpers matching Audacity 3.7.3's waveform drawing.
#![allow(dead_code)]

/// Converts an audio value into its vertical waveform pixel position.
pub fn get_wave_y_pos(
    mut value: f32,
    min: f32,
    max: f32,
    height: i32,
    db: bool,
    outer: bool,
    db_range: f32,
    clip: bool,
) -> i32 {
    if db {
        if height == 0 {
            return 0;
        }

        let sign = if value >= 0.0 { 1.0_f32 } else { -1.0_f32 };

        if value != 0.0 {
            let db_value = (20.0 * value.abs().log10() as f64) as f32;
            value = (db_value + db_range) / db_range;
            if !outer {
                value -= 0.5;
            }
            if value < 0.0 {
                value = 0.0;
            }
            value *= sign;
        }
    } else if !outer {
        if value >= 0.0 {
            value -= 0.5;
        } else {
            value += 0.5;
        }
    }

    if clip {
        if value < min {
            value = min;
        }
        if value > max {
            value = max;
        }
    }

    let value = (max - value) / (max - min);
    (value * (height - 1) as f32 + 0.5) as i32
}

/// Converts a signed, normalized dB display value back to a linear amplitude.
pub fn from_db(value: f32, db_range: f64) -> f32 {
    if value == 0.0 {
        return 0.0;
    }

    let sign = if value >= 0.0 { 1.0_f64 } else { -1.0_f64 };
    let db = (value.abs() as f64 * db_range) - db_range;
    (10.0_f64.powf(db / 20.0) * sign) as f32
}

/// Converts a vertical pixel position into its corresponding waveform value.
pub fn value_of_pixel(
    yy: i32,
    height: i32,
    offset: bool,
    db: bool,
    db_range: f64,
    zoom_min: f32,
    zoom_max: f32,
) -> f32 {
    let mut value = if height == 1 {
        (zoom_min + zoom_max) / 2.0
    } else {
        zoom_max
            - (yy as f32 / (height - 1) as f32) * (zoom_max - zoom_min)
    };

    if offset {
        if value > 0.0 {
            value += 0.5;
        } else {
            value -= 0.5;
        }
    }

    if db {
        value = from_db(value, db_range);
    }

    value
}

/// How individual samples are connected when drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleDisplay {
    /// Connect adjacent samples with straight lines.
    ConnectDots,
    /// Draw vertical stems from each sample to the zero line.
    StemPlot,
}

/// Returns whether the current zoom is sufficient to draw individual samples.
pub fn show_individual_samples(pixels_per_second: f64, sample_rate: f64) -> bool {
    pixels_per_second > 0.5 * sample_rate
}

/// Returns whether the current zoom is sufficient to draw individual sample points.
pub fn show_points(pixels_per_second: f64, sample_rate: f64) -> bool {
    pixels_per_second > 3.0 * sample_rate
}

/// Pixel coordinates for drawing a sequence of individual samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleGeometry {
    /// Horizontal pixel positions relative to the view.
    pub xs: Vec<i32>,
    /// Vertical pixel positions relative to the view.
    pub ys: Vec<i32>,
    /// Vertical position of zero, relative to the view.
    pub y_zero: i32,
}

/// Computes the pixel positions used to draw individual samples.
pub fn individual_samples(
    samples: &[f32],
    first_sample_time: f64,
    sample_rate: f64,
    view_t0: f64,
    pixels_per_second: f64,
    width: i32,
    height: i32,
    zoom_min: f32,
    zoom_max: f32,
    db: bool,
    db_range: f32,
) -> SampleGeometry {
    let mut xs = Vec::with_capacity(samples.len());
    let mut ys = Vec::with_capacity(samples.len());

    for (index, &sample) in samples.iter().enumerate() {
        let time = first_sample_time + index as f64 / sample_rate;
        let x = (((time - view_t0) * pixels_per_second) + 0.5) as i32;
        xs.push(x.clamp(-10_000, 10_000));

        let y = get_wave_y_pos(
            sample, zoom_min, zoom_max, height, db, true, db_range, false,
        );
        ys.push(y.clamp(-10_000, height.saturating_add(10_000)));
    }

    let y_zero = get_wave_y_pos(
        0.0, zoom_min, zoom_max, height, db, true, db_range, false,
    )
    .clamp(-10_000, height.saturating_add(10_000));

    let _ = width;

    SampleGeometry { xs, ys, y_zero }
}

/// Returns the display bounds used when a waveform is minimized.
pub fn collapsed_bounds(half_wave: bool, minimized: bool) -> Option<(f32, f32)> {
    if !half_wave {
        None
    } else if minimized {
        Some((-0.01, 1.0))
    } else {
        Some((-1.0, 1.0))
    }
}

/// Computes the top and bottom pixels for the waveform extrema and RMS bounds.
pub fn min_max_rms_pixels(
    min: f32,
    max: f32,
    rms: f32,
    height: i32,
    zoom_min: f32,
    zoom_max: f32,
    db: bool,
    db_range: f32,
) -> (i32, i32, i32, i32) {
    let h1 = get_wave_y_pos(max, zoom_min, zoom_max, height, db, true, db_range, true);
    let h2 = get_wave_y_pos(min, zoom_min, zoom_max, height, db, true, db_range, true);
    let r1 = get_wave_y_pos(rms, zoom_min, zoom_max, height, db, true, db_range, true);
    let r2 = get_wave_y_pos(-rms, zoom_min, zoom_max, height, db, true, db_range, true);
    (h1, h2, r1, r2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_mapping_at_full_scale() {
        assert_eq!(get_wave_y_pos(1.0, -1.0, 1.0, 101, false, true, 60.0, false), 0);
        assert_eq!(get_wave_y_pos(0.0, -1.0, 1.0, 101, false, true, 60.0, false), 50);
        assert_eq!(get_wave_y_pos(-1.0, -1.0, 1.0, 101, false, true, 60.0, false), 100);
    }

    #[test]
    fn db_mapping_and_sign() {
        assert_eq!(get_wave_y_pos(1.0, -1.0, 1.0, 101, true, true, 60.0, false), 0);
        assert_eq!(get_wave_y_pos(0.001, -1.0, 1.0, 101, true, true, 60.0, false), 50);
        assert_eq!(get_wave_y_pos(-0.001, -1.0, 1.0, 101, true, true, 60.0, false), 50);
    }

    #[test]
    fn from_db_inverts_db_display_mapping() {
        let amplitude = 0.1_f32;
        let display_value = (20.0 * amplitude.log10() + 60.0) / 60.0;
        let reconstructed = from_db(display_value, 60.0);
        assert!((reconstructed - amplitude).abs() < 1.0e-6);
        assert!((from_db(-display_value, 60.0) + amplitude).abs() < 1.0e-6);
    }

    #[test]
    fn pixel_value_round_trip_is_within_one_pixel() {
        for db in [false, true] {
            for y in 0..101 {
                let value = value_of_pixel(y, 101, false, db, 60.0, -1.0, 1.0);
                let mapped = get_wave_y_pos(value, -1.0, 1.0, 101, db, true, 60.0, false);
                assert!((mapped - y).abs() <= 1, "db={db}, y={y}, mapped={mapped}");
            }
        }
    }

    #[test]
    fn sample_zoom_thresholds_are_strict() {
        assert!(!show_individual_samples(50.0, 100.0));
        assert!(show_individual_samples(50.001, 100.0));
        assert!(!show_points(300.0, 100.0));
        assert!(show_points(300.001, 100.0));
    }

    #[test]
    fn minimized_bounds_follow_half_wave_setting() {
        assert_eq!(collapsed_bounds(false, true), None);
        assert_eq!(collapsed_bounds(false, false), None);
        assert_eq!(collapsed_bounds(true, true), Some((-0.01, 1.0)));
        assert_eq!(collapsed_bounds(true, false), Some((-1.0, 1.0)));
    }

    #[test]
    fn individual_sample_geometry_is_repeatable() {
        let samples = [0.0, 0.5, -0.5, 1.0, -1.0];
        let a = individual_samples(
            &samples, 0.0, 10.0, 0.0, 100.0, 100, 101, -1.0, 1.0, false, 60.0,
        );
        let b = individual_samples(
            &samples, 0.0, 10.0, 0.0, 100.0, 100, 101, -1.0, 1.0, false, 60.0,
        );
        assert_eq!(a, b);
        assert_eq!(a.xs, vec![0, 10, 20, 30, 40]);
        assert_eq!(a.y_zero, 50);
    }
}
