//! Time and frequency bounds for an Audacity selection.
//!
//! A selected region keeps its time bounds ordered and, when both frequency
//! bounds are defined, keeps those ordered as well. Undefined frequency bounds
//! use [`SelectedRegion::UndefinedFrequency`].

/// A selected portion of a project, with optional spectral-selection bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectedRegion {
    t0: f64,
    t1: f64,
    f0: f64,
    f1: f64,
}

impl Default for SelectedRegion {
    /// Creates a point selection at time zero with undefined frequency bounds.
    fn default() -> Self {
        Self {
            t0: 0.0,
            t1: 0.0,
            f0: Self::UndefinedFrequency,
            f1: Self::UndefinedFrequency,
        }
    }
}

impl SelectedRegion {
    /// Sentinel value for an undefined frequency bound.
    #[allow(non_upper_case_globals)]
    pub const UndefinedFrequency: f64 = -1.0;

    /// Creates a time selection and orders its bounds if necessary.
    pub fn new(t0: f64, t1: f64) -> Self {
        let mut result = Self {
            t0,
            t1,
            ..Self::default()
        };
        result.ensure_ordering();
        result
    }

    /// Returns the start time.
    pub fn t0(&self) -> f64 {
        self.t0
    }

    /// Returns the end time.
    pub fn t1(&self) -> f64 {
        self.t1
    }

    /// Returns the selection duration.
    pub fn duration(&self) -> f64 {
        self.t1 - self.t0
    }

    /// Returns whether the selection is a point or has reversed/equal bounds.
    pub fn is_point(&self) -> bool {
        self.t1 <= self.t0
    }

    /// Returns the low frequency bound.
    pub fn f0(&self) -> f64 {
        self.f0
    }

    /// Returns the high frequency bound.
    pub fn f1(&self) -> f64 {
        self.f1
    }

    /// Returns the geometric centre frequency, or the undefined sentinel if
    /// either bound is undefined.
    pub fn fc(&self) -> f64 {
        if self.f0 == Self::UndefinedFrequency || self.f1 == Self::UndefinedFrequency {
            Self::UndefinedFrequency
        } else {
            (self.f0 * self.f1).sqrt()
        }
    }

    /// Returns the natural-log frequency ratio, or the undefined sentinel if
    /// either bound is undefined.
    pub fn log_bandwidth(&self) -> f64 {
        if self.f0 == Self::UndefinedFrequency || self.f1 == Self::UndefinedFrequency {
            Self::UndefinedFrequency
        } else {
            (self.f1 / self.f0).ln()
        }
    }

    /// Returns the spectral width in octaves, or the undefined sentinel if
    /// either bound is undefined.
    pub fn octaves(&self) -> f64 {
        let bandwidth = self.log_bandwidth();
        if bandwidth == Self::UndefinedFrequency {
            Self::UndefinedFrequency
        } else {
            bandwidth / std::f64::consts::LN_2
        }
    }

    /// Sets the start time. If `may_swap` is false, an inverted interval is
    /// collapsed by moving the end time to the new start.
    ///
    /// Returns true only if the bounds were swapped.
    pub fn set_t0(&mut self, t: f64, may_swap: bool) -> bool {
        self.t0 = t;
        if may_swap {
            self.ensure_ordering()
        } else {
            if self.t1 < self.t0 {
                self.t1 = self.t0;
            }
            false
        }
    }

    /// Sets the end time. If `may_swap` is false, an inverted interval is
    /// collapsed by moving the start time to the new end.
    ///
    /// Returns true only if the bounds were swapped.
    pub fn set_t1(&mut self, t: f64, may_swap: bool) -> bool {
        self.t1 = t;
        if may_swap {
            self.ensure_ordering()
        } else {
            if self.t1 < self.t0 {
                self.t0 = self.t1;
            }
            false
        }
    }

