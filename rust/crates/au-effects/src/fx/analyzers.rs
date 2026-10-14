use crate::core::*;
use au_core::AudioBuffer;

/// Algorithms available to Plot Spectrum.
pub const SPECTRUM_ALGORITHMS: &[&str] = &[
    "Spectrum",
    "Standard Autocorrelation",
    "Cuberoot Autocorrelation",
    "Enhanced Autocorrelation",
    "Cepstrum",
];
/// Windows available to Plot Spectrum.
pub const SPECTRUM_WINDOWS: &[&str] = &[
    "Rectangular",
    "Bartlett",
    "Hamming",
    "Hann",
    "Blackman",
    "Blackman-Harris",
    "Welch",
    "Gaussian (a=2.5)",
    "Gaussian (a=3.5)",
    "Gaussian (a=4.5)",
];
const AXES: &[&str] = &["Linear", "Logarithmic"];

/// Result of measuring foreground and background contrast.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContrastResult {
    /// Foreground RMS level in dBFS.
    pub fg_rms_db: f64,
    /// Background RMS level in dBFS.
    pub bg_rms_db: f64,
    /// Foreground minus background level in dB.
    pub diff_db: f64,
    /// Whether the difference meets the WCAG 20 dB threshold.
    pub pass_wcag: bool,
}

fn rms_db(channels: &[Vec<f32>]) -> f64 {
    let count: usize = channels.iter().map(Vec::len).sum();
    let sum: f64 = channels
        .iter()
        .flat_map(|c| c.iter())
        .map(|&x| (x as f64) * (x as f64))
        .sum();
    let rms = if count == 0 { 0.0 } else { (sum / count as f64).sqrt() };
    if rms == 0.0 { f64::NEG_INFINITY } else { 20.0 * rms.log10() }
}

/// Plot Spectrum analyzer.
#[derive(Clone, Debug)]
pub struct PlotSpectrum {
    /// Spectrum algorithm index.
    pub algorithm: usize,
    /// Window function index.
    pub window: usize,
    /// FFT size.
    pub size: usize,
    /// Frequency axis index.
    pub axis: usize,
}

impl Default for PlotSpectrum {
    fn default() -> Self {
        Self { algorithm: 0, window: 3, size: 1024, axis: 1 }
    }
}

impl PlotSpectrum {
    fn window_value(&self, i: usize, n: usize) -> f64 {
        if n <= 1 {
            return 1.0;
        }
        let x = i as f64 / (n - 1) as f64;
        let phase = std::f64::consts::TAU * x;
        match self.window {
            0 => 1.0,
            1 => 1.0 - (2.0 * x - 1.0).abs(),
            2 => 0.54 - 0.46 * phase.cos(),
            3 => 0.5 - 0.5 * phase.cos(),
            4 => 0.42 - 0.5 * phase.cos() + 0.08 * (2.0 * phase).cos(),
            5 => {
                0.35875 - 0.48829 * phase.cos() + 0.14128 * (2.0 * phase).cos()
                    - 0.01168 * (3.0 * phase).cos()
            }
            6 => 1.0 - (2.0 * x - 1.0).powi(2),
            7..=9 => {
                let a = [2.5, 3.5, 4.5][self.window - 7];
                let z = a * (2.0 * x - 1.0);
                (-0.5 * z * z).exp()
            }
            _ => 1.0,
        }
    }

