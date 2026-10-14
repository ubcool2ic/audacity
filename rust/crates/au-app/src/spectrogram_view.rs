#![cfg(feature = "gui")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use au_core::timeline::ZoomInfo;
use au_core::{AudioBuffer, Track};
use eframe::egui;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

/// The visual mode used to draw a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TrackView {
    /// Draw the waveform.
    #[default]
    Waveform,
    /// Draw the spectrogram.
    Spectrogram,
    /// Draw the waveform above a spectrogram.
    Multiview,
}

/// The frequency scale used by the spectrogram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreqScale {
    /// A linear frequency scale.
    Linear,
    /// A logarithmic frequency scale.
    Logarithmic,
    /// A mel frequency scale.
    Mel,
}

/// The display settings for a spectrogram.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewSettings {
    /// FFT window size in samples.
    pub window_size: usize,
    /// Frequency-axis scale.
    pub scale: FreqScale,
    /// Minimum displayed frequency in hertz.
    pub min_freq: f32,
    /// Maximum displayed frequency in hertz, clamped to Nyquist.
    pub max_freq: f32,
    /// Spectrogram gain in decibels.
    pub gain_db: f32,
    /// Display range in decibels.
    pub range_db: f32,
}

impl Default for ViewSettings {
    fn default() -> Self {
        Self {
            window_size: 2048,
            scale: FreqScale::Mel,
            min_freq: 0.0,
            max_freq: 20_000.0,
            gain_db: 20.0,
            range_db: 80.0,
        }
    }
}

/// A time and frequency selection made with the spectral selection tool.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpectralSelection {
    /// Index of the selected track.
    pub track: usize,
    /// Start time in seconds.
    pub t0: f64,
    /// End time in seconds.
    pub t1: f64,
    /// Lower frequency in hertz.
    pub f0: f64,
    /// Upper frequency in hertz.
    pub f1: f64,
}

/// A replaceable function that computes decibel-valued spectrogram columns.
pub type ColumnFn = fn(
    &[f32],
    u32,
    &ViewSettings,
    f64,
    f64,
    usize,
    &au_runtime::pool::CancelToken,
) -> Option<Vec<Vec<f32>>>;

/// Convert a frequency to a vertical unit coordinate, where zero is the bottom
/// and one is the top of the displayed frequency range.
pub fn freq_to_unit(f: f32, settings: &ViewSettings, nyquist: f32) -> f32 {
    let (low, high) = frequency_bounds(settings, nyquist);
    if high <= low {
        return 0.5;
    }

    let frequency = f.clamp(low, high);
    let unit = match settings.scale {
        FreqScale::Linear => (frequency - low) / (high - low),
        FreqScale::Logarithmic => {
            let log_low = low.max(10.0).log10();
            let log_high = high.max(low.max(10.0)).log10();
            if log_high <= log_low {
                0.5
            } else {
                (frequency.max(low.max(10.0)).log10() - log_low) / (log_high - log_low)
            }
        }
        FreqScale::Mel => {
            let mel_low = hz_to_mel(low);
            let mel_high = hz_to_mel(high);
            if mel_high <= mel_low {
                0.5
            } else {
                (hz_to_mel(frequency) - mel_low) / (mel_high - mel_low)
            }
        }
    };
    unit.clamp(0.0, 1.0)
}

/// Convert a vertical unit coordinate to a frequency in hertz.
pub fn unit_to_freq(unit: f32, settings: &ViewSettings, nyquist: f32) -> f32 {
    let (low, high) = frequency_bounds(settings, nyquist);
    if high <= low {
        return low;
    }

    let unit = unit.clamp(0.0, 1.0);
    match settings.scale {
        FreqScale::Linear => low + unit * (high - low),
        FreqScale::Logarithmic => {
            let log_low = low.max(10.0).log10();
            let log_high = high.max(low.max(10.0)).log10();
            if log_high <= log_low {
                low
            } else {
                10.0_f32.powf(log_low + unit * (log_high - log_low))
            }
        }
        FreqScale::Mel => {
            let mel_low = hz_to_mel(low);
            let mel_high = hz_to_mel(high);
            mel_to_hz(mel_low + unit * (mel_high - mel_low))
        }
    }
}

