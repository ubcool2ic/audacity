//! Metadata editing and Export Multiple planning/UI helpers.

use au_core::tags::{
    Tags, ALBUM, ARTIST, COMMENTS, GENRE, TITLE, TRACK, YEAR,
};
use eframe::egui::{self, ComboBox, ScrollArea};
use std::path::{Path, PathBuf};

const STANDARD_TAGS: [(&str, &str); 7] = [
    ("Artist Name", ARTIST),
    ("Track Title", TITLE),
    ("Album Title", ALBUM),
    ("Track Number", TRACK),
    ("Year", YEAR),
    ("Genre", GENRE),
    ("Comments", COMMENTS),
];

const GENRES: &[&str] = &[
    "Blues",
    "Classic Rock",
    "Country",
    "Dance",
    "Disco",
    "Funk",
    "Grunge",
    "Hip-Hop",
    "Jazz",
    "Metal",
    "New Age",
    "Oldies",
    "Other",
    "Pop",
    "R&B",
    "Rap",
    "Reggae",
    "Rock",
    "Techno",
    "Industrial",
    "Alternative",
    "Ska",
    "Death Metal",
    "Pranks",
    "Soundtrack",
    "Euro-Techno",
    "Ambient",
    "Trip-Hop",
    "Vocal",
    "Jazz+Funk",
    "Fusion",
    "Trance",
    "Classical",
    "Instrumental",
    "Acid",
    "House",
    "Game",
    "Sound Clip",
    "Gospel",
    "Noise",
    "Alt. Rock",
    "Bass",
    "Soul",
    "Punk",
    "Space",
    "Meditative",
    "Instrumental Pop",
    "Instrumental Rock",
    "Ethnic",
    "Gothic",
    "Darkwave",
    "Techno-Industrial",
    "Electronic",
    "Pop-Folk",
    "Eurodance",
    "Dream",
    "Southern Rock",
    "Comedy",
    "Cult",
    "Gangsta Rap",
    "Top 40",
    "Christian Rap",
    "Pop/Funk",
    "Jungle",
    "Native American",
    "Cabaret",
    "New Wave",
    "Psychedelic",
    "Rave",
    "Showtunes",
    "Trailer",
    "Lo-Fi",
    "Tribal",
    "Acid Punk",
    "Acid Jazz",
    "Polka",
    "Retro",
    "Musical",
    "Rock & Roll",
    "Hard Rock",
    "Folk",
    "Folk/Rock",
    "National Folk",
    "Swing",
    "Fast-Fusion",
    "Bebob",
    "Latin",
    "Revival",
    "Celtic",
    "Bluegrass",
    "Avantgarde",
    "Gothic Rock",
    "Progressive Rock",
    "Psychedelic Rock",
    "Symphonic Rock",
    "Slow Rock",
    "Big Band",
    "Chorus",
    "Easy Listening",
    "Acoustic",
    "Humour",
    "Speech",
    "Chanson",
    "Opera",
    "Chamber Music",
    "Sonata",
    "Symphony",
    "Booty Bass",
    "Primus",
    "Porn Groove",
    "Satire",
    "Slow Jam",
    "Club",
    "Tango",
    "Samba",
    "Folklore",
    "Ballad",
    "Power Ballad",
    "Rhythmic Soul",
    "Freestyle",
    "Duet",
    "Punk Rock",
    "Drum Solo",
    "A Cappella",
    "Euro-House",
    "Dance Hall",
    "Goa",
    "Drum & Bass",
    "Club-House",
    "Hardcore",
    "Terror",
    "Indie",
    "BritPop",
    "Offensive",
    "Polsk Punk",
    "Beat",
    "Christian Gangsta Rap",
    "Heavy Metal",
    "Black Metal",
    "Crossover",
    "Contemporary Christian",
    "Christian Rock",
    "Merengue",
    "Salsa",
    "Thrash Metal",
    "Anime",
    "JPop",
    "Synthpop",
];

fn tags_from_entries(entries: &[(String, String)]) -> Tags {
    let mut tags = Tags::new();
    for (name, value) in entries {
        tags.set(name, value);
    }
    tags
}

fn clone_tags(tags: &Tags) -> Tags {
    tags_from_entries(&tags.entries())
}

