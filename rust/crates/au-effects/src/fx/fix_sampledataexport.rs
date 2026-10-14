// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Sample Data Export analyzer, ported from Audacity 3.7.3's Nyquist plug-in.

use crate::core::*;
use au_core::AudioBuffer;
use std::fmt::Write as _;
use std::path::Path;

const FILE_FORMATS: &[&str] = &["None", "Sample Count", "Time Indexed"];
const HEADER_CHOICES: &[&str] = &["None", "Minimal", "Standard", "All"];
const CHANNEL_LAYOUTS: &[&str] = &["L-R on Same Line", "Alternate Lines", "L Channel First"];
const MESSAGE_CHOICES: &[&str] = &["Yes", "Errors Only", "None"];

/// Exports sample data and reports statistics about the selection.
pub struct SampleDataExport {
    /// Maximum number of samples to export.
    pub number: i32,
    /// Measurement scale: 0 for dB, 1 for linear.
    pub units: f64,
    /// Output filename.
    pub filename: String,
    /// Text-file indexing format.
    pub fileformat: f64,
    /// Header verbosity.
    pub header: f64,
    /// Optional header text.
    pub optext: String,
    /// Stereo channel arrangement.
    pub channel_layout: f64,
    /// Whether to show success and error messages.
    pub messages: f64,
}

impl Default for SampleDataExport {
    fn default() -> Self {
        Self {
            number: 100,
            units: 0.0,
            filename: "*default*/sample-data.txt".to_string(),
            fileformat: 0.0,
            header: 0.0,
            optext: String::new(),
            channel_layout: 0.0,
            messages: 0.0,
        }
    }
}

impl SampleDataExport {
    fn effective_filename(&self) -> String {
        let prefix = "*default*";
        if self.filename == prefix || self.filename.starts_with(&format!("{prefix}/")) {
            if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
            {
                let suffix = &self.filename[prefix.len()..];
                return format!("{}{}", home.to_string_lossy(), suffix);
            }
        }
        self.filename.clone()
    }

