use super::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The identifier for a built-in toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolbarId {
    /// Transport controls.
    Transport,
    /// Tool and zoom controls.
    Tools,
    /// Undo, redo, and zoom controls.
    Edit,
    /// Cut, copy, paste, and delete controls.
    CutCopyPaste,
    /// Playback and recording meters.
    Meter,
    /// Mixer visibility control.
    Mixer,
    /// Play-at-speed controls.
    Transcription,
    /// Spectral selection controls.
    Spectral,
    /// Playhead time display.
    Time,
    /// Selection controls.
    Selection,
    /// Snapping and sync-lock controls.
    Snapping,
    /// Audio setup controls.
    AudioSetup,
    /// Sharing controls.
    Share,
}

impl ToolbarId {
    /// All built-in toolbar identifiers.
    pub const ALL: [Self; 13] = [
        Self::Transport,
        Self::Tools,
        Self::Edit,
        Self::CutCopyPaste,
        Self::Meter,
        Self::Mixer,
        Self::Transcription,
        Self::Spectral,
        Self::Time,
        Self::Selection,
        Self::Snapping,
        Self::AudioSetup,
        Self::Share,
    ];

    /// Returns the stable configuration key for this toolbar.
    pub fn key(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Tools => "tools",
            Self::Edit => "edit",
            Self::CutCopyPaste => "cutcopypaste",
            Self::Meter => "meter",
            Self::Mixer => "mixer",
            Self::Transcription => "transcription",
            Self::Spectral => "spectral",
            Self::Time => "time",
            Self::Selection => "selection",
            Self::Snapping => "snapping",
            Self::AudioSetup => "audiosetup",
            Self::Share => "share",
        }
    }

    /// Returns the toolbar's display title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Transport => "Transport",
            Self::Tools => "Tools",
            Self::Edit => "Edit",
            Self::CutCopyPaste => "Cut/Copy/Paste",
            Self::Meter => "Meter",
            Self::Mixer => "Mixer",
            Self::Transcription => "Play-at-Speed",
            Self::Spectral => "Spectral Selection",
            Self::Time => "Time",
            Self::Selection => "Selection",
            Self::Snapping => "Snapping",
            Self::AudioSetup => "Audio Setup",
            Self::Share => "Share Audio",
        }
    }

    /// Parses a stable configuration key.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.key() == key)
    }
}

/// The dock location of a toolbar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Dock {
    /// Docked at the top of the application.
    Top,
    /// Docked at the bottom of the application.
    Bottom,
    /// Floating at the given screen position.
    Floating {
        /// Horizontal screen position.
        x: f32,
        /// Vertical screen position.
        y: f32,
    },
}

/// The visibility and layout state of one toolbar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BarState {
    /// Toolbar identifier.
    pub id: ToolbarId,
    /// Whether the toolbar is visible.
    pub visible: bool,
    /// Toolbar dock location.
    pub dock: Dock,
    /// Row index within its dock.
    pub row: u32,
    /// Order within its row.
    pub order: u32,
}

/// The persisted layout of all toolbars.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarLayout {
    /// States for all toolbars.
    pub bars: Vec<BarState>,
}

impl ToolbarLayout {
    /// Creates the Audacity 3.7-style default toolbar layout.
    pub fn default_layout() -> Self {
        use ToolbarId::*;

        let bars = [
            (Transport, true, Dock::Top, 0, 0),
            (Tools, true, Dock::Top, 0, 1),
            (Edit, true, Dock::Top, 0, 4),
            (CutCopyPaste, false, Dock::Top, 1, 3),
            (Meter, true, Dock::Top, 0, 5),
            (Mixer, true, Dock::Top, 1, 0),
            (Transcription, true, Dock::Top, 1, 1),
            (Spectral, false, Dock::Top, 1, 2),
            (Time, true, Dock::Bottom, 0, 2),
            (Selection, true, Dock::Bottom, 0, 0),
            (Snapping, true, Dock::Bottom, 0, 1),
            (AudioSetup, true, Dock::Top, 0, 2),
            (Share, true, Dock::Top, 0, 3),
        ]
        .into_iter()
        .map(|(id, visible, dock, row, order)| BarState {
            id,
            visible,
            dock,
            row,
            order,
        })
        .collect();

        Self { bars }
    }

