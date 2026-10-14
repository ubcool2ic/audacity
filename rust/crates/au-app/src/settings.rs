//! Typed access to Audacity preferences and pure settings-related behaviours.

use au_core::prefs::{PrefValue, Prefs};
use std::path::{Path, PathBuf};

/// Track display modes available in the Tracks preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    /// Display the waveform.
    Waveform,
    /// Display a spectrogram.
    Spectrogram,
    /// Display multiple channel views.
    Multiview,
}

/// Waveform amplitude scales available in the Tracks preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveScale {
    /// Linear amplitude scale.
    Linear,
    /// Decibel amplitude scale.
    LinearDb,
}

/// Sample display modes available in the Tracks preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleView {
    /// Connect sample points with lines.
    ConnectDots,
    /// Draw samples as stems.
    StemPlot,
}

/// Zoom presets available in the Tracks preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomPreset {
    /// Fit the project to the available width.
    FitToWidth,
    /// Zoom to the current selection.
    ZoomToSelection,
    /// Audacity's default zoom.
    ZoomDefault,
    /// Minutes.
    Minutes,
    /// Seconds.
    Seconds,
    /// Fifths of seconds.
    FifthsOfSeconds,
    /// Tenths of seconds.
    TenthsOfSeconds,
    /// Twentieths of seconds.
    TwentiethsOfSeconds,
    /// Fiftieths of seconds.
    FiftiethsOfSeconds,
    /// Hundredths of seconds.
    HundredthsOfSeconds,
    /// Five hundredths of seconds.
    FiveHundredthsOfSeconds,
    /// Milliseconds.
    MilliSeconds,
    /// Individual samples.
    Samples,
    /// Four pixels per sample.
    FourPixelsPerSample,
    /// Maximum zoom.
    MaxZoom,
}

/// Solo-button behaviours available in Tracks Behaviors preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoloPolicy {
    /// Soloing one track clears solo on the others unless Shift is held.
    Simple,
    /// Solo buttons toggle independently; Shift makes the action exclusive.
    Multi,
    /// Solo buttons have no effect.
    None,
}

/// Project sample formats available in Quality preferences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// 16-bit integer samples.
    Int16,
    /// 24-bit integer samples.
    Int24,
    /// 32-bit floating-point samples.
    Float32,
}

/// A typed snapshot of the Audacity settings used by these preferences pages.
#[derive(Clone, Debug, PartialEq)]
pub struct AppSettings {
    /// Automatically scroll when the playback or recording head is unpinned.
    pub auto_scroll: bool,
    /// Fit track heights vertically when zooming.
    pub tracks_fit_vertically_zoomed: bool,
    /// Use a half-wave display for collapsed tracks.
    pub collapse_to_half_wave: bool,
    /// Default new-track view mode.
    pub default_view_mode: ViewMode,
    /// Default waveform scale.
    pub default_wave_scale: WaveScale,
    /// Sample display style.
    pub sample_view: SampleView,
    /// Default audio track name preference.
    pub default_audio_track_name: String,
    /// First zoom-toggle preset.
    pub zoom_preset1: ZoomPreset,
    /// Second zoom-toggle preset.
    pub zoom_preset2: ZoomPreset,

    /// Select all audio when a command requires a selection and none exists.
    pub select_all_on_none: bool,
    /// Enable cut lines.
    pub enable_cut_lines: bool,
    /// Allow editing a clip to move other clips.
    pub edit_clips_can_move: bool,
    /// Always paste audio as new clips.
    pub paste_as_new_clips: bool,
    /// Cycle repeatedly through tracks when moving track focus.
    pub circular_track_navigation: bool,
    /// Type to create a label.
    pub type_to_create_label: bool,
    /// Show a dialog for the name of a new label.
    pub dialog_for_name_new_label: bool,
    /// Audio-track paste policy symbol (`Keep`, `Discard`, or `Ask`).
    pub audio_track_paste_policy: String,
    /// Solo-button policy.
    pub solo: SoloPolicy,

    /// Effects preview length, in seconds.
    pub effects_preview_len: f64,
    /// Cut-preview audio before the cut, in seconds.
    pub cut_preview_before_len: f64,
    /// Cut-preview audio after the cut, in seconds.
    pub cut_preview_after_len: f64,
    /// Short playback seek period, in seconds.
    pub seek_short_period: f64,
    /// Long playback seek period, in seconds.
    pub seek_long_period: f64,
    /// Enable micro-fades.
    pub micro_fades: bool,
    /// Always scrub when the playback head is unpinned.
    pub unpinned_scrubbing: bool,

    /// Number of input channels to record.
    pub record_channels: usize,
    /// Hear other tracks while recording (duplex/overdub).
    pub duplex: bool,
    /// Enable audible input monitoring.
    pub sw_playthrough: bool,
    /// Prefer recording onto a new track.
    pub prefer_new_track_record: bool,
    /// Enable sound-activated recording.
    pub sound_activated_record: bool,
    /// Sound-activation silence threshold, in dB.
    pub silence_level_db: i64,
    /// Use a custom recording track name.
    pub recording_name_custom: bool,
    /// Custom recording track name.
    pub recording_track_name: String,
    /// Include a track number in new recording names.
    pub use_track_number: bool,
    /// Include the date in new recording names.
    pub use_date_stamp: bool,
    /// Include the time in new recording names.
    pub use_time_stamp: bool,
    /// Punch-and-roll pre-roll, in seconds.
    pub pre_roll: f64,
    /// Punch-and-roll crossfade, in milliseconds.
    pub roll_crossfade_ms: f64,
    /// Automated recording input-level adjustment.
    pub automated_input_level_adjustment: bool,
    /// Automated input-level target peak.
    pub target_peak: f64,
    /// Automated input-level peak tolerance.
    pub delta_peak_volume: f64,
    /// Automated input-level analysis time, in milliseconds.
    pub analysis_time: i64,
    /// Number of consecutive automated input-level analyses.
    pub number_analysis: i64,

    /// Default project sample rate, in hertz.
    pub default_rate: u32,
    /// Default project sample format.
    pub default_format: SampleFormat,
    /// Conversion dither symbol.
    pub dither: String,
    /// High-quality conversion dither symbol.
    pub hq_dither: String,

    /// Preferred interface language symbol.
    pub language: String,
    /// Preferred interface theme.
    pub theme: String,
    /// Beep when longer activities complete.
    pub beep_on_completion: bool,
    /// Retain labels when a selection snaps to a label.
    pub retain_labels: bool,
    /// Show RMS meters.
    pub show_rms: bool,
    /// Show clipping indicators.
    pub show_clipping: bool,
    /// Waveform envelope dB range.
    pub env_db_range: i64,
    /// Show extra menus.
    pub show_extra_menus: bool,
    /// Use mostly left-to-right layouts in right-to-left languages.
    pub rtl_workaround: bool,

