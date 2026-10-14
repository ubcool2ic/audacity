//! Dedicated egui windows for Audacity-style spectrum and contrast analysis.

use au_core::AudioBuffer;
use au_effects::fx::fix_contrast::{analyze, ContrastResult};
use au_effects::fx::fix_plotspectrum::{
    FixPlotSpectrum, SpectrumData, ALGORITHMS, AXES, FUNCTIONS, SIZES,
};
use eframe::egui;
use egui::{Align2, Color32, Pos2, Rect, Sense, Stroke, Vec2};
use std::time::{SystemTime, UNIX_EPOCH};

/// A dedicated Plot Spectrum analysis window.
pub struct SpectrumWindow {
    audio: AudioBuffer,
    effect: FixPlotSpectrum,
    data: SpectrumData,
    grids: bool,
    cursor_text: String,
}

impl SpectrumWindow {
    /// Creates a spectrum window and immediately calculates its initial plot.
    pub fn new(audio: AudioBuffer) -> Self {
        let effect = FixPlotSpectrum::default();
        let data = effect.analyse(&audio);
        Self {
            audio,
            effect,
            data,
            grids: true,
            cursor_text: String::new(),
        }
    }

    fn recompute(&mut self) {
        self.data = self.effect.analyse(&self.audio);
        self.cursor_text.clear();
    }