    /// Returns the state for a toolbar.
    ///
    /// Layouts produced by this module always contain every toolbar.
    pub fn get(&self, id: ToolbarId) -> &BarState {
        self.bars
            .iter()
            .find(|bar| bar.id == id)
            .expect("toolbar layout must contain every toolbar")
    }

    /// Returns the mutable state for a toolbar.
    ///
    /// Layouts produced by this module always contain every toolbar.
    pub fn get_mut(&mut self, id: ToolbarId) -> &mut BarState {
        self.bars
            .iter_mut()
            .find(|bar| bar.id == id)
            .expect("toolbar layout must contain every toolbar")
    }

    /// Returns visible docked toolbars grouped into rows, ordered by row and then order.
    pub fn rows(&self, top: bool) -> Vec<Vec<ToolbarId>> {
        let mut grouped: BTreeMap<u32, Vec<(u32, usize, ToolbarId)>> = BTreeMap::new();

        for (index, bar) in self.bars.iter().enumerate() {
            let matches_dock = if top {
                matches!(bar.dock, Dock::Top)
            } else {
                matches!(bar.dock, Dock::Bottom)
            };
            if bar.visible && matches_dock {
                grouped
                    .entry(bar.row)
                    .or_default()
                    .push((bar.order, index, bar.id));
            }
        }

        grouped
            .into_values()
            .map(|mut row| {
                row.sort_by_key(|(order, index, _)| (*order, *index));
                row.into_iter().map(|(_, _, id)| id).collect()
            })
            .collect()
    }

    /// Sets the visibility of a toolbar.
    pub fn set_visible(&mut self, id: ToolbarId, visible: bool) {
        self.get_mut(id).visible = visible;
    }

    /// Moves a toolbar to the end of a row in the requested dock.
    ///
    /// The requested row is clamped to the end of the existing rows, allowing
    /// callers to pass one past the last row to create a new row.
    pub fn dock_at(&mut self, id: ToolbarId, top: bool, row: u32) {
        let previous_dock = self.get(id).dock;
        self.get_mut(id).dock = Dock::Floating { x: 0.0, y: 0.0 };

        match previous_dock {
            Dock::Top => self.normalize_dock(true),
            Dock::Bottom => self.normalize_dock(false),
            Dock::Floating { .. } => {}
        }
        self.normalize_dock(top);

        let dock_matches = |dock: Dock| {
            if top {
                matches!(dock, Dock::Top)
            } else {
                matches!(dock, Dock::Bottom)
            }
        };
        let row_count = self
            .bars
            .iter()
            .filter(|bar| dock_matches(bar.dock))
            .map(|bar| bar.row)
            .max()
            .map_or(0, |last| last + 1);
        let destination_row = row.min(row_count);

        let order = self
            .bars
            .iter()
            .filter(|bar| dock_matches(bar.dock) && bar.row == destination_row)
            .map(|bar| bar.order)
            .max()
            .map_or(0, |last| last + 1);

        let bar = self.get_mut(id);
        bar.dock = if top { Dock::Top } else { Dock::Bottom };
        bar.row = destination_row;
        bar.order = order;
        self.normalize_dock(top);
    }

    /// Floats a toolbar at the given screen position.
    pub fn float(&mut self, id: ToolbarId, x: f32, y: f32) {
        let previous_dock = self.get(id).dock;
        self.get_mut(id).dock = Dock::Floating { x, y };
        match previous_dock {
            Dock::Top => self.normalize_dock(true),
            Dock::Bottom => self.normalize_dock(false),
            Dock::Floating { .. } => {}
        }
    }

    /// Moves a toolbar left within its row, if a previous toolbar exists.
    pub fn move_left(&mut self, id: ToolbarId) {
        self.move_in_row(id, false);
    }

    /// Moves a toolbar right within its row, if a following toolbar exists.
    pub fn move_right(&mut self, id: ToolbarId) {
        self.move_in_row(id, true);
    }

