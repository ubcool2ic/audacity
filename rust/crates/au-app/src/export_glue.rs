//! Export UI and project-to-file glue.
//!
//! The module provides format selection, range mixing, single-range export,
//! and parallel export of label- or track-based splits.

use crate::export_ui::{
    unique_path, ExportItem, ExportMultipleRequest, MetadataEditor, SplitMode,
};
use crate::fx_bridge;
use au_core::tags::Tags;
use au_core::{AudioBuffer, ProjectState};
use au_io::export::{
    export_any,
    intfmt::{
        FlacOptions, PcmOptions, WavPackOptions, SF_FORMAT_AIFF, SF_FORMAT_ALAW,
        SF_FORMAT_FLOAT, SF_FORMAT_PCM_16, SF_FORMAT_PCM_24, SF_FORMAT_PCM_32,
        SF_FORMAT_PCM_U8, SF_FORMAT_ULAW, SF_FORMAT_WAV,
    },
    mp3::{MP3Options, MP3RateMode},
    ogg_opus::{
        OpusApplication, OpusBitrate, OpusCutoff, OpusFrameDuration, OpusOptions,
        OpusVbrMode, VorbisOptions,
    },
    ExportFormat,
};
use eframe::egui;
use std::path::{Path, PathBuf};

/// Selects an export format and edits that format's encoder options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FormatChooser {
    /// Selected format and its current encoder options.
    pub format: ExportFormat,
}

impl Default for FormatChooser {
    fn default() -> Self {
        Self {
            format: ExportFormat::Pcm(PcmOptions::default()),
        }
    }
}

impl FormatChooser {
    /// Creates a chooser from an Audacity-style format name.
    pub fn from_name(name: &str) -> Self {
        let format = match name {
            "AIFF" => ExportFormat::Pcm(PcmOptions {
                header: SF_FORMAT_AIFF,
                ..PcmOptions::default()
            }),
            "FLAC" => ExportFormat::Flac(FlacOptions::default()),
            "MP3" => ExportFormat::Mp3(MP3Options::default()),
            "Ogg Vorbis" => ExportFormat::Vorbis(VorbisOptions::default()),
            "Opus" => ExportFormat::Opus(OpusOptions::default()),
            "WavPack" => ExportFormat::WavPack(WavPackOptions::default()),
            _ => ExportFormat::Pcm(PcmOptions::default()),
        };
        Self { format }
    }

