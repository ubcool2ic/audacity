// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! egui dialogs for built-in effects / generators / analyzers and the realtime
//! effect stack panel.

/// Realtime effects menu grouping, set from preferences.
pub static REALTIME_GROUP_BY: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

use au_effects::core::{BuiltinEffect, EffectKind, ParamType};
use au_effects::registry;
use au_effects::stack::{self, Preset};
use au_project::aup3::RealtimeEffectList;
use eframe::egui;

/// What the user asked the dialog to do this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogAction {
    /// Nothing.
    None,
    /// Render a preview of the selection and play it.
    Preview,
    /// Apply to the selection.
    Apply,
    /// Noise Reduction step 1: learn the profile from the selection.
    GetProfile,
    /// Close without applying.
    Close,
}

/// A modeless dialog editing one built-in's parameters.
pub struct EffectDialog {
    /// The effect being edited.
    pub effect: Box<dyn BuiltinEffect>,
    /// Name typed for "Save Preset".
    pub preset_name: String,
    /// Track used as `aux` input (Auto Duck control track / crossfade partner / Contrast background).
    pub aux_track: Option<usize>,
    /// Last analyzer report or error.
    pub report: String,
    /// Whether a noise profile has been captured.
    pub has_profile: bool,
}

impl EffectDialog {
    /// Opens a dialog for an effect id.
    pub fn open(id: &str) -> Option<Self> {
        registry::create(id).map(|effect| Self { effect, preset_name: String::new(), aux_track: None, report: String::new(), has_profile: false })
    }

    /// Whether the effect needs a second track.
    pub fn needs_aux(&self) -> bool {
        matches!(&*self.effect.id(), "AutoDuck" | "CrossfadeTracks" | "CrossfadeClips" | "Contrast" | "Vocoder")
    }

    /// Draws the dialog. `track_names` lists project tracks for the aux picker.
    pub fn show(&mut self, ctx: &egui::Context, track_names: &[String]) -> DialogAction {
        let mut action = DialogAction::None;
        let mut open = true;
        egui::Window::new(self.effect.name()).id(egui::Id::new("fx_dialog")).open(&mut open).resizable(true).show(ctx, |ui| {
            self.presets_row(ui);
            ui.separator();
            egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| { param_grid(ui, self.effect.as_mut()); });
            if self.needs_aux() {
                ui.horizontal(|ui| {
                    ui.label(if self.effect.id() == "AutoDuck" { "Control track:" } else { "Second track:" });
                    egui::ComboBox::from_id_salt("fx_aux").selected_text(self.aux_track.and_then(|i| track_names.get(i).cloned()).unwrap_or_else(|| "(choose)".into()))
                        .show_ui(ui, |ui| { for (i, n) in track_names.iter().enumerate() { ui.selectable_value(&mut self.aux_track, Some(i), n); } });
                });
            }
            if !self.report.is_empty() {
                ui.separator();
                egui::ScrollArea::vertical().id_salt("fx_report").max_height(200.0).show(ui, |ui| {
                    ui.add(egui::TextEdit::multiline(&mut self.report.as_str()).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY));
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                if self.effect.id() == "NoiseReduction" {
                    if ui.button(au_core::i18n::tr_ui("Get Noise Profile")).clicked() { action = DialogAction::GetProfile; }
                    ui.label(if self.has_profile { "profile ✓" } else { "no profile" });
                }
                if self.effect.kind() != EffectKind::Analyze && ui.button(au_core::i18n::tr_ui("Preview")).clicked() { action = DialogAction::Preview; }
                let apply = match self.effect.kind() { EffectKind::Analyze => "Analyze", EffectKind::Generate => "Generate", EffectKind::Process => "Apply" };
                if ui.button(apply).clicked() { action = DialogAction::Apply; }
                if ui.button(au_core::i18n::tr_ui("Close")).clicked() { action = DialogAction::Close; }
            });
        });
        if !open { action = DialogAction::Close; }
        action
    }

    fn presets_row(&mut self, ui: &mut egui::Ui) {
        let id_cow = self.effect.id();
        let id: &str = &id_cow;
        let dir = stack::user_preset_dir();
        ui.horizontal(|ui| {
            ui.menu_button(au_core::i18n::tr_ui("Presets & settings"), |ui| {
                let factory = stack::factory_presets(id);
                if !factory.is_empty() {
                    ui.label(au_core::i18n::tr_ui("Factory presets"));
                    for p in factory { if ui.button(&p.name).clicked() { p.apply(self.effect.as_mut()); ui.close_menu(); } }
                    ui.separator();
                }
                let user = stack::load_user_presets(&dir, id);
                if !user.is_empty() {
                    ui.label(au_core::i18n::tr_ui("User presets"));
                    for p in &user {
                        ui.horizontal(|ui| {
                            if ui.button(&p.name).clicked() { p.apply(self.effect.as_mut()); ui.close_menu(); }
                            if ui.small_button(au_core::i18n::tr_ui("🗑")).on_hover_text(au_core::i18n::tr_ui("Delete preset")).clicked() { let _ = stack::delete_user_preset(&dir, id, &p.name); }
                        });
                    }
                    ui.separator();
                }
                if ui.button(au_core::i18n::tr_ui("Defaults")).clicked() { au_effects::core::reset_defaults(self.effect.as_mut()); ui.close_menu(); }
            });
            ui.text_edit_singleline(&mut self.preset_name).on_hover_text(au_core::i18n::tr_ui("Preset name"));
            if ui.button(au_core::i18n::tr_ui("Save Preset")).clicked() && !self.preset_name.trim().is_empty() {
                let p = Preset::capture(self.preset_name.trim(), self.effect.as_mut());
                self.report = match stack::save_user_preset(&dir, id, &p) { Ok(()) => au_core::i18n::tr_format("Saved preset '%s'", &[&p.name]), Err(e) => au_core::i18n::tr_format("Preset save failed: %s", &[&e]) };
            }
        });
    }
}