    /// Serializes the layout as a line-oriented configuration file.
    pub fn to_text(&self) -> String {
        let mut text = String::from("# audacity-rs toolbars v1\n");
        for bar in &self.bars {
            let (dock, x, y) = match bar.dock {
                Dock::Top => ("top", 0.0, 0.0),
                Dock::Bottom => ("bottom", 0.0, 0.0),
                Dock::Floating { x, y } => ("float", x, y),
            };
            text.push_str(&format!(
                "{} {} {} {} {} {} {}\n",
                bar.id.key(),
                u8::from(bar.visible),
                dock,
                bar.row,
                bar.order,
                x,
                y
            ));
        }
        text
    }

    /// Parses a layout, retaining defaults for unknown or missing toolbars.
    ///
    /// Malformed lines are ignored.
    pub fn from_text(text: &str) -> Self {
        let mut layout = Self::default_layout();

        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let Some(key) = fields.next() else {
                continue;
            };
            let Some(id) = ToolbarId::from_key(key) else {
                continue;
            };
            let Some(visible_text) = fields.next() else {
                continue;
            };
            let visible = match visible_text {
                "0" => false,
                "1" => true,
                _ => continue,
            };
            let Some(dock_text) = fields.next() else {
                continue;
            };
            let Some(row_text) = fields.next() else {
                continue;
            };
            let Some(order_text) = fields.next() else {
                continue;
            };
            let Some(x_text) = fields.next() else {
                continue;
            };
            let Some(y_text) = fields.next() else {
                continue;
            };
            if fields.next().is_some() {
                continue;
            }

            let Ok(row) = row_text.parse::<u32>() else {
                continue;
            };
            let Ok(order) = order_text.parse::<u32>() else {
                continue;
            };
            let Ok(x) = x_text.parse::<f32>() else {
                continue;
            };
            let Ok(y) = y_text.parse::<f32>() else {
                continue;
            };
            let dock = match dock_text {
                "top" => Dock::Top,
                "bottom" => Dock::Bottom,
                "float" => Dock::Floating { x, y },
                _ => continue,
            };

            let bar = layout.get_mut(id);
            bar.visible = visible;
            bar.dock = dock;
            bar.row = row;
            bar.order = order;
        }

        layout
    }

    /// Returns the toolbar configuration path, if a configuration directory exists.
    ///
    /// `AU_TOOLBARS_CFG` overrides the complete path.
    pub fn config_path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("AU_TOOLBARS_CFG") {
            return Some(PathBuf::from(path));
        }

        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .map(|home| home.join(".config"))
            })?;

        Some(base.join("audacity-rs").join("toolbars.cfg"))
    }

    /// Loads the saved layout, returning the default layout on any error.
    pub fn load() -> Self {
        Self::config_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Self::from_text(&text))
            .unwrap_or_else(Self::default_layout)
    }

    /// Saves the layout, creating its parent directory if necessary.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_path().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no configuration directory is available",
            )
        })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.to_text())
    }

    fn normalize_dock(&mut self, top: bool) {
        let mut grouped: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (index, bar) in self.bars.iter().enumerate() {
            let matches_dock = if top {
                matches!(bar.dock, Dock::Top)
            } else {
                matches!(bar.dock, Dock::Bottom)
            };
            if matches_dock {
                grouped.entry(bar.row).or_default().push(index);
            }
        }

        for (new_row, (_, mut indices)) in grouped.into_iter().enumerate() {
            indices.sort_by_key(|index| (self.bars[*index].order, *index));
            for (order, index) in indices.into_iter().enumerate() {
                self.bars[index].row = new_row as u32;
                self.bars[index].order = order as u32;
            }
        }
    }

    fn move_in_row(&mut self, id: ToolbarId, right: bool) {
        let state = *self.get(id);
        let top = match state.dock {
            Dock::Top => Some(true),
            Dock::Bottom => Some(false),
            Dock::Floating { .. } => None,
        };
        let Some(top) = top else {
            return;
        };

        self.normalize_dock(top);
        let mut indices: Vec<usize> = self
            .bars
            .iter()
            .enumerate()
            .filter_map(|(index, bar)| {
                let same_dock = if top {
                    matches!(bar.dock, Dock::Top)
                } else {
                    matches!(bar.dock, Dock::Bottom)
                };
                (same_dock && bar.row == state.row).then_some(index)
            })
            .collect();
        indices.sort_by_key(|index| (self.bars[*index].order, *index));

        let Some(position) = indices.iter().position(|index| self.bars[*index].id == id) else {
            return;
        };
        let neighbor = if right {
            position.checked_add(1).filter(|next| *next < indices.len())
        } else {
            position.checked_sub(1)
        };
        let Some(neighbor) = neighbor else {
            return;
        };

        let first = indices[position];
        let second = indices[neighbor];
        let first_order = self.bars[first].order;
        self.bars[first].order = self.bars[second].order;
        self.bars[second].order = first_order;
    }
}

