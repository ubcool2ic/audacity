//! Parser for Audacity 3.7.3 Nyquist plug-in headers and program text.
//!
//! This module ports the header parsing and control-binding behavior of
//! `NyquistBase`. It does not evaluate Lisp or SAL programs.

/// The execution kind declared by a Nyquist plug-in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NyType {
    /// Processes selected audio.
    Process,
    /// Generates audio.
    Generate,
    /// Analyzes selected audio.
    Analyze,
    /// Runs as a tool without a required audio selection.
    Tool,
}

/// A Nyquist plug-in control type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NyControlType {
    /// Integer slider or numeric control.
    Int,
    /// Floating-point slider or numeric control.
    Float,
    /// String control.
    String,
    /// Choice control.
    Choice,
    /// Integer text control.
    IntText,
    /// Floating-point text control.
    FloatText,
    /// Fixed display text; it has no binding.
    Text,
    /// Time control, represented as a floating-point value.
    Time,
    /// File-path control.
    File,
}

/// A parsed Nyquist control.
#[derive(Clone, Debug, PartialEq)]
pub struct NyControl {
    /// Nyquist variable name.
    pub var: String,
    /// Control name presented to automation clients.
    pub name: String,
    /// Control type.
    pub ty: NyControlType,
    /// Visible control label.
    pub label: String,
    /// Numeric minimum, where applicable.
    pub low: f64,
    /// Numeric maximum, where applicable.
    pub high: f64,
    /// Default value in string form.
    pub default_value: String,
    /// Choice symbol and visible label pairs.
    pub choices: Vec<(String, String)>,
    /// File type descriptions paired with their extensions.
    ///
    /// An empty extension matches all file extensions, corresponding to
    /// Audacity's `*.*` handling.
    pub file_wildcards: Vec<(String, Vec<String>)>,
}

/// A parsed Nyquist plug-in program and its header metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct NyProgram {
    /// Nyquist plug-in header syntax version.
    pub version: i32,
    /// Effect execution kind.
    pub kind: NyType,
    /// Whether the plug-in declares spectral processing.
    pub is_spectral: bool,
    /// Display name.
    pub name: String,
    /// Action text.
    pub action: String,
    /// Informational text.
    pub info: String,
    /// Author.
    pub author: String,
    /// Plug-in release version.
    pub release: String,
    /// Copyright text.
    pub copyright: String,
    /// Audacity manual page identifier.
    pub manpage: String,
    /// Local help file name.
    pub helpfile: String,
    /// Whether preview is enabled.
    pub preview: bool,
    /// Whether the debug button is enabled.
    pub debug_button: bool,
    /// Clip merging mode: `-1` is automatic, `0` is disabled, `1` is enabled.
    pub merge_clips: i32,
    /// Whether split lines should be restored.
    pub restore_splits: bool,
    /// Optional maximum processing length, in samples.
    pub max_len: Option<f64>,
    /// Parsed controls in header order.
    pub controls: Vec<NyControl>,
    /// Program text with magic header lines replaced by blank lines.
    pub code: String,
    /// Whether the program uses SAL rather than Lisp.
    pub is_sal: bool,
}

#[derive(Clone, Debug, Default)]
struct Tokenizer {
    sl: bool,
    q: bool,
    paren: i32,
    tok: String,
    tokens: Vec<String>,
}

impl Tokenizer {
    fn end_token(&mut self) {
        if !self.tok.is_empty() {
            self.tokens.push(std::mem::take(&mut self.tok));
        }
    }

