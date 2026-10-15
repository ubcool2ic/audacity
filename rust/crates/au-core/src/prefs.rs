//! Audacity preferences definitions, in-memory storage, and `audacity.cfg` I/O.
//!
//! Defaults here are the static fallbacks used by the settings shown in the
//! Audacity 3.7.3 sources. Some runtime-selected values, such as audio hosts
//! and devices, cannot be known until the audio backend is initialized.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

/// A preference value.
#[derive(Clone, Debug, PartialEq)]
pub enum PrefValue {
    /// Boolean preference.
    Bool(bool),
    /// Integer preference.
    Int(i64),
    /// Floating-point preference.
    Double(f64),
    /// String preference.
    Str(String),
}

/// A default value for a preference definition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PrefDefault {
    /// Boolean default.
    Bool(bool),
    /// Integer default.
    Int(i64),
    /// Floating-point default.
    Double(f64),
    /// String default.
    Str(&'static str),
}

/// Describes a preference read or written by an Audacity settings page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PrefDef {
    /// Settings page or category.
    pub page: &'static str,
    /// Full wxConfig path, including its leading slash.
    pub key: &'static str,
    /// Default value.
    pub default: PrefDefault,
    /// Human-readable setting label.
    pub label: &'static str,
}

static PREF_DEFS: &[PrefDef] = &[
    PrefDef { page: "Application", key: "/Update/DefaultUpdatesChecking", default: PrefDefault::Bool(true), label: "Check for updates" },
    PrefDef { page: "Application", key: "/SendAnonymousUsageInfo", default: PrefDefault::Bool(false), label: "Usage info (UUID)" },
    PrefDef { page: "Application", key: "/InstanceId", default: PrefDefault::Str(""), label: "Instance ID" },

    // AudioIOBase chooses an optimal supported rate at runtime. 44100 Hz is
    // the static fallback used when that backend-specific choice is absent.
    PrefDef { page: "Quality", key: "/SamplingRate/DefaultProjectSampleRate", default: PrefDefault::Int(44100), label: "Default project sample rate" },
    PrefDef { page: "Quality", key: "/SamplingRate/DefaultProjectSampleFormatChoice", default: PrefDefault::Str("Format32BitFloat"), label: "Default project sample format" },
    PrefDef { page: "Quality", key: "/SamplingRate/DefaultProjectSampleFormat", default: PrefDefault::Int(3), label: "Legacy default project sample format" },
    PrefDef { page: "Quality", key: "/Quality/DitherAlgorithmChoice", default: PrefDefault::Str("None"), label: "Real-time dither" },
    PrefDef { page: "Quality", key: "/Quality/HQDitherAlgorithmChoice", default: PrefDefault::Str("Shaped"), label: "High-quality dither" },

    PrefDef { page: "Interface", key: "/Locale/Language", default: PrefDefault::Str("System"), label: "Language" },
    PrefDef { page: "Interface", key: "/GUI/ShowSplashScreen", default: PrefDefault::Bool(true), label: "Show splash screen" },
    PrefDef { page: "Interface", key: "/GUI/ShowExtraMenus", default: PrefDefault::Bool(false), label: "Show extra menus" },
    PrefDef { page: "Interface", key: "/GUI/ShowMac", default: PrefDefault::Bool(false), label: "Show alternative styling" },
    PrefDef { page: "Interface", key: "/GUI/BeepOnCompletion", default: PrefDefault::Bool(false), label: "Beep on completion" },
    PrefDef { page: "Interface", key: "/GUI/RetainLabels", default: PrefDefault::Bool(false), label: "Retain labels when snapping" },
    PrefDef { page: "Interface", key: "/GUI/RtlWorkaround", default: PrefDefault::Bool(true), label: "Mostly left-to-right layouts in RTL languages" },
    PrefDef { page: "Interface", key: "/Locale/CeeNumberFormat", default: PrefDefault::Bool(false), label: "Never use comma as decimal point" },
    PrefDef { page: "Interface", key: "/GUI/ShowRMS", default: PrefDefault::Bool(false), label: "Show RMS" },
    PrefDef { page: "Interface", key: "/GUI/ShowClipping", default: PrefDefault::Bool(false), label: "Show clipping" },
    PrefDef { page: "Interface", key: "/GUI/Theme", default: PrefDefault::Str(""), label: "Theme" },
    PrefDef { page: "Interface", key: "/GUI/EnvdBRange", default: PrefDefault::Int(60), label: "Meter dB range" },

    // Device and host names are backend-dependent; empty strings request the
    // PortAudio/default device chosen by Audacity.
    PrefDef { page: "Devices", key: "/AudioIO/Host", default: PrefDefault::Str(""), label: "Host" },
    PrefDef { page: "Devices", key: "/AudioIO/PlaybackDevice", default: PrefDefault::Str(""), label: "Playback device" },
    PrefDef { page: "Devices", key: "/AudioIO/RecordingDevice", default: PrefDefault::Str(""), label: "Recording device" },
    PrefDef { page: "Devices", key: "/AudioIO/RecordingSource", default: PrefDefault::Str(""), label: "Recording source" },
    PrefDef { page: "Devices", key: "/AudioIO/RecordingSourceIndex", default: PrefDefault::Int(0), label: "Recording source index" },
    PrefDef { page: "Devices", key: "/AudioIO/RecordChannels", default: PrefDefault::Int(2), label: "Recording channels" },
    PrefDef { page: "Devices", key: "/AudioIO/LatencyDuration", default: PrefDefault::Double(100.0), label: "Buffer length" },
    PrefDef { page: "Devices", key: "/AudioIO/LatencyCorrection", default: PrefDefault::Double(0.0), label: "Latency compensation" },

    PrefDef { page: "Playback", key: "/AudioIO/EffectsPreviewLen", default: PrefDefault::Double(6.0), label: "Effects preview length" },
    PrefDef { page: "Playback", key: "/AudioIO/CutPreviewBeforeLen", default: PrefDefault::Double(2.0), label: "Cut preview before length" },
    PrefDef { page: "Playback", key: "/AudioIO/CutPreviewAfterLen", default: PrefDefault::Double(1.0), label: "Cut preview after length" },
    PrefDef { page: "Playback", key: "/AudioIO/SeekShortPeriod", default: PrefDefault::Double(1.0), label: "Short seek period" },
    PrefDef { page: "Playback", key: "/AudioIO/SeekLongPeriod", default: PrefDefault::Double(15.0), label: "Long seek period" },
    PrefDef { page: "Playback", key: "/AudioIO/Microfades", default: PrefDefault::Bool(false), label: "Micro-fades" },
    PrefDef { page: "Playback", key: "/AudioIO/UnpinnedScrubbing", default: PrefDefault::Bool(true), label: "Always scrub unpinned" },
    PrefDef { page: "Playback", key: "/AudioIO/PinnedHead", default: PrefDefault::Bool(false), label: "Pinned playback head" },
    PrefDef { page: "Playback", key: "/AudioIO/PinnedHeadPosition", default: PrefDefault::Double(0.5), label: "Pinned head position" },

    PrefDef { page: "Recording", key: "/AudioIO/Duplex", default: PrefDefault::Bool(true), label: "Hear other tracks while recording" },
    PrefDef { page: "Recording", key: "/AudioIO/Playthrough", default: PrefDefault::Bool(false), label: "Hardware playthrough" },
    PrefDef { page: "Recording", key: "/AudioIO/SWPlaythrough", default: PrefDefault::Bool(false), label: "Audible input monitoring" },
    PrefDef { page: "Recording", key: "/GUI/PreferNewTrackRecord", default: PrefDefault::Bool(false), label: "Record on a new track" },
    PrefDef { page: "Recording", key: "/Warnings/DropoutDetected", default: PrefDefault::Bool(true), label: "Warn about recording dropouts" },
    PrefDef { page: "Recording", key: "/AudioIO/SilenceLevel", default: PrefDefault::Int(-50), label: "Sound-activated recording level" },
    PrefDef { page: "Recording", key: "/GUI/TrackNames/RecordingNameCustom", default: PrefDefault::Bool(false), label: "Custom recording track name" },
    PrefDef { page: "Recording", key: "/GUI/TrackNames/RecodingTrackName", default: PrefDefault::Str("Recorded_Audio"), label: "Recording track name" },
    PrefDef { page: "Recording", key: "/GUI/TrackNames/TrackNumber", default: PrefDefault::Bool(false), label: "Include track number" },
    PrefDef { page: "Recording", key: "/GUI/TrackNames/DateStamp", default: PrefDefault::Bool(false), label: "Include system date" },
    PrefDef { page: "Recording", key: "/GUI/TrackNames/TimeStamp", default: PrefDefault::Bool(false), label: "Include system time" },
    PrefDef { page: "Recording", key: "/AudioIO/AutomatedInputLevelAdjustment", default: PrefDefault::Bool(false), label: "Automated input level adjustment" },
    PrefDef { page: "Recording", key: "/AudioIO/TargetPeak", default: PrefDefault::Double(95.0), label: "Target peak" },
    PrefDef { page: "Recording", key: "/AudioIO/DeltaPeakVolume", default: PrefDefault::Double(5.0), label: "Peak tolerance" },
    PrefDef { page: "Recording", key: "/AudioIO/AnalysisTime", default: PrefDefault::Int(100), label: "Analysis time" },
    PrefDef { page: "Recording", key: "/AudioIO/NumberAnalysis", default: PrefDefault::Int(0), label: "Number of consecutive analyses" },
    PrefDef { page: "Recording", key: "/AudioIO/PreRoll", default: PrefDefault::Double(5.0), label: "Punch and roll pre-roll" },
    PrefDef { page: "Recording", key: "/AudioIO/RollCrossfade", default: PrefDefault::Double(10.0), label: "Punch and roll crossfade" },
    PrefDef { page: "Recording", key: "/AudioIO/SoundActivatedRecord", default: PrefDefault::Bool(false), label: "Sound-activated recording" },

    PrefDef { page: "Tracks", key: "/GUI/TracksFitVerticallyZoomed", default: PrefDefault::Bool(false), label: "Auto-fit track height" },
    PrefDef { page: "Tracks", key: "/GUI/CollapseToHalfWave", default: PrefDefault::Bool(false), label: "Half-wave display when collapsed" },
    PrefDef { page: "Tracks", key: "/GUI/AutoScroll", default: PrefDefault::Bool(true), label: "Auto-scroll when head is unpinned" },
    PrefDef { page: "Tracks", key: "/GUI/DefaultViewModeChoiceNew", default: PrefDefault::Str("Waveform"), label: "Default view mode" },
    PrefDef { page: "Tracks", key: "/GUI/DefaultWaveformScaleChoice", default: PrefDefault::Str("Linear"), label: "Default waveform scale" },
    PrefDef { page: "Tracks", key: "/GUI/SampleViewChoice", default: PrefDefault::Str("StemPlot"), label: "Sample display" },
    PrefDef { page: "Tracks", key: "/GUI/DefaultAudioTrackName", default: PrefDefault::Str("Audio Track"), label: "Default audio track name" },
    PrefDef { page: "Tracks", key: "/GUI/ZoomPreset1Choice", default: PrefDefault::Str("ZoomDefault"), label: "Zoom preset 1" },
    PrefDef { page: "Tracks", key: "/GUI/ZoomPreset2Choice", default: PrefDefault::Str("FourPixelsPerSample"), label: "Zoom preset 2" },
    PrefDef { page: "Tracks", key: "/GUI/DefaultWaveformScale", default: PrefDefault::Str("Linear"), label: "Legacy waveform scale" },
    PrefDef { page: "Tracks", key: "/GUI/SampleView", default: PrefDefault::Int(1), label: "Legacy sample display" },
    PrefDef { page: "Tracks", key: "/GUI/DefaultViewMode", default: PrefDefault::Int(0), label: "Legacy default view mode" },
    PrefDef { page: "Tracks", key: "/GUI/DefaultViewModeNew", default: PrefDefault::Int(0), label: "Legacy default view mode" },
    PrefDef { page: "Tracks", key: "/GUI/DefaultViewModeChoice", default: PrefDefault::Str("Waveform"), label: "Legacy default view mode" },

    PrefDef { page: "Tracks Behaviors", key: "/GUI/SelectAllOnNone", default: PrefDefault::Bool(false), label: "Select all audio if selection required" },
    PrefDef { page: "Tracks Behaviors", key: "/GUI/EnableCutLines", default: PrefDefault::Bool(false), label: "Enable cut lines" },
    PrefDef { page: "Tracks Behaviors", key: "/GUI/EditClipCanMove", default: PrefDefault::Bool(false), label: "Editing a clip can move other clips" },
    PrefDef { page: "Tracks Behaviors", key: "/GUI/PasteAsNewClips", default: PrefDefault::Bool(false), label: "Always paste audio as new clips" },
    PrefDef { page: "Tracks Behaviors", key: "/GUI/CircularTrackNavigation", default: PrefDefault::Bool(false), label: "Cycle repeatedly through tracks" },
    PrefDef { page: "Tracks Behaviors", key: "/GUI/TypeToCreateLabel", default: PrefDefault::Bool(false), label: "Type to create a label" },
    PrefDef { page: "Tracks Behaviors", key: "/GUI/DialogForNameNewLabel", default: PrefDefault::Bool(false), label: "Dialog for new label name" },
    PrefDef { page: "Tracks Behaviors", key: "/GUI/AudioTrackPastePolicy", default: PrefDefault::Str("Ask"), label: "Pasted audio policy" },
    PrefDef { page: "Tracks Behaviors", key: "/GUI/Solo", default: PrefDefault::Str("Simple"), label: "Solo button behavior" },

    PrefDef { page: "Directories", key: "/DefaultOpenPath", default: PrefDefault::Str(""), label: "Open directory" },
    PrefDef { page: "Directories", key: "/DefaultSavePath", default: PrefDefault::Str(""), label: "Save directory" },
    PrefDef { page: "Directories", key: "/DefaultImportPath", default: PrefDefault::Str(""), label: "Import directory" },
    PrefDef { page: "Directories", key: "/DefaultExportPath", default: PrefDefault::Str(""), label: "Export directory" },
    PrefDef { page: "Directories", key: "/DefaultMacrosPath", default: PrefDefault::Str(""), label: "Macro output directory" },
    PrefDef { page: "Directories", key: "/Directories/TempDir", default: PrefDefault::Str(""), label: "Temporary files directory" },

    PrefDef { page: "Effects", key: "/Effects/SkipEffectsScanAtStartup", default: PrefDefault::Bool(false), label: "Skip effects scanning at startup" },
    PrefDef { page: "Effects", key: "/Effects/GroupBy", default: PrefDefault::Str("default"), label: "Effect menu organization" },
    PrefDef { page: "Effects", key: "/Effects/RealtimeGroupBy", default: PrefDefault::Str("groupby:type:publisher"), label: "Realtime effect organization" },
    PrefDef { page: "Effects", key: "/SSE/GUI", default: PrefDefault::Bool(true), label: "Use SSE/SSE2/AVX" },

    PrefDef { page: "Import / Export", key: "/FileFormats/MusicFileImportSettingChoice", default: PrefDefault::Str("Ask"), label: "Music loop import into an empty project" },
    PrefDef { page: "Import / Export", key: "/FileFormats/MusicFileImportOnExistingProjectSettingChoice", default: PrefDefault::Str("MatchIfBeatsAndMeasures"), label: "Music loop import into an existing project" },
    PrefDef { page: "Import / Export", key: "/FileFormats/ExportDownMixChoice", default: PrefDefault::Str("StereoToMono"), label: "Export down-mix choice" },
    PrefDef { page: "Import / Export", key: "/AudioFiles/NormalizeOnLoad", default: PrefDefault::Bool(false), label: "Normalize audio on load" },
    PrefDef { page: "Import / Export", key: "/FileFormats/CopyOrReadDirectly", default: PrefDefault::Str("Copy"), label: "Copy or read audio files directly" },
    PrefDef { page: "Import / Export", key: "/FileFormats/ReadUncompressedFilesDirectly", default: PrefDefault::Bool(false), label: "Read uncompressed files directly" },

    PrefDef { page: "Batch", key: "/Batch/Debug", default: PrefDefault::Bool(false), label: "Do not apply effects in batch mode" },
];