    /// Sets both time bounds, ordering them if necessary.
    ///
    /// Returns true only if the bounds were swapped.
    pub fn set_times(&mut self, t0: f64, t1: f64) -> bool {
        self.t0 = t0;
        self.t1 = t1;
        self.ensure_ordering()
    }

    /// Moves the start time by `delta`, using the same ordering rules as
    /// [`Self::set_t0`].
    pub fn move_t0(&mut self, delta: f64, may_swap: bool) -> bool {
        self.set_t0(self.t0 + delta, may_swap)
    }

    /// Moves the end time by `delta`, using the same ordering rules as
    /// [`Self::set_t1`].
    pub fn move_t1(&mut self, delta: f64, may_swap: bool) -> bool {
        self.set_t1(self.t1 + delta, may_swap)
    }

    /// Moves both time bounds by the same amount.
    pub fn move_by(&mut self, delta: f64) {
        self.t0 += delta;
        self.t1 += delta;
    }

    /// Collapses the selection to its start time.
    pub fn collapse_to_t0(&mut self) {
        self.t1 = self.t0;
    }

    /// Collapses the selection to its end time.
    pub fn collapse_to_t1(&mut self) {
        self.t0 = self.t1;
    }

    /// Sets the low frequency bound. Negative values become undefined. If
    /// `may_swap` is false, an inverted defined range is collapsed at the new
    /// low bound.
    ///
    /// Returns true only if the bounds were swapped.
    pub fn set_f0(&mut self, f: f64, may_swap: bool) -> bool {
        let f = if f < 0.0 { Self::UndefinedFrequency } else { f };
        self.f0 = f;
        if may_swap {
            self.ensure_frequency_ordering()
        } else {
            if self.f1 >= 0.0 && self.f1 < self.f0 {
                self.f1 = self.f0;
            }
            false
        }
    }

    /// Sets the high frequency bound. Negative values become undefined. If
    /// `may_swap` is false, an inverted defined range is collapsed at the new
    /// high bound.
    ///
    /// Returns true only if the bounds were swapped.
    pub fn set_f1(&mut self, f: f64, may_swap: bool) -> bool {
        let f = if f < 0.0 { Self::UndefinedFrequency } else { f };
        self.f1 = f;
        if may_swap {
            self.ensure_frequency_ordering()
        } else {
            if self.f0 >= 0.0 && self.f1 < self.f0 {
                self.f0 = self.f1;
            }
            false
        }
    }

    /// Sets both frequency bounds and orders them if both are defined.
    ///
    /// Returns true only if the bounds were swapped.
    pub fn set_frequencies(&mut self, f0: f64, f1: f64) -> bool {
        self.f0 = f0;
        self.f1 = f1;
        self.ensure_frequency_ordering()
    }

    /// Changes the geometric centre frequency while preserving the current
    /// spectral width in octaves. Does nothing when either bound is undefined.
    pub fn set_fc(&mut self, fc: f64) {
        if self.f0 == Self::UndefinedFrequency || self.f1 == Self::UndefinedFrequency {
            return;
        }
        let half_width = self.octaves() / 2.0;
        let factor = 2.0_f64.powf(half_width);
        self.set_frequencies(fc / factor, fc * factor);
    }

    /// Sets the spectral width in octaves while preserving the geometric
    /// centre frequency. Does nothing when either bound is undefined.
    pub fn set_bandwidth(&mut self, bandwidth_octaves: f64) {
        if self.f0 == Self::UndefinedFrequency || self.f1 == Self::UndefinedFrequency {
            return;
        }
        let centre = self.fc();
        let factor = 2.0_f64.powf(bandwidth_octaves / 2.0);
        self.set_frequencies(centre / factor, centre * factor);
    }

    /// Sets the spectral width expressed as a natural-log frequency ratio,
    /// preserving the geometric centre frequency.
    pub fn set_log_bandwidth(&mut self, bandwidth: f64) {
        if self.f0 == Self::UndefinedFrequency || self.f1 == Self::UndefinedFrequency {
            return;
        }
        let centre = self.fc();
        let factor = (bandwidth / 2.0).exp();
        self.set_frequencies(centre / factor, centre * factor);
    }