    /// Tokenizes a line, maintaining quoted-string and parenthesis state across
    /// continued header lines.
    fn tokenize(
        &mut self,
        line: &str,
        eof: bool,
        trim_start: usize,
        trim_end: usize,
    ) -> bool {
        let chars: Vec<char> = line.chars().collect();
        if trim_start > chars.len() || trim_end > chars.len().saturating_sub(trim_start) {
            return true;
        }
        let end = chars.len() - trim_end;
        for &c in &chars[trim_start..end] {
            if self.q && !self.sl && c == '\\' {
                self.sl = true;
                continue;
            }

            if !self.sl && c == '"' {
                if !self.q {
                    if self.paren == 0 {
                        self.end_token();
                    }
                    self.tok.push(c);
                    self.q = true;
                } else {
                    self.tok.push(c);
                    if self.paren == 0 {
                        self.end_token();
                    }
                    self.q = false;
                }
            } else if !self.q && self.paren == 0 && (c == ' ' || c == '\t') {
                self.end_token();
            } else if !self.q && c == ';' {
                // Lisp comments are valid inside a continued header list too.
                break;
            } else if !self.q && c == '(' {
                self.paren += 1;
                if self.paren == 1 {
                    self.end_token();
                    self.tok.push(c);
                } else {
                    self.tok.push(c);
                }
            } else if !self.q && c == ')' {
                self.paren -= 1;
                if self.paren == 0 {
                    self.tok.push(c);
                    self.end_token();
                } else if self.paren < 0 {
                    self.paren = 0;
                    self.end_token();
                } else {
                    self.tok.push(c);
                }
            } else {
                if self.sl && self.paren != 0 {
                    // Nested list text is tokenized again when parsed.
                    self.tok.push('\\');
                }
                let actual = if self.sl && self.paren == 0 && c == 'n' {
                    '\n'
                } else {
                    c
                };
                self.tok.push(actual);
            }
            self.sl = false;
        }

        if eof || (!self.q && self.paren == 0) {
            self.end_token();
            true
        } else {
            if self.q {
                self.tok.push('\n');
            }
            false
        }
    }
}

fn unquote_msgid(s: &str, allow_parens: bool, extra: &mut Option<String>) -> String {
    unquote_msgid_x(s, allow_parens, Some(extra))
}