/// Returns the static definitions for settings used by the supplied Audacity
/// preference pages and settings files.
pub fn all_defs() -> &'static [PrefDef] {
    PREF_DEFS
}

/// In-memory preferences, including unknown keys loaded from a config file.
pub struct Prefs {
    values: HashMap<String, PrefValue>,
    listeners: Vec<Box<dyn Fn(&str) + Send + Sync>>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self::new()
    }
}

impl Prefs {
    /// Creates an empty preferences store. Reads use the definitions' defaults.
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            listeners: Vec::new(),
        }
    }

    /// Returns a boolean preference, using its definition default or `false`.
    pub fn get_bool(&self, key: &str) -> bool {
        self.read_bool_or(key, false)
    }

    /// Returns an integer preference, using its definition default or zero.
    pub fn get_int(&self, key: &str) -> i64 {
        self.read_int_or(key, 0)
    }

    /// Returns a floating-point preference, using its definition default or zero.
    pub fn get_double(&self, key: &str) -> f64 {
        self.read_double_or(key, 0.0)
    }

    /// Returns a string preference, using its definition default or an empty string.
    pub fn get_string(&self, key: &str) -> String {
        self.read_string_or(key, "")
    }

    /// Reads a boolean, using `default` only when the key has no stored value
    /// and no definition.
    pub fn read_bool_or(&self, key: &str, default: bool) -> bool {
        let key = normalize_key(key);
        match self.values.get(&key) {
            Some(value) => value_as_bool(value).unwrap_or(default),
            None => match find_def(&key).map(|def| def.default) {
                Some(PrefDefault::Bool(value)) => value,
                Some(PrefDefault::Int(value)) => value != 0,
                Some(PrefDefault::Double(value)) => value != 0.0,
                Some(PrefDefault::Str(value)) => parse_bool(value).unwrap_or(default),
                None => default,
            },
        }
    }

    /// Reads an integer, using `default` only when the key has no stored value
    /// and no definition.
    pub fn read_int_or(&self, key: &str, default: i64) -> i64 {
        let key = normalize_key(key);
        match self.values.get(&key) {
            Some(value) => value_as_int(value).unwrap_or(default),
            None => match find_def(&key).map(|def| def.default) {
                Some(PrefDefault::Bool(value)) => i64::from(value),
                Some(PrefDefault::Int(value)) => value,
                Some(PrefDefault::Double(value)) => value as i64,
                Some(PrefDefault::Str(value)) => value.parse().unwrap_or(default),
                None => default,
            },
        }
    }

    /// Reads a floating-point value, using `default` only when the key has no
    /// stored value and no definition.
    pub fn read_double_or(&self, key: &str, default: f64) -> f64 {
        let key = normalize_key(key);
        match self.values.get(&key) {
            Some(value) => value_as_double(value).unwrap_or(default),
            None => match find_def(&key).map(|def| def.default) {
                Some(PrefDefault::Bool(value)) => if value { 1.0 } else { 0.0 },
                Some(PrefDefault::Int(value)) => value as f64,
                Some(PrefDefault::Double(value)) => value,
                Some(PrefDefault::Str(value)) => value.parse().unwrap_or(default),
                None => default,
            },
        }
    }

    /// Reads a string, using `default` only when the key has no stored value
    /// and no definition.
    pub fn read_string_or(&self, key: &str, default: &str) -> String {
        let key = normalize_key(key);
        match self.values.get(&key) {
            Some(value) => value_as_string(value),
            None => match find_def(&key).map(|def| def.default) {
                Some(PrefDefault::Bool(value)) => bool_to_cfg(value),
                Some(PrefDefault::Int(value)) => value.to_string(),
                Some(PrefDefault::Double(value)) => value.to_string(),
                Some(PrefDefault::Str(value)) => value.to_owned(),
                None => default.to_owned(),
            },
        }
    }

    /// Stores a preference and notifies every registered listener, even if the
    /// new value is equal to the previous value.
    pub fn set(&mut self, key: &str, value: PrefValue) {
        let key = normalize_key(key);
        self.values.insert(key.clone(), value);
        for listener in &self.listeners {
            listener(&key);
        }
    }

    /// Removes a stored value. Subsequent reads use its definition default.
    pub fn remove(&mut self, key: &str) {
        self.values.remove(&normalize_key(key));
    }

    /// Removes all stored values, including unknown keys.
    pub fn reset_all(&mut self) {
        self.values.clear();
    }

    /// Registers a listener called with the normalized path on every `set`.
    pub fn subscribe(&mut self, listener: Box<dyn Fn(&str) + Send + Sync>) {
        self.listeners.push(listener);
    }

    /// Returns the configured default project sample rate in hertz.
    pub fn quality_default_rate(&self) -> u32 {
        self.get_int("/SamplingRate/DefaultProjectSampleRate")
            .clamp(1, u32::MAX as i64) as u32
    }

    /// Returns the configured sample format choice (`Format16Bit`,
    /// `Format24Bit`, or `Format32BitFloat`).
    pub fn quality_default_format(&self) -> String {
        self.get_string("/SamplingRate/DefaultProjectSampleFormatChoice")
    }

    /// Returns the configured dither symbol for conversion.
    pub fn dither_kind(&self) -> String {
        self.get_string("/Quality/DitherAlgorithmChoice")
    }

    /// Returns the configured high-quality dither symbol.
    pub fn high_quality_dither_kind(&self) -> String {
        self.get_string("/Quality/HQDitherAlgorithmChoice")
    }

    /// Returns the configured UI language code.
    pub fn language(&self) -> String {
        self.get_string("/Locale/Language")
    }

    /// Returns the configured temporary-directory path.
    pub fn temp_dir(&self) -> PathBuf {
        PathBuf::from(self.get_string("/Directories/TempDir"))
    }

    /// Returns the configured playback device name.
    pub fn playback_device_name(&self) -> String {
        self.get_string("/AudioIO/PlaybackDevice")
    }

    /// Returns the configured recording device name.
    pub fn recording_device_name(&self) -> String {
        self.get_string("/AudioIO/RecordingDevice")
    }

    /// Returns the configured recording channel count, with a minimum of one.
    pub fn recording_channels(&self) -> u32 {
        self.get_int("/AudioIO/RecordChannels").max(1).min(u32::MAX as i64) as u32
    }

    /// Whether commands requiring a selection should select all audio when
    /// nothing is selected.
    pub fn select_all_audio_if_required(&self) -> bool {
        self.get_bool("/GUI/SelectAllOnNone")
    }

    /// Whether editing a clip may move other clips.
    pub fn editing_clip_can_move_other_clips(&self) -> bool {
        self.get_bool("/GUI/EditClipCanMove")
    }

    /// Whether pasted audio should always become new clips.
    pub fn paste_as_new_clips(&self) -> bool {
        self.get_bool("/GUI/PasteAsNewClips")
    }

    /// Whether track navigation cycles repeatedly through tracks.
    pub fn circular_track_navigation(&self) -> bool {
        self.get_bool("/GUI/CircularTrackNavigation")
    }

    /// Returns the enabled state of a warning dialog identified by its
    /// Audacity warning name.
    pub fn warning_enabled(&self, name: &str) -> bool {
        let key = if name.starts_with('/') {
            name.to_owned()
        } else {
            format!("/Warnings/{name}")
        };
        self.get_bool(&key)
    }

    /// Returns the export down-mix choice symbol.
    pub fn export_downmix_choice(&self) -> String {
        self.get_string("/FileFormats/ExportDownMixChoice")
    }

    /// Whether audio should be normalized when loaded.
    pub fn normalize_on_load(&self) -> bool {
        self.get_bool("/AudioFiles/NormalizeOnLoad")
    }

    /// Returns the import choice for copying audio or reading it directly.
    pub fn copy_or_read_directly_choice(&self) -> String {
        self.get_string("/FileFormats/CopyOrReadDirectly")
    }

    /// Whether uncompressed audio files should be read directly.
    pub fn read_uncompressed_files_directly(&self) -> bool {
        self.get_bool("/FileFormats/ReadUncompressedFilesDirectly")
    }

    /// Parses a wxFileConfig-compatible `audacity.cfg` string.
    ///
    /// Known paths are parsed using their declared type. Unknown paths are
    /// retained as strings so that newer or plugin-specific settings survive
    /// a load/save cycle.
    pub fn parse_cfg(text: &str) -> Self {
        parse_cfg(text)
    }

    /// Serializes stored values in wxFileConfig-compatible group form.
    pub fn to_cfg(&self) -> String {
        let mut entries: Vec<(&String, &PrefValue)> = self.values.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));

        let mut root_lines = Vec::new();
        let mut groups: Vec<(String, Vec<(String, String)>)> = Vec::new();

        for (path, value) in entries {
            let trimmed = path.trim_start_matches('/');
            if let Some((group, name)) = trimmed.rsplit_once('/') {
                let group = group.to_owned();
                let name = name.to_owned();
                let rendered = encode_cfg_value(value);
                if let Some((_, values)) = groups.iter_mut().find(|(existing, _)| *existing == group) {
                    values.push((name, rendered));
                } else {
                    groups.push((group, vec![(name, rendered)]));
                }
            } else {
                root_lines.push(format!("{trimmed}={}", encode_cfg_value(value)));
            }
        }

        groups.sort_by(|a, b| a.0.cmp(&b.0));
        let mut output = String::new();
        for line in root_lines {
            output.push_str(&line);
            output.push('\n');
        }
        for (group, mut values) in groups {
            values.sort_by(|a, b| a.0.cmp(&b.0));
            if !output.is_empty() && !output.ends_with("\n\n") {
                output.push('\n');
            }
            output.push('[');
            output.push_str(&group);
            output.push_str("]\n");
            for (key, value) in values {
                output.push_str(&key);
                output.push('=');
                output.push_str(&value);
                output.push('\n');
            }
        }
        output
    }

    /// Loads preferences from a file.
    pub fn load(path: impl AsRef<std::path::Path>) -> io::Result<Self> {
        let text = fs::read_to_string(path)?;
        Ok(parse_cfg(&text))
    }

    /// Saves stored preferences to a file, creating or truncating it.
    pub fn save(&self, path: impl AsRef<std::path::Path>) -> io::Result<()> {
        fs::write(path, self.to_cfg())
    }

    /// Chooses Audacity's Linux configuration path, preferring an existing
    /// legacy `~/.audacity-data/audacity.cfg` when the modern file is absent.
    pub fn default_cfg_path() -> PathBuf {
        default_cfg_path()
    }
}

