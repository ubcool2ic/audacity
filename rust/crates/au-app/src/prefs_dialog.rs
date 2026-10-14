//! Audacity-style Preferences dialog.
//!
//! The dialog edits a private configuration copy. Changes are returned only
//! when the user chooses OK; cancelling discards the working copy.

use au_core::prefs::{all_defs, PrefDefault, PrefValue, Prefs};
use eframe::egui;

/// Device names made available to the Preferences dialog by its caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceLists {
    /// Available audio host names.
    pub hosts: Vec<String>,
    /// Available playback device names.
    pub playback: Vec<String>,
    /// Available recording device names.
    pub recording: Vec<String>,
}

/// Audacity preference pages, in dialog order.
const PAGE_TITLES: &[&str] = &[
    "Devices",
    "Playback",
    "Recording",
    "Quality",
    "Interface",
    "Tracks",
    "Tracks Behaviors",
    "Import / Export",
    "Directories",
    "Warnings",
    "Effects",
    "Keyboard",
    "Application",
];

/// Preferences dialog state.
pub struct PreferencesDialog {
    /// Whether the dialog is currently visible.
    pub open: bool,
    page: usize,
    original: Option<Prefs>,
    working: Option<Prefs>,
}

impl Default for PreferencesDialog {
    fn default() -> Self {
        Self::new()
    }
}

impl PreferencesDialog {
    /// Creates a closed Preferences dialog.
    pub fn new() -> Self {
        Self {
            open: false,
            page: 0,
            original: None,
            working: None,
        }
    }

    /// Opens the dialog with an isolated copy of `prefs`.
    ///
    /// The copy is made through the config serializer so that stored unknown
    /// keys are retained as well as all known preferences.
    pub fn open_with(&mut self, prefs: &Prefs) {
        self.begin_edit(prefs);
        self.open = true;
    }

    /// Shows the dialog.
    ///
    /// Returns `None` while the dialog remains open, `Some(Some(prefs))` when
    /// the user accepts the edited preferences, and `Some(None)` when the user
    /// cancels.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        devices: &DeviceLists,
        keyboard_page: &mut dyn FnMut(&mut egui::Ui),
    ) -> Option<Option<Prefs>> {
        if !self.open {
            return None;
        }

        if self.working.is_none() {
            self.open = false;
            return Some(None);
        }

        let mut action = DialogAction::None;
        let titles = page_titles();

        {
            let page = &mut self.page;
            let working = self.working.as_mut().expect("working preferences exist");
            let mut still_open = self.open;

            egui::Window::new(tr("Audacity Preferences"))
                .open(&mut still_open)
                .resizable(true)
                .default_width(900.0)
                .default_height(650.0)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.set_min_height(500.0);

                        ui.allocate_ui_with_layout(
                            egui::vec2(180.0, ui.available_height()),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    for (index, title) in titles.iter().enumerate() {
                                        if ui
                                            .selectable_label(*page == index, tr(title))
                                            .clicked()
                                        {
                                            *page = index;
                                        }
                                    }
                                });
                            },
                        );

                        ui.separator();

                        egui::ScrollArea::vertical()
                            .id_salt("preferences_page")
                            .show(ui, |ui| {
                                ui.heading(tr(titles.get(*page).copied().unwrap_or("Devices")));
                                ui.add_space(8.0);
                                render_page(ui, *page, working, devices, keyboard_page);
                            });
                    });

                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button(tr("OK")).clicked() {
                            action = DialogAction::Accept;
                        }
                        if ui.button(tr("Cancel")).clicked() {
                            action = DialogAction::Cancel;
                        }
                        if ui.button(tr("Help")).clicked() {
                            // Audacity's Help button is intentionally a no-op
                            // in this standalone GUI module.
                        }
                    });
                });

            self.open = still_open;
        }

        match action {
            DialogAction::Accept => {
                self.open = false;
                self.original = None;
                self.working.take().map(Some)
            }
            DialogAction::Cancel => {
                self.cancel_edit();
                Some(None)
            }
            DialogAction::None if !self.open => {
                self.cancel_edit();
                Some(None)
            }
            DialogAction::None => None,
        }
    }

    fn begin_edit(&mut self, prefs: &Prefs) {
        let copy = Prefs::parse_cfg(&prefs.to_cfg());
        self.original = Some(Prefs::parse_cfg(&prefs.to_cfg()));
        self.working = Some(copy);
    }

    fn cancel_edit(&mut self) {
        self.working = None;
        self.original = None;
        self.open = false;
    }
}