fn unquote_msgid_x(s: &str, allow_parens: bool, mut extra: Option<&mut Option<String>>) -> String {
    if let Some(e) = extra.as_deref_mut() {
        *e = None;
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= 2 && chars[0] == '"' && chars[chars.len() - 1] == '"' {
        return chars[1..chars.len() - 1].iter().collect();
    }

    if allow_parens && chars.len() >= 2 && chars[0] == '(' && chars[chars.len() - 1] == ')' {
        let mut tokenizer = Tokenizer::default();
        tokenizer.tokenize(s, true, 1, 1);
        if tokenizer.tokens.len() > 1 {
            if let Some(e) = extra {
                if tokenizer.tokens[1].starts_with('(') {
                    *e = Some(unquote_msgid_x(&tokenizer.tokens[0], false, None));
                    return unquote_msgid_x(&tokenizer.tokens[1], true, None);
                }
            }
            return unquote_msgid_x(&tokenizer.tokens[1], false, None);
        }
        return String::new();
    }

    s.to_owned()
}

fn unquote(s: &str, allow_parens: bool) -> String {
    unquote_msgid_x(s, allow_parens, None)
}

fn parse_choices(text: &str) -> Vec<(String, String)> {
    if text.starts_with('(') {
        let mut tokenizer = Tokenizer::default();
        tokenizer.tokenize(text, true, 1, 1);
        return tokenizer
            .tokens
            .iter()
            .map(|choice| {
                let mut symbol = None;
                let label = unquote_msgid(choice, true, &mut symbol);
                (symbol.unwrap_or_else(|| label.clone()), label)
            })
            .collect();
    }

    let list = if text.starts_with('"') && text.ends_with('"') && text.len() >= 2 {
        &text[1..text.len() - 1]
    } else {
        text
    };

    list.split(',')
        .map(|part| {
            let label = part.trim().to_owned();
            (label.clone(), label)
        })
        .collect()
}

fn parse_file_extensions(text: &str) -> Vec<String> {
    if !text.starts_with('(') {
        return Vec::new();
    }
    let mut tokenizer = Tokenizer::default();
    tokenizer.tokenize(text, true, 1, 1);
    tokenizer.tokens.iter().map(|s| unquote(s, true)).collect()
}

fn parse_file_types(text: &str) -> Vec<(String, Vec<String>)> {
    if text.starts_with('(') {
        let mut tokenizer = Tokenizer::default();
        tokenizer.tokenize(text, true, 1, 1);
        if tokenizer
            .tokens
            .first()
            .is_some_and(|token| token.starts_with('('))
        {
            let mut result = Vec::new();
            for file_type in &tokenizer.tokens {
                let mut type_tokenizer = Tokenizer::default();
                type_tokenizer.tokenize(file_type, true, 1, 1);
                if type_tokenizer.tokens.len() == 2 {
                    result.push((
                        unquote(&type_tokenizer.tokens[0], true),
                        parse_file_extensions(&type_tokenizer.tokens[1]),
                    ));
                }
            }
            if !result.is_empty() {
                return result;
            }
        }
    }

    let text = unquote(text, true);
    let pieces: Vec<&str> = text.split('|').collect();
    let pair_count = pieces.len() / 2;
    let mut result = Vec::with_capacity(pair_count);
    for pair in pieces.chunks_exact(2) {
        let extensions = pair[1]
            .split(';')
            .filter_map(|extension| {
                extension
                    .strip_prefix("*.")
                    .map(|ext| if ext == "*" { String::new() } else { ext.to_owned() })
            })
            .collect();
        result.push((pair[0].to_owned(), extensions));
    }
    result
}

fn compatible_to_double(s: &str) -> f64 {
    s.trim().parse::<f64>().unwrap_or(0.0)
}

/// Internat::ToString (Audacity lib-strings): `%f` with trailing zeros stripped
/// (keeping one) when `digits == -1`, otherwise `%.<digits>f`.
pub fn internat_to_string(value: f64, digits: i32) -> String {
    if !value.is_finite() {
        return if value.is_nan() { "nan".into() } else if value > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if digits >= 0 {
        return format!("{:.*}", digits as usize, value);
    }
    let r = format!("{value:.6}");
    let b = r.as_bytes();
    let mut pos = b.len() - 1;
    while pos > 1 && b[pos] == b'0' && b[pos - 1] != b'.' {
        pos -= 1;
    }
    r[..=pos].to_owned()
}

fn format_number(value: f64, precision: usize) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    if value == 0.0 {
        return "0".to_owned();
    }

    let precision = precision.max(1);
    let scientific = format!("{:.*e}", precision - 1, value);
    let (mantissa, exponent) = match scientific.split_once('e') {
        Some(parts) => parts,
        None => return value.to_string(),
    };
    let exponent: i32 = exponent.parse().unwrap_or(0);
    let use_scientific = exponent < -4 || exponent >= precision as i32;

    if use_scientific {
        let mut mantissa = mantissa.to_owned();
        while mantissa.contains('.') && mantissa.ends_with('0') {
            mantissa.pop();
        }
        if mantissa.ends_with('.') {
            mantissa.pop();
        }
        let sign = if exponent >= 0 { '+' } else { '-' };
        let abs_exponent = exponent.unsigned_abs();
        format!("{mantissa}e{sign}{abs_exponent:02}")
    } else {
        let decimals = (precision as i32 - exponent - 1).max(0) as usize;
        let mut fixed = format!("{:.*}", decimals, value);
        while fixed.contains('.') && fixed.ends_with('0') {
            fixed.pop();
        }
        if fixed.ends_with('.') {
            fixed.pop();
        }
        fixed
    }
}

fn default_for_control(control: &NyControl, version: i32) -> String {
    if control.ty == NyControlType::Choice && version >= 4 {
        if let Ok(index) = control.default_value.parse::<usize>() {
            if let Some((symbol, _)) = control.choices.get(index) {
                return symbol.clone();
            }
        }
    }
    control.default_value.clone()
}

struct ProgramParser {
    program: NyProgram,
    found_plugin_marker: bool,
    found_code_type: bool,
    found_type_line: bool,
    is_tool_menu: bool,
    error: Option<String>,
}

impl ProgramParser {
    fn new() -> Self {
        Self {
            program: NyProgram {
                version: 4,
                kind: NyType::Process,
                is_spectral: false,
                name: String::new(),
                action: "Applying Nyquist Effect...".to_owned(),
                info: String::new(),
                author: "n/a".to_owned(),
                release: "n/a".to_owned(),
                copyright: "n/a".to_owned(),
                manpage: String::new(),
                helpfile: String::new(),
                preview: true,
                debug_button: true,
                merge_clips: -1,
                restore_splits: true,
                max_len: None,
                controls: Vec::new(),
                code: String::new(),
                is_sal: false,
            },
            found_plugin_marker: false,
            found_code_type: false,
            found_type_line: false,
            is_tool_menu: false,
            error: None,
        }
    }

    fn parse_header_line(&mut self, tokens: &[String]) {
        if tokens.is_empty() {
            return;
        }

        let len = tokens.len();
        let key = tokens[0].as_str();

        if len == 2
            && key == "nyquist"
            && (tokens[1] == "plug-in" || tokens[1] == "plugin")
        {
            self.found_plugin_marker = true;
            return;
        }

        if len >= 2 && key == "type" {
            let mut ty = tokens[1].as_str();
            self.is_tool_menu = false;
            if ty == "tool" {
                self.is_tool_menu = true;
                self.program.kind = NyType::Tool;
                if len >= 3 {
                    ty = &tokens[2];
                }
            }

            match ty {
                "process" => self.program.kind = NyType::Process,
                "generate" => self.program.kind = NyType::Generate,
                "analyze" => self.program.kind = NyType::Analyze,
                _ => {}
            }

            if len >= 3 && tokens[2] == "spectral" {
                self.program.is_spectral = true;
            }
            self.found_type_line = true;
            return;
        }

        if len == 2 && key == "codetype" {
            match tokens[1].as_str() {
                "lisp" => {
                    self.program.is_sal = false;
                    self.found_code_type = true;
                }
                "sal" => {
                    self.program.is_sal = true;
                    self.found_code_type = true;
                }
                _ => {}
            }
            return;
        }

        if len >= 2 && key == "version" {
            let version = tokens[1].parse::<i32>().unwrap_or(0);
            if !(1..=4).contains(&version) {
                self.error = Some(format!(
                    "This version of Audacity does not support Nyquist plug-in version {version}"
                ));
                return;
            }
            self.program.version = version;
            return;
        }

        if len >= 2 && key == "name" {
            let mut name = unquote(&tokens[1], true);
            if name.ends_with("...") {
                name.truncate(name.len() - 3);
            }
            self.program.name = name;
            return;
        }
        if len >= 2 && key == "action" {
            self.program.action = unquote(&tokens[1], true);
            return;
        }
        if len >= 2 && key == "info" {
            self.program.info = unquote(&tokens[1], true);
            return;
        }
        if len >= 2 && key == "preview" {
            match tokens[1].as_str() {
                "enabled" | "true" | "linear" | "selection" => self.program.preview = true,
                "disabled" | "false" => self.program.preview = false,
                _ => {}
            }
            return;
        }
        if len >= 2 && key == "maxlen" {
            if let Ok(value) = tokens[1].parse::<i64>() {
                self.program.max_len = Some(value as f64);
            }
            return;
        }
        if len >= 2 && key == "mergeclips" {
            if let Ok(value) = tokens[1].parse::<i32>() {
                self.program.merge_clips = value;
            }
            return;
        }
        if len >= 2 && key == "restoresplits" {
            if let Ok(value) = tokens[1].parse::<i64>() {
                self.program.restore_splits = value != 0;
            }
            return;
        }
        if len >= 2 && key == "author" {
            self.program.author = unquote(&tokens[1], true);
            return;
        }
        if len >= 2 && key == "release" {
            self.program.release = unquote(&tokens[1], true);
            return;
        }
        if len >= 2 && key == "copyright" {
            self.program.copyright = unquote(&tokens[1], true);
            return;
        }
        if len >= 2 && key == "manpage" {
            self.program.manpage = unquote(&tokens[1], false);
            return;
        }
        if len >= 2 && key == "helpfile" {
            self.program.helpfile = unquote(&tokens[1], false);
            return;
        }
        if len >= 2 && key == "debugbutton" {
            if tokens[1] == "disabled" || tokens[1] == "false" {
                self.program.debug_button = false;
            }
            return;
        }

        if len >= 3 && key == "control" {
            if len == 3 && tokens[1] == "text" {
                self.program.controls.push(NyControl {
                    var: tokens[1].clone(),
                    name: String::new(),
                    ty: NyControlType::Text,
                    label: unquote(&tokens[2], true),
                    low: 0.0,
                    high: 0.0,
                    default_value: String::new(),
                    choices: Vec::new(),
                    file_wildcards: Vec::new(),
                });
                return;
            }

            if len < 5 {
                return;
            }

            let var = tokens[1].clone();
            let name = unquote(&tokens[2], true);
            let label = tokens[4].clone();
            let raw_default = tokens.get(5).cloned().unwrap_or_default();
            let mut default_value = raw_default.clone();
            let value = compatible_to_double(&raw_default);
            if raw_default.starts_with('(') || raw_default.starts_with('"') {
                default_value = unquote(&raw_default, true);
            }

            let mut control = NyControl {
                var,
                name,
                ty: NyControlType::Float,
                label: String::new(),
                low: 0.0,
                high: 0.0,
                default_value: String::new(),
                choices: Vec::new(),
                file_wildcards: Vec::new(),
            };

            match tokens[3].as_str() {
                "string" => {
                    control.ty = NyControlType::String;
                    control.label = unquote(&label, true);
                    control.default_value = default_value;
                }
                "choice" => {
                    control.ty = NyControlType::Choice;
                    control.choices = parse_choices(&label);
                    control.label.clear();
                    control.default_value = format_number(value, 14);
                    control.low = 0.0;
                    control.high = control.choices.len().saturating_sub(1) as f64;
                }
                "file" => {
                    control.ty = NyControlType::File;
                    control.file_wildcards = tokens
                        .get(6)
                        .map(|text| parse_file_types(text))
                        .unwrap_or_default();
                    control.label = unquote(&label, true);
                    control.default_value = default_value;
                }
                ty => {
                    control.label = unquote(&label, true);
                    if len < 8 {
                        return;
                    }
                    control.ty = match ty {
                        "float" | "real" => NyControlType::Float,
                        "int" => NyControlType::Int,
                        "float-text" => NyControlType::FloatText,
                        "int-text" => NyControlType::IntText,
                        "time" => NyControlType::Time,
                        _ => return,
                    };

                    let low_string = unquote(&tokens[6], true);
                    let high_string = unquote(&tokens[7], true);
                    control.low = if low_string.eq_ignore_ascii_case("nil") {
                        match control.ty {
                            NyControlType::IntText => i32::MIN as f64,
                            NyControlType::FloatText => -(f32::MAX as f64),
                            NyControlType::Time => 0.0,
                            _ => compatible_to_double(&low_string),
                        }
                    } else {
                        compatible_to_double(&low_string)
                    };
                    control.high = if high_string.eq_ignore_ascii_case("nil") {
                        match control.ty {
                            NyControlType::IntText => i32::MAX as f64,
                            NyControlType::FloatText | NyControlType::Time => f32::MAX as f64,
                            _ => compatible_to_double(&high_string),
                        }
                    } else {
                        compatible_to_double(&high_string)
                    };

                    if control.high < control.low {
                        control.high = control.low;
                    }
                    let value = value.clamp(control.low, control.high);
                    control.default_value = if matches!(
                        control.ty,
                        NyControlType::Int | NyControlType::IntText
                    ) {
                        (value as i32).to_string()
                    } else {
                        format_number(value, 14)
                    };
                }
            }
            self.program.controls.push(control);
            return;
        }
    }
}

fn parse_header_tokens(tokenizer: &mut Tokenizer, line: &str, eof: bool, first: bool) -> bool {
    tokenizer.tokenize(line, eof, usize::from(first), 0)
}

/// Parses a Nyquist plug-in program, including its magic-comment header.
///
/// Unsupported header versions and programs without the required
/// `nyquist plug-in` marker are returned as errors.
pub fn parse(text: &str) -> Result<NyProgram, String> {
    let mut parser = ProgramParser::new();
    let mut lines = text.lines().peekable();

    while let Some(line) = lines.next() {
        let chars: Vec<char> = line.chars().collect();
        let is_magic = chars.len() > 1 && (chars[0] == ';' || chars[0] == '$');

        if is_magic {
            let is_control_continuation =
                chars[0] == '$' || line.starts_with(";control");
            let mut tokenizer = Tokenizer::default();
            let mut current = line.to_owned();
            let mut first = true;
            let mut done;

            loop {
                let at_eof = lines.peek().is_none();
                done = parse_header_tokens(
                    &mut tokenizer,
                    &current,
                    !is_control_continuation || at_eof,
                    first,
                );
                first = false;
                if done || !is_control_continuation {
                    break;
                }
                match lines.next() {
                    Some(next_line) => current = next_line.to_owned(),
                    None => break,
                }
            }

            parser.parse_header_line(&tokenizer.tokens);
            if let Some(error) = parser.error.take() {
                return Err(error);
            }

            let blank_lines = tokenizer_line_count(&tokenizer, &current);
            // Header lines are removed from the executable text but retained
            // as blank lines so code line numbers remain aligned.
            for _ in 0..blank_lines {
                parser.program.code.push('\n');
            }
        } else {
            if !parser.found_code_type && !line.is_empty() {
                if line.starts_with('(') || line.starts_with("#|") {
                    parser.program.is_sal = false;
                    parser.found_code_type = true;
                } else if line.to_ascii_uppercase().contains("RETURN") {
                    parser.program.is_sal = true;
                    parser.found_code_type = true;
                }
            }
            parser.program.code.push_str(line);
            parser.program.code.push('\n');
        }
    }

    if !parser.found_plugin_marker {
        return Err("Ill-formed Nyquist plug-in header".to_owned());
    }

    // Keep this state calculation explicit: type tool with a subtype is
    // represented by the subtype's NyType, as it is in NyquistBase.
    let _ = (parser.found_type_line, parser.is_tool_menu);
    Ok(parser.program)
}

// The header loop consumes continuation lines, so track their count directly
// from the source spelling held by the tokenizer input is not possible after
// tokenization. This helper returns the number represented by the final
// accumulated token string; the caller overrides it below using line counts.
fn tokenizer_line_count(_tokenizer: &Tokenizer, _last_line: &str) -> usize {
    1
}

fn escape_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn find_value<'a>(values: &'a [(String, String)], var: &str) -> Option<&'a str> {
    values
        .iter()
        .find(|(key, _)| key == var)
        .map(|(_, value)| value.as_str())
}