    /// Selected audio host.
    pub host: String,
    /// Selected playback device.
    pub playback_device: String,
    /// Selected recording device.
    pub recording_device: String,
    /// Audio buffer duration, in milliseconds.
    pub latency_duration_ms: f64,
    /// Recording latency correction, in milliseconds.
    pub latency_correction_ms: f64,

    /// Default open directory.
    pub default_open_path: String,
    /// Default save directory.
    pub default_save_path: String,
    /// Default import directory.
    pub default_import_path: String,
    /// Default export directory.
    pub default_export_path: String,
    /// Default macro-output directory.
    pub default_macros_path: String,
    /// Temporary-files directory.
    pub temp_dir: String,

    /// Destructive-effects menu grouping symbol.
    pub group_by: String,
    /// Realtime-effects menu grouping symbol.
    pub realtime_group_by: String,

    /// Whether the recording-dropout warning is enabled.
    pub dropout_detected: bool,
}

impl AppSettings {
    /// Reads all settings from preferences, using Audacity's preference defaults.
    pub fn from_prefs(p: &Prefs) -> Self {
        let default_view_mode =
            parse_view_mode(&p.get_string("/GUI/DefaultViewModeChoiceNew"));
        let default_wave_scale =
            parse_wave_scale(&p.get_string("/GUI/DefaultWaveformScaleChoice"));
        let sample_view = parse_sample_view(&p.get_string("/GUI/SampleViewChoice"));
        let zoom_preset1 = parse_zoom_preset(&p.get_string("/GUI/ZoomPreset1Choice"));
        let zoom_preset2 = parse_zoom_preset(&p.get_string("/GUI/ZoomPreset2Choice"));
        let solo = parse_solo_policy(&p.get_string("/GUI/Solo"));
        let default_format =
            parse_sample_format(&p.get_string("/SamplingRate/DefaultProjectSampleFormatChoice"));

        Self {
            auto_scroll: p.get_bool("/GUI/AutoScroll"),
            tracks_fit_vertically_zoomed: p.get_bool("/GUI/TracksFitVerticallyZoomed"),
            collapse_to_half_wave: p.get_bool("/GUI/CollapseToHalfWave"),
            default_view_mode,
            default_wave_scale,
            sample_view,
            default_audio_track_name: default_track_name(p),
            zoom_preset1,
            zoom_preset2,

            select_all_on_none: p.get_bool("/GUI/SelectAllOnNone"),
            enable_cut_lines: p.get_bool("/GUI/EnableCutLines"),
            edit_clips_can_move: p.get_bool("/GUI/EditClipsCanMove"),
            paste_as_new_clips: p.get_bool("/GUI/PasteAsNewClips"),
            circular_track_navigation: p.get_bool("/GUI/CircularTrackNavigation"),
            type_to_create_label: p.get_bool("/GUI/TypeToCreateLabel"),
            dialog_for_name_new_label: p.get_bool("/GUI/DialogForNameNewLabel"),
            audio_track_paste_policy: p.get_string("/GUI/AudioTrackPastePolicy"),
            solo,

            effects_preview_len: p.get_double("/AudioIO/EffectsPreviewLen"),
            cut_preview_before_len: p.get_double("/AudioIO/CutPreviewBeforeLen"),
            cut_preview_after_len: p.get_double("/AudioIO/CutPreviewAfterLen"),
            seek_short_period: p.get_double("/AudioIO/SeekShortPeriod"),
            seek_long_period: p.get_double("/AudioIO/SeekLongPeriod"),
            micro_fades: p.get_bool("/AudioIO/Microfades"),
            unpinned_scrubbing: p.get_bool("/AudioIO/UnpinnedScrubbing"),

            record_channels: p.get_int("/AudioIO/RecordChannels").max(1) as usize,
            duplex: p.get_bool("/AudioIO/Duplex"),
            sw_playthrough: p.get_bool("/AudioIO/SWPlaythrough"),
            prefer_new_track_record: p.get_bool("/GUI/PreferNewTrackRecord"),
            sound_activated_record: p.get_bool("/AudioIO/SoundActivatedRecord"),
            silence_level_db: p.get_int("/AudioIO/SilenceLevel"),
            recording_name_custom: p.get_bool("/GUI/TrackNames/RecordingNameCustom"),
            recording_track_name: p.get_string("/GUI/TrackNames/RecodingTrackName"),
            use_track_number: p.get_bool("/GUI/TrackNames/TrackNumber"),
            use_date_stamp: p.get_bool("/GUI/TrackNames/DateStamp"),
            use_time_stamp: p.get_bool("/GUI/TrackNames/TimeStamp"),
            pre_roll: p.get_double("/AudioIO/PreRoll"),
            roll_crossfade_ms: p.get_double("/AudioIO/RollCrossfade"),
            automated_input_level_adjustment: p.get_bool("/AudioIO/AutomatedInputLevelAdjustment"),
            target_peak: p.get_double("/AudioIO/TargetPeak"),
            delta_peak_volume: p.get_double("/AudioIO/DeltaPeakVolume"),
            analysis_time: p.get_int("/AudioIO/AnalysisTime"),
            number_analysis: p.get_int("/AudioIO/NumberAnalysis"),

            default_rate: p.quality_default_rate(),
            default_format,
            dither: p.dither_kind(),
            hq_dither: p.high_quality_dither_kind(),

            language: p.language(),
            theme: p.get_string("/GUI/Theme"),
            beep_on_completion: p.get_bool("/GUI/BeepOnCompletion"),
            retain_labels: p.get_bool("/GUI/RetainLabels"),
            show_rms: p.get_bool("/GUI/ShowRMS"),
            show_clipping: p.get_bool("/GUI/ShowClipping"),
            env_db_range: p.get_int("/GUI/EnvdBRange"),
            show_extra_menus: p.get_bool("/GUI/ShowExtraMenus"),
            rtl_workaround: p.get_bool("/GUI/RtlWorkaround"),

            host: p.get_string("/AudioIO/Host"),
            playback_device: p.get_string("/AudioIO/PlaybackDevice"),
            recording_device: p.get_string("/AudioIO/RecordingDevice"),
            latency_duration_ms: p.get_double("/AudioIO/LatencyDuration"),
            latency_correction_ms: p.get_double("/AudioIO/LatencyCorrection"),

            default_open_path: p.read_string_or("/Directories/Open/Default", ""),
            default_save_path: p.read_string_or("/Directories/Save/Default", ""),
            default_import_path: p.read_string_or("/Directories/Import/Default", ""),
            default_export_path: p.read_string_or("/Directories/Export/Default", ""),
            default_macros_path: p.read_string_or("/Directories/MacrosOut/Default", ""),
            temp_dir: p.get_string("/Directories/TempDir"),

            group_by: p.get_string("/Effects/GroupBy"),
            realtime_group_by: p.get_string("/Effects/RealtimeGroupBy"),

            dropout_detected: p.get_bool("/Warnings/DropoutDetected"),
        }
    }
}