    /// Returns the display name of the selected format.
    pub fn name(&self) -> &'static str {
        match self.format {
            ExportFormat::Pcm(options) if options.header == SF_FORMAT_AIFF => "AIFF",
            ExportFormat::Pcm(_) => "WAV",
            ExportFormat::Flac(_) => "FLAC",
            ExportFormat::Mp3(_) => "MP3",
            ExportFormat::Vorbis(_) => "Ogg Vorbis",
            ExportFormat::Opus(_) => "Opus",
            ExportFormat::WavPack(_) => "WavPack",
        }
    }

    /// Draws the format selector and the currently selected format's options.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let names = ["WAV", "AIFF", "FLAC", "MP3", "Ogg Vorbis", "Opus", "WavPack"];
        let mut selected = self.name().to_owned();
        egui::ComboBox::from_id_salt("export_format")
            .selected_text(&selected)
            .show_ui(ui, |ui| {
                for name in names {
                    ui.selectable_value(&mut selected, name.to_owned(), name);
                }
            });
        if selected != self.name() {
            *self = Self::from_name(&selected);
        }

        ui.separator();
        match &mut self.format {
            ExportFormat::Pcm(options) => {
                ui.label(au_core::i18n::tr_ui("Format options"));
                let mut header = if options.header == SF_FORMAT_AIFF {
                    "AIFF"
                } else {
                    "WAV"
                };
                egui::ComboBox::from_id_salt("pcm_header")
                    .selected_text(header)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut header, "WAV", au_core::i18n::tr_ui("WAV"));
                        ui.selectable_value(&mut header, "AIFF", au_core::i18n::tr_ui("AIFF"));
                    });
                options.header = if header == "AIFF" {
                    SF_FORMAT_AIFF
                } else {
                    SF_FORMAT_WAV
                };

                let encoding_names = [
                    ("16-bit PCM", SF_FORMAT_PCM_16),
                    ("24-bit PCM", SF_FORMAT_PCM_24),
                    ("32-bit PCM", SF_FORMAT_PCM_32),
                    ("32-bit float", SF_FORMAT_FLOAT),
                    ("8-bit unsigned PCM", SF_FORMAT_PCM_U8),
                    ("U-law", SF_FORMAT_ULAW),
                    ("A-law", SF_FORMAT_ALAW),
                ];
                let mut encoding_name = encoding_names
                    .iter()
                    .find(|(_, value)| *value == options.encoding)
                    .map(|(name, _)| *name)
                    .unwrap_or("16-bit PCM");
                egui::ComboBox::from_id_salt("pcm_encoding")
                    .selected_text(encoding_name)
                    .show_ui(ui, |ui| {
                        for (name, _) in encoding_names {
                            ui.selectable_value(&mut encoding_name, name, name);
                        }
                    });
                if let Some((_, encoding)) = encoding_names
                    .iter()
                    .find(|(name, _)| *name == encoding_name)
                {
                    options.encoding = *encoding;
                }
                ui.checkbox(&mut options.needs_dither, au_core::i18n::tr_ui("Dither"));
            }
            ExportFormat::Flac(options) => {
                ui.label(au_core::i18n::tr_ui("FLAC options"));
                egui::ComboBox::from_id_salt("flac_bit_depth")
                    .selected_text(format!("{}-bit", options.bit_depth))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut options.bit_depth, 16, au_core::i18n::tr_ui("16-bit"));
                        ui.selectable_value(&mut options.bit_depth, 24, au_core::i18n::tr_ui("24-bit"));
                    });
                ui.add(egui::Slider::new(&mut options.level, 0..=8).text("Compression level"));
                ui.checkbox(&mut options.needs_dither, au_core::i18n::tr_ui("Dither"));
            }
            ExportFormat::WavPack(options) => {
                ui.label(au_core::i18n::tr_ui("WavPack options"));
                let quality_names = ["Low", "Normal", "High", "Very High"];
                egui::ComboBox::from_id_salt("wavpack_quality")
                    .selected_text(
                        quality_names
                            .get(options.quality as usize)
                            .copied()
                            .unwrap_or("Normal"),
                    )
                    .show_ui(ui, |ui| {
                        for (index, label) in quality_names.iter().enumerate() {
                            ui.selectable_value(&mut options.quality, index as _, *label);
                        }
                    });
                egui::ComboBox::from_id_salt("wavpack_depth")
                    .selected_text(if options.bit_depth == 32 {
                        "32-bit float"
                    } else {
                        match options.bit_depth {
                            16 => "16-bit",
                            24 => "24-bit",
                            _ => "24-bit",
                        }
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut options.bit_depth, 16, au_core::i18n::tr_ui("16-bit"));
                        ui.selectable_value(&mut options.bit_depth, 24, au_core::i18n::tr_ui("24-bit"));
                        ui.selectable_value(&mut options.bit_depth, 32, au_core::i18n::tr_ui("32-bit float"));
                    });
                ui.checkbox(&mut options.hybrid_mode, au_core::i18n::tr_ui("Hybrid mode"));
                ui.checkbox(&mut options.create_correction, au_core::i18n::tr_ui("Create correction file"));
                ui.add(
                    egui::Slider::new(&mut options.bit_rate, 0..=1000)
                        .text("Hybrid bitrate (kbps × 10)"),
                );
                ui.checkbox(&mut options.needs_dither, au_core::i18n::tr_ui("Dither"));
            }
            ExportFormat::Mp3(options) => {
                ui.label(au_core::i18n::tr_ui("MP3 options"));
                egui::ComboBox::from_id_salt("mp3_mode")
                    .selected_text(match options.mode {
                        MP3RateMode::Preset => "Preset",
                        MP3RateMode::Variable => "Variable",
                        MP3RateMode::Average => "Average",
                        MP3RateMode::Constant => "Constant",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut options.mode, MP3RateMode::Preset, au_core::i18n::tr_ui("Preset"));
                        ui.selectable_value(
                            &mut options.mode,
                            MP3RateMode::Variable,
                            "Variable",
                        );
                        ui.selectable_value(&mut options.mode, MP3RateMode::Average, au_core::i18n::tr_ui("Average"));
                        ui.selectable_value(&mut options.mode, MP3RateMode::Constant, au_core::i18n::tr_ui("Constant"));
                    });

                let preset_names = ["Insane", "Extreme", "Standard", "Medium"];
                egui::ComboBox::from_id_salt("mp3_preset")
                    .selected_text(
                        preset_names
                            .get(options.quality_set as usize)
                            .copied()
                            .unwrap_or("Standard"),
                    )
                    .show_ui(ui, |ui| {
                        for (index, label) in preset_names.iter().enumerate() {
                            ui.selectable_value(&mut options.quality_set, index as _, *label);
                        }
                    });
                ui.add(
                    egui::Slider::new(&mut options.quality_vbr, 0..=9)
                        .text("Variable quality"),
                );

                let bitrates = [
                    320, 256, 224, 192, 160, 144, 128, 112, 96, 80, 64, 56, 48, 40, 32,
                ];
                egui::ComboBox::from_id_salt("mp3_abr")
                    .selected_text(format!("{} kbps", options.bitrate_abr))
                    .show_ui(ui, |ui| {
                        for bitrate in bitrates {
                            ui.selectable_value(
                                &mut options.bitrate_abr,
                                bitrate as _,
                                format!("{bitrate} kbps"),
                            );
                        }
                    });
                egui::ComboBox::from_id_salt("mp3_cbr")
                    .selected_text(format!("{} kbps", options.bitrate_cbr))
                    .show_ui(ui, |ui| {
                        for bitrate in bitrates {
                            ui.selectable_value(
                                &mut options.bitrate_cbr,
                                bitrate as _,
                                format!("{bitrate} kbps"),
                            );
                        }
                    });
                ui.label(format!("Channel mode: {:?}", options.channel_mode));
            }
            ExportFormat::Vorbis(options) => {
                ui.label(au_core::i18n::tr_ui("Ogg Vorbis options"));
                ui.add(egui::Slider::new(&mut options.quality, 0..=10).text("Quality"));
            }
            ExportFormat::Opus(options) => {
                ui.label(au_core::i18n::tr_ui("Opus options"));
                let bitrate_choices = [
                    (OpusBitrate::Kbps6, "6 kbps"),
                    (OpusBitrate::Kbps8, "8 kbps"),
                    (OpusBitrate::Kbps16, "16 kbps"),
                    (OpusBitrate::Kbps24, "24 kbps"),
                    (OpusBitrate::Kbps32, "32 kbps"),
                    (OpusBitrate::Kbps40, "40 kbps"),
                    (OpusBitrate::Kbps48, "48 kbps"),
                    (OpusBitrate::Kbps64, "64 kbps"),
                    (OpusBitrate::Kbps80, "80 kbps"),
                    (OpusBitrate::Kbps96, "96 kbps"),
                    (OpusBitrate::Kbps128, "128 kbps"),
                    (OpusBitrate::Kbps160, "160 kbps"),
                    (OpusBitrate::Kbps192, "192 kbps"),
                    (OpusBitrate::Kbps256, "256 kbps"),
                    (OpusBitrate::Auto, "Auto"),
                    (OpusBitrate::Maximum, "Maximum"),
                ];
                egui::ComboBox::from_id_salt("opus_bitrate")
                    .selected_text(
                        bitrate_choices
                            .iter()
                            .find(|(value, _)| *value == options.bitrate)
                            .map(|(_, label)| *label)
                            .unwrap_or("Auto"),
                    )
                    .show_ui(ui, |ui| {
                        for (value, label) in bitrate_choices {
                            ui.selectable_value(&mut options.bitrate, value, label);
                        }
                    });

                ui.add(egui::Slider::new(&mut options.quality, 0..=10).text("Quality"));

                let frame_choices = [
                    (OpusFrameDuration::Ms2_5, "2.5 ms"),
                    (OpusFrameDuration::Ms5, "5 ms"),
                    (OpusFrameDuration::Ms10, "10 ms"),
                    (OpusFrameDuration::Ms20, "20 ms"),
                    (OpusFrameDuration::Ms40, "40 ms"),
                    (OpusFrameDuration::Ms60, "60 ms"),
                ];
                egui::ComboBox::from_id_salt("opus_frame_duration")
                    .selected_text(
                        frame_choices
                            .iter()
                            .find(|(value, _)| *value == options.frame_duration)
                            .map(|(_, label)| *label)
                            .unwrap_or("20 ms"),
                    )
                    .show_ui(ui, |ui| {
                        for (value, label) in frame_choices {
                            ui.selectable_value(&mut options.frame_duration, value, label);
                        }
                    });

                let vbr_choices = [
                    (OpusVbrMode::Off, "Off"),
                    (OpusVbrMode::On, "On"),
                    (OpusVbrMode::Constrained, "Constrained"),
                ];
                egui::ComboBox::from_id_salt("opus_vbr")
                    .selected_text(
                        vbr_choices
                            .iter()
                            .find(|(value, _)| *value == options.vbr_mode)
                            .map(|(_, label)| *label)
                            .unwrap_or("On"),
                    )
                    .show_ui(ui, |ui| {
                        for (value, label) in vbr_choices {
                            ui.selectable_value(&mut options.vbr_mode, value, label);
                        }
                    });

                let application_choices = [
                    (OpusApplication::Speech, "Speech"),
                    (OpusApplication::Audio, "Audio"),
                    (OpusApplication::LowDelay, "Low Delay"),
                ];
                egui::ComboBox::from_id_salt("opus_application")
                    .selected_text(
                        application_choices
                            .iter()
                            .find(|(value, _)| *value == options.application)
                            .map(|(_, label)| *label)
                            .unwrap_or("Audio"),
                    )
                    .show_ui(ui, |ui| {
                        for (value, label) in application_choices {
                            ui.selectable_value(&mut options.application, value, label);
                        }
                    });

                let cutoff_choices = [
                    (OpusCutoff::Auto, "Auto"),
                    (OpusCutoff::Narrowband, "Narrowband"),
                    (OpusCutoff::Mediumband, "Mediumband"),
                    (OpusCutoff::Wideband, "Wideband"),
                    (OpusCutoff::SuperWideband, "Super Wideband"),
                    (OpusCutoff::Fullband, "Fullband"),
                ];
                egui::ComboBox::from_id_salt("opus_cutoff")
                    .selected_text(
                        cutoff_choices
                            .iter()
                            .find(|(value, _)| *value == options.cutoff)
                            .map(|(_, label)| *label)
                            .unwrap_or("Auto"),
                    )
                    .show_ui(ui, |ui| {
                        for (value, label) in cutoff_choices {
                            ui.selectable_value(&mut options.cutoff, value, label);
                        }
                    });
            }
        }
    }
}