/// Map a normalized intensity to an Audacity-like black-blue-purple-red-orange-yellow-white
/// color gradient.
pub fn colormap(value01: f32) -> egui::Color32 {
    const STOPS: [(f32, [u8; 3]); 7] = [
        (0.0, [0, 0, 0]),
        (0.16, [0, 0, 105]),
        (0.34, [110, 0, 160]),
        (0.52, [220, 0, 45]),
        (0.70, [255, 105, 0]),
        (0.86, [255, 235, 0]),
        (1.0, [255, 255, 255]),
    ];

    let value = if value01.is_finite() {
        value01.clamp(0.0, 1.0)
    } else {
        0.0
    };

    for pair in STOPS.windows(2) {
        let (left_pos, left_rgb) = pair[0];
        let (right_pos, right_rgb) = pair[1];
        if value <= right_pos {
            let amount = ((value - left_pos) / (right_pos - left_pos)).clamp(0.0, 1.0);
            let channel = |index: usize| {
                (left_rgb[index] as f32
                    + amount * (right_rgb[index] as f32 - left_rgb[index] as f32))
                    .round() as u8
            };
            return egui::Color32::from_rgb(channel(0), channel(1), channel(2));
        }
    }

    egui::Color32::WHITE
}

/// Compute interim spectrogram columns using a Hann window and a forward FFT.
///
/// This is intentionally an interim implementation. The Audacity-compatible
/// window, zero-padding, frequency-scale, and gain calculations are being
/// ported separately and can replace this function through [`ColumnFn`].
pub fn interim_columns(
    mono: &[f32],
    sample_rate: u32,
    settings: &ViewSettings,
    t0: f64,
    t1: f64,
    columns: usize,
    cancel: &au_runtime::pool::CancelToken,
) -> Option<Vec<Vec<f32>>> {
    if columns == 0 || sample_rate == 0 {
        return Some(Vec::new());
    }

    let window_size = settings.window_size.max(2);
    let mut window = Vec::with_capacity(window_size);
    for index in 0..window_size {
        let phase = 2.0 * std::f32::consts::PI * index as f32 / (window_size - 1) as f32;
        window.push(0.5 - 0.5 * phase.cos());
    }
    let window_sum: f32 = window.iter().sum();
    let normalization = if window_sum > 0.0 {
        4.0 / (window_sum * window_sum)
    } else {
        0.0
    };

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(window_size);
    let mut output = Vec::with_capacity(columns);
    let mut input = vec![Complex::new(0.0, 0.0); window_size];

    for column in 0..columns {
        if cancel.check().is_err() {
            return None;
        }

        let fraction = if columns == 1 {
            0.5
        } else {
            column as f64 / (columns - 1) as f64
        };
        let time = t0 + fraction * (t1 - t0);
        let center = (time * sample_rate as f64).round() as isize;
        let first = center - (window_size as isize / 2);

        for (index, value) in input.iter_mut().enumerate() {
            let sample_index = first + index as isize;
            let sample = if sample_index >= 0 {
                mono.get(sample_index as usize).copied().unwrap_or(0.0)
            } else {
                0.0
            };
            *value = Complex::new(sample * window[index], 0.0);
        }

        fft.process(&mut input);
        let mut bins = Vec::with_capacity(window_size / 2);
        for value in input.iter().take(window_size / 2) {
            let power = value.norm_sqr() * normalization;
            let db = if power > 0.0 {
                10.0 * power.log10()
            } else {
                -200.0
            };
            bins.push(db.max(-200.0));
        }
        output.push(bins);
    }

    Some(output)
}

/// Cached spectrogram textures and background column-computation jobs.
pub struct SpectrogramCache {
    rt: au_runtime::Runtime,
    entries: Vec<CacheEntry>,
    pending: Vec<Pending>,
    sources: Vec<SourceBuffer>,
    /// The function used to compute spectrogram columns.
    pub column_fn: ColumnFn,
}