/// Generic parameter editor (sliders / checkboxes / combo boxes) for any built-in.
pub fn param_grid(ui: &mut egui::Ui, e: &mut dyn BuiltinEffect) -> bool {
    let mut changed = false;
    egui::Grid::new(("fx_params", e.id())).num_columns(2).striped(true).show(ui, |ui| {
        for p in e.params() {
            ui.label(if p.label.is_empty() { p.key } else { p.label });
            let mut row_changed = false;
            let mut v = e.get(p.key).unwrap_or(p.default);
            let r = match &p.ty {
                ParamType::Bool => { let mut b = v != 0.0; let r = ui.checkbox(&mut b, ""); v = if b { 1.0 } else { 0.0 }; r }
                ParamType::Choice(sy) => {
                    let mut idx = v as usize;
                    let r = egui::ComboBox::from_id_salt((e.id(), p.key)).selected_text(sy.get(idx).copied().unwrap_or("?"))
                        .show_ui(ui, |ui| { for (i, s) in sy.iter().enumerate() { ui.selectable_value(&mut idx, i, *s); } }).response;
                    if idx as f64 != v { row_changed = true; }
                    v = idx as f64;
                    r
                }
                ParamType::Int => { let mut i = v as i64; let r = ui.add(egui::Slider::new(&mut i, p.min as i64..=p.max as i64)); v = i as f64; r }
                ParamType::Float => {
                    let (lo, hi) = (p.min, p.max.min(1e7));
                    let log = lo > 0.0 && hi / lo > 1000.0;
                    ui.add(egui::Slider::new(&mut v, lo..=hi).logarithmic(log).max_decimals(4))
                }
            };
            if r.changed() || row_changed { e.set(p.key, p.clamp(v)); changed = true; }
            ui.end_row();
        }
    });
    changed
}

/// Which realtime list the panel edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RtTarget {
    /// A wave track.
    Track(usize),
    /// The master bus.
    Master,
}

/// Panel listing a realtime effect stack (add / bypass / reorder / remove / edit).
/// Returns true when the list changed (caller restarts playback / marks dirty).