fn is_standard_tag(name: &str) -> bool {
    STANDARD_TAGS
        .iter()
        .any(|(_, key)| name.eq_ignore_ascii_case(key))
}

/// State for Audacity-style metadata editing.
pub struct MetadataEditor {
    open: bool,
    standard: [String; 7],
    custom: Vec<(String, String)>,
    template_path: String,
    message: String,
}

impl MetadataEditor {
    /// Opens an editor initialized from the supplied metadata.
    pub fn new(tags: Tags) -> Self {
        let mut standard: [String; 7] = std::array::from_fn(|_| String::new());
        let mut custom = Vec::new();

        for (index, (_, key)) in STANDARD_TAGS.iter().enumerate() {
            standard[index] = tags.get(key).unwrap_or_default().to_owned();
        }

        for (name, value) in tags.entries() {
            if !is_standard_tag(&name) {
                custom.push((name, value));
            }
        }

        Self {
            open: true,
            standard,
            custom,
            template_path: String::new(),
            message: String::new(),
        }
    }

    fn build_tags(&self) -> Tags {
        let mut tags = Tags::new();
        for (index, (_, key)) in STANDARD_TAGS.iter().enumerate() {
            tags.set(key, &self.standard[index]);
        }
        for (name, value) in &self.custom {
            tags.set(name, value);
        }
        tags
    }

    fn load_template(&mut self) {
        match std::fs::read_to_string(&self.template_path) {
            Ok(contents) => {
                let mut parsed = Tags::new();
                for line in contents.lines() {
                    if let Some((name, value)) = line.split_once('=') {
                        parsed.set(name.trim(), value);
                    }
                }
                let entries = parsed.entries();
                self.standard = std::array::from_fn(|index| {
                    parsed
                        .get(STANDARD_TAGS[index].1)
                        .unwrap_or_default()
                        .to_owned()
                });
                self.custom = entries
                    .into_iter()
                    .filter(|(name, _)| !is_standard_tag(name))
                    .collect();
                self.message.clear();
            }
            Err(error) => self.message = format!("Unable to load template: {error}"),
        }
    }

    fn save_template(&mut self) {
        let mut contents = String::new();
        for (name, value) in self.build_tags().entries() {
            contents.push_str(&name);
            contents.push('=');
            contents.push_str(&value);
            contents.push('\n');
        }

        match std::fs::write(&self.template_path, contents) {
            Ok(()) => self.message = "Template saved.".to_owned(),
            Err(error) => self.message = format!("Unable to save template: {error}"),
        }
    }

    /// Displays the editor.
    ///
    /// Returns `None` while open, `Some(Some(tags))` after OK, or `Some(None)`
    /// after Cancel or closing the window.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<Option<Tags>> {
        if !self.open {
            return Some(None);
        }

        let mut open = self.open;
        let mut result = None;

