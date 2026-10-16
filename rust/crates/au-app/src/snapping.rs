//! Port of Audacity 3.7.3's au3-snapping (SnapUtils.cpp, Time/Frame/BeatsSnapFunctions.cpp,
//! ProjectSnap.cpp): the snap-function registry, Snap-To Nearest/Prior and SingleStep,
//! plus the `/SelectionFormat` -> `/Snap/To` migration (DeduceSnapTo).

/// Project values the project-dependent snap functions read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapContext {
    pub rate: f64,
    pub tempo: f64,
    pub upper: f64,
    pub lower: f64,
}

impl Default for SnapContext {
    fn default() -> Self {
        Self { rate: 44100.0, tempo: 120.0, upper: 4.0, lower: 4.0 }
    }
}

/// Default of SnapToSetting.
pub const DEFAULT_SNAP_TO: &str = "bar_1_8";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Nearest,
    Prior,
}

enum Mult {
    /// ConstantMultiplierSnapItem.
    Const(f64),
    /// ProjectDependentMultiplierSnapItem.
    Dyn(fn(&SnapContext) -> f64),
}

fn quarter(c: &SnapContext) -> f64 {
    60.0 / c.tempo
}
fn bar(c: &SnapContext) -> f64 {
    1.0 / (quarter(c) * 4.0 / c.lower * c.upper)
}
fn beat(c: &SnapContext, div: f64) -> f64 {
    1.0 / (quarter(c) * 4.0 / div)
}
fn triplet(c: &SnapContext, div: i32) -> f64 {
    let t = (3 * (div / 2)) as f64;
    1.0 / (60.0 / c.tempo * 4.0 / t)
}

/// (id, label, multiplier) in registry order "beats,triplets,time,video,cd".
fn lookup(id: &str) -> Option<Mult> {
    use Mult::*;
    Some(match id {
        "bar" => Dyn(bar),
        "bar_1_2" => Dyn(|c| beat(c, 2.0)),
        "bar_1_4" => Dyn(|c| beat(c, 4.0)),
        "bar_1_8" => Dyn(|c| beat(c, 8.0)),
        "bar_1_16" => Dyn(|c| beat(c, 16.0)),
        "bar_1_32" => Dyn(|c| beat(c, 32.0)),
        "bar_1_64" => Dyn(|c| beat(c, 64.0)),
        "bar_1_128" => Dyn(|c| beat(c, 128.0)),
        "triplet_1_2" => Dyn(|c| triplet(c, 2)),
        "triplet_1_4" => Dyn(|c| triplet(c, 4)),
        "triplet_1_8" => Dyn(|c| triplet(c, 8)),
        "triplet_1_16" => Dyn(|c| triplet(c, 16)),
        "triplet_1_32" => Dyn(|c| triplet(c, 32)),
        "triplet_1_64" => Dyn(|c| triplet(c, 64)),
        "triplet_1_128" => Dyn(|c| triplet(c, 128)),
        "seconds" => Const(1.0),
        "deciseconds" => Const(10.0),
        "centiseconds" => Const(100.0),
        "milliseconds" => Const(1000.0),
        "samples" => Dyn(|c| c.rate),
        "film_24_fps" => Const(24.0),
        // Audacity registers both NTSC entries with 30/1.001.
        "ntsc_29.97_fps" | "ntsc_30_fps" => Const(30.0 / 1.001),
        "film_25_fps" => Const(25.0),
        "cd_75_fps" => Const(75.0),
        _ => return None,
    })
}

pub fn is_known(id: &str) -> bool {
    lookup(id).is_some() || id == "cd_75_fps"
}

/// SnapWithMultiplier. Returns (time, snapped).
fn snap_with_multiplier(value: f64, m: f64, nearest: bool) -> (f64, bool) {
    if m <= 0.0 {
        return (value, false);
    }
    let r = if nearest { (value * m).round() / m } else { (value * m).floor() / m };
    (r, true)
}

/// SnapFunctionsRegistry::Snap.
pub fn snap(id: &str, c: &SnapContext, time: f64, nearest: bool) -> (f64, bool) {
    match lookup(id) {
        None => (time, false),
        Some(Mult::Const(m)) => snap_with_multiplier(time, m, nearest),
        Some(Mult::Dyn(f)) => snap_with_multiplier(time, f(c), nearest),
    }
}

/// SnapFunctionsRegistry::SingleStep.
pub fn single_step(id: &str, c: &SnapContext, time: f64, upwards: bool) -> (f64, bool) {
    match lookup(id) {
        None => (time, false),
        Some(Mult::Const(m)) => {
            let r = time + (if upwards { 1.0 } else { -1.0 }) / m;
            if r < 0.0 {
                return (0.0, false);
            }
            snap_with_multiplier(r, m, true)
        }
        Some(Mult::Dyn(f)) => {
            let m = f(c);
            let eps = time.max(1.0) * f64::EPSILON;
            let current = (time * (1.0 + eps) * m).floor() as i32;
            let next = if upwards { current + 1 } else { current - 1 };
            let mut r = next as f64 / m;
            if r < 0.0 {
                return (0.0, false);
            }
            while ((r * m).floor() as i32) < next {
                r += eps;
            }
            while ((r * m).floor() as i32) > next {
                r -= eps;
            }
            (r, true)
        }
    }
}

