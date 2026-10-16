//! Audacity theme parsing, persistence, image-cache support, and optional egui integration.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// The identity of an Audacity theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeId {
    /// Audacity's light theme.
    Light,
    /// Audacity's dark theme.
    Dark,
    /// Audacity's high-contrast theme.
    HighContrast,
    /// Audacity's classic theme.
    Classic,
    /// A user-provided theme.
    Custom,
}

impl ThemeId {
    /// The four built-in themes, in menu order.
    pub const BUILTIN: [ThemeId; 4] = [
        ThemeId::Light,
        ThemeId::Dark,
        ThemeId::HighContrast,
        ThemeId::Classic,
    ];

    /// Returns Audacity's stable key for this theme.
    pub fn key(self) -> &'static str {
        match self {
            ThemeId::Light => "light",
            ThemeId::Dark => "dark",
            ThemeId::HighContrast => "high-contrast",
            ThemeId::Classic => "classic",
            ThemeId::Custom => "custom",
        }
    }

    /// Returns a human-readable label for this theme.
    pub fn label(self) -> &'static str {
        match self {
            ThemeId::Light => "Light",
            ThemeId::Dark => "Dark",
            ThemeId::HighContrast => "High Contrast",
            ThemeId::Classic => "Classic",
            ThemeId::Custom => "Custom",
        }
    }

    /// Parses an Audacity theme key.
    pub fn from_key(key: &str) -> Option<Self> {
        match key {
            "light" => Some(ThemeId::Light),
            "dark" => Some(ThemeId::Dark),
            "high-contrast" => Some(ThemeId::HighContrast),
            "classic" => Some(ThemeId::Classic),
            "custom" => Some(ThemeId::Custom),
            _ => None,
        }
    }
}

/// An 8-bit red, green, blue, alpha colour.
pub type Rgba = [u8; 4];

/// A parsed Audacity theme and its colours, in source-file order.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// The theme's identity.
    pub id: ThemeId,
    colours: Vec<(String, Rgba)>,
}

impl Theme {
    /// Parses Audacity's CSS-like colour table.
    ///
    /// Colour values may be `#RRGGBB` or `#RRGGBBAA`. Lines that do not contain
    /// a valid colour entry are ignored; an input with no colour entries fails.
    pub fn parse_colors_txt(id: ThemeId, text: &str) -> Result<Theme, String> {
        let mut theme = Theme {
            id,
            colours: Vec::new(),
        };

        for line in text.lines() {
            let Some((raw_name, raw_value)) = line.split_once(':') else {
                continue;
            };
            let name = raw_name.trim();
            if name.is_empty() {
                continue;
            }

            let value = raw_value
                .split(';')
                .next()
                .unwrap_or(raw_value)
                .trim();
            if let Some(colour) = parse_hex_colour(value) {
                theme.set_colour(name, colour);
            }
        }

        if theme.colours.is_empty() {
            return Err("No colours found in Audacity colour table".to_owned());
        }

        Ok(theme)
    }

    /// Loads a built-in theme, filling missing light-theme names from the light
    /// table. Entries not present in the light table are omitted so that all
    /// built-in themes share the same set of names.
    pub fn builtin(id: ThemeId) -> Theme {
        let light = parse_builtin_table(ThemeId::Light);
        if id == ThemeId::Light || id == ThemeId::Custom {
            let mut theme = light;
            theme.id = id;
            return theme;
        }

        let mut theme = parse_builtin_table(id);
        let light_names: Vec<String> = light.names().map(str::to_owned).collect();
        theme
            .colours
            .retain(|(name, _)| light_names.iter().any(|light_name| light_name == name));

        for (name, colour) in light.colours {
            if !theme.colours.iter().any(|(existing, _)| existing == &name) {
                theme.colours.push((name, colour));
            }
        }

        theme.id = id;
        theme
    }