fn parse_view_mode(value: &str) -> ViewMode {
    match value {
        "Spectrogram" => ViewMode::Spectrogram,
        "MultiView" | "Multiview" => ViewMode::Multiview,
        _ => ViewMode::Waveform,
    }
}

fn parse_wave_scale(value: &str) -> WaveScale {
    match value {
        "dB" | "Db" | "LinearDb" | "WaveformDB" => WaveScale::LinearDb,
        _ => WaveScale::Linear,
    }
}

fn parse_sample_view(value: &str) -> SampleView {
    match value {
        "ConnectDots" => SampleView::ConnectDots,
        _ => SampleView::StemPlot,
    }
}

fn parse_zoom_preset(value: &str) -> ZoomPreset {
    match value {
        "FitToWidth" => ZoomPreset::FitToWidth,
        "ZoomToSelection" => ZoomPreset::ZoomToSelection,
        "Minutes" => ZoomPreset::Minutes,
        "Seconds" => ZoomPreset::Seconds,
        "FifthsOfSeconds" => ZoomPreset::FifthsOfSeconds,
        "TenthsOfSeconds" => ZoomPreset::TenthsOfSeconds,
        "TwentiethsOfSeconds" => ZoomPreset::TwentiethsOfSeconds,
        "FiftiethsOfSeconds" => ZoomPreset::FiftiethsOfSeconds,
        "HundredthsOfSeconds" => ZoomPreset::HundredthsOfSeconds,
        "FiveHundredthsOfSeconds" => ZoomPreset::FiveHundredthsOfSeconds,
        "MilliSeconds" => ZoomPreset::MilliSeconds,
        "Samples" => ZoomPreset::Samples,
        "FourPixelsPerSample" => ZoomPreset::FourPixelsPerSample,
        "MaxZoom" => ZoomPreset::MaxZoom,
        _ => ZoomPreset::ZoomDefault,
    }
}

fn parse_solo_policy(value: &str) -> SoloPolicy {
    match value {
        "Multi" => SoloPolicy::Multi,
        "None" => SoloPolicy::None,
        _ => SoloPolicy::Simple,
    }
}

fn parse_sample_format(value: &str) -> SampleFormat {
    match value {
        "Format16Bit" => SampleFormat::Int16,
        "Format24Bit" => SampleFormat::Int24,
        _ => SampleFormat::Float32,
    }
}

/// Produces Audacity's unique track name by appending the first unused number.
/// WaveTrack::GetDefaultAudioTrackNamePreference: the default msgid "Audio"
/// is shown translated; any other stored value is used literally
/// (verified: Audacity 3.7.3 with "Audio Track" names tracks "Audio Track 1").
fn default_track_name(p: &Prefs) -> String {
    let v = p.read_string_or("/GUI/TrackNames/DefaultTrackName", "Audio");
    if v.is_empty() || v == "Audio" {
        au_core::i18n::tr_ui("Audio")
    } else {
        v
    }
}

pub fn unique_track_name(base: &str, existing: &[String]) -> String {
    let mut n = 1usize;
    loop {
        let candidate = format!("{base} {n}");
        if !existing.iter().any(|name| name == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// Builds the names for newly-created recording tracks.
///
/// `now` is interpreted as local `(year, month, day, hour, minute, second)`.
/// For one or two recording channels Audacity creates one track; for more than
/// two channels it creates one track per channel.
pub fn recording_track_names(
    settings: &AppSettings,
    existing: &[String],
    num_wave_tracks: usize,
    channels: usize,
    now: (i32, u32, u32, u32, u32, u32),
) -> Vec<String> {
    let default_name = unique_track_name(&settings.default_audio_track_name, existing);
    let base_name = if settings.recording_name_custom {
        settings.recording_track_name.as_str()
    } else {
        default_name.as_str()
    };

    let track_count = if channels > 2 { channels } else { 1 };
    (0..track_count)
        .map(|track_index| {
            let mut suffix = String::new();

            if settings.use_track_number {
                suffix.push_str(&(num_wave_tracks + track_index + 1).to_string());
            }
            if settings.use_date_stamp {
                if !suffix.is_empty() {
                    suffix.push('_');
                }
                suffix.push_str(&format!("{:04}-{:02}-{:02}", now.0, now.1, now.2));
            }
            if settings.use_time_stamp {
                if !suffix.is_empty() {
                    suffix.push('_');
                }
                suffix.push_str(&format!("{:02}-{:02}-{:02}", now.3, now.4, now.5));
            }

            match (base_name.is_empty(), suffix.is_empty()) {
                (true, _) => suffix,
                (_, true) => base_name.to_owned(),
                (false, false) => format!("{base_name}_{suffix}"),
            }
        })
        .collect()
}

/// A file operation whose default and last-used directories can be remembered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DirOp {
    /// Temporary files.
    Temp,
    /// Open.
    Open,
    /// Save.
    Save,
    /// Import.
    Import,
    /// Export.
    Export,
    /// Macro output.
    Macros,
}

fn directory_keys(op: DirOp) -> (&'static str, &'static str) {
    match op {
        DirOp::Temp => ("/Directories/TempDir/Default", "/Directories/TempDir/LastUsed"),
        DirOp::Open => ("/Directories/Open/Default", "/Directories/Open/LastUsed"),
        DirOp::Save => ("/Directories/Save/Default", "/Directories/Save/LastUsed"),
        DirOp::Import => ("/Directories/Import/Default", "/Directories/Import/LastUsed"),
        DirOp::Export => ("/Directories/Export/Default", "/Directories/Export/LastUsed"),
        DirOp::Macros => ("/Directories/MacrosOut/Default", "/Directories/MacrosOut/LastUsed"),
    }
}

/// Finds the configured default directory, falling back to the last-used one.
pub fn find_default_path(p: &Prefs, op: DirOp) -> Option<PathBuf> {
    let (user_key, last_used_key) = directory_keys(op);
    for value in [
        p.read_string_or(user_key, ""),
        p.read_string_or(last_used_key, ""),
    ] {
        if !value.is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    None
}

/// Updates the last-used directory for an operation.
///
/// For a file path, its containing directory is stored. A directory path is
/// stored unchanged. Temporary-directory updates use the `_None` preference
/// key, `/Directories/TempDir`.
pub fn update_default_path(p: &mut Prefs, op: DirOp, path: &Path) {
    let key = if op == DirOp::Temp {
        "/Directories/TempDir"
    } else {
        directory_keys(op).1
    };

    let directory = if path.is_dir() || path.extension().is_none() {
        path
    } else {
        path.parent().unwrap_or_else(|| Path::new(""))
    };
    if !directory.as_os_str().is_empty() {
        p.set(
            key,
            PrefValue::Str(directory.to_string_lossy().into_owned()),
        );
    }
}

/// Metadata used to construct effects menus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectEntry {
    /// Stable plugin or command identifier.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Publisher name.
    pub publisher: String,
    /// Effect family, such as `Audacity`, `Nyquist`, or `VST3`.
    pub family: String,
    /// Whether this is an Audacity built-in effect.
    pub is_builtin_audacity: bool,
    /// Whether Audacity marks this effect as a default effect.
    pub is_default: bool,
    /// Whether this plug-in is bundled with Audacity.
    pub is_bundled: bool,
    /// Effect type, such as `Process`, `Generate`, `Analyze`, or `Tool`.
    pub effect_type: String,
}

/// Maps our effect ids whose display name differs from Audacity's to the Audacity name.
/// Publisher/family of effects that only this port provides.
pub const EXTRA_PUBLISHER: &str = "Audacity-rs Extras";

fn audacity_name<'a>(id: &str, name: &'a str) -> &'a str {
    match id {
        "RMS" => "Measure RMS",
        _ => name,
    }
}