/// Returns the visible page titles in Audacity's Preferences dialog order.
pub fn page_titles() -> &'static [&'static str] {
    PAGE_TITLES
}

/// Returns the preference keys whose effective values differ.
pub fn changed_keys(old: &Prefs, new: &Prefs) -> Vec<String> {
    all_defs()
        .iter()
        .filter_map(|def| {
            let changed = match def.default {
                PrefDefault::Bool(_) => old.get_bool(def.key) != new.get_bool(def.key),
                PrefDefault::Int(_) => old.get_int(def.key) != new.get_int(def.key),
                PrefDefault::Double(_) => old.get_double(def.key) != new.get_double(def.key),
                PrefDefault::Str(_) => old.get_string(def.key) != new.get_string(def.key),
            };
            changed.then(|| def.key.to_owned())
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogAction {
    None,
    Accept,
    Cancel,
}

fn tr(message: &str) -> String {
    au_core::i18n::strip_menu_accel(&au_core::i18n::tr(message)).to_string()
}

fn render_page(
    ui: &mut egui::Ui,
    page: usize,
    prefs: &mut Prefs,
    devices: &DeviceLists,
    keyboard_page: &mut dyn FnMut(&mut egui::Ui),
) {
    match PAGE_TITLES.get(page).copied().unwrap_or("Devices") {
        "Devices" => render_devices(ui, prefs, devices),
        "Playback" => render_playback(ui, prefs),
        "Recording" => render_recording(ui, prefs),
        "Quality" => render_quality(ui, prefs),
        "Interface" => render_interface(ui, prefs),
        "Tracks" => render_tracks(ui, prefs),
        "Tracks Behaviors" => render_track_behaviors(ui, prefs),
        "Import / Export" => render_import_export(ui, prefs),
        "Directories" => render_directories(ui, prefs),
        "Warnings" => render_warnings(ui, prefs),
        "Effects" => render_effects(ui, prefs),
        "Keyboard" => keyboard_page(ui),
        "Application" => render_application(ui, prefs),
        _ => {}
    }
}

fn group(ui: &mut egui::Ui, title: &str, contents: impl FnOnce(&mut egui::Ui)) {
    ui.group(|ui| {
        ui.vertical(|ui| {
            ui.heading(tr(title));
            ui.add_space(4.0);
            contents(ui);
        });
    });
    ui.add_space(6.0);
}

fn checkbox(ui: &mut egui::Ui, prefs: &mut Prefs, key: &str, label: &str) {
    let mut value = prefs.get_bool(key);
    if ui.checkbox(&mut value, tr(label)).changed() {
        prefs.set(key, PrefValue::Bool(value));
    }
}

fn text_field(ui: &mut egui::Ui, prefs: &mut Prefs, key: &str, label: &str) {
    let mut value = prefs.get_string(key);
    ui.horizontal(|ui| {
        ui.label(tr(label));
        if ui
            .add(egui::TextEdit::singleline(&mut value).desired_width(260.0))
            .changed()
        {
            prefs.set(key, PrefValue::Str(value.clone()));
        }
    });
}

fn int_field(ui: &mut egui::Ui, prefs: &mut Prefs, key: &str, label: &str) {
    let mut value = prefs.get_int(key);
    ui.horizontal(|ui| {
        ui.label(tr(label));
        if ui.add(egui::DragValue::new(&mut value)).changed() {
            prefs.set(key, PrefValue::Int(value));
        }
    });
}

fn double_field(ui: &mut egui::Ui, prefs: &mut Prefs, key: &str, label: &str) {
    let mut value = prefs.get_double(key);
    ui.horizontal(|ui| {
        ui.label(tr(label));
        if ui.add(egui::DragValue::new(&mut value)).changed() {
            prefs.set(key, PrefValue::Double(value));
        }
    });
}

fn string_choice(
    ui: &mut egui::Ui,
    prefs: &mut Prefs,
    key: &str,
    label: &str,
    choices: &[(&str, &str)],
) {
    let mut value = prefs.get_string(key);
    let selected = choices
        .iter()
        .find(|(stored, _)| *stored == value)
        .map(|(_, label)| tr(label))
        .unwrap_or_else(|| value.clone());

    ui.horizontal(|ui| {
        ui.label(tr(label));
        egui::ComboBox::from_id_salt(("preference", key))
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (stored, choice_label) in choices {
                    ui.selectable_value(&mut value, (*stored).to_owned(), tr(choice_label));
                }
            });
    });

    if value != prefs.get_string(key) {
        prefs.set(key, PrefValue::Str(value));
    }
}

fn device_field(
    ui: &mut egui::Ui,
    prefs: &mut Prefs,
    key: &str,
    label: &str,
    devices: &[String],
) {
    let mut value = prefs.get_string(key);
    ui.horizontal(|ui| {
        ui.label(tr(label));
        if ui
            .add(egui::TextEdit::singleline(&mut value).desired_width(260.0))
            .changed()
        {
            prefs.set(key, PrefValue::Str(value.clone()));
        }

        egui::ComboBox::from_id_salt(("device-list", key))
            .selected_text(tr("Select"))
            .show_ui(ui, |ui| {
                for device in devices {
                    if ui.selectable_label(value == *device, device).clicked() {
                        value = device.clone();
                    }
                }
            });
    });

    if value != prefs.get_string(key) {
        prefs.set(key, PrefValue::Str(value));
    }
}

fn render_devices(ui: &mut egui::Ui, prefs: &mut Prefs, devices: &DeviceLists) {
    group(ui, "Interface", |ui| {
        device_field(ui, prefs, "/AudioIO/Host", "&Host:", &devices.hosts);
        ui.label(tr("Using:"));
    });

    group(ui, "Playback", |ui| {
        device_field(
            ui,
            prefs,
            "/AudioIO/PlaybackDevice",
            "&Device:",
            &devices.playback,
        );
    });

    group(ui, "Recording", |ui| {
        device_field(
            ui,
            prefs,
            "/AudioIO/RecordingDevice",
            "De&vice:",
            &devices.recording,
        );

        let mut channels = prefs.get_int("/AudioIO/RecordChannels").max(1);
        ui.horizontal(|ui| {
            ui.label(tr("Cha&nnels:"));
            egui::ComboBox::from_id_salt("recording_channels")
                .selected_text(if channels == 1 {
                    tr("1 (Mono)")
                } else if channels == 2 {
                    tr("2 (Stereo)")
                } else {
                    channels.to_string()
                })
                .show_ui(ui, |ui| {
                    for count in 1..=16 {
                        let label = match count {
                            1 => tr("1 (Mono)"),
                            2 => tr("2 (Stereo)"),
                            _ => count.to_string(),
                        };
                        ui.selectable_value(&mut channels, count, label);
                    }
                });
        });
        if channels != prefs.get_int("/AudioIO/RecordChannels") {
            prefs.set("/AudioIO/RecordChannels", PrefValue::Int(channels));
        }
    });

    group(ui, "Quality", |ui| {
        int_field(
            ui,
            prefs,
            "/SamplingRate/DefaultProjectSampleRate",
            "D&efault Sample Rate:",
        );
        string_choice(
            ui,
            prefs,
            "/SamplingRate/DefaultProjectSampleFormatChoice",
            "Default Sample &Format:",
            &[
                ("Format16Bit", "16-bit"),
                ("Format24Bit", "24-bit"),
                ("Format32BitFloat", "32-bit float"),
            ],
        );
    });

    group(ui, "Latency", |ui| {
        double_field(
            ui,
            prefs,
            "/AudioIO/LatencyDuration",
            "&Buffer length:",
        );
        double_field(
            ui,
            prefs,
            "/AudioIO/LatencyCorrection",
            "&Latency compensation:",
        );
    });
}

fn render_playback(ui: &mut egui::Ui, prefs: &mut Prefs) {
    group(ui, "Effects Preview", |ui| {
        double_field(ui, prefs, "/AudioIO/EffectsPreviewLen", "&Length:");
    });

    group(ui, "Cut Preview", |ui| {
        double_field(
            ui,
            prefs,
            "/AudioIO/CutPreviewBeforeLen",
            "&Before cut region:",
        );
        double_field(
            ui,
            prefs,
            "/AudioIO/CutPreviewAfterLen",
            "&After cut region:",
        );
    });

    group(ui, "Seek Time when playing", |ui| {
        double_field(
            ui,
            prefs,
            "/AudioIO/SeekShortPeriod",
            "&Short period:",
        );
        double_field(
            ui,
            prefs,
            "/AudioIO/SeekLongPeriod",
            "Lo&ng period:",
        );
    });

    group(ui, "Options", |ui| {
        checkbox(ui, prefs, "/AudioIO/Microfades", "&Micro-fades");
        checkbox(
            ui,
            prefs,
            "/AudioIO/UnpinnedScrubbing",
            "Always scrub un&pinned",
        );
    });
}

fn render_recording(ui: &mut egui::Ui, prefs: &mut Prefs) {
    group(ui, "Options", |ui| {
        checkbox(
            ui,
            prefs,
            "/AudioIO/Duplex",
            "Hear &other tracks while recording (overdub)",
        );
        checkbox(
            ui,
            prefs,
            "/AudioIO/SWPlaythrough",
            "Audible input &monitoring",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/PreferNewTrackRecord",
            "Record on a new track",
        );
        checkbox(
            ui,
            prefs,
            "/Warnings/DropoutDetected",
            "Detect dropouts",
        );
    });

    group(ui, "Sound Activated Recording", |ui| {
        checkbox(ui, prefs, "/AudioIO/SoundActivatedRecord", "&Enable");
        int_field(ui, prefs, "/AudioIO/SilenceLevel", "Le&vel (dB):");
    });

    group(ui, "Name newly recorded tracks", |ui| {
        ui.label(tr("With:"));
        checkbox(
            ui,
            prefs,
            "/GUI/TrackNames/RecordingNameCustom",
            "Custom Track &Name",
        );
        text_field(
            ui,
            prefs,
            "/GUI/TrackNames/RecodingTrackName",
            "Custom name text",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/TrackNames/TrackNumber",
            "&Track Number",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/TrackNames/DateStamp",
            "System &Date",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/TrackNames/TimeStamp",
            "System T&ime",
        );
    });

    group(ui, "Punch and Roll Recording", |ui| {
        double_field(ui, prefs, "/AudioIO/PreRoll", "Pre-ro&ll:");
        double_field(ui, prefs, "/AudioIO/RollCrossfade", "Cross&fade:");
    });
}

fn render_quality(ui: &mut egui::Ui, prefs: &mut Prefs) {
    let dither_choices = [
        ("None", "None"),
        ("Rectangle", "Rectangle"),
        ("Triangle", "Triangle"),
        ("Shaped", "Shaped"),
    ];

    group(ui, "Real-time Conversion", |ui| {
        string_choice(
            ui,
            prefs,
            "/Quality/DitherAlgorithmChoice",
            "&Dither:",
            &dither_choices,
        );
    });

    group(ui, "High-quality Conversion", |ui| {
        string_choice(
            ui,
            prefs,
            "/Quality/DitherAlgorithmChoiceHigh",
            "Dit&her:",
            &dither_choices,
        );
    });
}

fn render_interface(ui: &mut egui::Ui, prefs: &mut Prefs) {
    group(ui, "Display", |ui| {
        let mut language = prefs.get_string("/Locale/Language");
        ui.horizontal(|ui| {
            ui.label(tr("&Language:"));
            egui::ComboBox::from_id_salt("preference_language")
                .selected_text(language.clone())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut language, "System".to_owned(), tr("System"));
                    ui.selectable_value(&mut language, "English".to_owned(), tr("English"));

                    for code in au_core::i18n::available_languages(
                        &au_core::i18n::default_search_dirs(),
                    ) {
                        ui.selectable_value(&mut language, code.clone(), code);
                    }
                });
        });
        if language != prefs.get_string("/Locale/Language") {
            prefs.set("/Locale/Language", PrefValue::Str(language));
        }

        text_field(ui, prefs, "/GUI/Theme", "Th&eme:");
        int_field(ui, prefs, "/GUI/EnvdBRange", "Meter dB &range:");
    });

    group(ui, "Options", |ui| {
        checkbox(
            ui,
            prefs,
            "/GUI/ShowSplashScreen",
            "Show 'How to Get &Help' at launch",
        );
        checkbox(ui, prefs, "/GUI/ShowExtraMenus", "Show e&xtra menus");
        checkbox(
            ui,
            prefs,
            "/GUI/ShowMac",
            "Show alternative &styling (Mac vs PC)",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/BeepOnCompletion",
            "&Beep on completion of longer activities",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/RetainLabels",
            "Re&tain labels if selection snaps to a label",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/RtlWorkaround",
            "Use mostly Left-to-Right layouts in RTL languages",
        );
        checkbox(
            ui,
            prefs,
            "/Locale/CeeNumberFormat",
            "Never use comma as decimal point",
        );
        checkbox(ui, prefs, "/GUI/ShowRMS", "Show RMS");
        checkbox(ui, prefs, "/GUI/ShowClipping", "Show clipping");
    });
}