    /// Draws the window and returns `false` when it has been closed.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut open = true; let mut close_clicked = false;
        egui::Window::new("Plot Spectrum")
            .open(&mut open)
            .resizable(true)
            .default_width(700.0)
            .show(ctx, |ui| {
                self.plot(ui);

                let mut replot = false;
                ui.horizontal(|ui| {
                    let mut algorithm = self.effect.algorithm.round() as usize;
                    egui::ComboBox::from_id_salt("spectrum_algorithm")
                        .selected_text(ALGORITHMS.get(algorithm).copied().unwrap_or("?"))
                        .show_ui(ui, |ui| {
                            for (i, label) in ALGORITHMS.iter().enumerate() {
                                ui.selectable_value(&mut algorithm, i, *label);
                            }
                        });
                    if algorithm as f64 != self.effect.algorithm {
                        self.effect.algorithm = algorithm as f64;
                        replot = true;
                    }

                    let mut function = self.effect.function.round() as usize;
                    egui::ComboBox::from_id_salt("spectrum_function")
                        .selected_text(FUNCTIONS.get(function).copied().unwrap_or("?"))
                        .show_ui(ui, |ui| {
                            for (i, label) in FUNCTIONS.iter().enumerate() {
                                ui.selectable_value(&mut function, i, *label);
                            }
                        });
                    if function as f64 != self.effect.function {
                        self.effect.function = function as f64;
                        replot = true;
                    }

                    let mut size = self.effect.size.round() as usize;
                    egui::ComboBox::from_id_salt("spectrum_size")
                        .selected_text(SIZES.get(size).copied().unwrap_or("?"))
                        .show_ui(ui, |ui| {
                            for (i, label) in SIZES.iter().enumerate() {
                                ui.selectable_value(&mut size, i, *label);
                            }
                        });
                    if size as f64 != self.effect.size {
                        self.effect.size = size as f64;
                        replot = true;
                    }
                });

                ui.horizontal(|ui| {
                    let mut axis = self.effect.axis.round() as usize;
                    let axis_enabled = self.effect.algorithm.round() as usize == 0;
                    ui.add_enabled_ui(axis_enabled, |ui| {
                        egui::ComboBox::from_id_salt("spectrum_axis")
                            .selected_text(AXES.get(axis).copied().unwrap_or("?"))
                            .show_ui(ui, |ui| {
                                for (i, label) in AXES.iter().enumerate() {
                                    ui.selectable_value(&mut axis, i, *label);
                                }
                            });
                    });
                    if axis as f64 != self.effect.axis {
                        self.effect.axis = axis as f64;
                        replot = true;
                    }

                    ui.checkbox(&mut self.grids, "Grids");
                    if ui.button("Replot").clicked() {
                        replot = true;
                    }
                    if ui.button("Export...").clicked() {
                        self.export();
                    }
                    if ui.button("Close").clicked() {
                        close_clicked = true;
                    }
                });

                if replot {
                    self.recompute();
                }
            });
        open && !close_clicked
    }

    fn plot(&mut self, ui: &mut egui::Ui) {
        let (outer, response) =
            ui.allocate_exact_size(Vec2::new(500.0, 300.0), Sense::hover());
        let painter = ui.painter_at(outer);
        painter.rect_filled(outer, 0.0, Color32::WHITE);
        painter.rect_stroke(
            outer,
            0.0,
            Stroke::new(1.0, Color32::DARK_GRAY),
        );

        let graph = Rect::from_min_max(
            Pos2::new(outer.left() + 55.0, outer.top() + 12.0),
            Pos2::new(outer.right() - 12.0, outer.bottom() - 36.0),
        );
        if graph.width() <= 1.0 || graph.height() <= 1.0 {
            return;
        }

        let spectrum = self.data.algorithm == 0;
        let enough_samples = self.audio.len() >= self.data.window_size
            && self.data.window_size > 0
            && !self.data.values.is_empty();

        let (y_min, y_max) = if spectrum {
            spectrum_y_range(self.data.y_min, self.data.y_max)
        } else {
            nonzero_range(self.data.y_min, self.data.y_max)
        };

        let (x_min, x_max, logarithmic) = plot_x_range(&self.data, self.effect.axis);
        let x_map = |x: f32| -> f32 {
            let t = if logarithmic && x_min > 0.0 && x > 0.0 {
                (x.ln() - x_min.ln()) / (x_max.ln() - x_min.ln())
            } else {
                (x - x_min) / (x_max - x_min)
            };
            graph.left() + t.clamp(0.0, 1.0) * graph.width()
        };
        let y_map = |y: f32| -> f32 {
            graph.bottom() - (y - y_min) / (y_max - y_min) * graph.height()
        };

        let y_ticks = (0..=5)
            .map(|i| y_min + (y_max - y_min) * i as f32 / 5.0)
            .collect::<Vec<_>>();
        for tick in y_ticks {
            let y = y_map(tick);
            if self.grids {
                painter.line_segment(
                    [Pos2::new(graph.left(), y), Pos2::new(graph.right(), y)],
                    Stroke::new(0.6, Color32::LIGHT_GRAY),
                );
            } else {
                painter.line_segment(
                    [Pos2::new(graph.left() - 4.0, y), Pos2::new(graph.left(), y)],
                    Stroke::new(0.8, Color32::DARK_GRAY),
                );
            }
            painter.text(
                Pos2::new(graph.left() - 7.0, y),
                Align2::RIGHT_CENTER,
                format!("{tick:.1}"),
                egui::FontId::proportional(11.0),
                Color32::DARK_GRAY,
            );
        }

        let x_ticks = make_x_ticks(x_min, x_max, logarithmic);
        for tick in x_ticks {
            let x = x_map(tick);
            if self.grids {
                painter.line_segment(
                    [Pos2::new(x, graph.top()), Pos2::new(x, graph.bottom())],
                    Stroke::new(0.6, Color32::LIGHT_GRAY),
                );
            } else {
                painter.line_segment(
                    [
                        Pos2::new(x, graph.bottom()),
                        Pos2::new(x, graph.bottom() + 4.0),
                    ],
                    Stroke::new(0.8, Color32::DARK_GRAY),
                );
            }
            painter.text(
                Pos2::new(x, graph.bottom() + 5.0),
                Align2::CENTER_TOP,
                format_tick(tick),
                egui::FontId::proportional(11.0),
                Color32::DARK_GRAY,
            );
        }

        painter.rect_stroke(
            graph,
            0.0,
            Stroke::new(1.0, Color32::DARK_GRAY),
        );
        painter.text(
            Pos2::new(graph.center().x, outer.bottom() - 4.0),
            Align2::CENTER_BOTTOM,
            if spectrum {
                "Frequency (Hz)"
            } else {
                self.data.x_label.as_str()
            },
            egui::FontId::proportional(11.0),
            Color32::DARK_GRAY,
        );
        painter.text(
            Pos2::new(outer.left() + 4.0, outer.top() + 3.0),
            Align2::LEFT_TOP,
            if spectrum {
                "Level (dB)"
            } else {
                self.data.y_label.as_str()
            },
            egui::FontId::proportional(11.0),
            Color32::DARK_GRAY,
        );

        if !enough_samples {
            let requested = self.data.window_size.max(window_size_for(&self.effect));
            painter.text(
                graph.center(),
                Align2::CENTER_CENTER,
                format!(
                    "Not enough data selected.\nTo plot the spectrum, select at least {requested} samples."
                ),
                egui::FontId::proportional(14.0),
                Color32::DARK_GRAY,
            );
            self.cursor_text.clear();
            ui.label("");
            return;
        }

        let points = self
            .data
            .values
            .iter()
            .enumerate()
            .filter_map(|(i, &value)| {
                let x = self
                    .data
                    .freq_or_lag
                    .get(i)
                    .copied()
                    .unwrap_or_else(|| default_x_for_bin(&self.data, i));
                if x.is_finite() && value.is_finite() && x >= x_min && x <= x_max {
                    Some(Pos2::new(x_map(x), y_map(value)))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        for pair in points.windows(2) {
            painter.line_segment(
                [pair[0], pair[1]],
                Stroke::new(1.4, Color32::from_rgb(30, 80, 180)),
            );
        }

        self.cursor_text.clear();
        if let Some(pointer) = response.hover_pos() {
            if graph.contains(pointer) {
                let cursor_x = unmap_x(pointer.x, graph, x_min, x_max, logarithmic);
                let cursor_value = interpolate(&self.data, cursor_x);
                if let Some(value) = cursor_value {
                    let (peak_x, peak_value) = self.effect.find_peak(&self.data, cursor_x);
                    painter.line_segment(
                        [
                            Pos2::new(pointer.x, graph.top()),
                            Pos2::new(pointer.x, graph.bottom()),
                        ],
                        Stroke::new(1.0, Color32::RED),
                    );
                    if spectrum {
                        self.cursor_text = format!(
                            "Cursor: {} Hz ({}) = {} dB   Peak: {} Hz ({}) = {:.1} dB",
                            (cursor_x + 0.5) as i32,
                            pitch_name(cursor_x),
                            (value + 0.5) as i32,
                            (peak_x + 0.5) as i32,
                            pitch_name(peak_x),
                            peak_value
                        );
                    } else {
                        self.cursor_text = format!(
                            "Cursor: {:.4} sec = {:.3}   Peak: {:.4} sec = {:.3}",
                            cursor_x, value, peak_x, peak_value
                        );
                    }
                }
            }
        }
        ui.label(&self.cursor_text);
    }

    fn export(&self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Export Spectral Data As:")
            .set_file_name("spectrum.txt")
            .save_file()
        {
            let text = self.effect.export_text(&self.data, self.data.rate);
            let _ = std::fs::write(path, text);
        }
    }
}

/// A dedicated Contrast Analyzer window.
pub struct ContrastWindow {
    /// Foreground selection start and end times, in seconds.
    pub fg: (f64, f64),
    /// Background selection start and end times, in seconds.
    pub bg: (f64, f64),
    fg_result: Option<Result<f32, String>>,
    bg_result: Option<Result<f32, String>>,
    filename: String,
    status: String,
}

impl Default for ContrastWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl ContrastWindow {
    /// Creates a contrast window with empty foreground and background ranges.
    pub fn new() -> Self {
        Self {
            fg: (0.0, 0.0),
            bg: (0.0, 0.0),
            fg_result: None,
            bg_result: None,
            filename: "Audio".to_owned(),
            status: String::new(),
        }
    }

    /// Draws the window, re-measuring changed ranges, and returns whether it remains open.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        measure_audio: &mut dyn FnMut(f64, f64) -> Result<f32, String>,
        selection: Option<(f64, f64)>,
    ) -> bool {
        let mut open = true; let mut close_clicked = false;
        egui::Window::new("Contrast Analysis (WCAG 2 compliance)")
            .open(&mut open)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(
                    "Contrast Analyzer, for measuring RMS volume differences between two selections of audio.",
                );
                ui.separator();
                ui.label("Parameters");
                ui.horizontal(|ui| {
                    ui.label("Foreground:");
                    ui.label("Start");
                    let start_changed = ui
                        .add(egui::DragValue::new(&mut self.fg.0).speed(0.01).fixed_decimals(2))
                        .changed();
                    ui.label("End");
                    let end_changed = ui
                        .add(egui::DragValue::new(&mut self.fg.1).speed(0.01).fixed_decimals(2))
                        .changed();
                    let mut measure_now = start_changed || end_changed;
                    if ui.button("Measure selection").clicked() {
                        if let Some((start, end)) = selection {
                            self.fg = (start, end);
                            measure_now = true;
                        } else {
                            self.fg_result =
                                Some(Err("No audio selection is available.".to_owned()));
                        }
                    }
                    ui.label(measurement_text(&self.fg_result));
                    if measure_now {
                        self.fg_result = Some(measure_audio(self.fg.0, self.fg.1));
                    }
                });

                ui.horizontal(|ui| {
                    ui.label("Background:");
                    ui.label("Start");
                    let start_changed = ui
                        .add(egui::DragValue::new(&mut self.bg.0).speed(0.01).fixed_decimals(2))
                        .changed();
                    ui.label("End");
                    let end_changed = ui
                        .add(egui::DragValue::new(&mut self.bg.1).speed(0.01).fixed_decimals(2))
                        .changed();
                    let mut measure_now = start_changed || end_changed;
                    if ui.button("Measure selection").clicked() {
                        if let Some((start, end)) = selection {
                            self.bg = (start, end);
                            measure_now = true;
                        } else {
                            self.bg_result =
                                Some(Err("No audio selection is available.".to_owned()));
                        }
                    }
                    ui.label(measurement_text(&self.bg_result));
                    if measure_now {
                        self.bg_result = Some(measure_audio(self.bg.0, self.bg.1));
                    }
                });

                ui.separator();
                ui.label("Result");
                let foreground = successful_measurement(&self.fg_result);
                let background = successful_measurement(&self.bg_result);
                let (result_text, difference_text) = match (foreground, background) {
                    (Some(fg), Some(bg)) => {
                        let result = analyze(fg, bg);
                        (
                            contrast_result_text(&result),
                            format_difference((fg - bg).abs()),
                        )
                    }
                    (None, _) => ("Foreground not yet measured".to_owned(), String::new()),
                    (_, None) => ("Background not yet measured".to_owned(), String::new()),
                };
                ui.horizontal(|ui| {
                    ui.label("Contrast Result:");
                    ui.label(result_text);
                });
                ui.horizontal(|ui| {
                    ui.label("Difference:");
                    ui.label(difference_text);
                });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Export...").clicked() {
                        self.export();
                    }
                    if ui.button("Reset").clicked() {
                        self.fg = (0.0, 0.0);
                        self.bg = (0.0, 0.0);
                        self.fg_result = None;
                        self.bg_result = None;
                        self.status.clear();
                    }
                    if ui.button("Close").clicked() {
                        close_clicked = true;
                    }
                });
                if !self.status.is_empty() {
                    ui.label(&self.status);
                }
            });
        open && !close_clicked
    }

    fn export(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("Export Contrast Result As:")
            .set_file_name("contrast.txt")
            .save_file()
        {
            let report = contrast_export_report(
                &self.filename,
                self.fg,
                self.bg,
                successful_measurement(&self.fg_result),
                successful_measurement(&self.bg_result),
            );
            match std::fs::write(path, report) {
                Ok(()) => self.status = "Contrast report exported.".to_owned(),
                Err(error) => self.status = format!("Couldn't write to file: {error}"),
            }
        }
    }
}