/// Parses a wxFileConfig-compatible `audacity.cfg` string.
pub fn parse_cfg(text: &str) -> Prefs {
    let mut prefs = Prefs::new();
    let mut group = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            group = unescape_cfg(&line[1..line.len() - 1]);
            group = group.trim_matches('/').to_owned();
            continue;
        }

        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key_name = unescape_cfg(raw_key.trim());
        if key_name.is_empty() {
            continue;
        }

        let full_key = if key_name.starts_with('/') {
            key_name
        } else if group.is_empty() {
            format!("/{key_name}")
        } else {
            format!("/{group}/{key_name}")
        };
        let full_key = normalize_key(&full_key);
        let decoded = decode_cfg_value(raw_value);
        let value = match find_def(&full_key).map(|def| def.default) {
            Some(PrefDefault::Bool(_)) => PrefValue::Bool(parse_bool(&decoded).unwrap_or(false)),
            Some(PrefDefault::Int(_)) => PrefValue::Int(decoded.parse().unwrap_or(0)),
            Some(PrefDefault::Double(_)) => PrefValue::Double(decoded.parse().unwrap_or(0.0)),
            Some(PrefDefault::Str(_)) | None => PrefValue::Str(decoded),
        };
        prefs.values.insert(full_key, value);
    }

    prefs
}