    fn effective_format(&self, filename: &str) -> usize {
        match Path::new(filename)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "csv" => 3,
            "html" | "htm" => 4,
            _ => self.fileformat.round().clamp(0.0, 2.0) as usize,
        }
    }

    fn limit(&self, buf: &AudioBuffer) -> usize {
        (self.number.max(0) as usize).min(buf.len())
    }

    fn channel_count(buf: &AudioBuffer) -> usize {
        if buf.channels.len() >= 2 {
            2
        } else {
            1
        }
    }

    fn channels_text(buf: &AudioBuffer) -> &'static str {
        if Self::channel_count(buf) == 2 {
            "2 channels (stereo)"
        } else {
            "1 channel (mono)"
        }
    }

    fn units_text(&self) -> &'static str {
        if self.units.round() as i32 == 0 {
            "dB"
        } else {
            "linear"
        }
    }

    fn channel_order(&self, buf: &AudioBuffer, fileformat: usize) -> String {
        let stereo = Self::channel_count(buf) == 2;
        if fileformat == 3 && self.channel_layout.round() as i32 == 0 {
            "One column per channel.\n".to_string()
        } else if fileformat == 3 && self.channel_layout.round() as i32 == 2 {
            "One row per channel.\n".to_string()
        } else if !stereo || fileformat == 4 {
            String::new()
        } else {
            match self.channel_layout.round() as i32 {
                0 => "Left channel then Right channel on same line.\n".to_string(),
                1 => "Left and right channels on alternate lines.\n".to_string(),
                2 => "Left channel first then right channel.\n".to_string(),
                _ => "Unspecified channel order".to_string(),
            }
        }
    }

    fn float_text(value: f64) -> String {
        if value.is_nan() {
            "NaN".to_string()
        } else if value == f64::INFINITY {
            "inf".to_string()
        } else if value == f64::NEG_INFINITY {
            "-inf".to_string()
        } else {
            format!("{value:.5}")
        }
    }

    fn sample_text(value: f32) -> String {
        // Nyquist's %1.5f formats the f32 sample after promotion to C double.
        format!("{:.5}", value as f64)
    }

    fn linear_to_db(value: f64) -> String {
        if value == 0.0 {
            "[-inf]".to_string()
        } else {
            let ln10_over_20 = 10.0_f64.ln() / 20.0;
            Self::float_text(value.ln() / ln10_over_20)
        }
    }

    fn get_sample(buf: &AudioBuffer, channel: usize, index: usize) -> f32 {
        buf.channels
            .get(channel)
            .and_then(|samples| samples.get(index))
            .copied()
            .unwrap_or(0.0)
    }

    fn peak_and_rms(&self, buf: &AudioBuffer, limit: usize) -> (f64, f64) {
        let channels = Self::channel_count(buf);
        let mut peak = 0.0_f32;
        let mut square_sum = 0.0_f64;

        for index in 0..limit {
            let left = Self::get_sample(buf, 0, index);
            peak = peak.max(left.abs());

            let left_square = left * left;
            if channels == 2 {
                let right = Self::get_sample(buf, 1, index);
                peak = peak.max(right.abs());
                let right_square = right * right;
                let combined = left_square + right_square;
                let averaged = 0.5_f32 * combined;
                square_sum += averaged as f64;
            } else {
                square_sum += left_square as f64;
            }
        }

        let mean_square = if limit == 0 {
            f64::NAN
        } else {
            (square_sum / limit as f64) as f32 as f64
        };
        let rms = (mean_square.sqrt() as f32) as f64;
        (peak as f64, rms)
    }

    fn dc_offset(&self, buf: &AudioBuffer, channel: usize, limit: usize) -> f64 {
        if limit == 0 {
            return f64::NAN;
        }

        let mut total = 0.0_f64;
        for index in 0..limit {
            total += Self::get_sample(buf, channel, index) as f64;
        }
        total / limit as f64
    }

    fn normhead(&self, buf: &AudioBuffer, filename: &str, fileformat: usize) -> String {
        let limit = self.limit(buf);
        let duration = limit as f64 / buf.sample_rate as f64;
        let optional = if self.optext.is_empty() {
            "\n\n".to_string()
        } else {
            format!("\n{}\n\n", self.optext)
        };

        if fileformat == 4 {
            format!(
                "{}   {}\n{}Sample Rate: {} Hz.\nLength processed: {} samples {} seconds.{}",
                filename,
                Self::channels_text(buf),
                self.channel_order(buf, fileformat),
                buf.sample_rate,
                limit,
                Self::float_text(duration),
                optional
            )
        } else {
            format!(
                "{}   {}\n{}Sample Rate: {} Hz. Sample values on {} scale.\nLength processed: {} samples {} seconds.{}",
                filename,
                Self::channels_text(buf),
                self.channel_order(buf, fileformat),
                buf.sample_rate,
                self.units_text(),
                limit,
                Self::float_text(duration),
                optional
            )
        }
    }

    fn minhead(&self, buf: &AudioBuffer, fileformat: usize) -> String {
        let order = self.channel_order(buf, fileformat);
        if self.optext.is_empty() {
            format!(
                "Sample Rate: {} Hz.  Sample values on {} scale.\n{}\n",
                buf.sample_rate,
                self.units_text(),
                order
            )
        } else {
            format!(
                "Sample Rate: {} Hz.  Sample values on {} scale.\n{}\n{}\n\n",
                buf.sample_rate,
                self.units_text(),
                order,
                self.optext
            )
        }
    }

    fn fullhead(&self, buf: &AudioBuffer, filename: &str, fileformat: usize) -> String {
        let limit = self.limit(buf);
        let duration = limit as f64 / buf.sample_rate as f64;
        let (peak, rms) = self.peak_and_rms(buf, limit);
        let dc = if Self::channel_count(buf) == 2 {
            let left = self.dc_offset(buf, 0, limit);
            let right = self.dc_offset(buf, 1, limit);
            format!(
                "Left: {} lin, {} dB | Right: {} lin, {} dB.",
                Self::float_text(left),
                Self::linear_to_db(left.abs()),
                Self::float_text(right),
                Self::linear_to_db(right.abs())
            )
        } else {
            let offset = self.dc_offset(buf, 0, limit);
            format!(
                "{} linear, {} dB.",
                Self::float_text(offset),
                Self::linear_to_db(offset.abs())
            )
        };
        let optional = if self.optext.is_empty() {
            "\n\n\n".to_string()
        } else {
            format!("\n{}\n\n\n", self.optext)
        };

        format!(
            "{}\nSample Rate: {} Hz. Sample values on {} scale. {}.\n{}Length processed: {} samples, {} seconds.\nPeak amplitude: {} (linear) {} dB.  Unweighted RMS: {} dB.\nDC offset: {}{}",
            filename,
            buf.sample_rate,
            self.units_text(),
            Self::channels_text(buf),
            self.channel_order(buf, fileformat),
            limit,
            Self::float_text(duration),
            Self::float_text(peak),
            Self::linear_to_db(peak),
            Self::linear_to_db(rms),
            dc,
            optional
        )
    }

    fn snd_value(&self, value: f32) -> String {
        if self.units.round() as i32 == 0 {
            Self::linear_to_db((value as f64).abs())
        } else {
            Self::sample_text(value)
        }
    }

    fn format_text_sample(
        &self,
        output: &mut String,
        index: usize,
        sample: f32,
        same_line: bool,
        fileformat: usize,
        sample_rate: u32,
    ) {
        let value = self.snd_value(sample);
        match fileformat {
            0 => {
                output.push_str(&value);
                output.push_str(if same_line { "\t" } else { "\n" });
            }
            1 => {
                let _ = write!(output, "{}\t{}", index + 1, value);
                output.push_str(if same_line { "\t" } else { "\n" });
            }
            2 => {
                let time = index as f64 / sample_rate as f64;
                let _ = write!(output, "{}\t{}", Self::float_text(time), value);
                output.push_str(if same_line { "\t" } else { "\n" });
            }
            3 => {
                output.push_str(&value);
                output.push_str(if self.channel_layout.round() as i32 == 2 || same_line {
                    ","
                } else {
                    "\n"
                });
            }
            _ => {}
        }
    }

    fn print_text_channel(
        &self,
        output: &mut String,
        buf: &AudioBuffer,
        channel: usize,
        limit: usize,
        fileformat: usize,
        same_line: bool,
    ) {
        for index in 0..limit {
            self.format_text_sample(
                output,
                index,
                Self::get_sample(buf, channel, index),
                same_line,
                fileformat,
                buf.sample_rate,
            );
        }
    }

    fn html_head() -> &'static str {
        "<!DOCTYPE html>\n\
<html>\n\
<head>\n\
<meta name=\"generator\" content=\n\
\"Sample Data Export by Steve Daulton, (https://www.audionyq.com). Released under GPL v2.0+\">\n\
<meta name=\"description\" content=\"Sample Printer, Free Audacity plug-in\" />\n\
<meta name=\"keywords\" content=\"sample printer,Audacity,plug-ins,plugins,effects,audio,audio processing,music,analyze\" />\n\
<meta name=\"author\" content=\"Steve Daulton\" />\n\
<meta charset=\"UTF-8\">\n\
\n\
<style type=\"text/css\">\n\
body {\n\
  margin: 1em 5%;\n\
  background-color: #dda;\n\
  font-family:Arial,Helvetica,sans-serif;\n\
  }\n\
table,th,td {\n\
  background-color: #fff;\n\
  border:1px solid black;\n\
  text-align: center;\n\
}\n\
table {\n\
  width: auto;\n\
  border: 2px;\n\
  border-style:ridge;\n\
  border-collapse:collapse;\n\
}\n\
td {\n\
  text-align: right;\n\
  padding-right: 0.5em;\n\
}\n\
tr:hover td {\n\
  background-color:#fcd;\n\
}\n\
th {\n\
  padding: 0 0.5em;\n\
  background-color: #ddf;\n\
  border-bottom-width: 2px;\n\
  border-bottom-style:ridge;\n\
}\n\
h1 {\n\
  font-size: 1.6em;\n\
  color: #633;\n\
}\n\
h2 {\n\
  font-size: 1.4em;\n\
  color: #633;\n\
}\n\
h3 {\n\
  font-size: 1em;\n\
  color: #366;\n\
}\n\
h4 {\n\
  font-size: 1em;\n\
  color: #000;\n\
}\n\
ul {\n\
  position:relative;\n\
  top: -0.5em;\n\
  }\n\
#footer {\n\
  font-size: 0.8em;\n\
  position:relative;\n\
  top: 0.5em;\n\
  left: 2%;\n\
  }\n\
#footer span {\n\
  font-style:italic;\n\
  font-weight: bold;\n\
  color: #633;\n\
  }\n\
#footer a:link,a:visited {\n\
  color: #639;\n\
  text-decoration: none;\n\
  }\n\
#footer a:hover,a:active {\n\
  text-decoration: underline;\n\
  color: blue;\n\
  }\n\
</style>\n\
<title>Sample Data Export</title>\n\
</head>\n"
    }

    fn html_table_head(stereo: bool) -> &'static str {
        if stereo {
            "<table title=\"audio sample value analysis\">\n\
<tr>\n\
<th>Sample #</th>\n\
<th>Seconds</th>\n\
<th>Left (linear)</th>\n\
<th>Right (linear)</th>\n\
<th>Left (dB)</th>\n\
<th>Right (dB)</th>\n\
</tr>"
        } else {
            "<table title=\"sample data\">\n\
<tr>\n\
<th>Sample #</th>\n\
<th>Seconds</th>\n\
<th>Value (linear)</th>\n\
<th>Value (dB)</th>\n\
</tr>"
        }
    }

    fn html_foot() -> &'static str {
        "</table>\n\
<p id=\"footer\">Produced with <span>Sample Data Export</span> for\n\
<a href=\"https://www.audacityteam.org/\">Audacity</a> by Steve\n\
Daulton (<a href=\n\
\"https://audionyq.com\">audionyq.com</a>)</p>\n\
</body>\n\
</html>"
    }

    fn html_text(&self, buf: &AudioBuffer, filename: &str) -> String {
        let limit = self.limit(buf);
        let stereo = Self::channel_count(buf) == 2;
        let duration = limit as f64 / buf.sample_rate as f64;
        let (peak, rms) = self.peak_and_rms(buf, limit);
        let dc = if stereo {
            let left = self.dc_offset(buf, 0, limit);
            let right = self.dc_offset(buf, 1, limit);
            format!(
                "Left: {} lin, {} dB | Right: {} linear, &nbsp;&nbsp;{} dB.",
                Self::float_text(left),
                Self::linear_to_db(left.abs()),
                Self::float_text(right),
                Self::linear_to_db(right.abs())
            )
        } else {
            let offset = self.dc_offset(buf, 0, limit);
            format!(
                "{} linear, &nbsp;&nbsp;{} dB.",
                Self::float_text(offset),
                Self::linear_to_db(offset.abs())
            )
        };

        let trimmed_name = filename.trim_end_matches(|c| ".html".contains(c));
        let mut output = String::new();
        output.push_str(Self::html_head());
        let _ = write!(
            output,
            "<body>\n<h1>Sample Data Export - {}</h1>\n<h2>{}</h2>\n<h4>{}. &nbsp;&nbsp;{} samples. &nbsp;&nbsp; {} seconds.<br></h4>\n<h3>Audio data analysis:</h3>\n<ul>\n<li><b>Sample Rate:</b> &nbsp;&nbsp;{} Hz.</li>\n<li><b>Peak Amplitude:</b> &nbsp;&nbsp;{} (linear) &nbsp;&nbsp;{} dB.</li>\n<li><b>RMS</b> (unweighted): &nbsp;&nbsp;{} dB.</li>\n<li><b>DC Offset:</b> &nbsp;&nbsp;{}</li>\n</ul>\n",
            trimmed_name,
            self.optext,
            Self::channels_text(buf),
            limit,
            Self::float_text(duration),
            buf.sample_rate,
            Self::float_text(peak),
            Self::linear_to_db(peak),
            Self::linear_to_db(rms),
            dc
        );
        output.push_str(Self::html_table_head(stereo));

        for index in 0..limit {
            let left = Self::get_sample(buf, 0, index);
            let time = Self::float_text(index as f64 / buf.sample_rate as f64);
            if stereo {
                let right = Self::get_sample(buf, 1, index);
                let _ = write!(
                    output,
                    "\n<tr>\n<td>{}</td>\n<td>{}</td>\n<td>{}</td>\n<td>{}</td>\n<td>{}</td>\n<td>{}</td>\n</tr>\n",
                    index + 1,
                    time,
                    Self::sample_text(left),
                    Self::sample_text(right),
                    Self::linear_to_db((left as f64).abs()),
                    Self::linear_to_db((right as f64).abs())
                );
            } else {
                let _ = write!(
                    output,
                    "\n<tr>\n<td>{}</td>\n<td>{}</td>\n<td>{}</td>\n<td>{}</td>\n</tr>\n",
                    index + 1,
                    time,
                    Self::sample_text(left),
                    Self::linear_to_db((left as f64).abs())
                );
            }
        }
        output.push_str(Self::html_foot());
        output
    }

    /// Renders the exact text that this effect writes to its output file.
    pub fn render_text(&self, buf: &AudioBuffer) -> String {
        let filename = self.effective_filename();
        let fileformat = self.effective_format(&filename);
        if fileformat == 4 {
            return self.html_text(buf, &filename);
        }

        let mut output = String::new();
        match self.header.round() as i32 {
            0 => {
                if !self.optext.is_empty() {
                    let _ = write!(
                        output,
                        "{}\n{}\n",
                        self.optext,
                        self.channel_order(buf, fileformat)
                    );
                }
            }
            1 => output.push_str(&self.minhead(buf, fileformat)),
            2 => output.push_str(&self.normhead(buf, &filename, fileformat)),
            3 => output.push_str(&self.fullhead(buf, &filename, fileformat)),
            _ => output.push_str(&self.normhead(buf, &filename, fileformat)),
        }

        let limit = self.limit(buf);
        if Self::channel_count(buf) == 2 && self.channel_layout.round() as i32 == 2 {
            if self.header.round() as i32 != 0 {
                output.push_str("Left Channel.\n\n");
            }
            self.print_text_channel(&mut output, buf, 0, limit, fileformat, false);
            if self.header.round() as i32 == 0 {
                output.push('\n');
            } else {
                output.push_str("\n\nRight Channel.\n\n");
            }
            self.print_text_channel(&mut output, buf, 1, limit, fileformat, false);
        } else if Self::channel_count(buf) == 2 {
            for index in 0..limit {
                self.format_text_sample(
                    &mut output,
                    index,
                    Self::get_sample(buf, 0, index),
                    self.channel_layout.round() as i32 == 0,
                    fileformat,
                    buf.sample_rate,
                );
                self.format_text_sample(
                    &mut output,
                    index,
                    Self::get_sample(buf, 1, index),
                    false,
                    fileformat,
                    buf.sample_rate,
                );
            }
        } else {
            self.print_text_channel(&mut output, buf, 0, limit, fileformat, false);
        }

        output
    }

    fn success_message(&self, buf: &AudioBuffer, filename: &str, fileformat: usize) -> String {
        format!(
            "{}Data written to:\n{}",
            self.normhead(buf, filename, fileformat),
            filename
        )
    }
}