/// Mixes selected project tracks over `[t0, t1)` at the project sample rate.
///
/// Muted tracks are omitted. If any track is soloed, only soloed tracks
/// contribute. A centered mono-only mix is returned as one channel; otherwise
/// the result is stereo.
pub fn mixdown(
    state: &ProjectState,
    extras: &au_project::aup3::Aup3Extras,
    t0: f64,
    t1: f64,
    tracks: Option<&[usize]>,
) -> Vec<Vec<f32>> {
    let rate = state.sample_rate.max(1);
    let start_frame = (t0.max(0.0) * rate as f64).floor() as usize;
    let end_frame = (t1.max(0.0) * rate as f64).floor() as usize;
    let frames = end_frame.saturating_sub(start_frame);

    let selected: Vec<usize> = tracks
        .map(|indices| indices.to_vec())
        .unwrap_or_else(|| (0..state.tracks.len()).collect());
    let any_solo = state.tracks.iter().any(|track| track.solo);

    let rendered: Vec<(usize, AudioBuffer)> = selected
        .into_iter()
        .filter_map(|index| {
            let track = state.tracks.get(index)?;
            if track.mute || (any_solo && !track.solo) {
                return None;
            }
            fx_bridge::render_track(state, extras, index).map(|buffer| (index, buffer))
        })
        .collect();

    let mono_output = rendered.iter().all(|(index, buffer)| {
        buffer.channels.len() <= 1
            && state
                .tracks
                .get(*index)
                .map(|track| track.pan == 0.0)
                .unwrap_or(true)
    });

    if mono_output {
        let mut output = vec![0.0f32; frames];
        for (index, buffer) in rendered {
            let Some(source) = buffer.channels.first() else {
                continue;
            };
            let Some(track) = state.tracks.get(index) else {
                continue;
            };
            let source_rate = buffer.sample_rate.max(1) as f64;
            let gain = track.gain;
            for (frame, sample) in output.iter_mut().enumerate() {
                let time = (start_frame + frame) as f64 / rate as f64;
                let position = time * source_rate;
                let left = position.floor() as usize;
                if left >= source.len() {
                    continue;
                }
                let fraction = (position - left as f64) as f32;
                let a = source[left];
                let b = source.get(left + 1).copied().unwrap_or(a);
                *sample += (a + (b - a) * fraction) * gain;
            }
        }
        return vec![output];
    }

    let mut left_output = vec![0.0f32; frames];
    let mut right_output = vec![0.0f32; frames];
    for (index, buffer) in rendered {
        let Some(track) = state.tracks.get(index) else {
            continue;
        };
        let Some(first) = buffer.channels.first() else {
            continue;
        };
        let second = buffer.channels.get(1).unwrap_or(first);
        let source_rate = buffer.sample_rate.max(1) as f64;
        let pan = track.pan.clamp(-1.0, 1.0);
        let left_gain = track.gain * if pan > 0.0 { 1.0 - pan } else { 1.0 };
        let right_gain = track.gain * if pan < 0.0 { 1.0 + pan } else { 1.0 };

        for frame in 0..frames {
            let time = (start_frame + frame) as f64 / rate as f64;
            let position = time * source_rate;
            let source_frame = position.floor() as usize;
            if source_frame >= first.len() {
                continue;
            }
            let fraction = (position - source_frame as f64) as f32;
            let interpolate = |channel: &[f32]| {
                let a = channel[source_frame];
                let b = channel.get(source_frame + 1).copied().unwrap_or(a);
                a + (b - a) * fraction
            };
            left_output[frame] += interpolate(first) * left_gain;
            right_output[frame] += interpolate(second) * right_gain;
        }
    }
    vec![left_output, right_output]
}