fn render_tracks(ui: &mut egui::Ui, prefs: &mut Prefs) {
    group(ui, "Display", |ui| {
        checkbox(
            ui,
            prefs,
            "/GUI/TracksFitVerticallyZoomed",
            "Auto-&fit track height",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/CollapseToHalfWave",
            "Use &half-wave display when collapsed",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/AutoScroll",
            "A&uto-scroll if head unpinned",
        );

        string_choice(
            ui,
            prefs,
            "/GUI/DefaultViewModeChoiceNew",
            "Default &view mode:",
            &[
                ("Waveform", "Waveform"),
                ("Spectrogram", "Spectrogram"),
                ("MultiView", "Multi-view"),
            ],
        );

        string_choice(
            ui,
            prefs,
            "/GUI/DefaultWaveformScaleChoice",
            "Default Waveform scale:",
            &[
                ("Linear", "Linear"),
                ("Logarithmic", "Logarithmic"),
                ("dB", "dB"),
            ],
        );

        string_choice(
            ui,
            prefs,
            "/GUI/SampleViewChoice",
            "Display &samples:",
            &[("ConnectDots", "Connect dots"), ("StemPlot", "Stem plot")],
        );

        text_field(
            ui,
            prefs,
            "/GUI/DefaultAudioTrackName",
            "Default audio track &name:",
        );
    });

    let zoom_choices = [
        ("FitToWidth", "Fit to Width"),
        ("ZoomToSelection", "Zoom to Selection"),
        ("ZoomDefault", "Zoom Default"),
        ("Minutes", "Minutes"),
        ("Seconds", "Seconds"),
        ("FifthsOfSeconds", "5ths of Seconds"),
        ("TenthsOfSeconds", "10ths of Seconds"),
        ("TwentiethsOfSeconds", "20ths of Seconds"),
        ("FiftiethsOfSeconds", "50ths of Seconds"),
        ("HundredthsOfSeconds", "100ths of Seconds"),
        ("FiveHundredthsOfSeconds", "500ths of Seconds"),
        ("MilliSeconds", "MilliSeconds"),
        ("Samples", "Samples"),
        ("FourPixelsPerSample", "4 Pixels per Sample"),
        ("MaxZoom", "Max Zoom"),
    ];
    group(ui, "Zoom Toggle", |ui| {
        string_choice(
            ui,
            prefs,
            "/GUI/ZoomPreset1Choice",
            "Preset 1:",
            &zoom_choices,
        );
        string_choice(
            ui,
            prefs,
            "/GUI/ZoomPreset2Choice",
            "Preset 2:",
            &zoom_choices,
        );
    });
}