impl SpectrogramCache {
    /// Create a spectrogram cache with a background runtime.
    pub fn new() -> Self {
        let threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(2)
            .clamp(1, 4);
        Self {
            rt: au_runtime::Runtime::new(threads, 16),
            entries: Vec::new(),
            pending: Vec::new(),
            sources: Vec::new(),
            column_fn: interim_columns,
        }
    }

    /// Cancel pending jobs and invalidate all cached spectrogram data.
    pub fn invalidate_all(&mut self) {
        for pending in &self.pending {
            pending.handle.cancel.cancel();
        }
        self.pending.clear();
        self.entries.clear();
        self.sources.clear();
    }

    fn poll_completed(&mut self, ctx: &egui::Context) {
        let mut index = 0;
        while index < self.pending.len() {
            let result = self.pending[index].handle.try_result();
            if let Some(result) = result {
                let pending = self.pending.remove(index);
                if let Ok(Some(image)) = result {
                    let texture = ctx.load_texture(
                        format!("spectrogram-{}-{}", pending.key.track_index, pending.key.fingerprint),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.entries.push(CacheEntry {
                        key: pending.key,
                        texture,
                    });
                    if self.entries.len() > MAX_CACHE_ENTRIES {
                        self.entries.remove(0);
                    }
                }
            } else {
                index += 1;
            }
        }
    }

    fn source_for(&mut self, index: usize, track: &Track) -> (Arc<AudioBuffer>, u64) {
        let now = Instant::now();
        let track_len = track.len();
        let sample_rate = track.sample_rate();
        let existing = self.sources.iter().position(|source| {
            source.track_index == index
                && source.track_len == track_len
                && source.sample_rate == sample_rate
        });
        let needs_refresh = existing
            .map(|position| now.duration_since(self.sources[position].checked_at) >= SOURCE_REFRESH)
            .unwrap_or(true);

        if needs_refresh {
            let buffer = Arc::new(track.render());
            let fingerprint = buffer_fingerprint(&buffer);
            let source = SourceBuffer {
                track_index: index,
                track_len,
                sample_rate,
                checked_at: now,
                buffer,
                fingerprint,
            };
            if let Some(position) = existing {
                self.sources[position] = source;
            } else {
                self.sources.push(source);
            }
        }

        let position = self
            .sources
            .iter()
            .position(|source| {
                source.track_index == index
                    && source.track_len == track_len
                    && source.sample_rate == sample_rate
            })
            .expect("source buffer was inserted or retained");
        (
            Arc::clone(&self.sources[position].buffer),
            self.sources[position].fingerprint,
        )
    }

    fn take_texture(&mut self, key: &CacheKey) -> Option<egui::TextureHandle> {
        let position = self.entries.iter().position(|entry| entry.key == *key)?;
        let entry = self.entries.remove(position);
        let texture = entry.texture.clone();
        self.entries.push(entry);
        Some(texture)
    }
}

impl Default for SpectrogramCache {
    fn default() -> Self {
        Self::new()
    }
}

const MAX_CACHE_ENTRIES: usize = 16;
const SOURCE_REFRESH: Duration = Duration::from_secs(1);

struct CacheEntry {
    key: CacheKey,
    texture: egui::TextureHandle,
}

struct Pending {
    key: CacheKey,
    handle: au_runtime::JobHandle<Option<egui::ColorImage>>,
}

struct SourceBuffer {
    track_index: usize,
    track_len: usize,
    sample_rate: u32,
    checked_at: Instant,
    buffer: Arc<AudioBuffer>,
    fingerprint: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CacheKey {
    track_index: usize,
    track_len: usize,
    sample_rate: u32,
    fingerprint: u64,
    t0_pixel: i64,
    t1_pixel: i64,
    pixels_per_second: u64,
    columns: usize,
    pixel_height: usize,
    window_size: usize,
    scale: FreqScale,
    min_freq: u32,
    max_freq: u32,
    gain_db: u32,
    range_db: u32,
}

impl CacheKey {
    fn new(
        track_index: usize,
        track_len: usize,
        sample_rate: u32,
        fingerprint: u64,
        t0_pixel: i64,
        t1_pixel: i64,
        pixels_per_second: f64,
        columns: usize,
        pixel_height: usize,
        settings: &ViewSettings,
    ) -> Self {
        Self {
            track_index,
            track_len,
            sample_rate,
            fingerprint,
            t0_pixel,
            t1_pixel,
            pixels_per_second: pixels_per_second.to_bits(),
            columns,
            pixel_height,
            window_size: settings.window_size,
            scale: settings.scale,
            min_freq: settings.min_freq.to_bits(),
            max_freq: settings.max_freq.to_bits(),
            gain_db: settings.gain_db.to_bits(),
            range_db: settings.range_db.to_bits(),
        }
    }
}

/// Draw one track in spectrogram view and update the optional spectral selection.
///
/// Returns `true` when the spectral selection changes.
pub fn draw_spectrogram_track(
    ui: &mut egui::Ui,
    cache: &mut SpectrogramCache,
    index: usize,
    track: &Track,
    zoom: &ZoomInfo,
    settings: &ViewSettings,
    height: f32,
    selection: &mut Option<SpectralSelection>,
) -> bool {
    cache.poll_completed(ui.ctx());

    ui.label(format!("{} · Spectrogram", track.name));
    let available_width = ui.available_width().max(1.0);
    let (track_rect, _) = ui.allocate_exact_size(
        egui::vec2(available_width, height.max(1.0)),
        egui::Sense::hover(),
    );

    let ruler_width = 48.0_f32.min(track_rect.width() * 0.3);
    let image_width = (track_rect.width() - ruler_width).max(1.0);
    let image_area = egui::Rect::from_min_size(
        track_rect.min,
        egui::vec2(image_width, track_rect.height()),
    );
    let ruler_area = egui::Rect::from_min_max(
        egui::pos2(image_area.right(), track_rect.top()),
        track_rect.max,
    );
    let pixels_per_point = ui.ctx().pixels_per_point().max(0.1);
    let pixel_height = (image_area.height() * pixels_per_point).round().max(1.0) as usize;
    let sample_rate = track.sample_rate().max(1);
    let nyquist = sample_rate as f32 * 0.5;
    let duration = track.len() as f64 / sample_rate as f64;

    ui.painter()
        .rect_filled(track_rect, 0.0, egui::Color32::from_rgb(24, 24, 27));
    ui.painter()
        .rect_filled(ruler_area, 0.0, egui::Color32::from_rgb(30, 30, 33));

    let visible_start = zoom.x_to_time(0.0);
    let visible_end = zoom.x_to_time(image_width as f64);
    let raw_start = visible_start.min(visible_end);
    let raw_end = visible_start.max(visible_end);
    let start_time = raw_start.clamp(0.0, duration);
    let end_time = raw_end.clamp(0.0, duration);
    let pixels_per_second = zoom.pixels_per_sec.abs().max(0.001);
    let visible_columns = ((image_width * pixels_per_point).round().max(1.0)) as usize;
    let mut active_key = None;
    let mut draw_rect = None;

    if end_time > start_time {
        let start_x = zoom.time_to_x(start_time).clamp(0.0, image_width as f64) as f32;
        let end_x = zoom.time_to_x(end_time).clamp(0.0, image_width as f64) as f32;
        let left = start_x.min(end_x);
        let right = start_x.max(end_x);
        if right > left {
            let columns = ((right - left) * pixels_per_point).round().max(1.0) as usize;
            let t0_pixel = (start_time * pixels_per_second).round() as i64;
            let t1_pixel = (end_time * pixels_per_second).round() as i64;
            let (buffer, fingerprint) = cache.source_for(index, track);
            let key = CacheKey::new(
                index,
                track.len(),
                sample_rate,
                fingerprint,
                t0_pixel,
                t1_pixel,
                pixels_per_second,
                columns,
                pixel_height,
                settings,
            );
            active_key = Some(key);
            draw_rect = Some(egui::Rect::from_min_max(
                egui::pos2(image_area.left() + left, image_area.top()),
                egui::pos2(image_area.left() + right, image_area.bottom()),
            ));

            for pending in &cache.pending {
                if pending.key.track_index == index && pending.key != key {
                    pending.handle.cancel.cancel();
                }
            }

            let already_pending = cache.pending.iter().any(|pending| pending.key == key);
            let already_cached = cache.entries.iter().any(|entry| entry.key == key);
            if !already_pending && !already_cached {
                let column_fn = cache.column_fn;
                let settings_copy = *settings;
                let job_buffer = Arc::clone(&buffer);
                let job_key = key;
                let job_t0 = t0_pixel as f64 / pixels_per_second;
                let job_t1 = t1_pixel as f64 / pixels_per_second;
                let mono = mono_mixdown(&job_buffer);
                let job_sample_rate = sample_rate;
                let job = cache.rt.submit(64, move |ctx| {
                    ctx.cancel.check()?;
                    if buffer_fingerprint(&job_buffer) != job_key.fingerprint {
                        return Ok(None);
                    }
                    let columns_data = match column_fn(
                        &mono,
                        job_sample_rate,
                        &settings_copy,
                        job_t0,
                        job_t1,
                        job_key.columns,
                        &ctx.cancel,
                    ) {
                        Some(columns_data) => columns_data,
                        None => return Ok(None),
                    };
                    ctx.cancel.check()?;
                    Ok(Some(columns_to_image(
                        &columns_data,
                        job_sample_rate,
                        &settings_copy,
                        job_key.columns,
                        job_key.pixel_height,
                    )))
                });
                cache.pending.push(Pending { key, handle: job });
            }
        }
    }

    let current_texture = active_key.and_then(|key| cache.take_texture(&key));
    let previous_texture = current_texture.clone().or_else(|| {
        cache
            .entries
            .iter()
            .rev()
            .find(|entry| entry.key.track_index == index)
            .map(|entry| entry.texture.clone())
    });

    if let (Some(texture), Some(rect)) = (previous_texture, draw_rect) {
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    } else {
        ui.painter()
            .rect_filled(image_area, 0.0, egui::Color32::from_rgb(15, 15, 18));
    }

    draw_frequency_ruler(ui, ruler_area, settings, nyquist);

    let pending_current = active_key
        .map(|key| cache.pending.iter().any(|pending| pending.key == key))
        .unwrap_or(false);
    if current_texture.is_none() {
        if draw_rect.is_none() {
            ui.painter().text(
                image_area.center(),
                egui::Align2::CENTER_CENTER,
                "No audio in visible range",
                egui::FontId::proportional(12.0),
                egui::Color32::GRAY,
            );
        } else if previous_texture_is_unavailable(cache, index, active_key) {
            ui.painter().text(
                image_area.center(),
                egui::Align2::CENTER_CENTER,
                "computing spectrogram…",
                egui::FontId::proportional(12.0),
                egui::Color32::LIGHT_GRAY,
            );
        }
        if pending_current {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }

    let image_response = ui.interact(
        image_area,
        ui.id().with(("spectrogram-image", index)),
        egui::Sense::click_and_drag(),
    );
    let changed = update_selection(
        ui,
        &image_response,
        track_rect,
        image_area,
        zoom,
        duration,
        nyquist,
        settings,
        index,
        selection,
    );

    if let Some(selected) = selection.filter(|selected| selected.track == index) {
        paint_selection(
            ui,
            selected,
            track_rect,
            image_area,
            zoom,
            settings,
            nyquist,
        );
    }

    let _ = visible_columns;
    changed
}

fn previous_texture_is_unavailable(
    cache: &SpectrogramCache,
    track_index: usize,
    active_key: Option<CacheKey>,
) -> bool {
    !cache.entries.iter().any(|entry| {
        entry.key.track_index == track_index && Some(entry.key) != active_key
    })
}

fn update_selection(
    ui: &egui::Ui,
    response: &egui::Response,
    track_rect: egui::Rect,
    image_area: egui::Rect,
    zoom: &ZoomInfo,
    duration: f64,
    nyquist: f32,
    settings: &ViewSettings,
    track_index: usize,
    selection: &mut Option<SpectralSelection>,
) -> bool {
    let original = *selection;
    let frequency_at = |position: egui::Pos2| {
        let unit = ((image_area.bottom() - position.y) / image_area.height().max(1.0))
            .clamp(0.0, 1.0);
        let frequency = unit_to_freq(unit, settings, nyquist);
        let minimum = settings.min_freq.max(0.0).min(nyquist);
        let maximum = settings.max_freq.max(minimum).min(nyquist);
        (frequency as f64).clamp(minimum as f64, maximum as f64)
    };
    let time_at = |position: egui::Pos2| {
        zoom.x_to_time((position.x - track_rect.left()).clamp(0.0, image_area.width()) as f64)
            .clamp(0.0, duration)
    };

    if response.dragged_by(egui::PointerButton::Primary) {
        if let Some(current) = response.interact_pointer_pos() {
            let anchor = ui
                .input(|input| input.pointer.press_origin())
                .unwrap_or(current);
            let start_time = time_at(anchor);
            let end_time = time_at(current);
            let start_frequency = frequency_at(anchor);
            let end_frequency = frequency_at(current);
            *selection = Some(SpectralSelection {
                track: track_index,
                t0: start_time.min(end_time),
                t1: start_time.max(end_time),
                f0: start_frequency.min(end_frequency),
                f1: start_frequency.max(end_frequency),
            });
        }
    } else if response.clicked_by(egui::PointerButton::Primary) {
        if let Some(position) = response.interact_pointer_pos() {
            let time = time_at(position);
            let frequency = frequency_at(position);
            *selection = Some(SpectralSelection {
                track: track_index,
                t0: time,
                t1: time,
                f0: frequency,
                f1: frequency,
            });
        }
    }

    *selection != original
}

fn paint_selection(
    ui: &egui::Ui,
    selection: SpectralSelection,
    track_rect: egui::Rect,
    image_area: egui::Rect,
    zoom: &ZoomInfo,
    settings: &ViewSettings,
    nyquist: f32,
) {
    let x0 = track_rect.left() + zoom.time_to_x(selection.t0) as f32;
    let x1 = track_rect.left() + zoom.time_to_x(selection.t1) as f32;
    let u0 = freq_to_unit(selection.f0 as f32, settings, nyquist);
    let u1 = freq_to_unit(selection.f1 as f32, settings, nyquist);
    let y0 = image_area.bottom() - u0 * image_area.height();
    let y1 = image_area.bottom() - u1 * image_area.height();
    let selection_rect = egui::Rect::from_min_max(
        egui::pos2(x0.min(x1), y0.min(y1)),
        egui::pos2(x0.max(x1), y0.max(y1)),
    )
    .intersect(image_area);

    if selection_rect.is_positive() {
        ui.painter().rect_filled(
            selection_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 35),
        );
        let outline = egui::Color32::from_rgba_unmultiplied(255, 255, 255, 220);
        ui.painter().line_segment(
            [selection_rect.left_top(), selection_rect.right_top()],
            egui::Stroke::new(1.0, outline),
        );
        ui.painter().line_segment(
            [selection_rect.right_top(), selection_rect.right_bottom()],
            egui::Stroke::new(1.0, outline),
        );
        ui.painter().line_segment(
            [selection_rect.right_bottom(), selection_rect.left_bottom()],
            egui::Stroke::new(1.0, outline),
        );
        ui.painter().line_segment(
            [selection_rect.left_bottom(), selection_rect.left_top()],
            egui::Stroke::new(1.0, outline),
        );
    }
}

fn draw_frequency_ruler(
    ui: &egui::Ui,
    ruler_area: egui::Rect,
    settings: &ViewSettings,
    nyquist: f32,
) {
    let (low, high) = frequency_bounds(settings, nyquist);
    let mut ticks = Vec::new();
    match settings.scale {
        FreqScale::Linear => {
            let span = (high - low).max(1.0);
            let raw_step = span / 6.0;
            let magnitude = 10.0_f32.powf(raw_step.log10().floor());
            let step = [1.0, 2.0, 5.0, 10.0]
                .into_iter()
                .map(|factor| factor * magnitude)
                .find(|candidate| *candidate >= raw_step)
                .unwrap_or(10.0 * magnitude);
            let mut frequency = (low / step).ceil() * step;
            while frequency <= high && ticks.len() < 64 {
                ticks.push(frequency);
                frequency += step;
            }
        }
        FreqScale::Logarithmic | FreqScale::Mel => {
            ticks.extend(
                [100.0, 200.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0]
                    .into_iter()
                    .filter(|frequency| *frequency >= low && *frequency <= high),
            );
        }
    }

    let painter = ui.painter();
    painter.line_segment(
        [ruler_area.left_top(), ruler_area.left_bottom()],
        egui::Stroke::new(1.0, egui::Color32::DARK_GRAY),
    );
    for frequency in ticks {
        let unit = freq_to_unit(frequency, settings, nyquist);
        let y = ruler_area.bottom() - unit * ruler_area.height();
        painter.line_segment(
            [
                egui::pos2(ruler_area.left(), y),
                egui::pos2(ruler_area.left() + 4.0, y),
            ],
            egui::Stroke::new(1.0, egui::Color32::LIGHT_GRAY),
        );
        painter.text(
            egui::pos2(ruler_area.left() + 6.0, y),
            egui::Align2::LEFT_CENTER,
            format_frequency(frequency),
            egui::FontId::proportional(10.0),
            egui::Color32::LIGHT_GRAY,
        );
    }
}

fn format_frequency(frequency: f32) -> String {
    if frequency >= 1_000.0 && (frequency % 1_000.0).abs() < 0.5 {
        format!("{:.0}k", frequency / 1_000.0)
    } else if frequency >= 1_000.0 {
        let kilo = frequency / 1_000.0;
        if (kilo - kilo.round()).abs() < 0.05 {
            format!("{:.0}k", kilo)
        } else {
            format!("{kilo:.1}k")
        }
    } else {
        format!("{frequency:.0}")
    }
}

fn columns_to_image(
    columns: &[Vec<f32>],
    sample_rate: u32,
    settings: &ViewSettings,
    width: usize,
    height: usize,
) -> egui::ColorImage {
    let width = width.max(1);
    let height = height.max(1);
    let nyquist = sample_rate as f32 * 0.5;
    let mut pixels = vec![egui::Color32::BLACK; width * height];

    for x in 0..width {
        let column = columns.get(x);
        for y in 0..height {
            let unit = if height == 1 {
                0.5
            } else {
                1.0 - y as f32 / (height - 1) as f32
            };
            let frequency = unit_to_freq(unit, settings, nyquist);
            let bin = (frequency * settings.window_size.max(2) as f32
                / sample_rate.max(1) as f32)
                .round() as usize;
            let db = column
                .and_then(|values| values.get(bin))
                .copied()
                .unwrap_or(-200.0);
            let range = settings.range_db.max(1.0);
            let value = ((db + settings.gain_db + range) / range).clamp(0.0, 1.0);
            pixels[y * width + x] = colormap(value);
        }
    }

    egui::ColorImage { size: [width, height], pixels }
}

fn mono_mixdown(buffer: &AudioBuffer) -> Vec<f32> {
    let frame_count = buffer.channels.iter().map(Vec::len).max().unwrap_or(0);
    if buffer.channels.is_empty() {
        return vec![0.0; frame_count];
    }

    (0..frame_count)
        .map(|frame| {
            buffer
                .channels
                .iter()
                .map(|channel| channel.get(frame).copied().unwrap_or(0.0))
                .sum::<f32>()
                / buffer.channels.len() as f32
        })
        .collect()
}

fn buffer_fingerprint(buffer: &AudioBuffer) -> u64 {
    let Some(channel) = buffer.channels.first() else {
        return 0.0_f64.to_bits();
    };
    if channel.is_empty() {
        return 0.0_f64.to_bits();
    }

    let mut sum = 0.0_f64;
    for sample in 0..64 {
        let position = if channel.len() == 1 {
            0
        } else {
            sample * (channel.len() - 1) / 63
        };
        sum += channel[position] as f64;
    }
    sum.to_bits()
}

fn frequency_bounds(settings: &ViewSettings, nyquist: f32) -> (f32, f32) {
    let nyquist = if nyquist.is_finite() {
        nyquist.max(0.0)
    } else {
        0.0
    };
    let requested_max = if settings.max_freq.is_finite() {
        settings.max_freq.max(0.0)
    } else {
        nyquist
    };
    let high = requested_max.min(nyquist);
    let requested_min = if settings.min_freq.is_finite() {
        settings.min_freq.max(0.0)
    } else {
        0.0
    };
    let mut low = requested_min.min(high);
    if settings.scale == FreqScale::Logarithmic {
        low = low.max(10.0).min(high);
    }
    (low, high)
}

fn hz_to_mel(frequency: f32) -> f32 {
    2595.0 * (1.0 + frequency.max(0.0) / 700.0).log10()
}

fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10.0_f32.powf(mel / 2595.0) - 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_scale_round_trips() {
        for scale in [FreqScale::Linear, FreqScale::Logarithmic, FreqScale::Mel] {
            let settings = ViewSettings {
                scale,
                min_freq: if scale == FreqScale::Logarithmic {
                    30.0
                } else {
                    0.0
                },
                max_freq: 20_000.0,
                ..ViewSettings::default()
            };
            for frequency in [100.0_f32, 500.0, 1_000.0, 5_000.0, 18_000.0] {
                let unit = freq_to_unit(frequency, &settings, 22_050.0);
                let round_trip = unit_to_freq(unit, &settings, 22_050.0);
                let tolerance = (frequency * 0.0001).max(0.02);
                assert!(
                    (frequency - round_trip).abs() <= tolerance,
                    "scale {scale:?}: {frequency} -> {round_trip}"
                );
            }
        }
    }