        egui::Window::new(au_core::i18n::tr_ui("Edit Metadata Tags"))
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| {
                ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    egui::Grid::new("metadata_standard_tags")
                        .num_columns(2)
                        .striped(true)
                        .show(ui, |ui| {
                            for (index, (label, _)) in STANDARD_TAGS.iter().enumerate() {
                                ui.label(*label);
                                if index == 5 {
                                    let current = &self.standard[index];
                                    ComboBox::from_id_salt("metadata_genre")
                                        .selected_text(if current.is_empty() {
                                            "Select genre"
                                        } else {
                                            current.as_str()
                                        })
                                        .show_ui(ui, |ui| {
                                            for genre in GENRES {
                                                ui.selectable_value(
                                                    &mut self.standard[index],
                                                    (*genre).to_owned(),
                                                    *genre,
                                                );
                                            }
                                        });
                                } else {
                                    ui.text_edit_singleline(&mut self.standard[index]);
                                }
                                ui.end_row();
                            }
                        });

                    ui.separator();
                    ui.label(au_core::i18n::tr_ui("Custom tags"));

                    let mut remove_index = None;
                    egui::Grid::new("metadata_custom_tags")
                        .num_columns(3)
                        .striped(true)
                        .show(ui, |ui| {
                            for (index, (name, value)) in self.custom.iter_mut().enumerate() {
                                ui.text_edit_singleline(name);
                                ui.text_edit_singleline(value);
                                if ui.small_button(au_core::i18n::tr_ui("Remove")).clicked() {
                                    remove_index = Some(index);
                                }
                                ui.end_row();
                            }
                        });
                    if let Some(index) = remove_index {
                        self.custom.remove(index);
                    }

                    ui.horizontal(|ui| {
                        if ui.button(au_core::i18n::tr_ui("Add")).clicked() {
                            self.custom.push((String::new(), String::new()));
                        }
                        if ui.button(au_core::i18n::tr_ui("Remove last")).clicked() {
                            self.custom.pop();
                        }
                        if ui.button(au_core::i18n::tr_ui("Clear")).clicked() {
                            self.standard = std::array::from_fn(|_| String::new());
                            self.custom.clear();
                            self.message.clear();
                        }
                    });

                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(au_core::i18n::tr_ui("Template path:"));
                        ui.text_edit_singleline(&mut self.template_path);
                        if ui.button(au_core::i18n::tr_ui("Load")).clicked() {
                            self.load_template();
                        }
                        if ui.button(au_core::i18n::tr_ui("Save")).clicked() {
                            self.save_template();
                        }
                    });
                    if !self.message.is_empty() {
                        ui.label(&self.message);
                    }
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(au_core::i18n::tr_ui("OK")).clicked() {
                        result = Some(Some(self.build_tags()));
                    }
                    if ui.button(au_core::i18n::tr_ui("Cancel")).clicked() {
                        result = Some(None);
                    }
                });
            });

        self.open = open;
        if result.is_some() {
            self.open = false;
            return result;
        }
        if !self.open {
            return Some(None);
        }
        None
    }
}

/// How Export Multiple divides the project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitMode {
    /// Divide at label boundaries.
    Labels,
    /// Export each unmuted track separately.
    Tracks,
}

/// How Export Multiple names its files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NamingRule {
    /// Use each label or track name.
    UseName,
    /// Prefix the name with a two-or-more-digit sequence number.
    NumberBeforeName,
    /// Use a prefix followed by a hyphen and a sequence number.
    NumberWithPrefix(String),
}

/// One planned file from an Export Multiple operation.
pub struct ExportItem {
    /// Start time in seconds.
    pub t0: f64,
    /// End time in seconds.
    pub t1: f64,
    /// Track indices contributing to the file.
    pub tracks: Vec<usize>,
    /// Sanitized filename without an extension.
    pub file_stem: String,
    /// Metadata to write to this file.
    pub tags: Tags,
}

fn sanitise_filename(name: &str) -> String {
    let mut output = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_control() || matches!(character, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            output.push('_');
        } else {
            output.push(character);
        }
    }

    let trimmed = output.trim_end_matches([' ', '.']).to_owned();
    if trimmed.is_empty() {
        "untitled".to_owned()
    } else {
        trimmed
    }
}

fn unique_stem(candidate: String, used: &mut Vec<String>) -> String {
    let candidate = sanitise_filename(&candidate);
    if !used
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        used.push(candidate.clone());
        return candidate;
    }

    let mut suffix = 2usize;
    loop {
        let proposed = format!("{candidate}-{suffix}");
        if !used
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&proposed))
        {
            used.push(proposed.clone());
            return proposed;
        }
        suffix += 1;
    }
}

fn label_or_track_stem(rule: &NamingRule, title: &str, number: usize) -> String {
    match rule {
        NamingRule::UseName => title.to_owned(),
        NamingRule::NumberBeforeName => format!("{number:02}-{title}"),
        NamingRule::NumberWithPrefix(prefix) => format!("{prefix}-{number:02}"),
    }
}

fn set_file_tags(base_tags: &Tags, title: &str, number: usize) -> Tags {
    let mut tags = clone_tags(base_tags);
    tags.set(TITLE, title);
    tags.set(TRACK, &number.to_string());
    tags
}