/// Builds an effect entry from Audacity's 3.7.3 metadata, matching by exact display name.
///
/// Unknown names use the Audacity built-in defaults: publisher and family
/// `Audacity`, type `Process`, default and bundled.
pub fn entry_for(id: &str, name: &str) -> EffectEntry {
    let name = audacity_name(id, name);
    let metadata = crate::effect_meta::AUDACITY_EFFECTS
        .iter()
        .find(|(effect_name, _, _, _, _, _)| *effect_name == audacity_name(id, name));

    let (publisher, family, effect_type, is_default, is_bundled) = match metadata {
        Some((_, vendor, family, effect_type, is_default, is_bundled)) => (
            (*vendor).to_owned(),
            (*family).to_owned(),
            (*effect_type).to_owned(),
            *is_default,
            *is_bundled,
        ),
        // Effects Audacity 3.7.3 does not ship get their own publisher and
        // family so Audacity's own groups stay identical.
        None => (
            EXTRA_PUBLISHER.to_owned(),
            EXTRA_PUBLISHER.to_owned(),
            "Process".to_owned(),
            // Effects that Audacity 3.7.3 does not ship are not default/bundled.
            false,
            false,
        ),
    };

    EffectEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        is_builtin_audacity: family == "Audacity",
        publisher,
        family,
        is_default,
        is_bundled,
        effect_type,
    }
}

/// A node in an effects menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuNode {
    /// An effect command.
    Item {
        /// Effect identifier.
        id: String,
        /// Menu label.
        label: String,
    },
    /// A submenu.
    Submenu {
        /// Submenu label.
        label: String,
        /// Child entries.
        items: Vec<MenuNode>,
    },
    /// A separator between menu sections.
    Separator,
}

fn item(entry: &EffectEntry, label: String) -> MenuNode {
    MenuNode::Item {
        id: entry.id.clone(),
        label,
    }
}

fn compare_text(a: &str, b: &str) -> std::cmp::Ordering {
    a.cmp(b)
}

fn by_name(a: &EffectEntry, b: &EffectEntry) -> std::cmp::Ordering {
    compare_text(&a.name, &b.name).then_with(|| a.id.cmp(&b.id))
}

fn by_publisher(a: &EffectEntry, b: &EffectEntry) -> std::cmp::Ordering {
    compare_text(publisher_compare_key(a), publisher_compare_key(b))
        .then_with(|| compare_text(&a.name, &b.name))
        .then_with(|| a.id.cmp(&b.id))
}

fn by_publisher_and_name(a: &EffectEntry, b: &EffectEntry) -> std::cmp::Ordering {
    let a_publisher = if a.is_default { "" } else { a.publisher.as_str() };
    let b_publisher = if b.is_default { "" } else { b.publisher.as_str() };
    compare_text(a_publisher, b_publisher)
        .then_with(|| compare_text(&a.name, &b.name))
        .then_with(|| a.id.cmp(&b.id))
}

fn by_type(a: &EffectEntry, b: &EffectEntry) -> std::cmp::Ordering {
    compare_text(type_compare_key(a), type_compare_key(b))
        .then_with(|| compare_text(&a.name, &b.name))
        .then_with(|| a.id.cmp(&b.id))
}

fn by_type_publisher(a: &EffectEntry, b: &EffectEntry) -> std::cmp::Ordering {
    compare_text(type_compare_key(a), type_compare_key(b))
        .then_with(|| publisher_type_compare_key(a).cmp(publisher_type_compare_key(b)))
        .then_with(|| compare_text(&a.name, &b.name))
        .then_with(|| a.id.cmp(&b.id))
}

fn publisher_compare_key(entry: &EffectEntry) -> &str {
    if entry.publisher.is_empty() {
        "Uncategorized"
    } else {
        &entry.publisher
    }
}

fn publisher_type_compare_key(entry: &EffectEntry) -> &str {
    if entry.publisher.is_empty() {
        "Unknown"
    } else {
        &entry.publisher
    }
}

fn type_compare_key(entry: &EffectEntry) -> &str {
    if entry.family.is_empty() {
        "Uncategorized"
    } else {
        &entry.family
    }
}

fn group_key_publisher(entry: &EffectEntry) -> &str {
    &entry.publisher
}

fn group_key_type(entry: &EffectEntry) -> &str {
    &entry.family
}

fn sorted_by_name<'a>(entries: &[&'a EffectEntry]) -> Vec<&'a EffectEntry> {
    let mut result = entries.to_vec();
    result.sort_by(|a, b| by_name(a, b));
    result
}