pub fn realtime_panel(
    ui: &mut egui::Ui,
    list: &mut RealtimeEffectList,
    editing: &mut Option<usize>,
    plugins: &crate::plugin_bridge::PluginCatalog,
) -> bool {
    let mut changed = false;
    if ui.checkbox(&mut list.active, au_core::i18n::tr_ui("Stack enabled")).changed() {
        changed = true;
    }

    let mut action: Option<(usize, i32)> = None; // (index, -1 up / +1 down / 0 remove)
    for (i, e) in list.effects.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            if ui.checkbox(&mut e.active, "").on_hover_text(au_core::i18n::tr_ui("Bypass")).changed() {
                changed = true;
            }
            let name = if crate::plugin_bridge::is_plugin_id(&e.id) {
                au_plugin_host::state::effect_name(&e.id).unwrap_or_else(|| e.id.clone())
            } else {
                stack::symbol_from_plugin_id(&e.id).unwrap_or_else(|| e.id.clone())
            };
            if ui.selectable_label(*editing == Some(i), name).clicked() {
                *editing = if *editing == Some(i) { None } else { Some(i) };
            }
            if ui.small_button(au_core::i18n::tr_ui("▲")).clicked() {
                action = Some((i, -1));
            }
            if ui.small_button(au_core::i18n::tr_ui("▼")).clicked() {
                action = Some((i, 1));
            }
            if ui.small_button(au_core::i18n::tr_ui("✖")).clicked() {
                action = Some((i, 0));
            }
        });
    }

    if let Some((i, d)) = action {
        changed = true;
        match d {
            0 => {
                list.effects.remove(i);
                *editing = None;
            }
            -1 if i > 0 => list.effects.swap(i, i - 1),
            1 if i + 1 < list.effects.len() => list.effects.swap(i, i + 1),
            _ => {}
        }
    }

    ui.menu_button(au_core::i18n::tr_ui("Add effect…"), |ui| {
        // Preferences > Effects > Realtime effects grouping (/Effects/RealtimeGroupBy).
        let entries: Vec<crate::settings::EffectEntry> = registry::all()
            .into_iter()
            .filter(|e| e.supports_realtime())
            .map(|e| crate::settings::entry_for(&e.id(), &e.name()))
            .collect();
        let group_by = REALTIME_GROUP_BY.read().map(|g| g.clone()).unwrap_or_default();
        let nodes = crate::settings::effect_menu(&entries, &group_by);
        if let Some(id) = crate::gui::show_effect_nodes(ui, &nodes) {
            {
                let mut st = au_effects::stack::EffectStack::new();
                st.push(&id);
                list.effects
                    .extend(crate::fx_bridge::list_from_stack(&mut st).effects);
                changed = true;
                ui.close_menu();
            }
        }

        ui.separator();
        ui.label(au_core::i18n::tr_ui("Plugins"));
        for d in plugins.enabled() {
            let format_debug = format!("{:?}", d.format).to_ascii_lowercase();
            let format = if format_debug.contains("vst3") {
                "VST3"
            } else {
                "CLAP"
            };
            if ui.button(au_core::i18n::tr_format("%s (%s)", &[&d.name, &format])).clicked() {
                if let Ok(eff) = plugins.instantiate(&d.id) {
                    let mut st = au_effects::stack::EffectStack::new();
                    if st.push_effect(Box::new(eff)) {
                        list.effects
                            .extend(crate::fx_bridge::list_from_stack(&mut st).effects);
                        changed = true;
                    }
                }
                ui.close_menu();
            }
        }
    });

    if let Some(i) = *editing {
        if let Some(saved) = list.effects.get_mut(i) {
            ui.separator();
            if crate::plugin_bridge::is_plugin_id(&saved.id) {
                if let Some(mut slot) = crate::plugin_bridge::slot_from_saved(saved) {
                    if param_grid(ui, slot.effect.as_mut()) {
                        if let Some(updated) = crate::plugin_bridge::saved_from_effect(
                            slot.effect.as_mut(),
                            slot.active,
                        ) {
                            saved.params = updated.params;
                            changed = true;
                        }
                    }
                }
            } else {
                let sym = stack::symbol_from_plugin_id(&saved.id).unwrap_or_default();
                if let Some(mut e) = registry::create(&sym) {
                    stack::params_from_strings(e.as_mut(), &saved.params);
                    if param_grid(ui, e.as_mut()) {
                        saved.params = stack::params_to_strings(e.as_mut());
                        changed = true;
                    }
                }
            }
        }
    }

    changed
}