/// Chooses the Linux Audacity configuration path. An existing modern config
/// takes precedence; an existing legacy file is used if the modern one is
/// absent.
pub fn default_cfg_path() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let modern = if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("audacity").join("audacity.cfg")
    } else if let Some(home) = &home {
        home.join(".config").join("audacity").join("audacity.cfg")
    } else {
        PathBuf::from(".config").join("audacity").join("audacity.cfg")
    };

    if modern.exists() {
        return modern;
    }
    if let Some(home) = home {
        let legacy = home.join(".audacity-data").join("audacity.cfg");
        if legacy.exists() {
            return legacy;
        }
    }
    modern
}

fn normalize_key(key: &str) -> String {
    if key.starts_with('/') {
        key.to_owned()
    } else {
        format!("/{key}")
    }
}

fn find_def(key: &str) -> Option<&'static PrefDef> {
    PREF_DEFS.iter().find(|def| def.key == key)
}

fn value_as_bool(value: &PrefValue) -> Option<bool> {
    match value {
        PrefValue::Bool(value) => Some(*value),
        PrefValue::Int(value) => Some(*value != 0),
        PrefValue::Double(value) => Some(*value != 0.0),
        PrefValue::Str(value) => parse_bool(value),
    }
}

fn value_as_int(value: &PrefValue) -> Option<i64> {
    match value {
        PrefValue::Bool(value) => Some(i64::from(*value)),
        PrefValue::Int(value) => Some(*value),
        PrefValue::Double(value) => Some(*value as i64),
        PrefValue::Str(value) => value.parse().ok(),
    }
}