fn measurement_text(result: &Option<Result<f32, String>>) -> String {
    match result {
        Some(Ok(value)) if value.is_infinite() && value.is_sign_negative() => "zero".to_owned(),
        Some(Ok(value)) => format!("{value:.2} dB"),
        Some(Err(error)) => error.clone(),
        None => String::new(),
    }
}

fn successful_measurement(result: &Option<Result<f32, String>>) -> Option<f32> {
    match result {
        Some(Ok(value)) => Some(*value),
        _ => None,
    }
}

fn contrast_result_text(result: &ContrastResult) -> String {
    if result.fg_db > 0.0 {
        "Foreground level too high".to_owned()
    } else if result.bg_db > 0.0 {
        "Background level too high".to_owned()
    } else if result.bg_db > result.fg_db {
        "Background higher than foreground".to_owned()
    } else if result.diff_db > 20.0 {
        "WCAG2 Pass".to_owned()
    } else {
        "WCAG2 Fail".to_owned()
    }
}

fn format_difference(diff_db: f32) -> String {
    if diff_db.is_nan() {
        "indeterminate".to_owned()
    } else if diff_db == f32::INFINITY {
        "Infinite dB difference".to_owned()
    } else {
        format!("{diff_db:.2} dB RMS")
    }
}

fn format_rms_message(value: Option<f32>) -> String {
    match value {
        None => "RMS =  dB.".to_owned(),
        Some(value) if value.is_infinite() => "RMS = zero.".to_owned(),
        Some(value) => format!("RMS = {value:.2} dB."),
    }
}