impl BuiltinEffect for SampleDataExport {
    fn id(&self) -> &'static str {
        "SampleDataExport"
    }

    fn name(&self) -> &'static str {
        "Sample Data Export"
    }

    fn kind(&self) -> EffectKind {
        EffectKind::Analyze
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::int("NUMBER", "Limit output to first samples", 1.0, 1_000_000.0, 100.0),
            ParamDef::choice("UNITS", "Measurement scale", &["dB", "Linear"], 0),
            ParamDef::choice("FILEFORMAT", "Index (text files only)", FILE_FORMATS, 0),
            ParamDef::choice("HEADER", "Include header information", HEADER_CHOICES, 0),
            ParamDef::choice("CHANNEL-LAYOUT", "Channel layout for stereo", CHANNEL_LAYOUTS, 0),
            ParamDef::choice("MESSAGES", "Show messages", MESSAGE_CHOICES, 0),
        ]
    }

    fn choice_symbols(&self, key: &str) -> Option<&'static [&'static str]> {
        match key {
            "FILEFORMAT" => Some(&["None", "Count", "Time"]),
            "CHANNEL-LAYOUT" => Some(&["SameLine", "Alternate", "LFirst"]),
            "MESSAGES" => Some(&["Yes", "Errors", "None"]),
            _ => None,
        }
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "NUMBER" => Some(self.number as f64),
            "UNITS" => Some(self.units),
            "FILEFORMAT" => Some(self.fileformat),
            "HEADER" => Some(self.header),
            "CHANNEL-LAYOUT" => Some(self.channel_layout),
            "MESSAGES" => Some(self.messages),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let value = if value.is_nan() { 0.0 } else { value };
        match key {
            "NUMBER" => {
                self.number = value.round().clamp(1.0, 1_000_000.0) as i32;
                true
            }
            "UNITS" => {
                self.units = value.round().clamp(0.0, 1.0);
                true
            }
            "FILEFORMAT" => {
                self.fileformat = value.round().clamp(0.0, 2.0);
                true
            }
            "HEADER" => {
                self.header = value.round().clamp(0.0, 3.0);
                true
            }
            "CHANNEL-LAYOUT" => {
                self.channel_layout = value.round().clamp(0.0, 2.0);
                true
            }
            "MESSAGES" => {
                self.messages = value.round().clamp(0.0, 2.0);
                true
            }
            _ => false,
        }
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        let filename = self.effective_filename();
        let fileformat = self.effective_format(&filename);
        let text = self.render_text(buf);

        match std::fs::write(&filename, text.as_bytes()) {
            Ok(()) => {
                ctx.report = text;
                if self.messages.round() as i32 == 0 {
                    ctx.report.push_str(&self.success_message(buf, &filename, fileformat));
                }
            }
            Err(_) => {
                let message = format!("Error.\n\"{}\" cannot be written.", filename);
                ctx.report = text;
                if self.messages.round() as i32 != 2 {
                    ctx.report.push_str(&message);
                }
            }
        }
        Ok(())
    }

    fn text_keys(&self) -> Vec<&'static str> {
        vec!["FILENAME", "OPTEXT"]
    }

    fn get_text(&self, key: &str) -> Option<String> {
        match key {
            "FILENAME" => Some(self.filename.clone()),
            "OPTEXT" => Some(self.optext.clone()),
            _ => None,
        }
    }

    fn set_text(&mut self, key: &str, value: &str) -> bool {
        match key {
            "FILENAME" => {
                self.filename = value.to_string();
                true
            }
            "OPTEXT" => {
                self.optext = value.to_string();
                true
            }
            _ => false,
        }
    }
}