fn choice_index(control: &NyControl, value: &str) -> Option<usize> {
    if let Ok(index) = value.parse::<usize>() {
        if index < control.choices.len() {
            return Some(index);
        }
    }
    control
        .choices
        .iter()
        .position(|(symbol, label)| symbol == value || label == value)
}

/// Builds the control assignment prelude NyquistBase sends to Nyquist.
///
/// Missing values use their parsed header defaults. String and file values
/// are escaped in the same manner as NyquistBase's Lisp string assignments.
pub fn bindings(prog: &NyProgram, values: &[(String, String)]) -> String {
    let mut result = String::new();

    for control in &prog.controls {
        if matches!(control.ty, NyControlType::Text) {
            continue;
        }
        let value = find_value(values, &control.var).unwrap_or("");

        let fallback;
        let value = if value.is_empty() && find_value(values, &control.var).is_none() {
            fallback = default_for_control(control, prog.version);
            fallback.as_str()
        } else {
            value
        };

        match control.ty {
            NyControlType::Float | NyControlType::FloatText | NyControlType::Time => {
                let number = compatible_to_double(value);
                result.push_str(&format!(
                    "(setf {} {})\n",
                    control.var,
                    internat_to_string(number, 14)
                ));
            }
            NyControlType::Int | NyControlType::IntText => {
                let number = compatible_to_double(value) as i32;
                result.push_str(&format!("(setf {} {})\n", control.var, number));
            }
            NyControlType::Choice => {
                // NyquistBase always passes the choice index: (setf VAR %d).
                // Unknown automation values leave the control at its default.
                let index = choice_index(control, value)
                    .or_else(|| choice_index(control, &default_for_control(control, prog.version)))
                    .unwrap_or(0);
                result.push_str(&format!("(setf {} {})\n", control.var, index));
            }
            NyControlType::String | NyControlType::File => {
                result.push_str(&format!(
                    "(setf {} \"{}\")\n",
                    control.var,
                    escape_string(value)
                ));
            }
            NyControlType::Text => {}
        }
    }

    result
}