fn duplicate_name_nodes(entries: &[&EffectEntry], labels: &[String]) -> Vec<MenuNode> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < entries.len() {
        let mut end = i + 1;
        while end < entries.len() && labels[end] == labels[i] {
            end += 1;
        }
        if end - i > 1 {
            result.push(MenuNode::Submenu {
                label: labels[i].clone(),
                items: entries[i..end]
                    .iter()
                    .zip(&labels[i..end])
                    .map(|(entry, label)| item(entry, label.clone()))
                    .collect(),
            });
        } else {
            result.push(item(entries[i], labels[i].clone()));
        }
        i = end;
    }
    result
}

/// Adds sorted menu items, applying Audacity's publisher/type prefixes and
/// combining adjacent equal translated names into a submenu.
fn add_sorted_items(
    entries: &[&EffectEntry],
    prefix: Option<fn(&EffectEntry) -> &str>,
) -> Vec<MenuNode> {
    let labels: Vec<String> = entries
        .iter()
        .map(|entry| {
            let name = entry.name.as_str();
            let group = if entry.is_default {
                ""
            } else {
                prefix.map_or("", |get| get(entry))
            };
            if group.is_empty() {
                name.to_owned()
            } else {
                au_core::i18n::tr_format("%s: %s", &[&group, &name])
            }
        })
        .collect();
    duplicate_name_nodes(entries, &labels)
}

/// Adds contiguous publisher/type groups in the order established by the
/// relevant Audacity comparator. A one-item group is flattened; larger groups
/// get a submenu unless the group label is empty.
fn add_grouped_items(
    entries: &[&EffectEntry],
    key: fn(&EffectEntry) -> &str,
) -> Vec<MenuNode> {
    let mut result = Vec::new();
    let mut start = 0;
    while start < entries.len() {
        let label = key(entries[start]).to_owned();
        let mut end = start + 1;
        while end < entries.len() && key(entries[end]) == label {
            end += 1;
        }

        let group_entries = &entries[start..end];
        let group_labels: Vec<String> = group_entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        let children = duplicate_name_nodes(group_entries, &group_labels);

        if !label.is_empty() && group_entries.len() > 1 {
            result.push(MenuNode::Submenu {
                label,
                items: children,
            });
        } else {
            result.extend(children);
        }
        start = end;
    }
    result
}

fn with_separator(first: Vec<MenuNode>, second: Vec<MenuNode>) -> Vec<MenuNode> {
    match (first.is_empty(), second.is_empty()) {
        (true, _) => second,
        (_, true) => first,
        (false, false) => {
            let mut result = first;
            result.push(MenuNode::Separator);
            result.extend(second);
            result
        }
    }
}

/// The fixed XML ordering used by Audacity's EffectsMenuDefaults.xml.
/// Entries absent from the supplied effect list are simply omitted.
const EFFECT_MENU_DEFAULTS: &[(&str, &[&str])] = &[
    (
        "Volume and Compression",
        &[
            "Amplify",
            "Compressor",
            "Limiter",
            "Normalize",
            "Loudness Normalization",
            "Auto Duck",
        ],
    ),
    (
        "Fading",
        &[
            "Fade In",
            "Fade Out",
            "Studio Fade Out",
            "Adjustable Fade",
            "Crossfade Clips",
            "Crossfade Tracks",
        ],
    ),
    (
        "Pitch and Tempo",
        &[
            "Change Pitch",
            "Change Speed and Pitch",
            "Change Tempo",
            "Paulstretch",
            "Sliding Stretch",
        ],
    ),
    (
        "EQ and Filters",
        &[
            "Bass and Treble",
            "Graphic EQ",
            "Filter Curve EQ",
            "High-Pass Filter",
            "Low-Pass Filter",
            "Shelf Filter",
            "Notch Filter",
        ],
    ),
    (
        "Noise Removal and Repair",
        &[
            "Click Removal",
            "Noise Reduction",
            "Noise Gate",
            "Repair",
            "Clip Fix",
        ],
    ),
    ("Delay and Reverb", &["Echo", "Reverb", "Delay"]),
    (
        "Distortion and Modulation",
        &["Tremolo", "Distortion", "Wahwah", "Phaser", "Vocoder"],
    ),
    (
        "Special",
        &[
            "Repeat",
            "Reverse",
            "Invert",
            "Truncate Silence",
            "Vocal Reduction and Isolation",
            "Vocal Remover",
        ],
    ),
    (
        "Spectral Tools",
        &[
            "Spectral Delete",
            "Spectral Edit Multi Tool",
            "Spectral Edit Parametric EQ",
            "Spectral Edit Shelves",
        ],
    ),
    (
        "Legacy",
        &["Legacy Compressor", "Legacy Limiter", "Classic Filters"],
    ),
];

fn process_default_group_items(entries: &[EffectEntry]) -> (Vec<MenuNode>, Vec<bool>) {
    let mut selected = vec![false; entries.len()];
    let mut result = Vec::new();

    for (group_name, effect_names) in EFFECT_MENU_DEFAULTS {
        let mut group_items = Vec::new();
        for effect_name in *effect_names {
            if let Some((index, entry)) = entries.iter().enumerate().find(|(index, entry)| {
                !selected[*index]
                    && entry.is_bundled
                    && (entry.is_default || entry.is_bundled)
                    && entry.name == *effect_name
            }) {
                selected[index] = true;
                group_items.push(item(entry, entry.name.clone()));
            }
        }
        // MakeAddGroupItems sorts each group's names (TranslationLess).
        group_items.sort_by(|a, b| match (a, b) {
            (MenuNode::Item { label: x, .. }, MenuNode::Item { label: y, .. }) => x.cmp(y),
            _ => std::cmp::Ordering::Equal,
        });

        if !group_items.is_empty() {
            result.push(MenuNode::Submenu {
                label: (*group_name).to_owned(),
                items: group_items,
            });
        }
    }

    (result, selected)
}

fn sorted_refs<'a>(
    entries: &'a [EffectEntry],
    selected: &[bool],
    include_selected: bool,
    comparator: fn(&EffectEntry, &EffectEntry) -> std::cmp::Ordering,
) -> Vec<&'a EffectEntry> {
    let mut result: Vec<_> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| {
            (selected[index] == include_selected).then_some(entry)
        })
        .collect();
    result.sort_by(|a, b| comparator(a, b));
    result
}