fn value_as_double(value: &PrefValue) -> Option<f64> {
    match value {
        PrefValue::Bool(value) => Some(if *value { 1.0 } else { 0.0 }),
        PrefValue::Int(value) => Some(*value as f64),
        PrefValue::Double(value) => Some(*value),
        PrefValue::Str(value) => value.parse().ok(),
    }
}

fn value_as_string(value: &PrefValue) -> String {
    match value {
        PrefValue::Bool(value) => bool_to_cfg(*value),
        PrefValue::Int(value) => value.to_string(),
        PrefValue::Double(value) => value.to_string(),
        PrefValue::Str(value) => value.clone(),
    }
}

fn bool_to_cfg(value: bool) -> String {
    if value { "1" } else { "0" }.to_owned()
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn encode_cfg_value(value: &PrefValue) -> String {
    match value {
        PrefValue::Bool(value) => bool_to_cfg(*value),
        PrefValue::Int(value) => value.to_string(),
        PrefValue::Double(value) => value.to_string(),
        PrefValue::Str(value) => {
            let quoted = value.starts_with(char::is_whitespace)
                || value.ends_with(char::is_whitespace);
            let escaped = escape_cfg(value);
            if quoted {
                format!("\"{escaped}\"")
            } else {
                escaped
            }
        }
    }
}

fn escape_cfg(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\t' => result.push_str("\\t"),
            '\r' => result.push_str("\\r"),
            '"' => result.push_str("\\\""),
            _ => result.push(ch),
        }
    }
    result
}