/// ProjectSnap::SnapTime.
pub fn snap_time(mode: Mode, id: &str, c: &SnapContext, time: f64) -> (f64, bool) {
    match mode {
        Mode::Off => (time, false),
        m => snap(id, c, time, m == Mode::Nearest),
    }
}

/// ProjectSnap::SingleStep.
pub fn project_single_step(mode: Mode, id: &str, c: &SnapContext, time: f64, up: bool) -> (f64, bool) {
    match mode {
        Mode::Off => (time, false),
        _ => single_step(id, c, time, up),
    }
}

/// SnapUtils DeduceSnapTo: maps an old `/SelectionFormat` to a snap id.
pub fn deduce_snap_to(selection_format: Option<&str>) -> &'static str {
    let Some(f) = selection_format else { return DEFAULT_SNAP_TO };
    match f {
        "seconds" | "hh:mm:ss" | "dd:hh:mm:ss" => "seconds",
        "seconds + milliseconds" | "hh:mm:ss + milliseconds" => "milliseconds",
        "hh:mm:ss + hundredths" => "centiseconds",
        "hh:mm:ss + samples" | "samples" => "samples",
        "hh:mm:ss + film frames (24 fps)" | "film frames (24 fps)" => "film_24_fps",
        "hh:mm:ss + NTSC drop frames" | "NTSC frames" => "ntsc_29.97_fps",
        "hh:mm:ss + NTSC non-drop frames" => "ntsc_30_fps",
        "hh:mm:ss + PAL frames (25 fps)" | "PAL frames (25 fps)" => "film_25_fps",
        "hh:mm:ss + CDDA frames (75 fps)" | "CDDA frames (75 fps)" => "cd_75_fps",
        _ => DEFAULT_SNAP_TO,
    }
}

/// ReadSnapMode: `/Snap/Mode` (OFF/NEAREST/PRIOR), else legacy `/SnapTo` integer.
pub fn parse_mode(snap_mode: Option<&str>, legacy: Option<i64>) -> Mode {
    match snap_mode {
        Some("NEAREST") => Mode::Nearest,
        Some("PRIOR") => Mode::Prior,
        Some(_) => Mode::Off,
        None => match legacy {
            Some(1) => Mode::Nearest,
            Some(2) => Mode::Prior,
            _ => Mode::Off,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }
    const C: SnapContext = SnapContext { rate: 44100.0, tempo: 120.0, upper: 4.0, lower: 4.0 };

    // Mirrors au3-snapping/tests/SnappingTest.cpp expectations.
    #[test]
    fn nearest_and_prior() {
        close(snap("seconds", &C, 1.6, true).0, 2.0);
        close(snap("seconds", &C, 1.6, false).0, 1.0);
        close(snap("milliseconds", &C, 0.0016, true).0, 0.002);
        close(snap("bar", &C, 2.9, true).0, 2.0); // 120 bpm 4/4: bar = 2 s
        close(snap("bar_1_8", &C, 0.3, false).0, 0.25);
        close(snap("triplet_1_4", &C, 0.4, true).0, 1.0 / 3.0);
        close(snap("samples", &C, 1.5 / 44100.0, false).0, 1.0 / 44100.0);
        close(snap("cd_75_fps", &C, 0.019, true).0, 1.0 / 75.0);
        close(snap("ntsc_30_fps", &C, 1.0, true).0, 30.0 / (30.0 / 1.001));
        assert!(!snap("bogus", &C, 1.0, true).1);
        assert_eq!(snap_time(Mode::Off, "seconds", &C, 1.4), (1.4, false));
    }

    // SnappingTest.cpp BeatsCase at 60 bpm.
    #[test]
    fn audacity_beats_cases() {
        for (id, t, want, up) in [
            ("bar_1_2", 0.5, 0.0, 3.0), ("bar_1_2", 1.0, 2.0, 3.0), ("bar_1_2", 3.0, 4.0, 4.0),
            ("bar_1_4", 0.5, 1.0, 3.0), ("bar_1_4", 3.0, 3.0, 4.0),
        ] {
            let c = SnapContext { rate: 44100.0, tempo: 60.0, upper: up, lower: 4.0 };
            close(snap(id, &c, t, true).0, want);
        }
    }

    #[test]
    fn single_steps() {
        close(single_step("seconds", &C, 1.2, true).0, 2.0);
        close(single_step("seconds", &C, 1.2, false).0, 0.0);
        assert_eq!(single_step("seconds", &C, 0.5, false), (0.0, false));
        let (t, ok) = single_step("bar_1_4", &C, 0.5, true);
        assert!(ok);
        assert_eq!((t * 2.0).floor() as i32, 2);
        let (t, _) = single_step("bar_1_4", &C, 1.0, false);
        assert_eq!((t * 2.0).floor() as i32, 1);
    }

    #[test]
    fn migration_and_modes() {
        assert_eq!(deduce_snap_to(None), "bar_1_8");
        assert_eq!(deduce_snap_to(Some("hh:mm:ss + hundredths")), "centiseconds");
        assert_eq!(deduce_snap_to(Some("NTSC frames")), "ntsc_29.97_fps");
        assert_eq!(deduce_snap_to(Some("weird")), "bar_1_8");
        assert_eq!(parse_mode(Some("PRIOR"), None), Mode::Prior);
        assert_eq!(parse_mode(None, Some(1)), Mode::Nearest);
        assert!(is_known("cd_75_fps"));
    }
}