    /// Computes one dB value per non-negative frequency bin for a mono mix.
    pub fn compute(&self, buf: &AudioBuffer) -> Vec<f32> {
        let n = self.size.clamp(128, 65536);
        let frame_count = buf.channels.iter().map(Vec::len).min().unwrap_or(0);
        let mono: Vec<f64> = (0..frame_count)
            .map(|i| {
                if buf.channels.is_empty() {
                    0.0
                } else {
                    buf.channels.iter().map(|c| c[i] as f64).sum::<f64>()
                        / buf.channels.len() as f64
                }
            })
            .collect();
        if mono.is_empty() {
            return vec![f32::NEG_INFINITY; n / 2 + 1];
        }

        let hop = (n / 2).max(1);
        let mut starts = Vec::new();
        if mono.len() <= n {
            starts.push(0);
        } else {
            let mut start = 0;
            while start + n <= mono.len() {
                starts.push(start);
                start += hop;
            }
            if starts.last().copied().unwrap_or(0) + n < mono.len() {
                starts.push(mono.len() - n);
            }
        }

        let mut powers = vec![0.0f64; n / 2 + 1];
        for start in &starts {
            for k in 0..=n / 2 {
                let mut re = 0.0;
                let mut im = 0.0;
                for j in 0..n {
                    let sample = mono.get(start + j).copied().unwrap_or(0.0)
                        * self.window_value(j, n);
                    let phase = std::f64::consts::TAU * k as f64 * j as f64 / n as f64;
                    re += sample * phase.cos();
                    im -= sample * phase.sin();
                }
                powers[k] += re * re + im * im;
            }
        }
        let frames = starts.len().max(1) as f64;
        powers
            .into_iter()
            .map(|power| {
                // Audacity's plotted spectrum is normalized by the windowed FFT size.
                let value = (power / (frames * (n as f64).powi(2))).sqrt();
                if value == 0.0 { f32::NEG_INFINITY } else { (20.0 * value.log10()) as f32 }
            })
            .collect()
    }

    /// Formats spectrum data as tab-separated frequency and dB columns.
    pub fn export_text(&self, data: &[f32], sr: u32) -> String {
        let n = self.size.clamp(128, 65536);
        let mut out = String::from("Frequency (Hz)\tLevel (dB)\n");
        for (i, value) in data.iter().enumerate() {
            let hz = i as f64 * sr as f64 / n as f64;
            out.push_str(&format!("{hz:.6}\t{value:.6}\n"));
        }
        out
    }
}

impl BuiltinEffect for PlotSpectrum {
    fn id(&self) -> &'static str { "PlotSpectrum" }
    fn kind(&self) -> EffectKind { EffectKind::Analyze }
    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::choice("Algorithm", "Algorithm", SPECTRUM_ALGORITHMS, 0),
            ParamDef::choice("Window", "Window", SPECTRUM_WINDOWS, 3),
            ParamDef::int("Size", "Size", 128.0, 65536.0, 1024.0),
            ParamDef::choice("Axis", "Axis", AXES, 1),
        ]
    }
    fn get(&self, key: &str) -> Option<f64> {
        match key { "Algorithm" => Some(self.algorithm as f64), "Window" => Some(self.window as f64),
            "Size" => Some(self.size as f64), "Axis" => Some(self.axis as f64), _ => None }
    }
    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Algorithm" => self.algorithm = value.round().clamp(0.0, 4.0) as usize,
            "Window" => self.window = value.round().clamp(0.0, 9.0) as usize,
            "Size" => self.size = value.round().clamp(128.0, 65536.0) as usize,
            "Axis" => self.axis = value.round().clamp(0.0, 1.0) as usize,
            _ => return false,
        }
        true
    }
    fn process(&mut self, buf: &mut AudioBuffer, ctx: &mut EffectContext) -> Result<(), EffectError> {
        let data = self.compute(buf);
        ctx.report = self.export_text(&data, buf.sample_rate);
        Ok(())
    }
}

/// Contrast analyzer.
#[derive(Clone, Debug, Default)]
pub struct Contrast;

impl Contrast {
    /// Measures RMS contrast between two buffers.
    pub fn contrast(fg: &AudioBuffer, bg: &AudioBuffer) -> ContrastResult {
        let fg_rms_db = rms_db(&fg.channels);
        let bg_rms_db = rms_db(&bg.channels);
        let diff_db = fg_rms_db - bg_rms_db;
        ContrastResult { fg_rms_db, bg_rms_db, diff_db, pass_wcag: diff_db >= 20.0 }
    }
}

impl BuiltinEffect for Contrast {
    fn id(&self) -> &'static str { "Contrast" }
    fn kind(&self) -> EffectKind { EffectKind::Analyze }
    fn params(&self) -> Vec<ParamDef> { vec![] }
    fn get(&self, _: &str) -> Option<f64> { None }
    fn set(&mut self, _: &str, _: f64) -> bool { false }
    fn process(&mut self, buf: &mut AudioBuffer, ctx: &mut EffectContext) -> Result<(), EffectError> {
        let bg = ctx.aux.ok_or_else(|| EffectError::Invalid("Contrast requires a background buffer".into()))?;
        let r = Self::contrast(buf, bg);
        ctx.report = format!(
            "Foreground: {:.2} dB\nBackground: {:.2} dB\nDifference: {:.2} dB\n{}",
            r.fg_rms_db, r.bg_rms_db, r.diff_db,
            if r.pass_wcag { "Passes WCAG" } else { "Fails WCAG" }
        );
        Ok(())
    }
}