    /// Writes the selection attributes used by Audacity project files.
    ///
    /// Time attributes are always present. Frequency attributes are included
    /// independently when their corresponding bounds are defined.
    pub fn write_attrs(&self) -> Vec<(&'static str, f64)> {
        let mut attrs = vec![("sel0", self.t0), ("sel1", self.t1)];
        if self.f0 >= 0.0 {
            attrs.push(("selLow", self.f0));
        }
        if self.f1 >= 0.0 {
            attrs.push(("selHigh", self.f1));
        }
        attrs
    }

    /// Handles one Audacity project-file selection attribute.
    ///
    /// Returns true when `name` is recognized. The time and frequency setters
    /// use non-swapping behavior, matching Audacity's XML attribute handling.
    pub fn handle_attr(&mut self, name: &str, value: f64) -> bool {
        match name {
            "sel0" => {
                self.set_t0(value, false);
                true
            }
            "sel1" => {
                self.set_t1(value, false);
                true
            }
            "selLow" => {
                self.set_f0(value, false);
                true
            }
            "selHigh" => {
                self.set_f1(value, false);
                true
            }
            _ => false,
        }
    }

    /// Orders the time bounds and returns true if they were swapped.
    pub fn ensure_ordering(&mut self) -> bool {
        if self.t1 < self.t0 {
            std::mem::swap(&mut self.t0, &mut self.t1);
            true
        } else {
            false
        }
    }

    fn ensure_frequency_ordering(&mut self) -> bool {
        if self.f1 < 0.0 {
            self.f1 = Self::UndefinedFrequency;
        }
        if self.f0 < 0.0 {
            self.f0 = Self::UndefinedFrequency;
        }

        if self.f0 != Self::UndefinedFrequency
            && self.f1 != Self::UndefinedFrequency
            && self.f1 < self.f0
        {
            std::mem::swap(&mut self.f0, &mut self.f1);
            true
        } else {
            false
        }
    }

    /// C++-style alias for [`Self::set_t0`].
    #[allow(non_snake_case)]
    pub fn setT0(&mut self, t: f64, may_swap: bool) -> bool {
        self.set_t0(t, may_swap)
    }

    /// C++-style alias for [`Self::set_t1`].
    #[allow(non_snake_case)]
    pub fn setT1(&mut self, t: f64, may_swap: bool) -> bool {
        self.set_t1(t, may_swap)
    }

    /// C++-style alias for [`Self::set_times`].
    #[allow(non_snake_case)]
    pub fn setTimes(&mut self, t0: f64, t1: f64) -> bool {
        self.set_times(t0, t1)
    }

    /// C++-style alias for [`Self::move_t0`].
    #[allow(non_snake_case)]
    pub fn moveT0(&mut self, delta: f64, may_swap: bool) -> bool {
        self.move_t0(delta, may_swap)
    }

    /// C++-style alias for [`Self::move_t1`].
    #[allow(non_snake_case)]
    pub fn moveT1(&mut self, delta: f64, may_swap: bool) -> bool {
        self.move_t1(delta, may_swap)
    }

    /// C++-style alias for [`Self::set_f0`].
    #[allow(non_snake_case)]
    pub fn setF0(&mut self, f: f64, may_swap: bool) -> bool {
        self.set_f0(f, may_swap)
    }

    /// C++-style alias for [`Self::set_f1`].
    #[allow(non_snake_case)]
    pub fn setF1(&mut self, f: f64, may_swap: bool) -> bool {
        self.set_f1(f, may_swap)
    }

    /// C++-style alias for [`Self::set_frequencies`].
    #[allow(non_snake_case)]
    pub fn setFrequencies(&mut self, f0: f64, f1: f64) -> bool {
        self.set_frequencies(f0, f1)
    }

