use eframe::egui;
use au_plugin_api::PluginFormat;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowFilter {
    All,
    Enabled,
    Disabled,
}

impl Default for ShowFilter {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Clone, Debug, Default)]
pub struct PluginManagerState {
    pub filter: String,
    pub format: Option<PluginFormat>,
    pub show: ShowFilter,
}

pub fn plugin_manager_ui(
    ui: &mut egui::Ui,
    catalog: &mut crate::plugin_bridge::PluginCatalog,
    state: &mut PluginManagerState,
) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut state.filter)
                .hint_text("Filter plugins…"),
        );

        let selected_format = state
            .format
            .map(crate::plugin_bridge::PluginCatalog::format_label)
            .unwrap_or("All");
        egui::ComboBox::from_id_salt("plugin_manager_format")
            .selected_text(selected_format)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut state.format, None, "All");
                let mut formats = Vec::new();
                for descriptor in &catalog.plugins {
                    if !formats.contains(&descriptor.format) {
                        formats.push(descriptor.format);
                    }
                }
                for format in formats {
                    ui.selectable_value(
                        &mut state.format,
                        Some(format),
                        crate::plugin_bridge::PluginCatalog::format_label(format),
                    );
                }
            });

        ui.radio_value(&mut state.show, ShowFilter::All, "All");
        ui.radio_value(&mut state.show, ShowFilter::Enabled, "Enabled");
        ui.radio_value(&mut state.show, ShowFilter::Disabled, "Disabled");

        let scanning = catalog.is_scanning();
        if ui
            .add_enabled(!scanning, egui::Button::new("Rescan"))
            .clicked()
        {
            catalog.start_scan();
        }
        if catalog.is_scanning() {
            ui.spinner();
            ui.label("Scanning…");
        }
    });

    let filter = state.filter.to_lowercase();
    let rows: Vec<(String, String, PluginFormat, String, String, bool)> = catalog
        .plugins
        .iter()
        .filter_map(|descriptor| {
            let enabled = catalog.is_enabled(&descriptor.id);
            let matches_format = state
                .format
                .map(|format| format == descriptor.format)
                .unwrap_or(true);
            let matches_enabled = match state.show {
                ShowFilter::All => true,
                ShowFilter::Enabled => enabled,
                ShowFilter::Disabled => !enabled,
            };
            let path = descriptor.path.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            let matches_text = filter.is_empty()
                || descriptor.name.to_lowercase().contains(&filter)
                || descriptor.vendor.to_lowercase().contains(&filter)
                || descriptor.id.to_lowercase().contains(&filter)
                || path.to_lowercase().contains(&filter);

            (matches_format && matches_enabled && matches_text).then(|| {
                (
                    descriptor.id.clone(),
                    descriptor.name.clone(),
                    descriptor.format,
                    descriptor.vendor.clone(),
                    path,
                    enabled,
                )
            })
        })
        .collect();

    let mut checkbox_changes: Vec<(String, bool)> = Vec::new();
    let mut bulk_enabled = None;

    ui.horizontal(|ui| {
        if ui.button("Enable all").clicked() {
            bulk_enabled = Some(true);
        }
        if ui.button("Disable all").clicked() {
            bulk_enabled = Some(false);
        }
    });

    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("plugin_manager_grid")
            .striped(true)
            .show(ui, |ui| {
                ui.label("Enabled");
                ui.label("Name");
                ui.label("Type");
                ui.label("Vendor");
                ui.label("Path");
                ui.end_row();

                for (id, name, format, vendor, path, initial_enabled) in &rows {
                    let mut enabled = *initial_enabled;
                    if ui.checkbox(&mut enabled, "").changed() {
                        checkbox_changes.push((id.clone(), enabled));
                    }
                    ui.label(name);
                    ui.label(crate::plugin_bridge::PluginCatalog::format_label(
                        *format,
                    ));
                    ui.label(vendor);
                    ui.label(path);
                    ui.end_row();
                }
            });
    });

    for (id, enabled) in checkbox_changes {
        catalog.set_enabled(&id, enabled);
    }
    if let Some(enabled) = bulk_enabled {
        for (id, _, _, _, _, _) in &rows {
            catalog.set_enabled(id, enabled);
        }
    }

    ui.label(format!(
        "{} plugins ({} enabled)",
        catalog.plugins.len(),
        catalog.enabled().count()
    ));
    if let Some(error) = catalog.last_error.as_deref() {
        ui.colored_label(egui::Color32::RED, error);
    }
}