/// Find Clipping analyzer.
#[derive(Clone, Debug)]
pub struct FindClipping {
    /// Required consecutive clipped samples.
    pub duty_cycle_start: usize,
    /// Required consecutive non-clipped samples to finish a run.
    pub duty_cycle_end: usize,
}
impl Default for FindClipping {
    fn default() -> Self { Self { duty_cycle_start: 3, duty_cycle_end: 3 } }
}
impl BuiltinEffect for FindClipping {
    fn id(&self) -> &'static str { "FindClipping" }
    fn kind(&self) -> EffectKind { EffectKind::Analyze }
    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int("Duty Cycle Start", "Duty Cycle Start", 1.0, i32::MAX as f64, 3.0),
            ParamDef::int("Duty Cycle End", "Duty Cycle End", 1.0, i32::MAX as f64, 3.0),
        ]
    }
    fn get(&self, key: &str) -> Option<f64> {
        match key { "Duty Cycle Start" => Some(self.duty_cycle_start as f64),
            "Duty Cycle End" => Some(self.duty_cycle_end as f64), _ => None }
    }
    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Duty Cycle Start" => self.duty_cycle_start = value.max(1.0) as usize,
            "Duty Cycle End" => self.duty_cycle_end = value.max(1.0) as usize,
            _ => return false,
        }
        true
    }
    fn process(&mut self, buf: &mut AudioBuffer, ctx: &mut EffectContext) -> Result<(), EffectError> {
        for channel in &buf.channels {
            let mut i = 0;
            while i < channel.len() {
                if channel[i].abs() >= 1.0 {
                    let start = i;
                    while i < channel.len() && channel[i].abs() >= 1.0 { i += 1; }
                    if i - start >= self.duty_cycle_start {
                        let t0 = start as f64 / buf.sample_rate.max(1) as f64;
                        let t1 = i as f64 / buf.sample_rate.max(1) as f64;
                        ctx.labels.push(OutLabel { t0, t1, text: "clipped".into() });
                    }
                } else {
                    i += 1;
                }
            }
        }
        Ok(())
    }
}

/// Beat Finder analyzer.
#[derive(Clone, Debug)]
pub struct BeatFinder {
    /// Detection threshold as a percentage of peak.
    pub threshold_percentage: usize,
}
impl Default for BeatFinder { fn default() -> Self { Self { threshold_percentage: 65 } } }
impl BuiltinEffect for BeatFinder {
    fn id(&self) -> &'static str { "BeatFinder" }
    fn kind(&self) -> EffectKind { EffectKind::Analyze }
    fn params(&self) -> Vec<ParamDef> {
        vec![ParamDef::int("THRESVAL", "Threshold Percentage", 5.0, 100.0, 65.0)]
    }
    fn get(&self, key: &str) -> Option<f64> { (key == "THRESVAL").then_some(self.threshold_percentage as f64) }
    fn set(&mut self, key: &str, value: f64) -> bool {
        if key != "THRESVAL" { return false; }
        self.threshold_percentage = value.clamp(5.0, 100.0) as usize;
        true
    }
    fn process(&mut self, buf: &mut AudioBuffer, ctx: &mut EffectContext) -> Result<(), EffectError> {
        let n = buf.channels.iter().map(Vec::len).min().unwrap_or(0);
        if n == 0 { return Ok(()); }
        let mono: Vec<f32> = (0..n).map(|i| {
            buf.channels.iter().map(|c| c[i]).sum::<f32>()
        }).collect();
        let peak = mono.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
        if peak == 0.0 { return Ok(()); }
        let threshold = peak * self.threshold_percentage as f32 / 100.0;
        let mut previous = false;
        for (i, &v) in mono.iter().enumerate() {
            let active = v > threshold;
            if active && !previous {
                ctx.labels.push(OutLabel {
                    t0: i as f64 / buf.sample_rate.max(1) as f64,
                    t1: i as f64 / buf.sample_rate.max(1) as f64,
                    text: "B".into(),
                });
            }
            previous = v < threshold;
        }
        Ok(())
    }
}