fn decode_cfg_value(raw: &str) -> String {
    let trimmed = raw.trim();
    let content = if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        &trimmed[1..trimmed.len() - 1]
    } else {
        trimmed
    };
    unescape_cfg(content)
}

fn unescape_cfg(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => result.push('\n'),
            Some('t') => result.push('\t'),
            Some('r') => result.push('\r'),
            Some('\\') => result.push('\\'),
            Some('"') => result.push('"'),
            Some(other) => {
                result.push('\\');
                result.push(other);
            }
            None => result.push('\\'),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn audacity_defaults_are_available() {
        let prefs = Prefs::new();
        assert_eq!(prefs.quality_default_rate(), 44100);
        assert_eq!(prefs.quality_default_format(), "Format32BitFloat");
        assert_eq!(prefs.dither_kind(), "None");
        assert_eq!(prefs.get_bool("/GUI/ShowSplashScreen"), true);
        assert_eq!(prefs.get_bool("/GUI/SelectAllOnNone"), false);
        assert_eq!(prefs.get_double("/AudioIO/EffectsPreviewLen"), 6.0);
    }

    #[test]
    fn cfg_round_trip_preserves_groups_root_and_escapes() {
        let mut prefs = Prefs::new();
        prefs.set("/RootFlag", PrefValue::Bool(true));
        prefs.set("/AudioIO/PlaybackDevice", PrefValue::Str("Device\nName\t\\".into()));
        prefs.set("/Unknown/Spaced", PrefValue::Str(" padded value ".into()));
        prefs.set("/Unknown/Plain", PrefValue::Str("ordinary".into()));

        let cfg = prefs.to_cfg();
        let parsed = parse_cfg(&cfg);
        assert_eq!(parsed.get_bool("/RootFlag"), true);
        assert_eq!(parsed.get_string("/AudioIO/PlaybackDevice"), "Device\nName\t\\");
        assert_eq!(parsed.get_string("/Unknown/Spaced"), " padded value ");
        assert_eq!(parsed.get_string("/Unknown/Plain"), "ordinary");
    }

    #[test]
    fn parses_realistic_cfg_snippet() {
        let prefs = parse_cfg(
            "# Audacity configuration\n\
             SendAnonymousUsageInfo=1\n\
             [SamplingRate]\n\
             DefaultProjectSampleRate=48000\n\
             DefaultProjectSampleFormatChoice=Format24Bit\n\
             [AudioIO]\n\
             PlaybackDevice=\" Speakers \"\n\
             RecordChannels=2\n\
             [GUI]\n\
             ShowSplashScreen=0\n\
             [Unknown/Plugin]\n\
             Path=C:\\\\plugins\\\\audio\n",
        );
        assert!(prefs.get_bool("/SendAnonymousUsageInfo"));
        assert_eq!(prefs.quality_default_rate(), 48000);
        assert_eq!(prefs.quality_default_format(), "Format24Bit");
        assert_eq!(prefs.playback_device_name(), " Speakers ");
        assert_eq!(prefs.recording_channels(), 2);
        assert!(!prefs.get_bool("/GUI/ShowSplashScreen"));
        assert_eq!(prefs.get_string("/Unknown/Plugin/Path"), "C:\\plugins\\audio");
    }

    #[test]
    fn listeners_fire_on_each_set() {
        let count = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&count);
        let mut prefs = Prefs::new();
        prefs.subscribe(Box::new(move |key| {
            assert_eq!(key, "/GUI/ShowSplashScreen");
            observed.fetch_add(1, Ordering::SeqCst);
        }));
        prefs.set("/GUI/ShowSplashScreen", PrefValue::Bool(false));
        prefs.set("/GUI/ShowSplashScreen", PrefValue::Bool(false));
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }
}