fn format_difference_for_export(diff_db: f32) -> String {
    if diff_db.is_nan() {
        "Difference is indeterminate.".to_owned()
    } else if diff_db.is_infinite() {
        "Difference = infinite RMS dB.".to_owned()
    } else {
        format!("Difference = {diff_db:.2} RMS dB.")
    }
}

fn format_time_line(label: &str, time: f64) -> String {
    let t = time as f32;
    let hours = (t / 3600.0) as i32;
    let minutes = ((t - hours as f32 * 3600.0) / 60.0) as i32;
    let seconds = t - hours as f32 * 3600.0 - minutes as f32 * 60.0;
    format!(
        "{label} = {hours:2} hour(s), {minutes:2} minute(s), {seconds:.2} seconds."
    )
}

fn contrast_export_report(
    filename: &str,
    fg: (f64, f64),
    bg: (f64, f64),
    fg_db: Option<f32>,
    bg_db: Option<f32>,
) -> String {
    let fg_value = fg_db.unwrap_or(0.0);
    let bg_value = bg_db.unwrap_or(0.0);
    let diff = fg_value - bg_value;
    let (day, month, year, hour, minute, second) = utc_date_parts();
    let month_name = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ][month.saturating_sub(1).min(11) as usize];

    format!(
        "===================================\n\
WCAG 2.0 Success Criteria 1.4.7 Contrast Results\n\
\n\
Filename = {filename}.\n\
\n\
Foreground\n\
{}\n\
{}\n\
{}\n\
\n\
Background\n\
{}\n\
{}\n\
{}\n\
\n\
Results\n\
{}\n\
Success Criteria 1.4.7 of WCAG 2.0: {}\n\
\n\
Data gathered\n\
{day} {month_name} {year:04} {hour:02}h {minute:02}m {second:02}s\n\
===================================\n\
\n",
        format_time_line("Time started", fg.0),
        format_time_line("Time ended", fg.1),
        format_rms_message(fg_db),
        format_time_line("Time started", bg.0),
        format_time_line("Time ended", bg.1),
        format_rms_message(bg_db),
        format_difference_for_export(diff),
        if diff > 20.0 { "Pass" } else { "Fail" },
    )
}