/// Label Sounds analyzer.
#[derive(Clone, Debug)]
pub struct LabelSounds {
    /// Threshold level in dB.
    pub threshold: f64,
    /// Measurement: peak, average absolute, or RMS.
    pub measurement: usize,
    /// Minimum silence duration in seconds.
    pub silence_duration: f64,
    /// Minimum sound duration in seconds.
    pub sound_duration: f64,
    /// Label type: before, after, around, or between.
    pub label_type: usize,
    /// Maximum leading silence included.
    pub pre_offset: f64,
    /// Maximum trailing silence included.
    pub post_offset: f64,
    /// Label text template.
    pub text: String,
}
impl Default for LabelSounds {
    fn default() -> Self {
        Self { threshold: -30.0, measurement: 0, silence_duration: 1.0, sound_duration: 1.0,
            label_type: 2, pre_offset: 0.0, post_offset: 0.0, text: "Sound ##1".into() }
    }
}
impl BuiltinEffect for LabelSounds {
    fn id(&self) -> &'static str { "LabelSounds" }
    fn kind(&self) -> EffectKind { EffectKind::Analyze }
    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("THRESHOLD", "Threshold level (dB)", -100.0, 0.0, -30.0),
            ParamDef::choice("MEASUREMENT", "Threshold measurement", &["peak", "avg", "rms"], 0),
            ParamDef::float("SIL-DUR", "Minimum silence duration", 0.01, 3600.0, 1.0),
            ParamDef::float("SND-DUR", "Minimum label interval", 0.01, 7200.0, 1.0),
            ParamDef::choice("TYPE", "Label type", &["before", "after", "around", "between"], 2),
            ParamDef::float("PRE-OFFSET", "Maximum leading silence", 0.0, 3600.0, 0.0),
            ParamDef::float("POST-OFFSET", "Maximum trailing silence", 0.0, 3600.0, 0.0),
        ]
    }
    fn get(&self, key: &str) -> Option<f64> {
        match key { "THRESHOLD" => Some(self.threshold), "MEASUREMENT" => Some(self.measurement as f64),
            "SIL-DUR" => Some(self.silence_duration), "SND-DUR" => Some(self.sound_duration),
            "TYPE" => Some(self.label_type as f64), "PRE-OFFSET" => Some(self.pre_offset),
            "POST-OFFSET" => Some(self.post_offset), _ => None }
    }
    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "THRESHOLD" => self.threshold = value,
            "MEASUREMENT" => self.measurement = value.round().clamp(0.0, 2.0) as usize,
            "SIL-DUR" => self.silence_duration = value,
            "SND-DUR" => self.sound_duration = value,
            "TYPE" => self.label_type = value.round().clamp(0.0, 3.0) as usize,
            "PRE-OFFSET" => self.pre_offset = value,
            "POST-OFFSET" => self.post_offset = value,
            _ => return false,
        }
        true
    }
    fn process(&mut self, buf: &mut AudioBuffer, ctx: &mut EffectContext) -> Result<(), EffectError> {
        let rate = buf.sample_rate.max(1) as f64;
        let len = buf.channels.iter().map(Vec::len).min().unwrap_or(0);
        let threshold = 10.0f64.powf(self.threshold / 20.0) as f32;
        let mut segments = Vec::new();
        let mut start = None;
        for i in 0..len {
            let level = match self.measurement {
                0 => buf.channels.iter().map(|c| c[i].abs()).fold(0.0f32, f32::max),
                1 => buf.channels.iter().map(|c| c[i].abs()).sum::<f32>()
                    / buf.channels.len().max(1) as f32,
                _ => (buf.channels.iter().map(|c| (c[i] as f64).powi(2)).sum::<f64>()
                    / buf.channels.len().max(1) as f64).sqrt() as f32,
            };
            if level >= threshold && start.is_none() { start = Some(i); }
            if level < threshold {
                if let Some(s) = start {
                    if i - s >= (self.sound_duration * rate) as usize {
                        segments.push((s, i));
                    }
                    start = None;
                }
            }
        }
        if let Some(s) = start {
            if len - s >= (self.sound_duration * rate) as usize { segments.push((s, len)); }
        }
        for (index, (s, e)) in segments.iter().copied().enumerate() {
            let t0 = s as f64 / rate;
            let t1 = e as f64 / rate;
            let name = self.text.replace("##", &format!("{:02}", index + 1)).replace('#', &(index + 1).to_string());
            let (a, b) = match self.label_type {
                0 => ((t0 - self.pre_offset).max(0.0), (t0 - self.pre_offset).max(0.0)),
                1 => (t1 + self.post_offset, t1 + self.post_offset),
                _ => ((t0 - self.pre_offset).max(0.0), t1 + self.post_offset),
            };
            ctx.labels.push(OutLabel { t0: a, t1: b, text: name });
        }
        Ok(())
    }
}