    /// Returns the named colour, or opaque magenta if the name is unknown.
    pub fn colour(&self, name: &str) -> Rgba {
        self.colours
            .iter()
            .find(|(existing, _)| existing == name)
            .map(|(_, colour)| *colour)
            .unwrap_or([255, 0, 255, 255])
    }

    /// Iterates over colour names in this theme's stored order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.colours.iter().map(|(name, _)| name.as_str())
    }

    /// Inserts or replaces a colour. New names are appended to the end.
    pub fn set_colour(&mut self, name: &str, colour: Rgba) {
        if let Some((_, existing)) = self
            .colours
            .iter_mut()
            .find(|(existing, _)| existing == name)
        {
            *existing = colour;
        } else {
            self.colours.push((name.to_owned(), colour));
        }
    }

    /// Encodes this theme as an RGBA8 PNG image cache.
    ///
    /// Each colour is a 10-by-10 swatch, with 16 swatches per row. The image
    /// stores the colour names and theme key in PNG `tEXt` chunks.
    pub fn to_image_cache_png(&self) -> Result<Vec<u8>, String> {
        if self.colours.is_empty() {
            return Err("Cannot encode an empty theme".to_owned());
        }

        let rows = self
            .colours
            .len()
            .checked_add(15)
            .ok_or_else(|| "Too many theme colours".to_owned())?
            / 16;
        let width = 160_u32;
        let height = u32::try_from(rows)
            .ok()
            .and_then(|value| value.checked_mul(10))
            .ok_or_else(|| "Theme image dimensions are too large".to_owned())?;
        let pixel_count = usize::try_from(width)
            .ok()
            .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
            .ok_or_else(|| "Theme image dimensions are too large".to_owned())?;
        let byte_count = pixel_count
            .checked_mul(4)
            .ok_or_else(|| "Theme image dimensions are too large".to_owned())?;
        let mut pixels = vec![0_u8; byte_count];

        for (index, (_, colour)) in self.colours.iter().enumerate() {
            let swatch_x = (index % 16) * 10;
            let swatch_y = (index / 16) * 10;
            for y in swatch_y..swatch_y + 10 {
                for x in swatch_x..swatch_x + 10 {
                    let offset = (y * 160 + x) * 4;
                    pixels[offset..offset + 4].copy_from_slice(colour);
                }
            }
        }

        let mut output = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut output, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let names = self
                .colours
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(",");
            encoder
                .add_text_chunk("audacity-rs-colours".to_owned(), names)
                .map_err(|error| error.to_string())?;
            encoder
                .add_text_chunk("audacity-rs-theme".to_owned(), self.id.key().to_owned())
                .map_err(|error| error.to_string())?;
            let mut writer = encoder
                .write_header()
                .map_err(|error| error.to_string())?;
            writer
                .write_image_data(&pixels)
                .map_err(|error| error.to_string())?;
        }

        Ok(output)
    }

    /// Decodes a theme from an image-cache PNG, reading each swatch's centre
    /// pixel. The theme identity defaults to [`ThemeId::Custom`].
    pub fn from_image_cache_png(bytes: &[u8]) -> Result<Theme, String> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        decoder.set_transformations(
            png::Transformations::EXPAND | png::Transformations::STRIP_16,
        );
        let mut reader = decoder
            .read_info()
            .map_err(|error| error.to_string())?;
        let mut buffer = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
        let frame = reader
            .next_frame(&mut buffer)
            .map_err(|error| error.to_string())?;

        let width = usize::try_from(frame.width)
            .map_err(|_| "Invalid theme image width".to_owned())?;
        let height = usize::try_from(frame.height)
            .map_err(|_| "Invalid theme image height".to_owned())?;
        let data = &buffer[..frame.buffer_size()];
        let rgba = decode_rgba_pixels(data, frame.color_type)?;
        if rgba.len() != width.saturating_mul(height) {
            return Err("Invalid theme image pixel data".to_owned());
        }

        let (names_text, id_text) = {
            let info = reader.info();
            let names = info
                .uncompressed_latin1_text
                .iter()
                .find(|chunk| chunk.keyword == "audacity-rs-colours")
                .map(|chunk| chunk.text.clone());
            let id = info
                .uncompressed_latin1_text
                .iter()
                .find(|chunk| chunk.keyword == "audacity-rs-theme")
                .map(|chunk| chunk.text.clone());
            (names, id)
        };

        let names_text =
            names_text.ok_or_else(|| "Theme image is missing colour names".to_owned())?;
        let names: Vec<&str> = names_text.split(',').collect();
        if names.is_empty() || names.iter().any(|name| name.is_empty()) {
            return Err("Theme image contains invalid colour names".to_owned());
        }

        let mut theme = Theme {
            id: id_text
                .as_deref()
                .and_then(ThemeId::from_key)
                .unwrap_or(ThemeId::Custom),
            colours: Vec::with_capacity(names.len()),
        };

        for (index, name) in names.into_iter().enumerate() {
            let x = (index % 16) * 10 + 5;
            let y = (index / 16) * 10 + 5;
            if x >= width || y >= height {
                return Err("Theme image is too small for its colour names".to_owned());
            }
            let colour = rgba[y * width + x];
            theme.colours.push((name.to_owned(), colour));
        }

        Ok(theme)
    }

    /// Loads `<dir>/Colors.txt` or `<dir>/Components/Colors.txt` as a custom
    /// theme.
    pub fn load_components_dir(dir: &Path) -> Result<Theme, String> {
        let direct = dir.join("Colors.txt");
        let components = dir.join("Components").join("Colors.txt");
        let path = if direct.is_file() {
            direct
        } else if components.is_file() {
            components
        } else {
            return Err(au_core::i18n::tr_format("No Colors.txt found in %s or %s", &[&dir.display().to_string(), &dir.join("Components").display().to_string()]));
        };

        let text = fs::read_to_string(&path)
            .map_err(|error| au_core::i18n::tr_format("Could not read %s: %s", &[&path.display().to_string(), &error]))?;
        Theme::parse_colors_txt(ThemeId::Custom, &text)
    }

    /// Saves `Components/Colors.txt` in Audacity's component format.
    pub fn save_components_dir(&self, dir: &Path) -> io::Result<()> {
        let components_dir = dir.join("Components");
        fs::create_dir_all(&components_dir)?;
        let mut file = fs::File::create(components_dir.join("Colors.txt"))?;
        writeln!(file, ".customTheme {{")?;

        for (name, colour) in &self.colours {
            if colour[3] == 255 {
                writeln!(
                    file,
                    "{:>30}: #{:02x}{:02x}{:02x};",
                    name, colour[0], colour[1], colour[2]
                )?;
            } else {
                writeln!(
                    file,
                    "{:>30}: #{:02x}{:02x}{:02x}{:02x};",
                    name, colour[0], colour[1], colour[2], colour[3]
                )?;
            }
        }

        writeln!(file, "}}")
    }

    /// Returns whether the theme's `Blank` colour has luminance below 0.5.
    /// Audacity themes are judged by their "Medium" (dialog/panel) colour; "Blank" is dark even in Light.
    pub fn is_dark(&self) -> bool {
        let blank = self.colour("Medium");
        let linear = |channel: u8| {
            let value = f64::from(channel) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        let luminance =
            0.2126 * linear(blank[0]) + 0.7152 * linear(blank[1]) + 0.0722 * linear(blank[2]);
        luminance < 0.5
    }
}