/// Builds an effects menu using Audacity 3.7.3's `PopulateEffectsMenu` section
/// filters, comparators, and item-building rules.
///
/// `entries` is the complete list supplied for a single effect type. The menu
/// helper does not filter entries by `effect_type`. Supported grouping symbols
/// are `default`, `sortby:name`, `sortby:publisher:name`,
/// `sortby:type:name`, `groupby:publisher`, `groupby:type`, and
/// `groupby:type:publisher`.
pub fn effect_menu(entries: &[EffectEntry], group_by: &str) -> Vec<MenuNode> {
    if group_by == "default" {
        let (first, selected) = process_default_group_items(entries);

        // The default layout uses the XML groups only for Process effects.
        // Other kinds put bundled effects in a name-sorted section, then group
        // the remaining enabled effects by publisher.
        let process_entries = entries
            .iter()
            .any(|entry| entry.effect_type == "Process");
        if process_entries {
            let other = sorted_refs(entries, &selected, false, by_publisher);
            return with_separator(first, add_grouped_items(&other, group_key_publisher));
        }

        let bundled: Vec<&EffectEntry> = entries
            .iter()
            .filter(|entry| entry.is_default || entry.is_bundled)
            .collect();
        let bundled = sorted_by_name(&bundled);
        let bundled_indices: std::collections::HashSet<&str> =
            bundled.iter().map(|entry| entry.id.as_str()).collect();
        let rest: Vec<&EffectEntry> = entries
            .iter()
            .filter(|entry| !bundled_indices.contains(entry.id.as_str()))
            .collect();
        let mut rest = rest;
        rest.sort_by(|a, b| by_publisher(a, b));
        return with_separator(
            add_sorted_items(&bundled, None),
            add_grouped_items(&rest, group_key_publisher),
        );
    }

    let defaults: Vec<&EffectEntry> = entries.iter().filter(|entry| entry.is_default).collect();
    let other: Vec<&EffectEntry> = entries.iter().filter(|entry| !entry.is_default).collect();

    match group_by {
        "sortby:publisher:name" => {
            let mut first = defaults;
            first.sort_by(|a, b| by_name(a, b));
            let mut second = other;
            second.sort_by(|a, b| by_publisher_and_name(a, b));
            with_separator(
                add_sorted_items(&first, Some(|entry| entry.publisher.as_str())),
                add_sorted_items(&second, Some(|entry| entry.publisher.as_str())),
            )
        }
        "sortby:type:name" => {
            let mut first = defaults;
            first.sort_by(|a, b| by_name(a, b));
            // Audacity 3.7.3 uses CompareEffectsByPublisherAndName for this
            // section, even though the displayed prefix is the effect family.
            let mut second = other;
            second.sort_by(|a, b| by_publisher_and_name(a, b));
            with_separator(
                add_sorted_items(&first, Some(|entry| entry.family.as_str())),
                add_sorted_items(&second, Some(|entry| entry.family.as_str())),
            )
        }
        "groupby:publisher" => {
            let mut first = defaults;
            first.sort_by(|a, b| by_publisher(a, b));
            let mut second = other;
            second.sort_by(|a, b| by_publisher(a, b));
            with_separator(
                add_grouped_items(&first, group_key_publisher),
                add_grouped_items(&second, group_key_publisher),
            )
        }
        "groupby:type" => {
            let mut first = defaults;
            first.sort_by(|a, b| by_type(a, b));
            let mut second = other;
            second.sort_by(|a, b| by_type(a, b));
            with_separator(
                add_grouped_items(&first, group_key_type),
                add_grouped_items(&second, group_key_type),
            )
        }
        "groupby:type:publisher" => {
            let mut first = defaults;
            first.sort_by(|a, b| by_type(a, b));
            let mut second = other;
            second.sort_by(|a, b| by_type_publisher(a, b));

            let first_nodes = add_grouped_items(&first, group_key_type);
            let second_nodes = add_type_publisher_groups(&second);
            with_separator(first_nodes, second_nodes)
        }
        _ => {
            let mut first = defaults;
            first.sort_by(|a, b| by_name(a, b));
            let mut second = other;
            second.sort_by(|a, b| by_name(a, b));
            with_separator(
                add_sorted_items(&first, None),
                add_sorted_items(&second, None),
            )
        }
    }
}

fn add_type_publisher_groups(entries: &[&EffectEntry]) -> Vec<MenuNode> {
    let mut result = Vec::new();
    let mut type_start = 0;

    while type_start < entries.len() {
        let type_name = entries[type_start].family.clone();
        let mut type_end = type_start + 1;
        while type_end < entries.len() && entries[type_end].family == type_name {
            type_end += 1;
        }

        let children = add_grouped_items(
            &entries[type_start..type_end],
            group_key_publisher,
        );
        result.push(MenuNode::Submenu {
            label: type_name,
            items: children,
        });
        type_start = type_end;
    }

    result
}

/// Applies a solo-button click using the selected Audacity solo policy.
pub fn apply_solo_click(
    policy: SoloPolicy,
    solos: &mut [bool],
    index: usize,
    shift_down: bool,
) {
    if index >= solos.len() || policy == SoloPolicy::None {
        return;
    }

    match policy {
        SoloPolicy::None => {}
        SoloPolicy::Simple if shift_down => solos[index] = !solos[index],
        SoloPolicy::Simple => {
            if solos[index] {
                solos.fill(false);
            } else {
                solos.fill(false);
                solos[index] = true;
            }
        }
        SoloPolicy::Multi if shift_down => {
            solos.fill(false);
            solos[index] = true;
        }
        SoloPolicy::Multi => solos[index] = !solos[index],
    }
}

/// Returns the configured short or long seek period, in seconds.
pub fn seek_seconds(settings: &AppSettings, long: bool) -> f64 {
    if long {
        settings.seek_long_period
    } else {
        settings.seek_short_period
    }
}

/// Returns pixels per second for an Audacity zoom preset.
///
/// `current_zoom` is used for `ZoomToSelection`, since the selection bounds
/// are not supplied to this pure helper.
pub fn zoom_of_preset(
    preset: ZoomPreset,
    project_len_secs: f64,
    screen_width_px: f64,
    current_zoom: f64,
) -> f64 {
    const MAX_ZOOM_OUT_FACTOR: f64 = 4.0;
    const PIXELS_PER_UNIT: f64 = 5.0;
    const MAX_ZOOM: f64 = 1_000_000.0;

    let fit_zoom = if project_len_secs > 0.0 {
        screen_width_px / project_len_secs
    } else {
        current_zoom
    };

    let result = match preset {
        ZoomPreset::FitToWidth => fit_zoom,
        ZoomPreset::ZoomToSelection => current_zoom,
        ZoomPreset::ZoomDefault => 4410.0,
        ZoomPreset::Minutes => PIXELS_PER_UNIT / 60.0,
        ZoomPreset::Seconds => PIXELS_PER_UNIT,
        ZoomPreset::FifthsOfSeconds => PIXELS_PER_UNIT * 5.0,
        ZoomPreset::TenthsOfSeconds => PIXELS_PER_UNIT * 10.0,
        ZoomPreset::TwentiethsOfSeconds => PIXELS_PER_UNIT * 20.0,
        ZoomPreset::FiftiethsOfSeconds => PIXELS_PER_UNIT * 50.0,
        ZoomPreset::HundredthsOfSeconds => PIXELS_PER_UNIT * 100.0,
        ZoomPreset::FiveHundredthsOfSeconds => PIXELS_PER_UNIT * 500.0,
        ZoomPreset::MilliSeconds => PIXELS_PER_UNIT * 1000.0,
        ZoomPreset::Samples => 44100.0,
        ZoomPreset::FourPixelsPerSample => 44100.0 * 4.0,
        ZoomPreset::MaxZoom => MAX_ZOOM,
    };

    result
        .max(fit_zoom / MAX_ZOOM_OUT_FACTOR)
        .min(MAX_ZOOM)
}