fn render_track_behaviors(ui: &mut egui::Ui, prefs: &mut Prefs) {
    group(ui, "Behaviors", |ui| {
        checkbox(
            ui,
            prefs,
            "/GUI/SelectAllOnNone",
            "&Select all audio, if selection required",
        );
        checkbox(ui, prefs, "/GUI/EnableCutLines", "Enable cut &lines");
        checkbox(
            ui,
            prefs,
            "/GUI/EditClipCanMove",
            "Editing a clip can &move other clips",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/PasteAsNewClips",
            "&Always paste audio as new clips",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/CircularTrackNavigation",
            "\"Move track focus\" c&ycles repeatedly through tracks",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/TypeToCreateLabel",
            "&Type to create a label",
        );
        checkbox(
            ui,
            prefs,
            "/GUI/DialogForNameNewLabel",
            "Use dialog for the &name of a new label",
        );

        string_choice(
            ui,
            prefs,
            "/GUI/Solo",
            "Solo &Button:",
            &[
                ("Simple", "Simple"),
                ("Multi", "Multi-track"),
                ("None", "None"),
            ],
        );
    });

    group(ui, "Pasted audio", |ui| {
        ui.label(tr(
            "Paste audio from other Audacity project as",
        ));
        string_choice(
            ui,
            prefs,
            "/GUI/AudioTrackPastePolicy",
            "",
            &[
                (
                    "Keep",
                    "Smart clip.\nThe entire source clip will be pasted into your project, allowing you to access\ntrimmed audio data anytime.",
                ),
                (
                    "Discard",
                    "Selected audio only.\nOnly the selected portion of the source clip will be pasted.",
                ),
                (
                    "Ask",
                    "Ask me each time.\nShow dialog each time audio is pasted.",
                ),
            ],
        );
    });
}