/// Plans one export for each label region, optionally including audio before
/// the first label.
///
/// Point labels end at the next label's start time, or at `project_end` for
/// the final label. Non-point labels use their own end time.
pub fn plan_by_labels(
    labels: &[(f64, f64, String)],
    project_end: f64,
    include_before_first: bool,
    first_file_name: &str,
    rule: &NamingRule,
    base_tags: &Tags,
) -> Vec<ExportItem> {
    let mut items = Vec::new();
    let mut used_names = Vec::new();

    let mut indices: Vec<Option<usize>> = Vec::new();
    if include_before_first {
        indices.push(None);
    }
    indices.extend((0..labels.len()).map(Some));

    for label_index in indices {
        let (t0, t1, raw_title, number) = match label_index {
            None => {
                let title = first_file_name
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(first_file_name)
                    .rsplit_once('.')
                    .map(|(stem, _)| stem)
                    .unwrap_or(first_file_name);
                let end = labels.first().map(|label| label.0).unwrap_or(project_end);
                (0.0, end, title.to_owned(), 0)
            }
            Some(index) => {
                let (start, end, title) = &labels[index];
                let region_end = if start != end {
                    *end
                } else if let Some(next) = labels.get(index + 1) {
                    next.0
                } else {
                    project_end
                };
                (*start, region_end, title.clone(), index + 1)
            }
        };

        let title = if raw_title.is_empty() {
            "untitled".to_owned()
        } else {
            raw_title
        };
        let numbered_name = match rule {
            NamingRule::NumberWithPrefix(prefix) => {
                format!("{prefix}-{:02}", number)
            }
            _ => label_or_track_stem(rule, &title, number),
        };
        let file_stem = unique_stem(numbered_name, &mut used_names);
        items.push(ExportItem {
            t0,
            t1,
            tracks: Vec::new(),
            file_stem,
            tags: set_file_tags(base_tags, &title, number),
        });
    }

    items
}

/// Plans one export per unmuted track.
///
/// The supplied track start and end times are used as the exported range.
pub fn plan_by_tracks(
    tracks: &[(String, bool, f64, f64)],
    rule: &NamingRule,
    prefix_or_first: &str,
    base_tags: &Tags,
) -> Vec<ExportItem> {
    let mut items = Vec::new();
    let mut used_names = Vec::new();
    let mut file_number = 0usize;

    for (track_index, (raw_title, muted, start, end)) in tracks.iter().enumerate() {
        if *muted {
            continue;
        }

        file_number += 1;
        let title = if raw_title.is_empty() {
            "untitled"
        } else {
            raw_title.as_str()
        };
        let name = match rule {
            NamingRule::UseName => title.to_owned(),
            NamingRule::NumberBeforeName => format!("{file_number:02}-{title}"),
            NamingRule::NumberWithPrefix(prefix) => {
                let prefix = if prefix.is_empty() {
                    prefix_or_first
                } else {
                    prefix.as_str()
                };
                format!("{prefix}-{file_number:02}")
            }
        };

        items.push(ExportItem {
            t0: *start,
            t1: *end,
            tracks: vec![track_index],
            file_stem: unique_stem(name, &mut used_names),
            tags: set_file_tags(base_tags, title, file_number),
        });
    }

    items
}

/// Returns the destination path for an export.
///
/// When `overwrite` is false, existing paths are avoided using Audacity's
/// `-2`, `-3`, ... suffix convention.
pub fn unique_path(dir: &Path, stem: &str, ext: &str, overwrite: bool) -> PathBuf {
    let extension = ext.strip_prefix('.').unwrap_or(ext);
    let make_path = |name: &str| {
        if extension.is_empty() {
            dir.join(name)
        } else {
            dir.join(format!("{name}.{extension}"))
        }
    };

    let original = make_path(stem);
    if overwrite || !original.exists() {
        return original;
    }

    let mut suffix = 2usize;
    loop {
        let candidate = make_path(&format!("{stem}-{suffix}"));
        if !candidate.exists() {
            return candidate;
        }
        suffix += 1;
    }
}

/// Values returned after confirming the Export Multiple dialog.
pub struct ExportMultipleRequest {
    /// Selected format name.
    pub format: String,
    /// Destination folder.
    pub folder: PathBuf,
    /// Whether to split at labels or tracks.
    pub split_mode: SplitMode,
    /// Include the region before the first label.
    pub include_audio_before_first_label: bool,
    /// Name used for the initial region before the first label.
    pub first_file_name: String,
    /// Rule for naming split files.
    pub naming_rule: NamingRule,
    /// Filename prefix used by the number-with-prefix rule.
    pub prefix: String,
    /// Whether existing files may be replaced.
    pub overwrite: bool,
    /// Base metadata for the exported files.
    pub tags: Tags,
}