/// Exports a mixed project range to `path`.
pub fn export_range(
    path: &Path,
    state: &ProjectState,
    extras: &au_project::aup3::Aup3Extras,
    t0: f64,
    t1: f64,
    tracks: Option<&[usize]>,
    fmt: &ExportFormat,
    tags: &Tags,
) -> Result<(), String> {
    let channels = mixdown(state, extras, t0, t1, tracks);
    export_any(path, &channels, state.sample_rate.max(1), fmt, tags)
        .map_err(|error| error.to_string())
}

/// Plans, renders, and exports a label- or track-split batch in parallel.
pub fn export_multiple(
    state: &ProjectState,
    extras: &au_project::aup3::Aup3Extras,
    req: &ExportMultipleRequest,
    fmt: &ExportFormat,
) -> Result<Vec<PathBuf>, String> {
    let mut items: Vec<ExportItem> = match req.split_mode {
        SplitMode::Labels => {
            let mut labels: Vec<(f64, f64, String)> = state
                .labels
                .iter()
                .flat_map(|label_track| {
                    label_track
                        .labels
                        .iter()
                        .map(|label| (label.start, label.end, label.text.clone()))
                })
                .collect();
            labels.sort_by(|a, b| a.0.total_cmp(&b.0));

            let project_end = (0..state.tracks.len())
                .filter_map(|index| fx_bridge::render_track(state, extras, index))
                .map(|buffer| buffer.len() as f64 / buffer.sample_rate.max(1) as f64)
                .fold(0.0f64, f64::max);

            crate::export_ui::plan_by_labels(
                &labels,
                project_end,
                req.include_audio_before_first_label,
                &req.first_file_name,
                &req.naming_rule,
                &req.tags,
            )
        }
        SplitMode::Tracks => {
            let tracks: Vec<(String, bool, f64, f64)> = state
                .tracks
                .iter()
                .enumerate()
                .filter_map(|(index, track)| {
                    let buffer = fx_bridge::render_track(state, extras, index)?;
                    let end = buffer.len() as f64 / buffer.sample_rate.max(1) as f64;
                    Some((track.name.clone(), track.mute, 0.0, end))
                })
                .collect();
            crate::export_ui::plan_by_tracks(
                &tracks,
                &req.naming_rule,
                &req.prefix,
                &req.tags,
            )
        }
    };

    std::fs::create_dir_all(&req.folder).map_err(|error| error.to_string())?;
    let extension = fmt.extension();
    let mut jobs = Vec::with_capacity(items.len());
    for item in items.drain(..) {
        let path = unique_path(
            &req.folder,
            &item.file_stem,
            extension,
            req.overwrite,
        );
        jobs.push((item, path));
    }

    let count = jobs.len();
    let cancel = au_runtime::CancelToken::new();
    let progress = au_runtime::pool::Progress::new(count as u64);
    au_effects::offline::runtime()
        .pool()
        .install(|| {
            au_runtime::compute::map_items(
                au_runtime::compute::Exec::Parallel,
                &jobs,
                &cancel,
                &progress,
                |(item, path)| {
                    let channels =
                        mixdown(state, extras, item.t0, item.t1, Some(&item.tracks));
                    export_any(path, &channels, state.sample_rate.max(1), fmt, &item.tags)
                        .map_err(|error| au_runtime::JobError::Failed(error.to_string()))?;
                    Ok(path.clone())
                },
            )
        })
        .map_err(|error| format!("{error:?}"))
}