fn render_import_export(ui: &mut egui::Ui, prefs: &mut Prefs) {
    group(ui, "Music Imports", |ui| {
        ui.label(tr(
            "When importing a music loop into an empty project, Audacity will:",
        ));
        string_choice(
            ui,
            prefs,
            "/FileFormats/MusicFileImportSettingChoice",
            "",
            &[
                (
                    "Yes",
                    "S&witch view to Beats and Measures and align with musical grid",
                ),
                ("Ask", "&Ask me each time"),
                ("No", "Do &nothing"),
            ],
        );

        ui.add_space(8.0);
        ui.label(tr(
            "When importing a music loop into an existing (non-empty) project, Audacity will:",
        ));
        string_choice(
            ui,
            prefs,
            "/FileFormats/MusicFileImportOnExistingProjectSettingChoice",
            "",
            &[
                (
                    "Match",
                    "Always &match the loop tempo to the project tempo",
                ),
                (
                    "MatchIfBeatsAndMeasures",
                    "&Only match the loop tempo to the project tempo when the ruler displays \"Beats and Measures\"",
                ),
                ("DoNothing", "&Do nothing"),
            ],
        );
    });

    group(ui, "Export", |ui| {
        string_choice(
            ui,
            prefs,
            "/FileFormats/ExportDownMixChoice",
            "Export down-mix choice",
            &[
                ("StereoToMono", "Stereo to Mono"),
                ("MonoToStereo", "Mono to Stereo"),
                ("SplitStereoToMono", "Split stereo to mono"),
            ],
        );
    });

    group(ui, "Import", |ui| {
        checkbox(
            ui,
            prefs,
            "/AudioFiles/NormalizeOnLoad",
            "Normalize audio on load",
        );
        string_choice(
            ui,
            prefs,
            "/FileFormats/CopyOrReadDirectly",
            "Copy or read audio files directly",
            &[("Copy", "Copy"), ("Read", "Read audio files directly")],
        );
        checkbox(
            ui,
            prefs,
            "/FileFormats/ReadUncompressedFilesDirectly",
            "Read uncompressed files directly",
        );
    });
}