/// Egui state for the Export Multiple dialog.
pub struct ExportMultipleDialog {
    open: bool,
    format_index: usize,
    folder: String,
    split_mode: SplitMode,
    include_audio_before_first_label: bool,
    first_file_name: String,
    naming_rule_index: usize,
    prefix: String,
    overwrite: bool,
    tags: Tags,
    metadata_editor: Option<MetadataEditor>,
}

impl Default for ExportMultipleDialog {
    fn default() -> Self {
        Self::new(Tags::new())
    }
}

impl ExportMultipleDialog {
    /// Creates the dialog with initial metadata.
    pub fn new(tags: Tags) -> Self {
        Self {
            open: true,
            format_index: 0,
            folder: String::new(),
            split_mode: SplitMode::Tracks,
            include_audio_before_first_label: false,
            first_file_name: "untitled".to_owned(),
            naming_rule_index: 0,
            prefix: String::new(),
            overwrite: false,
            tags,
            metadata_editor: None,
        }
    }

    /// Displays the dialog and returns a request after Export is confirmed.
    ///
    /// Returns `None` while the dialog is open or after it is cancelled.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<ExportMultipleRequest> {
        if !self.open {
            return None;
        }

        const FORMATS: &[&str] = &["WAV", "AIFF", "FLAC", "MP3", "Ogg Vorbis", "Opus", "WavPack"];
        const RULES: &[&str] = &[
            "Using Label/Track Name",
            "Numbering before Label/Track Name",
            "Numbering after File name prefix",
        ];

        let mut open = self.open;
        let mut cancel = false;
        let mut request = None;
        let mut edit_metadata = false;