/// Sample Data Export analyzer.
#[derive(Clone, Debug)]
pub struct SampleDataExport {
    /// Number of samples to export.
    pub number: usize,
    /// Units: 0 is dB, 1 is linear.
    pub units: usize,
    /// File format index: list, sample count, or time.
    pub fileformat: usize,
    /// Header verbosity.
    pub header: usize,
    /// Stereo channel layout.
    pub channel_layout: usize,
}
impl Default for SampleDataExport {
    fn default() -> Self { Self { number: 100, units: 0, fileformat: 0, header: 0, channel_layout: 0 } }
}
impl BuiltinEffect for SampleDataExport {
    fn id(&self) -> &'static str { "SampleDataExport" }
    fn kind(&self) -> EffectKind { EffectKind::Analyze }
    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int("NUMBER", "Limit output to first samples", 1.0, 1_000_000.0, 100.0),
            ParamDef::choice("UNITS", "Measurement scale", &["dB", "Linear"], 0),
            ParamDef::choice("FILEFORMAT", "Index", &["None", "Sample Count", "Time"], 0),
            ParamDef::choice("HEADER", "Include header information", &["None", "Minimal", "Standard", "All"], 0),
            ParamDef::choice("CHANNEL-LAYOUT", "Channel layout for stereo", &["SameLine", "Alternate", "LFirst"], 0),
        ]
    }
    fn get(&self, key: &str) -> Option<f64> {
        match key { "NUMBER" => Some(self.number as f64), "UNITS" => Some(self.units as f64),
            "FILEFORMAT" => Some(self.fileformat as f64), "HEADER" => Some(self.header as f64),
            "CHANNEL-LAYOUT" => Some(self.channel_layout as f64), _ => None }
    }
    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "NUMBER" => self.number = value.round().clamp(1.0, 1_000_000.0) as usize,
            "UNITS" => self.units = value.round().clamp(0.0, 1.0) as usize,
            "FILEFORMAT" => self.fileformat = value.round().clamp(0.0, 2.0) as usize,
            "HEADER" => self.header = value.round().clamp(0.0, 3.0) as usize,
            "CHANNEL-LAYOUT" => self.channel_layout = value.round().clamp(0.0, 2.0) as usize,
            _ => return false,
        }
        true
    }
    fn process(&mut self, buf: &mut AudioBuffer, ctx: &mut EffectContext) -> Result<(), EffectError> {
        let n = self.number.min(buf.channels.iter().map(Vec::len).min().unwrap_or(0));
        let mut out = String::new();
        let channels = buf.channels.len();
        if self.header > 0 {
            out.push_str(&format!("Sample Rate: {} Hz\n", buf.sample_rate));
            out.push_str(&format!("Channels: {}\n", channels));
        }
        for i in 0..n {
            let mut values = Vec::new();
            for c in &buf.channels {
                let v = c[i] as f64;
                values.push(if self.units == 0 {
                    if v == 0.0 { "[-inf]".into() } else { format!("{:.5}", 20.0 * v.abs().log10()) }
                } else { format!("{v:.5}") });
            }
            let prefix = match self.fileformat {
                1 => format!("{}\t", i + 1),
                2 => format!("{:.6}\t", i as f64 / buf.sample_rate.max(1) as f64),
                _ => String::new(),
            };
            if channels == 2 && self.channel_layout == 1 {
                for (ch, value) in values.iter().enumerate() {
                    out.push_str(&format!("{prefix}{value}\t{}\n", if ch == 0 { "L" } else { "R" }));
                }
            } else if channels == 2 && self.channel_layout == 2 {
                out.push_str(&format!("{prefix}{}\n", values[0]));
            } else {
                out.push_str(&prefix);
                out.push_str(&values.join("\t"));
                out.push('\n');
            }
        }
        if channels == 2 && self.channel_layout == 2 {
            out.push_str("\nRight Channel\n");
            for i in 0..n { out.push_str(&format!("{}\n", if self.units == 0 && buf.channels[1][i] == 0.0 {
                "[-inf]".into()
            } else if self.units == 0 {
                format!("{:.5}", 20.0 * (buf.channels[1][i] as f64).abs().log10())
            } else { format!("{:.5}", buf.channels[1][i]) })); }
        }
        ctx.report = out;
        Ok(())
    }
}