    #[test]
    fn interim_columns_peak_near_one_kilohertz_bin() {
        let sample_rate = 44_100_u32;
        let sample_count = sample_rate as usize;
        let mono: Vec<f32> = (0..sample_count)
            .map(|sample| {
                (2.0 * std::f32::consts::PI * 1_000.0 * sample as f32
                    / sample_rate as f32)
                    .sin()
            })
            .collect();
        let settings = ViewSettings::default();
        let runtime = au_runtime::Runtime::new(1, 1);
        let handle = runtime.submit(0, move |ctx| {
            Ok(interim_columns(
                &mono, sample_rate, &settings, 0.5, 0.5, 1, &ctx.cancel,
            ))
        });
        let result = loop {
            if let Some(result) = handle.try_result() {
                break result;
            }
            std::thread::yield_now();
        };
        let columns = result
            .expect("interim spectrogram job failed")
            .expect("interim spectrogram job was cancelled");
        let bins = &columns[0];
        let peak_bin = bins
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .map(|(index, _)| index)
            .expect("expected FFT bins");
        let expected_bin = 1_000.0 / (sample_rate as f32 / 2048.0);
        assert!((peak_bin as f32 - expected_bin).abs() <= 2.0);
    }

    #[test]
    fn colormap_has_dark_and_bright_endpoints() {
        let dark = colormap(0.0);
        let bright = colormap(1.0);
        assert!((dark.r() as u16 + dark.g() as u16 + dark.b() as u16) < 16);
        assert!(bright.r() > 240 && bright.g() > 240 && bright.b() > 240);
    }
}

