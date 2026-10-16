use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Scales captured samples by a recording gain in the range `0.0..=1.0`.
///
/// Samples are not clipped. A unity gain leaves the buffer bit-identical.
pub fn apply_input_gain(buf: &mut [f32], gain: f32) {
    let gain = clamp_gain(gain);
    if gain == 1.0 {
        return;
    }
    if gain == 0.0 {
        buf.fill(0.0);
        return;
    }
    for sample in buf {
        *sample *= gain;
    }
}

fn clamp_gain(gain: f32) -> f32 {
    if gain.is_nan() {
        0.0
    } else {
        gain.clamp(0.0, 1.0)
    }
}

/// A lock-free, shareable software input-gain value.
#[derive(Clone, Debug)]
pub struct InputGain {
    inner: Arc<AtomicU32>,
}

impl InputGain {
    /// Creates a gain handle initialized to `gain`, clamped to `0.0..=1.0`.
    pub fn new(gain: f32) -> Self {
        Self {
            inner: Arc::new(AtomicU32::new(clamp_gain(gain).to_bits())),
        }
    }

    /// Sets the shared gain, clamped to `0.0..=1.0`.
    pub fn set(&self, gain: f32) {
        self.inner
            .store(clamp_gain(gain).to_bits(), Ordering::Relaxed);
    }

    /// Returns the current shared gain.
    pub fn get(&self) -> f32 {
        f32::from_bits(self.inner.load(Ordering::Relaxed))
    }
}

impl Default for InputGain {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// Indicates whether recording gain is controlled by the OS mixer or software.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MixerKind {
    /// Gain is controlled by an input mixer.
    Os,
    /// Gain is applied to captured samples in software.
    Software,
}

/// Interface for an operating-system input-volume mixer.
pub trait InputMixer: Send {
    /// Sets the mixer volume and returns whether the operation succeeded.
    fn set_volume(&mut self, v: f32) -> bool;

    /// Returns the current mixer volume when it can be read.
    fn volume(&self) -> Option<f32>;
}

/// A PulseAudio/PipeWire source mixer controlled through the `pactl` CLI.
pub struct PulseSourceMixer {
    source: String,
}

impl PulseSourceMixer {
    /// Detects a usable `pactl` source mixer.
    ///
    /// If `source` is `None`, the default source (`@DEFAULT_SOURCE@`) is used.
    /// Returns `None` if the platform is not Linux or the source volume cannot
    /// be queried successfully.
    pub fn detect(source: Option<&str>) -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            let source = source.unwrap_or("@DEFAULT_SOURCE@").to_owned();
            let output = std::process::Command::new("pactl")
                .arg("get-source-volume")
                .arg(&source)
                .output()
                .ok()?;
            if output.status.success() {
                Some(Self { source })
            } else {
                None
            }
        }

        #[cfg(not(target_os = "linux"))]
        {
            let _ = source;
            None
        }
    }
}

impl InputMixer for PulseSourceMixer {
    fn set_volume(&mut self, v: f32) -> bool {
        let percent = (clamp_gain(v) * 100.0).round() as u32;
        std::process::Command::new("pactl")
            .arg("set-source-volume")
            .arg(&self.source)
            .arg(format!("{percent}%"))
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    fn volume(&self) -> Option<f32> {
        let output = std::process::Command::new("pactl")
            .arg("get-source-volume")
            .arg(&self.source)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        parse_pactl_volume(&String::from_utf8_lossy(&output.stdout))
    }
}

/// Parses the first percentage in `pactl get-source-volume` output as a
/// fraction, such that `50%` becomes `0.5`.
pub fn parse_pactl_volume(s: &str) -> Option<f32> {
    for (percent_index, character) in s.char_indices() {
        if character != '%' {
            continue;
        }

        let mut start = percent_index;
        while start > 0 {
            let byte = s.as_bytes()[start - 1];
            if byte.is_ascii_digit() || matches!(byte, b'.' | b'+' | b'-') {
                start -= 1;
            } else {
                break;
            }
        }

        if start == percent_index {
            continue;
        }

        if let Ok(percent) = s[start..percent_index].parse::<f32>() {
            if percent.is_finite() {
                return Some(percent / 100.0);
            }
        }
    }
    None
}

/// Coordinates OS mixer control and software input-gain fallback.
pub struct RecordVolume {
    gain: InputGain,
    mixer: Option<Box<dyn InputMixer>>,
    last_value: f32,
}

impl RecordVolume {
    /// Creates recording-volume control, initially set to unity gain.
    pub fn new(mixer: Option<Box<dyn InputMixer>>) -> Self {
        Self {
            gain: InputGain::default(),
            mixer,
            last_value: 1.0,
        }
    }