/// Returns the effects defined in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![Box::new(SampleDataExport::default())]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::set_param;

    fn short_stereo() -> AudioBuffer {
        let sample_rate = 8_000;
        let left = (0..16)
            .map(|i| ((i as f32) * 0.2).sin())
            .collect::<Vec<_>>();
        let right = (0..16)
            .map(|i| ((i as f32) * 0.3).cos())
            .collect::<Vec<_>>();
        AudioBuffer::from_channels(sample_rate, vec![left, right])
    }

    #[test]
    fn default_runs_on_short_stereo() {
        let effect = SampleDataExport::default();
        let text = effect.render_text(&short_stereo());
        assert!(!text.is_empty());
    }

    #[test]
    fn empty_buffer_does_not_panic() {
        let buffer = AudioBuffer::from_channels(44_100, vec![Vec::new(), Vec::new()]);
        let effect = SampleDataExport::default();
        let _ = effect.render_text(&buffer);
    }

    #[test]
    fn rendering_is_repeatable() {
        let effect = SampleDataExport::default();
        let buffer = short_stereo();
        assert_eq!(effect.render_text(&buffer), effect.render_text(&buffer));
    }

    #[test]
    fn parameters_clamp() {
        let mut effect = SampleDataExport::default();
        assert!(set_param(&mut effect, "NUMBER", 2_000_000.0));
        assert_eq!(effect.number, 1_000_000);
        assert!(set_param(&mut effect, "UNITS", -10.0));
        assert_eq!(effect.units, 0.0);
        assert!(set_param(&mut effect, "HEADER", 99.0));
        assert_eq!(effect.header, 3.0);
    }
}