fn utc_date_parts() -> (u32, u32, i32, u32, u32, u32) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let day_seconds = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    (
        day,
        month,
        year,
        (day_seconds / 3_600) as u32,
        ((day_seconds % 3_600) / 60) as u32,
        (day_seconds % 60) as u32,
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 {
        z / 146_097
    } else {
        (z - 146_096) / 146_097
    };
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

fn spectrum_y_range(y_min: f32, y_max: f32) -> (f32, f32) {
    let minimum = y_min.max(-90.0);
    let maximum = if y_max <= -90.0 {
        -80.0
    } else {
        y_max + 0.5
    };
    nonzero_range(minimum, maximum)
}

fn nonzero_range(minimum: f32, maximum: f32) -> (f32, f32) {
    if !minimum.is_finite() || !maximum.is_finite() {
        return (0.0, 1.0);
    }
    if maximum > minimum {
        (minimum, maximum)
    } else {
        let padding = minimum.abs().max(1.0) * 0.05;
        (minimum - padding, maximum + padding)
    }
}

fn window_size_for(effect: &FixPlotSpectrum) -> usize {
    let index = (effect.size.round() as usize).min(SIZES.len() - 1);
    SIZES[index].parse::<usize>().unwrap_or(8192)
}

fn plot_x_range(data: &SpectrumData, axis: f64) -> (f32, f32, bool) {
    if data.algorithm == 0 {
        let first = data
            .freq_or_lag
            .iter()
            .copied()
            .find(|x| x.is_finite() && *x > 0.0)
            .unwrap_or_else(|| (data.rate / data.window_size.max(1) as f64) as f32);
        let last = (data.rate * 0.5) as f32;
        (first.max(f32::MIN_POSITIVE), last.max(first + f32::EPSILON), axis.round() as usize == 1)
    } else {
        let first = data
            .freq_or_lag
            .iter()
            .copied()
            .find(|x| x.is_finite())
            .unwrap_or(0.0);
        let last = data
            .freq_or_lag
            .iter()
            .copied()
            .rev()
            .find(|x| x.is_finite())
            .unwrap_or(1.0);
        let (lo, hi) = nonzero_range(first, last);
        (lo, hi, false)
    }
}

fn default_x_for_bin(data: &SpectrumData, index: usize) -> f32 {
    if data.algorithm == 0 {
        (index as f64 * data.rate / data.window_size.max(1) as f64) as f32
    } else {
        index as f32 / data.rate.max(1.0) as f32
    }
}

fn make_x_ticks(minimum: f32, maximum: f32, logarithmic: bool) -> Vec<f32> {
    if logarithmic && minimum > 0.0 {
        let mut ticks = Vec::new();
        let start_power = minimum.log10().floor() as i32;
        let end_power = maximum.log10().ceil() as i32;
        for power in start_power..=end_power {
            let decade = 10.0_f32.powi(power);
            for multiplier in [1.0_f32, 2.0, 5.0] {
                let tick = decade * multiplier;
                if tick >= minimum && tick <= maximum {
                    ticks.push(tick);
                }
            }
        }
        ticks
    } else {
        (0..=5)
            .map(|i| minimum + (maximum - minimum) * i as f32 / 5.0)
            .collect()
    }
}

fn format_tick(value: f32) -> String {
    if value.abs() >= 10_000.0 || (value != 0.0 && value.abs() < 0.01) {
        format!("{value:.0e}")
    } else if value.abs() >= 100.0 {
        format!("{value:.0}")
    } else if value.abs() >= 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.2}")
    }
}