/// Returns the integer quantization depth required by a sample format.
pub fn quantize_needed(format: SampleFormat) -> Option<u32> {
    match format {
        SampleFormat::Int16 => Some(16),
        SampleFormat::Int24 => Some(24),
        SampleFormat::Float32 => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_settings() -> AppSettings {
        AppSettings::from_prefs(&Prefs::new())
    }

    fn entry(
        id: &str,
        name: &str,
        publisher: &str,
        family: &str,
        is_default: bool,
        is_bundled: bool,
        effect_type: &str,
    ) -> EffectEntry {
        EffectEntry {
            id: id.into(),
            name: name.into(),
            publisher: publisher.into(),
            family: family.into(),
            is_builtin_audacity: family == "Audacity",
            is_default,
            is_bundled,
            effect_type: effect_type.into(),
        }
    }

    #[test]
    fn settings_read_preferences_defaults() {
        let settings = example_settings();
        assert!(settings.auto_scroll);
        assert_eq!(settings.default_view_mode, ViewMode::Waveform);
        assert_eq!(settings.sample_view, SampleView::StemPlot);
        assert_eq!(settings.zoom_preset1, ZoomPreset::ZoomDefault);
        assert_eq!(settings.zoom_preset2, ZoomPreset::FourPixelsPerSample);
        assert_eq!(settings.solo, SoloPolicy::Simple);
        assert_eq!(settings.default_rate, 44100);
        assert_eq!(settings.default_format, SampleFormat::Float32);
    }

    #[test]
    fn unique_names_append_first_unused_number() {
        let existing = vec!["Audio 1".to_owned(), "Audio 3".to_owned()];
        assert_eq!(unique_track_name("Audio", &existing), "Audio 2");
        let existing = vec!["Audio 1".to_owned()];
        assert_eq!(unique_track_name("Audio", &existing), "Audio 2");
    }

    #[test]
    fn recording_names_add_number_date_and_time() {
        let mut settings = example_settings();
        settings.default_audio_track_name = "Audio".to_owned();
        settings.use_track_number = true;
        settings.use_date_stamp = true;
        settings.use_time_stamp = true;
        let names = recording_track_names(
            &settings,
            &["Audio 1".to_owned()],
            4,
            4,
            (2024, 2, 3, 9, 8, 7),
        );
        assert_eq!(
            names,
            vec![
                "Audio 2_5_2024-02-03_09-08-07",
                "Audio 2_6_2024-02-03_09-08-07",
                "Audio 2_7_2024-02-03_09-08-07",
                "Audio 2_8_2024-02-03_09-08-07",
            ]
        );
    }

    #[test]
    fn directory_defaults_and_last_used_paths() {
        let mut prefs = Prefs::new();
        prefs.set(
            "/Directories/Open/LastUsed",
            PrefValue::Str("/last".to_owned()),
        );
        assert_eq!(
            find_default_path(&prefs, DirOp::Open),
            Some(PathBuf::from("/last"))
        );
        prefs.set(
            "/Directories/Open/Default",
            PrefValue::Str("/default".to_owned()),
        );
        assert_eq!(
            find_default_path(&prefs, DirOp::Open),
            Some(PathBuf::from("/default"))
        );

        update_default_path(&mut prefs, DirOp::Save, Path::new("/tmp/project.aup3"));
        assert_eq!(
            prefs.get_string("/Directories/Save/LastUsed"),
            "/tmp"
        );
        update_default_path(&mut prefs, DirOp::Temp, Path::new("/tmp/session"));
        assert_eq!(
            prefs.get_string("/Directories/TempDir"),
            "/tmp/session"
        );
    }

    #[test]
    fn entry_for_uses_metadata_and_unknown_defaults() {
        let find_clipping = entry_for("FindClipping", "Find Clipping");
        assert_eq!(find_clipping.publisher, "Audacity");
        assert_eq!(find_clipping.family, "Audacity");
        assert_eq!(find_clipping.effect_type, "Analyze");
        assert!(find_clipping.is_default);
        assert!(find_clipping.is_bundled);
        assert!(find_clipping.is_builtin_audacity);

        let unknown = entry_for("Unknown", "Unknown Effect");
        assert_eq!(unknown.publisher, EXTRA_PUBLISHER);
        assert_eq!(unknown.family, EXTRA_PUBLISHER);
        assert_eq!(unknown.effect_type, "Process");
        assert!(!unknown.is_default);
        assert!(!unknown.is_bundled);
    }

    #[test]
    fn effect_menu_sort_by_name_places_defaults_first() {
        let entries = vec![
            entry("label", "Label Sounds", "Steve Daulton", "Nyquist", false, true, "Analyze"),
            entry("beat", "Beat Finder", "Audacity", "Nyquist", false, true, "Analyze"),
            entry("find", "Find Clipping", "Audacity", "Audacity", true, true, "Analyze"),
        ];

        let menu = effect_menu(&entries, "sortby:name");
        assert!(matches!(&menu[0], MenuNode::Item { label, .. } if label == "Find Clipping"));
        assert!(matches!(menu[1], MenuNode::Separator));
        assert!(matches!(&menu[2], MenuNode::Item { label, .. } if label == "Beat Finder"));
        assert!(matches!(&menu[3], MenuNode::Item { label, .. } if label == "Label Sounds"));
    }

    #[test]
    fn effect_menu_default_generate_bundled_then_publisher_groups() {
        let entries = vec![
            entry("pluck", "Pluck", "David R.Sky", "Nyquist", false, true, "Generate"),
            entry("tone", "Tone", "Audacity", "Audacity", true, true, "Generate"),
            entry("chirp", "Chirp", "Audacity", "Audacity", true, true, "Generate"),
            entry("rhythm", "Rhythm Track", "Rhythm Publisher", "Nyquist", false, true, "Generate"),
            entry("dtmf", "DTMF Tones", "Audacity", "Audacity", true, true, "Generate"),
            entry("noise", "Noise", "Audacity", "Audacity", true, true, "Generate"),
            entry("silence", "Silence", "Audacity", "Audacity", true, true, "Generate"),
        ];

        // Audacity 3.7.3 (observed): bundled generators flat, sorted by name.
        let default_menu = effect_menu(&entries, "default");
        let labels: Vec<&str> = default_menu
            .iter()
            .filter_map(|n| match n {
                MenuNode::Item { label, .. } => Some(label.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            labels,
            ["Chirp", "DTMF Tones", "Noise", "Pluck", "Rhythm Track", "Silence", "Tone"]
        );
        // groupby:publisher: the five defaults form an "Audacity" submenu,
        // one-item publisher groups are flattened.
        let menu = effect_menu(&entries, "groupby:publisher");
        assert!(matches!(&menu[0], MenuNode::Submenu { label, items } if label == "Audacity" && items.len() == 5));
        assert!(menu.iter().any(|n| matches!(n, MenuNode::Item { label, .. } if label == "Pluck")));
    }

    #[test]
    fn process_default_menu_uses_xml_group_order_and_members() {
        let entries = vec![
            entry("auto", "Auto Duck", "Audacity", "Audacity", true, true, "Process"),
            entry("amp", "Amplify", "Audacity", "Audacity", true, true, "Process"),
            entry("fade", "Fade In", "Audacity", "Audacity", true, true, "Process"),
            entry("eq", "Bass and Treble", "Audacity", "Audacity", true, true, "Process"),
        ];

        let menu = effect_menu(&entries, "default");
        assert!(matches!(
            &menu[0],
            MenuNode::Submenu { label, items }
                if label == "Volume and Compression"
                    && matches!(&items[0], MenuNode::Item { label, .. } if label == "Amplify")
                    && matches!(&items[1], MenuNode::Item { label, .. } if label == "Auto Duck")
        ));
        assert!(matches!(
            &menu[1],
            MenuNode::Submenu { label, items }
                if label == "Fading"
                    && matches!(&items[0], MenuNode::Item { label, .. } if label == "Fade In")
        ));
        assert!(matches!(
            &menu[2],
            MenuNode::Submenu { label, items }
                if label == "EQ and Filters"
                    && matches!(&items[0], MenuNode::Item { label, .. } if label == "Bass and Treble")
        ));
    }

    #[test]
    fn effect_menus_sort_and_group() {
        let entries = vec![
            entry("third-z", "Zeta", "Vendor", "VST3", false, false, "Process"),
            entry("built-b", "Beta", "Audacity", "Audacity", true, true, "Process"),
            entry("third-a", "Alpha", "Vendor", "VST3", false, false, "Process"),
        ];

        let menu = effect_menu(&entries, "sortby:name");
        assert!(matches!(&menu[0], MenuNode::Item { label, .. } if label == "Beta"));
        assert!(matches!(menu[1], MenuNode::Separator));
        assert!(matches!(&menu[2], MenuNode::Item { label, .. } if label == "Alpha"));

        let grouped = effect_menu(&entries, "groupby:publisher");
        assert!(grouped.iter().any(|node| matches!(node, MenuNode::Separator)));
        assert!(grouped.iter().any(|node| matches!(
            node,
            MenuNode::Submenu { label, items } if label == "Vendor" && items.len() == 2
        )));
    }

    #[test]
    fn sort_by_publisher_and_type_adds_expected_prefixes() {
        let entries = vec![
            entry("one", "Alpha", "Vendor A", "Nyquist", false, true, "Process"),
            entry("two", "Beta", "Vendor B", "VST3", false, true, "Process"),
        ];

        let publisher_menu = effect_menu(&entries, "sortby:publisher:name");
        assert!(matches!(
            &publisher_menu[0],
            MenuNode::Item { label, .. } if label == "Vendor A: Alpha"
        ));
        assert!(matches!(
            &publisher_menu[1],
            MenuNode::Item { label, .. } if label == "Vendor B: Beta"
        ));

        let type_menu = effect_menu(&entries, "sortby:type:name");
        assert!(matches!(
            &type_menu[0],
            MenuNode::Item { label, .. } if label == "Nyquist: Alpha"
        ));
        assert!(matches!(
            &type_menu[1],
            MenuNode::Item { label, .. } if label == "VST3: Beta"
        ));
    }

    #[test]
    fn solo_policies_obey_shift_behaviour() {
        let mut solos = [true, false, false];
        apply_solo_click(SoloPolicy::Simple, &mut solos, 1, false);
        assert_eq!(solos, [false, true, false]);
        apply_solo_click(SoloPolicy::Simple, &mut solos, 2, true);
        assert_eq!(solos, [false, true, true]);

        let mut solos = [true, false, false];
        apply_solo_click(SoloPolicy::Multi, &mut solos, 1, false);
        assert_eq!(solos, [true, true, false]);
        apply_solo_click(SoloPolicy::Multi, &mut solos, 2, true);
        assert_eq!(solos, [false, false, true]);

        let mut solos = [true, false];
        apply_solo_click(SoloPolicy::None, &mut solos, 1, false);
        assert_eq!(solos, [true, false]);
    }

    #[test]
    fn zoom_and_seek_behaviours() {
        assert_eq!(
            zoom_of_preset(ZoomPreset::Seconds, 100.0, 1000.0, 10.0),
            5.0
        );
        assert_eq!(
            zoom_of_preset(ZoomPreset::FitToWidth, 100.0, 1000.0, 10.0),
            10.0
        );
        assert_eq!(
            zoom_of_preset(ZoomPreset::ZoomToSelection, 100.0, 1000.0, 17.0),
            17.0
        );
        assert_eq!(
            zoom_of_preset(ZoomPreset::Samples, 100.0, 1000.0, 10.0),
            44100.0
        );

        let mut settings = example_settings();
        settings.seek_short_period = 0.75;
        settings.seek_long_period = 12.0;
        assert_eq!(seek_seconds(&settings, false), 0.75);
        assert_eq!(seek_seconds(&settings, true), 12.0);
    }

    #[test]
    fn sample_format_quantization_depths() {
        assert_eq!(quantize_needed(SampleFormat::Int16), Some(16));
        assert_eq!(quantize_needed(SampleFormat::Int24), Some(24));
        assert_eq!(quantize_needed(SampleFormat::Float32), None);
    }
}