fn parse_hex_colour(value: &str) -> Option<Rgba> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }

    let mut channels = [0_u8; 4];
    for (index, channel) in channels.iter_mut().enumerate().take(hex.len() / 2) {
        *channel = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    if hex.len() == 6 {
        channels[3] = 255;
    }

    Some(channels)
}

fn parse_builtin_table(id: ThemeId) -> Theme {
    let text = match id {
        ThemeId::Light | ThemeId::Custom => include_str!("../themes/light.txt"),
        ThemeId::Dark => include_str!("../themes/dark.txt"),
        ThemeId::HighContrast => include_str!("../themes/high-contrast.txt"),
        ThemeId::Classic => include_str!("../themes/classic.txt"),
    };
    Theme::parse_colors_txt(id, text).expect("vendored Audacity theme colour table is valid")
}

fn decode_rgba_pixels(data: &[u8], colour_type: png::ColorType) -> Result<Vec<Rgba>, String> {
    let mut pixels = Vec::new();
    match colour_type {
        png::ColorType::Rgba => {
            if data.len() % 4 != 0 {
                return Err("Invalid RGBA theme image data".to_owned());
            }
            pixels.reserve(data.len() / 4);
            for pixel in data.chunks_exact(4) {
                pixels.push([pixel[0], pixel[1], pixel[2], pixel[3]]);
            }
        }
        png::ColorType::Rgb => {
            if data.len() % 3 != 0 {
                return Err("Invalid RGB theme image data".to_owned());
            }
            pixels.reserve(data.len() / 3);
            for pixel in data.chunks_exact(3) {
                pixels.push([pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        png::ColorType::Grayscale => {
            pixels.reserve(data.len());
            for &value in data {
                pixels.push([value, value, value, 255]);
            }
        }
        png::ColorType::GrayscaleAlpha => {
            if data.len() % 2 != 0 {
                return Err("Invalid grayscale-alpha theme image data".to_owned());
            }
            pixels.reserve(data.len() / 2);
            for pixel in data.chunks_exact(2) {
                pixels.push([pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
        }
        _ => return Err("Unsupported theme image colour type".to_owned()),
    }
    Ok(pixels)
}

/// Returns the directory used for theme settings and caches, if a configuration
/// directory can be determined.
pub fn theme_dir() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AU_THEME_DIR").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(path));
    }

    let config_home = au_core::paths::user_config_base();

    Some(config_home.join("audacity-rs").join("Theme"))
}

/// Loads the saved theme, falling back to the light theme when settings or a
/// custom cache are missing or invalid.
pub fn load_saved() -> Theme {
    let Some(dir) = theme_dir() else {
        return Theme::builtin(ThemeId::Light);
    };

    let config = match fs::read_to_string(dir.join("theme.cfg")) {
        Ok(config) => config,
        Err(_) => return Theme::builtin(ThemeId::Light),
    };
    let key = config
        .lines()
        .filter_map(|line| line.trim().strip_prefix("theme="))
        .map(str::trim)
        .next();
    let Some(id) = key.and_then(ThemeId::from_key) else {
        return Theme::builtin(ThemeId::Light);
    };

    if id == ThemeId::Custom {
        let loaded = fs::read(dir.join("ImageCache.png"))
            .ok()
            .and_then(|bytes| Theme::from_image_cache_png(&bytes).ok());
        if let Some(mut theme) = loaded {
            theme.id = ThemeId::Custom;
            theme
        } else {
            Theme::builtin(ThemeId::Light)
        }
    } else {
        Theme::builtin(id)
    }
}

/// Saves the selected theme key and, for a custom theme, its PNG image cache.
pub fn save_choice(theme: &Theme) -> io::Result<()> {
    let dir = theme_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "No configuration directory is available",
        )
    })?;
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("theme.cfg"), format!("theme={}\n", theme.id.key()))?;

    if theme.id == ThemeId::Custom {
        let bytes = theme
            .to_image_cache_png()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        fs::write(dir.join("ImageCache.png"), bytes)?;
    }

    Ok(())
}

#[cfg(feature = "gui")]
use eframe::egui;

#[cfg(feature = "gui")]
/// Converts an RGBA colour to egui's unmultiplied colour representation.
pub fn c32(colour: Rgba) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(colour[0], colour[1], colour[2], colour[3])
}