/// RMS analyzer.
#[derive(Clone, Debug, Default)]
pub struct Rms;
impl BuiltinEffect for Rms {
    fn id(&self) -> &'static str { "RMS" }
    fn kind(&self) -> EffectKind { EffectKind::Analyze }
    fn params(&self) -> Vec<ParamDef> { vec![] }
    fn get(&self, _: &str) -> Option<f64> { None }
    fn set(&mut self, _: &str, _: f64) -> bool { false }
    fn process(&mut self, buf: &mut AudioBuffer, ctx: &mut EffectContext) -> Result<(), EffectError> {
        if buf.channels.len() >= 2 {
            let left = rms_db(std::slice::from_ref(&buf.channels[0]));
            let right = rms_db(std::slice::from_ref(&buf.channels[1]));
            let stereo = rms_db(&buf.channels[..2]);
            ctx.report = format!("Left: \t{left} dB\nRight: \t{right} dB\nStereo: \t{stereo} dB");
        } else {
            ctx.report = format!("Mono: \t{} dB", rms_db(&buf.channels));
        }
        Ok(())
    }
}

/// Returns all analyzer effects in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(PlotSpectrum::default()),
        Box::new(Contrast),
        Box::new(FindClipping::default()),
        Box::new(BeatFinder::default()),
        Box::new(LabelSounds::default()),
        Box::new(SampleDataExport::default()),
        Box::new(Rms),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine() -> AudioBuffer {
        let sr = 8000;
        let a: Vec<f32> = (0..256).map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / sr as f32).sin() * 0.5).collect();
        AudioBuffer::from_channels(sr, vec![a.clone(), a])
    }

    #[test]
    fn analyzers_smoke_empty_repeat_and_clamp() {
        for mut e in all() {
            if e.id() == "Contrast" { continue; }
            let mut b = sine();
            let original = b.clone();
            let mut c1 = EffectContext::default();
            e.process(&mut b, &mut c1).unwrap();
            assert_eq!(b, original);
            let mut b2 = sine();
            let mut c2 = EffectContext::default();
            e.process(&mut b2, &mut c2).unwrap();
            assert_eq!(c1.report, c2.report);
            assert_eq!(c1.labels, c2.labels);
            let mut empty = AudioBuffer::from_channels(8000, vec![vec![], vec![]]);
            let _ = e.process(&mut empty, &mut EffectContext::default());
            if e.id() == "FindClipping" {
                assert!(set_param(&mut *e, "Duty Cycle Start", 0.0));
                assert_eq!(e.get("Duty Cycle Start"), Some(1.0));
            }
        }
    }

    #[test]
    fn spectrum_has_expected_bin_count_and_contrast() {
        let p = PlotSpectrum::default();
        assert_eq!(p.compute(&sine()).len(), 513);
        let fg = sine();
        let bg = AudioBuffer::from_channels(8000, vec![vec![0.001; 256], vec![0.001; 256]]);
        let r = Contrast::contrast(&fg, &bg);
        assert!(r.diff_db > 20.0);
        assert!(r.pass_wcag);
    }

    #[test]
    fn clipping_and_rms_sanity() {
        let mut b = AudioBuffer::from_channels(10, vec![vec![1.0, 1.0, 1.0, 0.0], vec![]]);
        let mut ctx = EffectContext::default();
        FindClipping::default().process(&mut b, &mut ctx).unwrap();
        assert_eq!(ctx.labels.len(), 1);
        assert_eq!(ctx.labels[0].text, "clipped");
        assert!((rms_db(&[vec![1.0, 1.0]])).abs() < 1e-12);
    }
}