    /// Returns the active mixer kind.
    pub fn kind(&self) -> MixerKind {
        if self.mixer.is_some() {
            MixerKind::Os
        } else {
            MixerKind::Software
        }
    }

    /// Sets recording volume, falling back permanently to software if the
    /// mixer is absent or cannot set the requested value.
    pub fn set(&mut self, v: f32) {
        let v = clamp_gain(v);
        self.last_value = v;

        if let Some(mixer) = self.mixer.as_mut() {
            if mixer.set_volume(v) {
                self.gain.set(1.0);
                return;
            }
            self.mixer = None;
        }

        self.gain.set(v);
    }

    /// Returns a cloneable gain handle for use by the audio callback.
    pub fn software_gain(&self) -> InputGain {
        self.gain.clone()
    }

    /// Returns the readable OS mixer volume, or the last value set when it
    /// cannot be read.
    pub fn get(&self) -> f32 {
        self.mixer
            .as_ref()
            .and_then(|mixer| mixer.volume())
            .unwrap_or(self.last_value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn unity_gain_is_bit_identical() {
        let mut samples = [0.0, -0.0, 0.25, -1.5, f32::INFINITY, f32::NAN];
        let original: Vec<u32> = samples.iter().map(|sample| sample.to_bits()).collect();

        apply_input_gain(&mut samples, 1.0);

        let result: Vec<u32> = samples.iter().map(|sample| sample.to_bits()).collect();
        assert_eq!(result, original);
    }

    #[test]
    fn half_gain_scales_samples() {
        let mut samples = [2.0, -1.0, 0.5];
        apply_input_gain(&mut samples, 0.5);
        assert_eq!(samples, [1.0, -0.5, 0.25]);
    }

    #[test]
    fn zero_gain_silences_samples() {
        let mut samples = [1.0, -2.0, f32::NAN, f32::INFINITY];
        apply_input_gain(&mut samples, 0.0);
        assert_eq!(samples, [0.0; 4]);
    }

    #[test]
    fn gain_is_clamped() {
        let mut above = [0.5];
        apply_input_gain(&mut above, 2.0);
        assert_eq!(above, [0.5]);

        let mut below = [0.5];
        apply_input_gain(&mut below, -1.0);
        assert_eq!(below, [0.0]);

        assert_eq!(InputGain::new(2.0).get(), 1.0);
        assert_eq!(InputGain::new(-1.0).get(), 0.0);
    }

    #[test]
    fn input_gain_is_shared_across_clones_and_threads() {
        let gain = InputGain::default();
        let cloned = gain.clone();

        let thread = thread::spawn(move || cloned.set(0.375));
        thread.join().unwrap();

        assert_eq!(gain.get(), 0.375);
    }

    #[test]
    fn parses_pactl_volume() {
        let output = "Volume: front-left: 32768 /  50% / -18.06 dB,   front-right: 32768 /  50% / -18.06 dB\n        balance 0.00";
        assert_eq!(parse_pactl_volume(output), Some(0.5));
        assert_eq!(parse_pactl_volume("garbage"), None);
    }

    struct MockMixer {
        succeeds: bool,
        current: f32,
    }

    impl InputMixer for MockMixer {
        fn set_volume(&mut self, v: f32) -> bool {
            if self.succeeds {
                self.current = v;
                true
            } else {
                false
            }
        }

        fn volume(&self) -> Option<f32> {
            Some(self.current)
        }
    }

    #[test]
    fn record_volume_uses_successful_mixer() {
        let mixer = MockMixer {
            succeeds: true,
            current: 1.0,
        };
        let mut volume = RecordVolume::new(Some(Box::new(mixer)));

        volume.set(0.4);

        assert_eq!(volume.kind(), MixerKind::Os);
        assert_eq!(volume.software_gain().get(), 1.0);
        assert_eq!(volume.get(), 0.4);
    }

    #[test]
    fn record_volume_falls_back_after_mixer_failure() {
        let mixer = MockMixer {
            succeeds: false,
            current: 1.0,
        };
        let mut volume = RecordVolume::new(Some(Box::new(mixer)));

        volume.set(0.4);

        assert_eq!(volume.kind(), MixerKind::Software);
        assert_eq!(volume.software_gain().get(), 0.4);
        assert_eq!(volume.get(), 0.4);
    }

    #[test]
    fn record_volume_without_mixer_uses_software() {
        let mut volume = RecordVolume::new(None);

        assert_eq!(volume.kind(), MixerKind::Software);
        assert_eq!(volume.get(), 1.0);

        volume.set(0.25);

        assert_eq!(volume.software_gain().get(), 0.25);
        assert_eq!(volume.get(), 0.25);
    }
}