        egui::Window::new(au_core::i18n::tr_ui("Export Multiple"))
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(au_core::i18n::tr_ui("Format:"));
                    ComboBox::from_id_salt("export_multiple_format")
                        .selected_text(FORMATS[self.format_index])
                        .show_ui(ui, |ui| {
                            for (index, format) in FORMATS.iter().enumerate() {
                                ui.selectable_value(
                                    &mut self.format_index,
                                    index,
                                    *format,
                                );
                            }
                        });
                });

                ui.horizontal(|ui| {
                    ui.label(au_core::i18n::tr_ui("Folder:"));
                    ui.text_edit_singleline(&mut self.folder);
                });

                ui.separator();
                ui.label(au_core::i18n::tr_ui("Split files based on:"));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.split_mode, SplitMode::Tracks, au_core::i18n::tr_ui("Tracks"));
                    ui.radio_value(&mut self.split_mode, SplitMode::Labels, au_core::i18n::tr_ui("Labels"));
                });

                ui.add_enabled_ui(self.split_mode == SplitMode::Labels, |ui| {
                    ui.checkbox(
                        &mut self.include_audio_before_first_label,
                        "Include audio before first label",
                    );
                    ui.horizontal(|ui| {
                        ui.label(au_core::i18n::tr_ui("First file name:"));
                        ui.text_edit_singleline(&mut self.first_file_name);
                    });
                });

                ui.separator();
                ui.label(au_core::i18n::tr_ui("Name files:"));
                ComboBox::from_id_salt("export_multiple_naming_rule")
                    .selected_text(RULES[self.naming_rule_index])
                    .show_ui(ui, |ui| {
                        for (index, name) in RULES.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.naming_rule_index,
                                index,
                                *name,
                            );
                        }
                    });

                if self.naming_rule_index == 2 {
                    ui.horizontal(|ui| {
                        ui.label(au_core::i18n::tr_ui("File name prefix:"));
                        ui.text_edit_singleline(&mut self.prefix);
                    });
                }

                ui.checkbox(&mut self.overwrite, au_core::i18n::tr_ui("Overwrite existing files"));

                ui.horizontal(|ui| {
                    if ui.button(au_core::i18n::tr_ui("Edit Metadata...")).clicked() {
                        edit_metadata = true;
                    }
                    ui.add_space(ui.available_width());
                    if ui.button(au_core::i18n::tr_ui("Cancel")).clicked() {
                        cancel = true;
                    }
                    if ui.button(au_core::i18n::tr_ui("Export")).clicked() {
                        let naming_rule = match self.naming_rule_index {
                            1 => NamingRule::NumberBeforeName,
                            2 => NamingRule::NumberWithPrefix(self.prefix.clone()),
                            _ => NamingRule::UseName,
                        };
                        request = Some(ExportMultipleRequest {
                            format: FORMATS[self.format_index].to_owned(),
                            folder: PathBuf::from(&self.folder),
                            split_mode: self.split_mode,
                            include_audio_before_first_label: self
                                .include_audio_before_first_label,
                            first_file_name: self.first_file_name.clone(),
                            naming_rule,
                            prefix: self.prefix.clone(),
                            overwrite: self.overwrite,
                            tags: clone_tags(&self.tags),
                        });
                        cancel = true;
                    }
                });
            });

        self.open = open && !cancel;
        if edit_metadata {
            self.metadata_editor = Some(MetadataEditor::new(clone_tags(&self.tags)));
        }

        if let Some(editor) = &mut self.metadata_editor {
            if let Some(result) = editor.show(ctx) {
                if let Some(tags) = result {
                    self.tags = tags;
                }
                self.metadata_editor = None;
            }
        }

        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_planning_handles_points_and_numbering() {
        let labels = vec![
            (1.0, 1.0, "Intro".to_owned()),
            (2.0, 3.0, "Part/One".to_owned()),
        ];
        let tags = Tags::new();
        let items = plan_by_labels(
            &labels,
            5.0,
            true,
            "Project.wav",
            &NamingRule::NumberBeforeName,
            &tags,
        );

        assert_eq!(items.len(), 3);
        assert_eq!((items[0].t0, items[0].t1), (0.0, 1.0));
        assert_eq!(items[0].file_stem, "00-Project");
        assert_eq!((items[1].t0, items[1].t1), (1.0, 2.0));
        assert_eq!(items[1].file_stem, "01-Intro");
        assert_eq!((items[2].t0, items[2].t1), (2.0, 3.0));
        assert_eq!(items[2].file_stem, "02-Part_One");
        assert_eq!(items[0].tags.get(TRACK), Some("0"));
        assert_eq!(items[2].tags.get(TITLE), Some("Part/One"));
    }

    #[test]
    fn track_planning_skips_muted_tracks_and_uniquifies_names() {
        let tracks = vec![
            ("Same".to_owned(), false, 0.5, 2.0),
            ("Muted".to_owned(), true, 0.0, 4.0),
            ("Same".to_owned(), false, 1.0, 3.0),
        ];
        let items = plan_by_tracks(
            &tracks,
            &NamingRule::UseName,
            "",
            &Tags::new(),
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].tracks, vec![0]);
        assert_eq!(items[1].tracks, vec![2]);
        assert_eq!(items[0].file_stem, "Same");
        assert_eq!(items[1].file_stem, "Same-2");
        assert_eq!(items[1].tags.get(TRACK), Some("2"));
    }

    #[test]
    fn number_prefix_and_unique_path_follow_export_conventions() {
        let items = plan_by_tracks(
            &[("A".to_owned(), false, 0.0, 1.0)],
            &NamingRule::NumberWithPrefix("Export".to_owned()),
            "",
            &Tags::new(),
        );
        assert_eq!(items[0].file_stem, "Export-01");

        let dir = std::env::temp_dir().join(format!(
            "audacity-export-ui-{}",
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        let first = unique_path(&dir, "sample", "wav", false);
        std::fs::write(&first, []).expect("create test file");
        let second = unique_path(&dir, "sample", ".wav", false);
        assert_eq!(second.file_stem().and_then(|v| v.to_str()), Some("sample-2"));
        assert_eq!(unique_path(&dir, "sample", ".wav", true), first);
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_dir(dir);
    }

    #[test]
    fn genre_lookup_uses_audacity_genre_table() {
        assert_eq!(au_core::tags::genre_index("Blues"), Some(0));
        assert_eq!(au_core::tags::genre_index("Synthpop"), Some(GENRES.len() - 1));
    }
}