/// Dialog state for a single-range audio export.
pub struct ExportAudioDialog {
    /// Whether the dialog is open.
    pub open: bool,
    /// The format and its encoder options.
    pub chooser: FormatChooser,
    /// Destination path being edited.
    pub path: String,
    /// Whether to export only the current selection.
    pub selection_only: bool,
    tags: Tags,
    metadata_editor: Option<MetadataEditor>,
}

impl ExportAudioDialog {
    /// Creates a dialog initialized with project metadata.
    pub fn new(tags: Tags) -> Self {
        Self {
            open: true,
            chooser: FormatChooser::default(),
            path: String::new(),
            selection_only: false,
            tags,
            metadata_editor: None,
        }
    }

    /// Shows the dialog and returns an export request when Export is pressed.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
    ) -> Option<(PathBuf, ExportFormat, Tags, bool)> {
        let mut close = false;
        let mut open = self.open;
        let mut result = None;

        egui::Window::new(au_core::i18n::tr_ui("Export Audio"))
            .open(&mut open)
            .show(ctx, |ui| {
                self.chooser.ui(ui);
                ui.horizontal(|ui| {
                    ui.label(au_core::i18n::tr_ui("File:"));
                    ui.text_edit_singleline(&mut self.path);
                    if ui.button(au_core::i18n::tr_ui("Browse…")).clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("untitled")
                            .save_file()
                        {
                            self.path = path.display().to_string();
                        }
                    }
                });
                ui.checkbox(&mut self.selection_only, au_core::i18n::tr_ui("Export current selection"));

                if ui.button(au_core::i18n::tr_ui("Edit Metadata…")).clicked() {
                    self.metadata_editor = Some(MetadataEditor::new(self.tags.clone()));
                }

                ui.horizontal(|ui| {
                    if ui.button(au_core::i18n::tr_ui("Cancel")).clicked() {
                        close = true;
                    }
                    if ui.button(au_core::i18n::tr_ui("Export")).clicked() && !self.path.trim().is_empty() {
                        result = Some((
                            PathBuf::from(self.path.trim()),
                            self.chooser.format,
                            self.tags.clone(),
                            self.selection_only,
                        ));
                        close = true;
                    }
                });
            });
        self.open = open && !close;

        if let Some(editor) = &mut self.metadata_editor {
            if let Some(updated) = editor.show(ctx) {
                if let Some(tags) = updated {
                    self.tags = tags;
                }
                self.metadata_editor = None;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au_core::Track;

    fn state_with_tracks() -> ProjectState {
        let audio = AudioBuffer::from_channels(8_000, vec![vec![0.25; 8_000]]);
        ProjectState {
            tracks: vec![Track::new("Track", audio)],
            sample_rate: 8_000,
            ..ProjectState::default()
        }
    }

    #[test]
    fn mixdown_honours_gain_pan_and_mute() {
        let audio = AudioBuffer::from_channels(8_000, vec![vec![0.25; 16]]);
        let mut state = ProjectState {
            tracks: vec![
                Track::new("Panned", audio.clone()),
                Track::new("Muted", audio),
            ],
            sample_rate: 8_000,
            ..ProjectState::default()
        };
        state.tracks[0].gain = 2.0;
        state.tracks[0].pan = 1.0;
        state.tracks[1].mute = true;

        let output = mixdown(
            &state,
            &au_project::aup3::Aup3Extras::default(),
            0.0,
            0.001,
            None,
        );
        assert_eq!(output.len(), 2);
        assert_eq!(output[0][0], 0.0);
        assert!((output[1][0] - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn split_exports_write_unique_wav_files() {
        let directory = tempfile::tempdir().unwrap();
        let mut state = state_with_tracks();
        state.tracks.push(Track::new(
            "Track",
            AudioBuffer::from_channels(8_000, vec![vec![0.1; 8_000]]),
        ));
        let extras = au_project::aup3::Aup3Extras::default();
        let format = ExportFormat::Pcm(PcmOptions::default());

        let by_tracks = ExportMultipleRequest {
            format: "WAV".into(),
            folder: directory.path().to_path_buf(),
            split_mode: SplitMode::Tracks,
            include_audio_before_first_label: false,
            first_file_name: "first".into(),
            naming_rule: crate::export_ui::NamingRule::UseName,
            prefix: "Track".into(),
            overwrite: false,
            tags: Tags::default(),
        };
        let track_paths = export_multiple(&state, &extras, &by_tracks, &format).unwrap();
        assert_eq!(track_paths.len(), 2);
        assert!(track_paths.iter().all(|path| path.exists()));

        let mut by_labels = ExportMultipleRequest {
            format: "WAV".into(),
            folder: directory.path().to_path_buf(),
            split_mode: SplitMode::Labels,
            include_audio_before_first_label: false,
            first_file_name: "label".into(),
            naming_rule: crate::export_ui::NamingRule::UseName,
            prefix: String::new(),
            overwrite: false,
            tags: Tags::default(),
        };
        state.labels.push(au_core::timeline::LabelTrack { name: "Labels".into(),
            labels: vec![
                au_core::timeline::Label {
                    start: 0.0,
                    end: 0.5,
                    text: "Part".into(),
                },
                au_core::timeline::Label {
                    start: 0.5,
                    end: 1.0,
                    text: "Part".into(),
                },
            ],
        });
        by_labels.include_audio_before_first_label = false;
        let label_paths = export_multiple(&state, &extras, &by_labels, &format).unwrap();
        assert_eq!(label_paths.len(), 2);
        assert!(label_paths.iter().all(|path| path.exists()));
    }
}