fn unmap_x(x: f32, graph: Rect, minimum: f32, maximum: f32, logarithmic: bool) -> f32 {
    let t = ((x - graph.left()) / graph.width()).clamp(0.0, 1.0);
    if logarithmic && minimum > 0.0 {
        (minimum.ln() + t * (maximum.ln() - minimum.ln())).exp()
    } else {
        minimum + t * (maximum - minimum)
    }
}

fn interpolate(data: &SpectrumData, x: f32) -> Option<f32> {
    if data.values.is_empty() {
        return None;
    }
    let coordinate = |i: usize| {
        data.freq_or_lag
            .get(i)
            .copied()
            .unwrap_or_else(|| default_x_for_bin(data, i))
    };

    let mut previous = (coordinate(0), data.values[0]);
    if x <= previous.0 {
        return Some(previous.1);
    }
    for i in 1..data.values.len() {
        let current = (coordinate(i), data.values[i]);
        if x <= current.0 {
            let width = current.0 - previous.0;
            if width == 0.0 {
                return Some(current.1);
            }
            let t = (x - previous.0) / width;
            return Some(previous.1 + t * (current.1 - previous.1));
        }
        previous = current;
    }
    Some(previous.1)
}

fn pitch_name(frequency: f32) -> String {
    if !frequency.is_finite() || frequency <= 0.0 {
        return "?".to_owned();
    }
    let midi = (69.0 + 12.0 * (frequency / 440.0).log2()).round() as i32;
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let pitch_class = midi.rem_euclid(12) as usize;
    let octave = midi.div_euclid(12) - 1;
    format!("{}{octave}", names[pitch_class])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_names_match_midi_pitch_names() {
        assert_eq!(pitch_name(440.0), "A4");
        assert_eq!(pitch_name(261.62555), "C4");
        assert_eq!(pitch_name(0.0), "?");
    }

    #[test]
    fn difference_formatting_handles_equal_and_infinite_levels() {
        assert_eq!(format_difference(0.0), "0.00 dB RMS");
        assert_eq!(format_difference(f32::INFINITY), "Infinite dB difference");
        assert_eq!(format_difference(f32::NAN), "indeterminate");
        assert_eq!(format_rms_message(Some(f32::NEG_INFINITY)), "RMS = zero.");
    }

    #[test]
    fn spectrum_y_range_follows_audacity_cutoff_rule() {
        assert_eq!(spectrum_y_range(-100.0, -100.0), (-90.0, -80.0));
        assert_eq!(spectrum_y_range(-120.0, -30.0), (-90.0, -29.5));
    }
}