#[cfg(feature = "gui")]
/// Applies the theme's colours and widget styling to an egui context.
pub fn apply(theme: &Theme, ctx: &egui::Context) {
    let mut visuals = if theme.is_dark() {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.panel_fill = c32(theme.colour("Medium"));
    visuals.window_fill = c32(theme.colour("Medium"));
    // Text fields and slider rails: TrackInfo contrasts with TrackPanelText in every theme
    // (TrackBackground/Blank are dark even in Light).
    visuals.extreme_bg_color = c32(theme.colour("TrackInfo"));
    visuals.faint_bg_color = c32(theme.colour("Unselected"));
    visuals.selection.bg_fill = c32(theme.colour("Selected"));
    visuals.hyperlink_color = c32(theme.colour("Sample"));
    visuals.override_text_color = Some(c32(theme.colour("TrackPanelText")));

    let light = c32(theme.colour("Light"));
    let light_selected = c32(theme.colour("LightSelected"));
    let medium_selected = c32(theme.colour("MediumSelected"));

    visuals.widgets.inactive.weak_bg_fill = light;
    visuals.widgets.inactive.bg_fill = light;
    visuals.widgets.hovered.weak_bg_fill = light_selected;
    visuals.widgets.hovered.bg_fill = light_selected;
    visuals.widgets.active.weak_bg_fill = medium_selected;
    visuals.widgets.active.bg_fill = medium_selected;

    if theme.id == ThemeId::HighContrast {
        visuals.widgets.noninteractive.bg_stroke.width = 2.0;
        visuals.widgets.inactive.bg_stroke.width = 2.0;
        visuals.widgets.hovered.bg_stroke.width = 2.0;
        visuals.widgets.active.bg_stroke.width = 2.0;
        // Audacity's HC buttons are black with bright text and outlines.
        let text = c32(theme.colour("TrackPanelText"));
        let dark = c32(theme.colour("Dark"));
        for widget in [&mut visuals.widgets.inactive, &mut visuals.widgets.noninteractive] {
            widget.bg_fill = dark;
            widget.weak_bg_fill = dark;
            widget.bg_stroke.color = text;
        }
    }

    ctx.set_visuals(visuals);
}

#[cfg(feature = "gui")]
fn activate_and_save(theme: &Theme, ctx: &egui::Context) -> String {
    apply(theme, ctx);
    match save_choice(theme) {
        Ok(()) => au_core::i18n::tr_format("Theme changed to %s.", &[&theme.id.label()]),
        Err(error) => au_core::i18n::tr_format("Theme applied, but could not save the choice: %s", &[&error]),
    }
}

#[cfg(feature = "gui")]
/// Renders the View > Theme submenu and returns a status message when an
/// operation or theme change occurs.
pub fn theme_menu(
    ui: &mut egui::Ui,
    theme: &mut Theme,
    custom: &mut Option<Theme>,
) -> Option<String> {
    let ctx = ui.ctx().clone();
    let mut status = None;

    ui.menu_button("Theme", |ui| {
        for id in ThemeId::BUILTIN {
            if ui
                .selectable_label(theme.id == id, id.label())
                .clicked()
            {
                let selected = Theme::builtin(id);
                if *theme != selected {
                    *theme = selected;
                    status = Some(activate_and_save(theme, &ctx));
                }
            }
        }

        let custom_response = ui.add_enabled_ui(custom.is_some(), |ui| {
            ui.selectable_label(theme.id == ThemeId::Custom, ThemeId::Custom.label())
        });
        if custom_response.inner.clicked() {
            if let Some(mut selected) = custom.clone() {
                selected.id = ThemeId::Custom;
                if *theme != selected {
                    *theme = selected;
                    status = Some(activate_and_save(theme, &ctx));
                }
            }
        }

        ui.separator();

        if ui.button("Load Theme Components…").clicked() {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                match Theme::load_components_dir(&dir) {
                    Ok(loaded) => {
                        *custom = Some(loaded.clone());
                        *theme = loaded;
                        status = Some(activate_and_save(theme, &ctx));
                    }
                    Err(error) => {
                        status = Some(au_core::i18n::tr_format("Could not load theme components: %s", &[&error]));
                    }
                }
            }
        }

        if ui.button("Save Theme Components…").clicked() {
            if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                status = Some(match theme.save_components_dir(&dir) {
                    Ok(()) => au_core::i18n::tr_format("Theme components saved to %s.", &[&dir.display().to_string()]),
                    Err(error) => au_core::i18n::tr_format("Could not save theme components: %s", &[&error]),
                });
            }
        }

        if ui.button("Save Theme Cache").clicked() {
            let mut custom_copy = theme.clone();
            custom_copy.id = ThemeId::Custom;
            *custom = Some(custom_copy.clone());
            status = Some(match save_choice(&custom_copy) {
                Ok(()) => "Custom theme cache saved.".to_owned(),
                Err(error) => au_core::i18n::tr_format("Could not save theme cache: %s", &[&error]),
            });
        }

        if ui.button("Load Theme Cache").clicked() {
            match theme_dir() {
                None => status = Some("No theme cache directory is available.".to_owned()),
                Some(dir) => match fs::read(dir.join("ImageCache.png")) {
                    Ok(bytes) => match Theme::from_image_cache_png(&bytes) {
                        Ok(mut loaded) => {
                            loaded.id = ThemeId::Custom;
                            *custom = Some(loaded.clone());
                            *theme = loaded;
                            status = Some(activate_and_save(theme, &ctx));
                        }
                        Err(error) => {
                            status = Some(au_core::i18n::tr_format("Could not load theme cache: %s", &[&error]));
                        }
                    },
                    Err(error) => {
                        status = Some(au_core::i18n::tr_format("Could not read theme cache: %s", &[&error]));
                    }
                },
            }
        }
    });

    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn builtins_have_expected_names_and_equal_counts() {
        let light = Theme::builtin(ThemeId::Light);
        let expected_count = light.names().count();

        for id in ThemeId::BUILTIN {
            let theme = Theme::builtin(id);
            assert!(theme.names().any(|name| name == "Blank"));
            assert!(theme.names().any(|name| name == "Sample"));
            assert!(theme.names().any(|name| name == "TrackPanelText"));
            assert_eq!(theme.names().count(), expected_count);
        }

        assert_eq!(Theme::builtin(ThemeId::Dark).colour("Blank"), [46, 50, 54, 255]);
    }

    #[test]
    fn image_cache_round_trips_colours_and_names() {
        let original = Theme::builtin(ThemeId::Dark);
        let bytes = original.to_image_cache_png().expect("encode theme cache");
        let mut decoded = Theme::from_image_cache_png(&bytes).expect("decode theme cache");

        decoded.id = original.id;
        assert_eq!(decoded, original);
    }

    #[test]
    fn components_directory_round_trips() {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let unique = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "audacity-rs-theme-test-{}-{unique}",
            std::process::id()
        ));

        let original = Theme::builtin(ThemeId::Dark);
        original
            .save_components_dir(&dir)
            .expect("save components");
        let loaded = Theme::load_components_dir(&dir).expect("load components");
        assert_eq!(loaded, Theme {
            id: ThemeId::Custom,
            colours: original.colours.clone(),
        });

        fs::remove_dir_all(dir).expect("remove temporary components directory");
    }

    #[test]
    fn empty_colour_table_is_an_error() {
        assert!(Theme::parse_colors_txt(ThemeId::Custom, "").is_err());
    }

    #[test]
    fn builtin_darkness_matches_its_name() {
        assert!(Theme::builtin(ThemeId::Dark).is_dark());
        assert!(!Theme::builtin(ThemeId::Light).is_dark());
    }
}