    /// C++-style alias for [`Self::set_fc`].
    #[allow(non_snake_case)]
    pub fn setFc(&mut self, fc: f64) {
        self.set_fc(fc);
    }

    /// C++-style alias for [`Self::set_bandwidth`].
    #[allow(non_snake_case)]
    pub fn setBandwidth(&mut self, bandwidth_octaves: f64) {
        self.set_bandwidth(bandwidth_octaves);
    }

    /// C++-style alias for [`Self::log_bandwidth`].
    #[allow(non_snake_case)]
    pub fn logBandwidth(&self) -> f64 {
        self.log_bandwidth()
    }

    /// C++-style alias for [`Self::is_point`].
    #[allow(non_snake_case)]
    pub fn isPoint(&self) -> bool {
        self.is_point()
    }
}

#[cfg(test)]
mod tests {
    use super::SelectedRegion;

    #[test]
    fn time_ordering_and_non_swapping_setters() {
        let mut region = SelectedRegion::new(4.0, 1.0);
        assert_eq!((region.t0(), region.t1()), (1.0, 4.0));

        assert!(region.set_times(8.0, 3.0));
        assert_eq!((region.t0(), region.t1()), (3.0, 8.0));

        assert!(!region.set_t0(10.0, false));
        assert_eq!((region.t0(), region.t1()), (10.0, 10.0));

        region.set_times(2.0, 5.0);
        assert!(!region.set_t1(1.0, false));
        assert_eq!((region.t0(), region.t1()), (1.0, 1.0));
    }

    #[test]
    fn frequency_ordering_and_undefined_bounds() {
        let mut region = SelectedRegion::default();
        assert_eq!(region.fc(), SelectedRegion::UndefinedFrequency);
        assert_eq!(region.octaves(), SelectedRegion::UndefinedFrequency);

        assert!(region.set_frequencies(800.0, 200.0));
        assert_eq!((region.f0(), region.f1()), (200.0, 800.0));
        assert_eq!(region.fc(), 400.0);

        region.set_f0(-5.0, true);
        assert_eq!(region.f0(), SelectedRegion::UndefinedFrequency);
        assert_eq!(region.f1(), 800.0);

        region.set_frequencies(-2.0, -3.0);
        assert_eq!(
            (region.f0(), region.f1()),
            (
                SelectedRegion::UndefinedFrequency,
                SelectedRegion::UndefinedFrequency
            )
        );
    }

    #[test]
    fn centre_and_bandwidth_helpers() {
        let mut region = SelectedRegion::default();
        region.set_frequencies(100.0, 400.0);
        assert_eq!(region.fc(), 200.0);
        assert!((region.octaves() - 2.0).abs() < 1.0e-12);

        region.set_fc(400.0);
        assert!((region.fc() - 400.0).abs() < 1.0e-12);
        assert!((region.octaves() - 2.0).abs() < 1.0e-12);

        region.set_bandwidth(1.0);
        assert!((region.fc() - 400.0).abs() < 1.0e-12);
        assert!((region.octaves() - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn xml_attributes_round_trip() {
        let mut region = SelectedRegion::new(1.25, 3.5);
        region.set_frequencies(120.0, 960.0);

        let attrs = region.write_attrs();
        assert_eq!(
            attrs,
            vec![
                ("sel0", 1.25),
                ("sel1", 3.5),
                ("selLow", 120.0),
                ("selHigh", 960.0)
            ]
        );

        let mut restored = SelectedRegion::default();
        for (name, value) in attrs {
            assert!(restored.handle_attr(name, value));
        }
        assert_eq!(restored, region);
        assert!(!restored.handle_attr("unknown", 7.0));

        let times_only = SelectedRegion::default();
        assert_eq!(
            times_only.write_attrs(),
            vec![("sel0", 0.0), ("sel1", 0.0)]
        );
    }
}