fn render_directories(ui: &mut egui::Ui, prefs: &mut Prefs) {
    group(ui, "Default directories", |ui| {
        ui.label(tr(
            "Leave a field empty to go to the last directory used for that operation.\nFill in a field to always go to that directory for that operation.",
        ));
        text_field(ui, prefs, "/DefaultOpenPath", "O&pen:");
        text_field(ui, prefs, "/DefaultSavePath", "S&ave:");
        text_field(ui, prefs, "/DefaultImportPath", "&Import:");
        text_field(ui, prefs, "/DefaultExportPath", "&Export:");
        text_field(ui, prefs, "/DefaultMacrosPath", "&Macro output:");
    });

    group(ui, "Temporary files directory", |ui| {
        text_field(
            ui,
            prefs,
            "/Directories/TempDir",
            "&Location:",
        );
    });
}

fn render_warnings(ui: &mut egui::Ui, prefs: &mut Prefs) {
    group(ui, "Warnings", |ui| {
        checkbox(
            ui,
            prefs,
            "/Warnings/DropoutDetected",
            "Detect dropouts",
        );
    });
}

fn render_effects(ui: &mut egui::Ui, prefs: &mut Prefs) {
    let effect_choices = [
        ("sortby:name", "Sort by effect name"),
        ("sortby:publisher:name", "Sort by publisher and effect name"),
        ("sortby:type:name", "Sort by type and effect name"),
        ("groupby:publisher", "Group by publisher"),
        ("groupby:type", "Group by type"),
        ("default", "Group by category"),
        ("groupby:type:publisher", "Group by type and publisher"),
    ];

    group(ui, "Effect Options", |ui| {
        string_choice(
            ui,
            prefs,
            "/Effects/GroupBy",
            "Effect menu &organization:",
            &effect_choices,
        );
        string_choice(
            ui,
            prefs,
            "/Effects/RealtimeGroupBy",
            "Realtime effect o&rganization:",
            &effect_choices,
        );
    });

    group(ui, "Instruction Set", |ui| {
        checkbox(ui, prefs, "/SSE/GUI", "&Use SSE/SSE2/.../AVX");
    });

    checkbox(
        ui,
        prefs,
        "/Effects/SkipEffectsScanAtStartup",
        "&Skip effects scanning at startup",
    );
}