/// Actions requested from a toolbar grip.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ToolbarAction {
    DockTop(u32),
    DockBottom(u32),
    Float(f32, f32),
    MoveLeft,
    MoveRight,
    NewRow,
    Hide,
}

impl AudacityApp {
    fn toolbar_contents(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        id: ToolbarId,
    ) {
        match id {
            ToolbarId::Transport => {
                ui.horizontal(|ui| {
                    if ui.button(au_core::i18n::tr_ui("Play")).clicked() {
                        self.play();
                    }
                    if ui.button(au_core::i18n::tr_ui("Pause")).clicked() {
                        self.toggle_pause();
                    }
                    if ui.button(au_core::i18n::tr_ui("Stop")).clicked() {
                        self.stop_playback();
                    }
                    if ui
                        .add_enabled(
                            self.recorder.is_none(),
                            egui::Button::new(au_core::i18n::tr_ui("Record")),
                        )
                        .clicked()
                    {
                        self.open_record_options();
                    }
                    if self.recorder.is_some() && ui.button(au_core::i18n::tr_ui("Stop Record")).clicked() {
                        self.stop_recording();
                    }
                    if ui.button(au_core::i18n::tr_ui("Skip to Start")).clicked() {
                        self.seek_transport(0);
                    }
                    if ui.button(au_core::i18n::tr_ui("Skip to End")).clicked() {
                        let duration = self.project_duration();
                        let rate = self.project_rate();
                        let frame = if duration.is_finite() && duration > 0.0 {
                            (duration * f64::from(rate)).min(usize::MAX as f64) as usize
                        } else {
                            0
                        };
                        self.seek_transport(frame);
                    }
                    ui.checkbox(&mut self.loop_play, au_core::i18n::tr_ui("Loop"));
                });
            }
            ToolbarId::Tools => {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tool, Tool::Selection, au_core::i18n::tr_ui("Selection"));
                    ui.selectable_value(&mut self.tool, Tool::Envelope, au_core::i18n::tr_ui("Envelope"));
                    ui.selectable_value(&mut self.tool, Tool::TimeShift, au_core::i18n::tr_ui("Time Shift"));
                    ui.selectable_value(&mut self.tool, Tool::Draw, au_core::i18n::tr_ui("Draw"));
                    if ui.button(au_core::i18n::tr_ui("Zoom In")).clicked() {
                        self.zoom.zoom_in();
                    }
                    if ui.button(au_core::i18n::tr_ui("Zoom Out")).clicked() {
                        self.zoom.zoom_out();
                    }
                    if ui.button(au_core::i18n::tr_ui("Fit")).clicked() {
                        self.fit_zoom(ui.available_width() as f64);
                    }
                });
            }
            ToolbarId::Edit => {
                ui.horizontal(|ui| {
                    if ui.button(au_core::i18n::tr_ui("Undo")).clicked() {
                        let result = self.editor.undo();
                        self.report(result);
                    }
                    if ui.button(au_core::i18n::tr_ui("Redo")).clicked() {
                        let result = self.editor.redo();
                        self.report(result);
                    }
                    if ui.button(au_core::i18n::tr_ui("Zoom In")).clicked() {
                        self.zoom.zoom_in();
                    }
                    if ui.button(au_core::i18n::tr_ui("Zoom Out")).clicked() {
                        self.zoom.zoom_out();
                    }
                    if ui.button(au_core::i18n::tr_ui("Fit")).clicked() {
                        self.fit_zoom(ui.available_width() as f64);
                    }
                });
            }
            ToolbarId::CutCopyPaste => {
                ui.horizontal(|ui| {
                    if ui.button(au_core::i18n::tr_ui("Cut")).clicked() {
                        if let Some(track) = self.selected_track {
                            let selection = self.selection_for_track(track);
                            self.edit_selection(|editor| editor.cut(selection));
                        }
                    }
                    if ui.button(au_core::i18n::tr_ui("Copy")).clicked() {
                        if let Some(track) = self.selected_track {
                            let selection = self.selection_for_track(track);
                            self.edit_selection(|editor| editor.copy(selection));
                        }
                    }
                    if ui.button(au_core::i18n::tr_ui("Paste")).clicked() {
                        if let Some(track) = self.selected_track {
                            let at_frame = self.selection_for_track(track).start;
                            self.edit_selection(|editor| editor.paste(track, at_frame));
                        }
                    }
                    if ui.button(au_core::i18n::tr_ui("Delete")).clicked() {
                        if let Some(track) = self.selected_track {
                            let selection = self.selection_for_track(track);
                            self.edit_selection(|editor| editor.delete(selection));
                        }
                    }
                });
            }
            ToolbarId::Meter => {
                let reading = self.player.as_ref().map(|player| player.handle.meters.load());
                ui.horizontal(|ui| {
                    if let Some(reading) = reading {
                        Self::draw_meter_bar(ui, "L", reading.peak[0], reading.rms[0], self.settings.env_db_range.max(1) as f32);
                        Self::draw_meter_bar(ui, "R", reading.peak[1], reading.rms[1], self.settings.env_db_range.max(1) as f32);
                    } else {
                        Self::draw_meter_bar(ui, "L", 0.0, 0.0, self.settings.env_db_range.max(1) as f32);
                        Self::draw_meter_bar(ui, "R", 0.0, 0.0, self.settings.env_db_range.max(1) as f32);
                    }
                    Self::draw_meter_bar(ui, "REC", self.record_peak, self.record_peak, self.settings.env_db_range.max(1) as f32);
                });
            }
            ToolbarId::Mixer => {
                ui.selectable_value(&mut self.mixer_visible, true, au_core::i18n::tr_ui("Mixer"));
                if !self.mixer_visible {
                    ui.selectable_value(&mut self.mixer_visible, false, au_core::i18n::tr_ui("Mixer"));
                }
            }
            ToolbarId::Transcription => {
                ui.horizontal(|ui| {
                    let mut speed = self.play_speed;
                    if ui
                        .add(egui::Slider::new(&mut speed, 0.1..=3.0).text("Speed"))
                        .changed()
                    {
                        self.set_play_speed(speed);
                    }
                    if ui.button(au_core::i18n::tr_ui("▶ at speed")).clicked() {
                        self.play();
                    }
                });
            }
            ToolbarId::Spectral => {
                ui.horizontal(|ui| {
                    if let Some(selection) = &self.spectral_selection {
                        ui.label(format!("{:.0}–{:.0} Hz", selection.f0(), selection.f1()));
                    } else {
                        ui.label(au_core::i18n::tr_ui("No spectral selection"));
                    }
                    if ui.button(au_core::i18n::tr_ui("Clear")).clicked() {
                        self.spectral_selection = None;
                    }
                    if ui.button(au_core::i18n::tr_ui("Show spectrogram")).clicked() {
                        if let Some(track) = self.selected_track {
                            self.track_views.resize_with(track + 1, || {
                                crate::spectrogram_view::TrackView::Waveform
                            });
                            self.track_views[track] =
                                crate::spectrogram_view::TrackView::Spectrogram;
                        }
                    }
                });
            }
            ToolbarId::Time => {
                let rate = self.project_rate().max(1);
                let frame = self.player.as_ref().map_or(0, Player::position);
                let seconds = frame as f64 / f64::from(rate);
                let hours = (seconds / 3600.0).floor() as u64;
                let minutes = ((seconds / 60.0).floor() as u64) % 60;
                let whole_seconds = seconds.floor() as u64 % 60;
                let milliseconds = (seconds.fract() * 1000.0).round() as u64;
                ui.label(
                    egui::RichText::new(format!(
                        "{hours:02}:{minutes:02}:{whole_seconds:02}.{milliseconds:03}"
                    ))
                    .monospace()
                    .size(20.0),
                );
            }
            ToolbarId::Selection => self.selection_toolbar(ui, ctx),
            ToolbarId::Snapping => {
                let mut mode = self.snapper.mode.clone();
                egui::ComboBox::from_id_salt("toolbar_snap_mode")
                    .selected_text(match &mode {
                        SnapMode::Off => "Off",
                        SnapMode::Grid { secs } if (*secs - 1.0).abs() < f64::EPSILON => {
                            "Grid 1s"
                        }
                        SnapMode::Grid { .. } => "Grid 0.1s",
                        SnapMode::Samples => "Samples",
                        SnapMode::ZeroCrossing => "Zero crossings",
                        SnapMode::Labels => "Labels",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut mode, SnapMode::Off, au_core::i18n::tr_ui("Off"));
                        ui.selectable_value(
                            &mut mode,
                            SnapMode::Grid { secs: 1.0 },
                            "Grid 1s",
                        );
                        ui.selectable_value(
                            &mut mode,
                            SnapMode::Grid { secs: 0.1 },
                            "Grid 0.1s",
                        );
                        ui.selectable_value(&mut mode, SnapMode::Samples, au_core::i18n::tr_ui("Samples"));
                        ui.selectable_value(
                            &mut mode,
                            SnapMode::ZeroCrossing,
                            "Zero crossings",
                        );
                        ui.selectable_value(&mut mode, SnapMode::Labels, au_core::i18n::tr_ui("Labels"));
                    });
                if mode != self.snapper.mode {
                    self.snapper.mode = mode;
                }
                if ui.checkbox(&mut self.sync_lock, au_core::i18n::tr_ui("Sync-Lock")).changed() {
                    self.editor.set_sync_lock(self.sync_lock);
                }
            }
            ToolbarId::AudioSetup => {
                ui.menu_button(au_core::i18n::tr_ui("Audio Setup"), |ui| {
                    if ui.button(au_core::i18n::tr_ui("Recording options…")).clicked() {
                        self.open_record_options();
                        ui.close_menu();
                    }
                });
            }
            ToolbarId::Share => {
                ui.add_enabled(false, egui::Button::new(au_core::i18n::tr_ui("Share Audio")))
                    .on_hover_text(au_core::i18n::tr_ui("audio.com sharing is not available yet"));
            }
        }
    }

    fn draw_toolbar_frame(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        id: ToolbarId,
    ) -> Option<ToolbarAction> {
        let mut action = None;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                let grip_response = ui
                    .add(egui::Label::new("☰").sense(egui::Sense::click_and_drag()))
                    .on_hover_text(id.title());
                let started = grip_response.drag_started();
                let dragged = grip_response.dragged();
                let drag_id = egui::Id::new(("toolbar_grip_drag", id.key()));
                let was_dragging = ctx
                    .data(|data| data.get_temp::<bool>(drag_id).unwrap_or(false));

                if started || dragged {
                    ctx.data_mut(|data| data.insert_temp(drag_id, true));
                }

                grip_response.context_menu(|ui| {
                    if ui.button(au_core::i18n::tr_ui("Dock at top")).clicked() {
                        action = Some(ToolbarAction::DockTop(self.toolbars.get(id).row));
                        ui.close_menu();
                    }
                    if ui.button(au_core::i18n::tr_ui("Dock at bottom")).clicked() {
                        action = Some(ToolbarAction::DockBottom(self.toolbars.get(id).row));
                        ui.close_menu();
                    }
                    if ui.button(au_core::i18n::tr_ui("Float")).clicked() {
                        let position = ctx.pointer_latest_pos().unwrap_or(egui::Pos2::ZERO);
                        action = Some(ToolbarAction::Float(position.x, position.y));
                        ui.close_menu();
                    }
                    if ui.button(au_core::i18n::tr_ui("Move left")).clicked() {
                        action = Some(ToolbarAction::MoveLeft);
                        ui.close_menu();
                    }
                    if ui.button(au_core::i18n::tr_ui("Move right")).clicked() {
                        action = Some(ToolbarAction::MoveRight);
                        ui.close_menu();
                    }
                    if ui.button(au_core::i18n::tr_ui("New row")).clicked() {
                        action = Some(ToolbarAction::NewRow);
                        ui.close_menu();
                    }
                    if ui.button(au_core::i18n::tr_ui("Hide")).clicked() {
                        action = Some(ToolbarAction::Hide);
                        ui.close_menu();
                    }
                });

                let dragging = was_dragging || started || dragged;
                let released = ctx.input(|input| input.pointer.any_released());
                if released && dragging {
                    ctx.data_mut(|data| data.insert_temp(drag_id, false));
                    if action.is_none() {
                        let position = ctx
                            .pointer_latest_pos()
                            .unwrap_or(grip_response.rect.center());
                        let screen = ctx.input(|input| input.screen_rect);
                        let relative_y = position.y - screen.min.y;
                        if relative_y < screen.height() / 3.0 {
                            action = Some(ToolbarAction::DockTop(0));
                        } else if relative_y > screen.height() * 2.0 / 3.0 {
                            action = Some(ToolbarAction::DockBottom(0));
                        } else {
                            action = Some(ToolbarAction::Float(position.x, position.y));
                        }
                    }
                }

                self.toolbar_contents(ui, ctx, id);
            });
        });
        action
    }

    /// Draws the visible docked toolbars for one dock and applies requested actions.
    pub(super) fn toolbar_dock(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        top: bool,
    ) {
        let rows = self.toolbars.rows(top);
        let mut actions = Vec::new();
        let available_width = ui.available_width();
        let item_spacing = ui.spacing().item_spacing.x;

        for (row_index, row) in rows.iter().enumerate() {
            let ids: Vec<_> = row.iter().copied().collect();
            let mut lines: Vec<Vec<_>> = Vec::new();
            let mut used_width = 0.0;

            for id in ids {
                let width = ctx
                    .data(|data| {
                        data.get_temp::<f32>(egui::Id::new(("toolbar-width", id.key())))
                    })
                    .unwrap_or(0.0);

                if lines
                    .last()
                    .is_some_and(|line| !line.is_empty())
                    && used_width + item_spacing + width > available_width
                {
                    lines.push(Vec::new());
                    used_width = 0.0;
                }

                if lines.is_empty() {
                    lines.push(Vec::new());
                } else if !lines.last().is_some_and(Vec::is_empty) {
                    used_width += item_spacing;
                }

                lines.last_mut().unwrap().push(id);
                used_width += width;
            }

            for line in lines {
                ui.horizontal(|ui| {
                    for id in line {
                        let inner = ui.scope(|ui| self.draw_toolbar_frame(ui, ctx, id));
                        let width = inner.response.rect.width();
                        ctx.data_mut(|data| {
                            data.insert_temp(
                                egui::Id::new(("toolbar-width", id.key())),
                                width,
                            );
                        });

                        if let Some(action) = inner.inner {
                            actions.push((id, action, row_index as u32));
                        }
                    }
                });
            }
        }

        if !actions.is_empty() {
            for (id, action, row) in actions {
                self.apply_toolbar_action(id, action, top, row);
            }
            self.persist_toolbars();
        }
    }

    pub(super) fn floating_toolbars(&mut self, ctx: &egui::Context) {
        let floating: Vec<(ToolbarId, f32, f32)> = self
            .toolbars
            .bars
            .iter()
            .filter_map(|bar| {
                if !bar.visible {
                    return None;
                }
                match bar.dock {
                    Dock::Floating { x, y } => Some((bar.id, x, y)),
                    Dock::Top | Dock::Bottom => None,
                }
            })
            .collect();

        for (id, x, y) in floating {
            let mut open = true;
            let mut action = None;
            egui::Window::new(id.title())
                .id(egui::Id::new(("toolbar", id.key())))
                .default_pos(egui::pos2(x, y))
                .resizable(false)
                .collapsible(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    action = self.draw_toolbar_frame(ui, ctx, id);
                });

            let mut changed = false;
            if !open {
                self.toolbars.set_visible(id, false);
                changed = true;
            }
            if let Some(action) = action {
                self.apply_toolbar_action(id, action, true, 0);
                changed = true;
            }
            if changed {
                self.persist_toolbars();
            }
        }
    }

    /// Adds the toolbar visibility and reset controls to a Toolbars menu.
    pub(super) fn toolbars_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(au_core::i18n::tr_ui("Toolbars"), |ui| {
            for id in ToolbarId::ALL {
                let mut visible = self.toolbars.get(id).visible;
                if ui.checkbox(&mut visible, id.title()).changed() {
                    self.toolbars.set_visible(id, visible);
                    self.persist_toolbars();
                }
            }
            ui.separator();
            if ui.button(au_core::i18n::tr_ui("Reset Toolbars")).clicked() {
                self.toolbars = ToolbarLayout::default_layout();
                self.persist_toolbars();
                ui.close_menu();
            }
        });
    }

    fn apply_toolbar_action(
        &mut self,
        id: ToolbarId,
        action: ToolbarAction,
        new_row_top: bool,
        _current_row: u32,
    ) {
        match action {
            ToolbarAction::DockTop(row) => self.toolbars.dock_at(id, true, row),
            ToolbarAction::DockBottom(row) => self.toolbars.dock_at(id, false, row),
            ToolbarAction::Float(x, y) => self.toolbars.float(id, x, y),
            ToolbarAction::MoveLeft => self.toolbars.move_left(id),
            ToolbarAction::MoveRight => self.toolbars.move_right(id),
            ToolbarAction::NewRow => {
                let row = self.toolbars.rows(new_row_top).len() as u32;
                self.toolbars.dock_at(id, new_row_top, row);
            }
            ToolbarAction::Hide => self.toolbars.set_visible(id, false),
        }
    }

    fn persist_toolbars(&mut self) {
        if let Err(error) = self.toolbars.save() {
            self.status = au_core::i18n::tr_format("Could not save toolbar layout: %s", &[&error]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_has_all_toolbars_and_expected_first_row() {
        let layout = ToolbarLayout::default_layout();
        assert_eq!(layout.bars.len(), 13);
        assert_eq!(layout.rows(true)[0][0], ToolbarId::Transport);
    }

    #[test]
    fn layout_round_trips_after_edits() {
        let mut layout = ToolbarLayout::default_layout();
        layout.dock_at(ToolbarId::Tools, true, 1);
        layout.float(ToolbarId::Meter, 123.5, 456.25);
        layout.move_right(ToolbarId::Transport);
        layout.set_visible(ToolbarId::Spectral, true);

        assert_eq!(ToolbarLayout::from_text(&layout.to_text()), layout);
    }

    #[test]
    fn moving_the_only_bar_out_of_a_row_compacts_rows() {
        let mut layout = ToolbarLayout::default_layout();
        layout.dock_at(ToolbarId::Selection, true, 2);
        assert_eq!(layout.rows(true).len(), 3);

        layout.dock_at(ToolbarId::Selection, true, 0);
        assert_eq!(layout.rows(true).len(), 2);
        assert_eq!(layout.get(ToolbarId::Mixer).row, 1);
    }

    #[test]
    fn malformed_lines_are_ignored() {
        let defaults = ToolbarLayout::default_layout();
        let parsed = ToolbarLayout::from_text(
            "garbage\ntransport 2 top 0 0 0 0\nunknown 1 top 0 0 0 0\n",
        );
        assert_eq!(parsed, defaults);
    }
}