/// Returns each control's default value in header order.
///
/// For version 4 choice controls, the default is returned as its Lisp symbol;
/// earlier versions return the numeric choice index.
pub fn default_values(prog: &NyProgram) -> Vec<(String, String)> {
    prog.controls
        .iter()
        .filter(|control| control.ty != NyControlType::Text)
        .map(|control| {
            (
                control.var.clone(),
                default_for_control(control, prog.version),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_process_controls() {
        let source = r#";nyquist plug-in
;version 4
;type process
;name "Example"
;control amount "Amount" float "Amount" 1.5 0 10
;control count "Count" int "Count" 3 0 20
;control mode "Mode" choice "One, Two" 1
;control title "Title" string "Title" "hello"
(mult *track* amount)
"#;
        let program = parse(source).expect("valid plug-in");
        assert_eq!(program.kind, NyType::Process);
        assert_eq!(program.name, "Example");
        assert_eq!(program.controls.len(), 4);
        assert_eq!(program.controls[0].default_value, "1.5");
        assert_eq!(program.controls[1].default_value, "3");
        assert_eq!(program.controls[2].choices[1], ("Two".into(), "Two".into()));
        assert_eq!(program.controls[3].default_value, "hello");
    }

    #[test]
    fn parses_version_four_dollar_headers_and_choice_symbols() {
        let source = r#"$nyquist plug-in
$version 4
$type generate
$name "Dollar header"
$control shape "Shape" choice (("sin" (_ "Sine wave"))) 0
(return 0)
"#;
        let program = parse(source).expect("valid plug-in");
        assert_eq!(program.version, 4);
        assert_eq!(program.kind, NyType::Generate);
        assert_eq!(
            program.controls[0].choices,
            vec![("sin".to_owned(), "Sine wave".to_owned())]
        );
        assert_eq!(program.controls[0].default_value, "0");
    }

    #[test]
    fn builds_bindings_and_escapes_strings() {
        let source = r#";nyquist plug-in
;version 4
;type process
;control amount "Amount" float "Amount" 1.5 0 10
;control mode "Mode" choice ("fast" (_ "Fast")) 0
;control title "Title" string "Title" "default"
(mult *track* amount)
"#;
        let program = parse(source).expect("valid plug-in");
        let output = bindings(
            &program,
            &[
                ("amount".into(), "2.25".into()),
                ("mode".into(), "fast".into()),
                ("title".into(), "say \"hi\" \\".into()),
            ],
        );
        assert_eq!(
            output,
            "(setf amount 2.25000000000000)\n(setf mode 0)\n(setf title \"say \\\"hi\\\" \\\\\")\n"
        );
    }

    #[test]
    fn old_choice_bindings_use_indices() {
        let source = r#";nyquist plug-in
;version 3
;type process
;control mode "Mode" choice "First, Second" 1
(mult *track* 1)
"#;
        let program = parse(source).expect("valid plug-in");
        assert_eq!(
            bindings(&program, &[("mode".into(), "Second".into())]),
            "(setf mode 1)\n"
        );
    }

    #[test]
    fn tokenizer_handles_quoted_continuations_and_escapes() {
        let mut tokenizer = Tokenizer::default();
        assert!(!tokenizer.tokenize(";control text \"first", false, 1, 0));
        assert!(tokenizer.tokenize("second\"", true, 0, 0));
        assert_eq!(tokenizer.tokens, vec!["control", "text", "\"first\nsecond\""]);
    }

    #[test]
    fn rejects_missing_plugin_marker_and_unsupported_version() {
        assert!(parse(";type process\n(return 0)\n").is_err());
        assert!(parse(";nyquist plug-in\n;version 9\n").is_err());
    }
}