fn render_application(ui: &mut egui::Ui, prefs: &mut Prefs) {
    group(ui, "Update notifications", |ui| {
        checkbox(
            ui,
            prefs,
            "/Update/DefaultUpdatesChecking",
            "&Check for updates",
        );
        ui.label(tr(
            "App update checking requires network access. In order to protect your privacy, Audacity does not store any personal information.",
        ));
        ui.add_space(8.0);
        ui.label(tr("See %s for more info."));
        ui.label(tr("our Privacy Policy"));
    });

    group(ui, "UUID", |ui| {
        checkbox(
            ui,
            prefs,
            "/SendAnonymousUsageInfo",
            "&Usage info (UUID)",
        );
        ui.label(tr(
            "To help us understand how often people use Audacity, we generate a random ID (UUID) for each installation. This helps us improve features and plan for future updates. This ID does not contain any personally identifiable information.",
        ));
        ui.add_space(8.0);
        ui.label(tr("See %s for more info."));
        ui.label(tr("our Privacy Policy"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_titles_are_in_audacity_order() {
        assert_eq!(
            page_titles(),
            &[
                "Devices",
                "Playback",
                "Recording",
                "Quality",
                "Interface",
                "Tracks",
                "Tracks Behaviors",
                "Import / Export",
                "Directories",
                "Warnings",
                "Effects",
                "Keyboard",
                "Application",
            ]
        );
    }

    #[test]
    fn changed_keys_detects_effective_value_changes() {
        let old = Prefs::new();
        let mut new = Prefs::new();
        new.set(
            "/GUI/ShowExtraMenus",
            PrefValue::Bool(true),
        );
        new.set(
            "/AudioIO/EffectsPreviewLen",
            PrefValue::Double(8.0),
        );

        let changed = changed_keys(&old, &new);
        assert!(changed.contains(&"/GUI/ShowExtraMenus".to_owned()));
        assert!(changed.contains(&"/AudioIO/EffectsPreviewLen".to_owned()));
        assert!(!changed.contains(&"/GUI/ShowSplashScreen".to_owned()));
    }

    #[test]
    fn opening_and_cancelling_does_not_modify_original_preferences() {
        let mut original = Prefs::new();
        original.set(
            "/GUI/ShowExtraMenus",
            PrefValue::Bool(false),
        );
        original.set(
            "/Unknown/PluginSetting",
            PrefValue::Str("kept".to_owned()),
        );

        let before_cfg = original.to_cfg();
        let mut dialog = PreferencesDialog::new();
        dialog.open_with(&original);

        let working = dialog.working.as_mut().expect("working copy");
        working.set(
            "/GUI/ShowExtraMenus",
            PrefValue::Bool(true),
        );
        working.set(
            "/Unknown/PluginSetting",
            PrefValue::Str("changed".to_owned()),
        );

        dialog.cancel_edit();

        assert_eq!(original.to_cfg(), before_cfg);
        assert!(dialog.working.is_none());
        assert!(!dialog.open);
    }
}
