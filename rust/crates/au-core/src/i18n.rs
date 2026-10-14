//! Gettext catalog loading and runtime translation support.
//!
//! Supports GNU PO and MO catalogs, contextual messages, plural forms, and
//! switching the active catalog at runtime.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

#[derive(Clone, Debug)]
struct Message {
    translations: Vec<String>,
    is_plural: bool,
}

/// A parsed gettext catalog.
#[derive(Clone, Debug)]
pub struct Catalog {
    messages: HashMap<String, Message>,
    language: String,
    nplurals: usize,
    plural_expression: Option<PluralExpr>,
}

impl Default for Catalog {
    fn default() -> Self {
        Self {
            messages: HashMap::new(),
            language: String::new(),
            nplurals: 2,
            plural_expression: None,
        }
    }
}

impl Catalog {
    /// Parses a GNU gettext PO catalog.
    ///
    /// Fuzzy and obsolete entries are skipped. The header entry is used to
    /// read `Language` and `Plural-Forms`.
    pub fn parse_po(text: &str) -> Result<Catalog, String> {
        let mut parser = PoParser::default();

        for (line_index, line) in text.lines().enumerate() {
            parser
                .parse_line(line)
                .map_err(|error| format!("PO line {}: {error}", line_index + 1))?;
        }
        parser.finish();

        let mut catalog = Catalog::default();
        let mut header = String::new();

        for raw in parser.entries {
            let Some(msgid) = raw.msgid else {
                continue;
            };

            if msgid.is_empty() && raw.context.is_none() {
                if let Some(value) = raw.translations.get(&0) {
                    header = value.clone();
                }
                continue;
            }

            if raw.fuzzy {
                continue;
            }

            let key = make_key(raw.context.as_deref(), &msgid);
            let max_index = raw.translations.keys().copied().max().unwrap_or(0);
            let mut translations = vec![String::new(); max_index + 1];
            for (index, value) in raw.translations {
                translations[index] = value;
            }

            catalog.messages.insert(
                key,
                Message {
                    translations,
                    is_plural: raw.plural.is_some(),
                },
            );
        }

        catalog.read_header(&header)?;
        Ok(catalog)
    }

    /// Parses a GNU gettext MO catalog in either byte order.
    pub fn parse_mo(bytes: &[u8]) -> Result<Catalog, String> {
        if bytes.len() < 28 {
            return Err("MO file is shorter than its header".into());
        }

        let little_endian = match bytes.get(0..4) {
            Some([0xde, 0x12, 0x04, 0x95]) => true,
            Some([0x95, 0x04, 0x12, 0xde]) => false,
            _ => return Err("invalid MO magic number".into()),
        };

        let read_u32 = |offset: usize| -> Result<u32, String> {
            let data = bytes
                .get(offset..offset + 4)
                .ok_or_else(|| "truncated MO header".to_string())?;
            let array: [u8; 4] = data.try_into().map_err(|_| "truncated MO header")?;
            Ok(if little_endian {
                u32::from_le_bytes(array)
            } else {
                u32::from_be_bytes(array)
            })
        };

        let count = read_u32(8)? as usize;
        let originals_offset = read_u32(12)? as usize;
        let translations_offset = read_u32(16)? as usize;
        let table_bytes = count
            .checked_mul(8)
            .ok_or_else(|| "MO string table is too large".to_string())?;

        if originals_offset
            .checked_add(table_bytes)
            .filter(|end| *end <= bytes.len())
            .is_none()
            || translations_offset
                .checked_add(table_bytes)
                .filter(|end| *end <= bytes.len())
                .is_none()
        {
            return Err("MO string table is outside the file".into());
        }

        let read_string = |table_offset: usize, index: usize| -> Result<&[u8], String> {
            let descriptor = table_offset + index * 8;
            let length = read_u32(descriptor)? as usize;
            let offset = read_u32(descriptor + 4)? as usize;
            let end = offset
                .checked_add(length)
                .ok_or_else(|| "MO string length overflow".to_string())?;
            bytes
                .get(offset..end)
                .ok_or_else(|| "MO string is outside the file".to_string())
        };

        let mut catalog = Catalog::default();
        let mut header = String::new();

        for index in 0..count {
            let original = read_string(originals_offset, index)?;
            let translated = read_string(translations_offset, index)?;
            let original_parts: Vec<&[u8]> = original.split(|byte| *byte == 0).collect();
            let translated_parts: Vec<&[u8]> = translated.split(|byte| *byte == 0).collect();

            let first_original = original_parts.first().copied().unwrap_or_default();
            let source = String::from_utf8_lossy(first_original).into_owned();

            if source.is_empty() && original_parts.len() == 1 {
                header = String::from_utf8_lossy(
                    translated_parts.first().copied().unwrap_or_default(),
                )
                .into_owned();
                continue;
            }

            let (context, msgid) = split_context(&source);
            let key = make_key(context.as_deref(), &msgid);
            let translations = translated_parts
                .iter()
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect();

            catalog.messages.insert(
                key,
                Message {
                    translations,
                    is_plural: original_parts.len() > 1,
                },
            );
        }

        catalog.read_header(&header)?;
        Ok(catalog)
    }

    /// Returns the translation for a context-free message, if translated.
    pub fn get(&self, msgid: &str) -> Option<&str> {
        self.messages
            .get(msgid)
            .and_then(|message| message.translations.first())
            .filter(|translation| !translation.is_empty())
            .map(String::as_str)
    }

    /// Index for UI labels that drop Audacity's `&` accelerators: stripped msgid -> stripped translation.
    fn ui_index(&self) -> HashMap<String, String> {
        let mut index = HashMap::new();
        // Accelerated msgids (menu items, "&Edit") take priority over plain ones ("Edit" toolbar).
        let mut entries: Vec<_> = self.messages.iter().collect();
        entries.sort_by_key(|(key, _)| (!key.contains('&'), (*key).clone()));
        for (key, message) in entries {
            if key.contains('\u{4}') || message.is_plural {
                continue;
            }
            let Some(translation) = message.translations.first().filter(|t| !t.is_empty()) else { continue };
            let stripped = strip_menu_accel(key);
            let value = strip_menu_accel(translation);
            for variant in [stripped.clone(), stripped.replace("...", "\u{2026}"), stripped.replace('\u{2026}', "...")] {
                index.entry(variant).or_insert_with(|| value.clone());
            }
        }
        index
    }

    /// Returns the translation for a contextual message, if translated.
    pub fn pgettext(&self, context: &str, msgid: &str) -> Option<&str> {
        let key = make_key(Some(context), msgid);
        self.messages
            .get(&key)
            .and_then(|message| message.translations.first())
            .filter(|translation| !translation.is_empty())
            .map(String::as_str)
    }

    /// Returns the appropriate translated singular or plural form, if present.
    ///
    /// The catalog's `Plural-Forms` expression selects the form for `n`.
    pub fn ngettext(&self, msgid: &str, _plural: &str, n: u64) -> Option<&str> {
        let message = self.messages.get(msgid)?;

        if !message.is_plural {
            if n != 1 {
                return None;
            }
            return message
                .translations
                .first()
                .filter(|translation| !translation.is_empty())
                .map(String::as_str);
        }

        let index = self.plural_index(n);
        message
            .translations
            .get(index)
            .filter(|translation| !translation.is_empty())
            .map(String::as_str)
    }

    /// Returns the language declared in the catalog header, if any.
    pub fn language(&self) -> &str {
        &self.language
    }

    /// Returns the number of non-header messages in the catalog.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    fn plural_index(&self, n: u64) -> usize {
        let value = self
            .plural_expression
            .as_ref()
            .map(|expression| expression.evaluate(n.min(i64::MAX as u64) as i64))
            .unwrap_or_else(|| i64::from(n != 1));
        let index = if value < 0 { 0 } else { value as usize };
        index.min(self.nplurals.saturating_sub(1))
    }

    fn read_header(&mut self, header: &str) -> Result<(), String> {
        let mut plural_forms = None;

        for line in header.lines() {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };

            if name.trim().eq_ignore_ascii_case("Language") {
                self.language = value.trim().to_string();
            } else if name.trim().eq_ignore_ascii_case("Plural-Forms") {
                plural_forms = Some(value.trim().to_string());
            }
        }

        if let Some(forms) = plural_forms {
            for part in forms.split(';') {
                let Some((name, value)) = part.split_once('=') else {
                    continue;
                };
                let name = name.trim();
                let value = value.trim();

                if name == "nplurals" {
                    let count = value
                        .parse::<usize>()
                        .map_err(|_| format!("invalid nplurals value: {value}"))?;
                    if count == 0 {
                        return Err("nplurals must be greater than zero".into());
                    }
                    self.nplurals = count;
                } else if name == "plural" {
                    self.plural_expression = Some(
                        PluralParser::parse(value)
                            .map_err(|error| format!("invalid plural expression: {error}"))?,
                    );
                }
            }
        }

        Ok(())
    }
}

fn make_key(context: Option<&str>, msgid: &str) -> String {
    match context {
        Some(context) => format!("{context}\u{4}{msgid}"),
        None => msgid.to_string(),
    }
}

fn split_context(source: &str) -> (Option<String>, String) {
    match source.split_once('\u{4}') {
        Some((context, msgid)) => (Some(context.to_string()), msgid.to_string()),
        None => (None, source.to_string()),
    }
}

#[derive(Clone, Debug)]
enum PoField {
    Context,
    Id,
    Plural,
    Translation(usize),
}

#[derive(Default)]
struct RawPoEntry {
    context: Option<String>,
    msgid: Option<String>,
    plural: Option<String>,
    translations: HashMap<usize, String>,
    fuzzy: bool,
}

impl RawPoEntry {
    fn has_content(&self) -> bool {
        self.context.is_some()
            || self.msgid.is_some()
            || self.plural.is_some()
            || !self.translations.is_empty()
    }
}

#[derive(Default)]
struct PoParser {
    current: Option<RawPoEntry>,
    active: Option<PoField>,
    pending_fuzzy: bool,
    entries: Vec<RawPoEntry>,
}

impl PoParser {
    fn ensure_current(&mut self) -> &mut RawPoEntry {
        self.current.get_or_insert_with(|| RawPoEntry {
            fuzzy: std::mem::take(&mut self.pending_fuzzy),
            ..RawPoEntry::default()
        })
    }

    fn finish_current(&mut self) {
        self.active = None;
        if let Some(entry) = self.current.take() {
            if entry.has_content() {
                self.entries.push(entry);
            }
        }
    }

    fn finish(&mut self) {
        self.finish_current();
    }

    fn parse_line(&mut self, line: &str) -> Result<(), String> {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            self.finish_current();
            self.pending_fuzzy = false;
            return Ok(());
        }

        if trimmed.starts_with("#~") {
            return Ok(());
        }

        if trimmed.starts_with('#') {
            if self.current.as_ref().is_some_and(RawPoEntry::has_content) {
                self.finish_current();
            }
            if let Some(flags) = trimmed.strip_prefix("#,") {
                if flags.split(',').any(|flag| flag.trim() == "fuzzy") {
                    self.pending_fuzzy = true;
                }
            }
            return Ok(());
        }

        if trimmed.starts_with('"') {
            let value = parse_po_quoted(trimmed)?;
            let field = self
                .active
                .clone()
                .ok_or_else(|| "string continuation without a field".to_string())?;
            let entry = self.ensure_current();
            match field {
                PoField::Context => entry.context.get_or_insert_with(String::new).push_str(&value),
                PoField::Id => entry.msgid.get_or_insert_with(String::new).push_str(&value),
                PoField::Plural => entry.plural.get_or_insert_with(String::new).push_str(&value),
                PoField::Translation(index) => entry
                    .translations
                    .entry(index)
                    .or_default()
                    .push_str(&value),
            }
            return Ok(());
        }

        let (directive, quoted) = trimmed
            .split_once(char::is_whitespace)
            .ok_or_else(|| format!("expected a PO directive, found {trimmed:?}"))?;
        let value = parse_po_quoted(quoted.trim_start())?;

        if directive == "msgid" {
            if self
                .current
                .as_ref()
                .is_some_and(|entry| entry.msgid.is_some())
            {
                self.finish_current();
            }
            let entry = self.ensure_current();
            entry.msgid = Some(value);
            self.active = Some(PoField::Id);
            return Ok(());
        }

        let (field, translation_index) = match directive {
            "msgctxt" => (PoField::Context, None),
            "msgid_plural" => (PoField::Plural, None),
            "msgstr" => (PoField::Translation(0), Some(0)),
            value if value.starts_with("msgstr[") && value.ends_with(']') => {
                let index_text = &value[7..value.len() - 1];
                let index = index_text
                    .parse::<usize>()
                    .map_err(|_| format!("invalid plural translation index: {index_text}"))?;
                (PoField::Translation(index), Some(index))
            }
            _ => return Err(format!("unknown PO directive: {directive}")),
        };

        let entry = self.ensure_current();
        match field {
            PoField::Context => entry.context = Some(value),
            PoField::Plural => entry.plural = Some(value),
            PoField::Translation(index) => {
                entry.translations.insert(index, value);
            }
            PoField::Id => unreachable!(),
        }
        self.active = translation_index
            .map(PoField::Translation)
            .or(Some(field));
        Ok(())
    }
}

fn parse_po_quoted(input: &str) -> Result<String, String> {
    let bytes = input.as_bytes();
    if bytes.first() != Some(&b'"') {
        return Err("expected a quoted PO string".into());
    }

    let mut output = Vec::new();
    let mut index = 1;
    let mut closed = false;

    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index += 1;
                closed = true;
                break;
            }
            b'\\' => {
                index += 1;
                let escaped = *bytes
                    .get(index)
                    .ok_or_else(|| "incomplete escape sequence".to_string())?;
                index += 1;
                match escaped {
                    b'n' => output.push(b'\n'),
                    b't' => output.push(b'\t'),
                    b'r' => output.push(b'\r'),
                    b'b' => output.push(8),
                    b'f' => output.push(12),
                    b'v' => output.push(11),
                    b'a' => output.push(7),
                    b'\\' => output.push(b'\\'),
                    b'"' => output.push(b'"'),
                    b'\'' => output.push(b'\''),
                    b'?' => output.push(b'?'),
                    b'0'..=b'7' => {
                        let mut value = (escaped - b'0') as u32;
                        let mut digits = 1;
                        while digits < 3 && index < bytes.len() && (b'0'..=b'7').contains(&bytes[index]) {
                            value = value * 8 + (bytes[index] - b'0') as u32;
                            index += 1;
                            digits += 1;
                        }
                        output.push(value as u8);
                    }
                    b'x' => {
                        let mut value = 0u32;
                        let mut digits = 0usize;
                        while index < bytes.len() {
                            let digit = match bytes[index] {
                                b'0'..=b'9' => (bytes[index] - b'0') as u32,
                                b'a'..=b'f' => (bytes[index] - b'a' + 10) as u32,
                                b'A'..=b'F' => (bytes[index] - b'A' + 10) as u32,
                                _ => break,
                            };
                            value = value.wrapping_mul(16).wrapping_add(digit);
                            index += 1;
                            digits += 1;
                        }
                        if digits == 0 {
                            return Err("hex escape has no digits".into());
                        }
                        output.push(value as u8);
                    }
                    other => return Err(format!("unknown escape sequence \\{}", other as char)),
                }
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }

    if !closed {
        return Err("unterminated quoted PO string".into());
    }
    if !bytes[index..].iter().all(u8::is_ascii_whitespace) {
        return Err("unexpected characters after quoted PO string".into());
    }

    String::from_utf8(output).map_err(|_| "PO string is not valid UTF-8".into())
}

#[derive(Clone, Debug)]
enum PluralExpr {
    Number(i64),
    N,
    Unary(char, Box<PluralExpr>),
    Binary(String, Box<PluralExpr>, Box<PluralExpr>),
    Conditional(Box<PluralExpr>, Box<PluralExpr>, Box<PluralExpr>),
}

impl PluralExpr {
    fn evaluate(&self, n: i64) -> i64 {
        match self {
            PluralExpr::Number(value) => *value,
            PluralExpr::N => n,
            PluralExpr::Unary('-', expression) => expression.evaluate(n).wrapping_neg(),
            PluralExpr::Unary('+', expression) => expression.evaluate(n),
            PluralExpr::Unary('!', expression) => i64::from(expression.evaluate(n) == 0),
            PluralExpr::Unary(_, _) => 0,
            PluralExpr::Conditional(condition, yes, no) => {
                if condition.evaluate(n) != 0 {
                    yes.evaluate(n)
                } else {
                    no.evaluate(n)
                }
            }
            PluralExpr::Binary(operator, left, right) => {
                let lhs = left.evaluate(n);
                match operator.as_str() {
                    "&&" if lhs == 0 => 0,
                    "||" if lhs != 0 => 1,
                    _ => {
                        let rhs = right.evaluate(n);
                        match operator.as_str() {
                            "&&" => i64::from(rhs != 0),
                            "||" => i64::from(rhs != 0),
                            "*" => lhs.wrapping_mul(rhs),
                            "/" => lhs.checked_div(rhs).unwrap_or(0),
                            "%" => lhs.checked_rem(rhs).unwrap_or(0),
                            "+" => lhs.wrapping_add(rhs),
                            "-" => lhs.wrapping_sub(rhs),
                            "<" => i64::from(lhs < rhs),
                            ">" => i64::from(lhs > rhs),
                            "<=" => i64::from(lhs <= rhs),
                            ">=" => i64::from(lhs >= rhs),
                            "==" => i64::from(lhs == rhs),
                            "!=" => i64::from(lhs != rhs),
                            _ => 0,
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum PluralToken {
    Number(i64),
    N,
    Operator(String),
    LeftParen,
    RightParen,
    Question,
    Colon,
    End,
}

struct PluralParser {
    tokens: Vec<PluralToken>,
    position: usize,
}

impl PluralParser {
    fn parse(input: &str) -> Result<PluralExpr, String> {
        let tokens = tokenize_plural(input)?;
        let mut parser = Self {
            tokens,
            position: 0,
        };
        let expression = parser.parse_conditional()?;
        if parser.peek() != &PluralToken::End {
            return Err("unexpected token after plural expression".into());
        }
        Ok(expression)
    }

    fn peek(&self) -> &PluralToken {
        &self.tokens[self.position]
    }

    fn advance(&mut self) -> PluralToken {
        let token = self.tokens[self.position].clone();
        if token != PluralToken::End {
            self.position += 1;
        }
        token
    }

    fn consume_operator(&mut self, operator: &str) -> bool {
        if self.peek() == &PluralToken::Operator(operator.to_string()) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn parse_conditional(&mut self) -> Result<PluralExpr, String> {
        let condition = self.parse_or()?;
        if self.peek() != &PluralToken::Question {
            return Ok(condition);
        }
        self.advance();
        let yes = self.parse_conditional()?;
        if self.peek() != &PluralToken::Colon {
            return Err("expected ':' in conditional expression".into());
        }
        self.advance();
        let no = self.parse_conditional()?;
        Ok(PluralExpr::Conditional(
            Box::new(condition),
            Box::new(yes),
            Box::new(no),
        ))
    }

    fn parse_or(&mut self) -> Result<PluralExpr, String> {
        self.parse_left_associative(Self::parse_and, &["||"])
    }

    fn parse_and(&mut self) -> Result<PluralExpr, String> {
        self.parse_left_associative(Self::parse_equality, &["&&"])
    }

    fn parse_equality(&mut self) -> Result<PluralExpr, String> {
        self.parse_left_associative(Self::parse_relational, &["==", "!="])
    }

    fn parse_relational(&mut self) -> Result<PluralExpr, String> {
        self.parse_left_associative(Self::parse_additive, &["<=", ">=", "<", ">"])
    }

    fn parse_additive(&mut self) -> Result<PluralExpr, String> {
        self.parse_left_associative(Self::parse_multiplicative, &["+", "-"])
    }

    fn parse_multiplicative(&mut self) -> Result<PluralExpr, String> {
        self.parse_left_associative(Self::parse_unary, &["*", "/", "%"])
    }

    fn parse_left_associative(
        &mut self,
        next: fn(&mut Self) -> Result<PluralExpr, String>,
        operators: &[&str],
    ) -> Result<PluralExpr, String> {
        let mut expression = next(self)?;
        loop {
            let operator = operators
                .iter()
                .find(|operator| self.peek() == &PluralToken::Operator((**operator).to_string()))
                .copied();
            let Some(operator) = operator else {
                break;
            };
            self.advance();
            let right = next(self)?;
            expression = PluralExpr::Binary(
                operator.to_string(),
                Box::new(expression),
                Box::new(right),
            );
        }
        Ok(expression)
    }

    fn parse_unary(&mut self) -> Result<PluralExpr, String> {
        for operator in ["!", "-", "+"] {
            if self.consume_operator(operator) {
                let expression = self.parse_unary()?;
                return Ok(PluralExpr::Unary(
                    operator.chars().next().unwrap_or('+'),
                    Box::new(expression),
                ));
            }
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<PluralExpr, String> {
        match self.advance() {
            PluralToken::Number(value) => Ok(PluralExpr::Number(value)),
            PluralToken::N => Ok(PluralExpr::N),
            PluralToken::LeftParen => {
                let expression = self.parse_conditional()?;
                if self.peek() != &PluralToken::RightParen {
                    return Err("expected ')'".into());
                }
                self.advance();
                Ok(expression)
            }
            _ => Err("expected a number, 'n', or parenthesized expression".into()),
        }
    }
}

fn tokenize_plural(input: &str) -> Result<Vec<PluralToken>, String> {
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut tokens = Vec::new();

    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }

        if bytes[index].is_ascii_digit() {
            let start = index;
            let radix;
            if bytes[index] == b'0'
                && index + 1 < bytes.len()
                && matches!(bytes[index + 1], b'x' | b'X')
            {
                index += 2;
                let digits_start = index;
                while index < bytes.len() && bytes[index].is_ascii_hexdigit() {
                    index += 1;
                }
                if index == digits_start {
                    return Err("hexadecimal integer has no digits".into());
                }
                radix = 16;
                let number = i64::from_str_radix(
                    std::str::from_utf8(&bytes[digits_start..index])
                        .map_err(|_| "invalid integer")?,
                    radix,
                )
                .map_err(|_| "integer is out of range")?;
                tokens.push(PluralToken::Number(number));
            } else {
                while index < bytes.len() && bytes[index].is_ascii_digit() {
                    index += 1;
                }
                let number = std::str::from_utf8(&bytes[start..index])
                    .map_err(|_| "invalid integer")?
                    .parse::<i64>()
                    .map_err(|_| "integer is out of range")?;
                tokens.push(PluralToken::Number(number));
            }
            continue;
        }

        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            let identifier = std::str::from_utf8(&bytes[start..index])
                .map_err(|_| "invalid identifier")?;
            if identifier == "n" {
                tokens.push(PluralToken::N);
            } else {
                return Err(format!("unknown identifier: {identifier}"));
            }
            continue;
        }

        match bytes[index] {
            b'(' => {
                tokens.push(PluralToken::LeftParen);
                index += 1;
            }
            b')' => {
                tokens.push(PluralToken::RightParen);
                index += 1;
            }
            b'?' => {
                tokens.push(PluralToken::Question);
                index += 1;
            }
            b':' => {
                tokens.push(PluralToken::Colon);
                index += 1;
            }
            _ => {
                let remaining = &input[index..];
                let operator = ["&&", "||", "<=", ">=", "==", "!=", "*", "/", "%", "+", "-", "<", ">", "!"]
                    .iter()
                    .find(|operator| remaining.starts_with(**operator))
                    .copied()
                    .ok_or_else(|| format!("unexpected character {:?}", bytes[index] as char))?;
                tokens.push(PluralToken::Operator(operator.to_string()));
                index += operator.len();
            }
        }
    }

    tokens.push(PluralToken::End);
    Ok(tokens)
}

struct Runtime {
    language: String,
    catalog: Option<Catalog>,
    generation: u64,
    /// UI lookup: msgid with menu accelerators removed and "..." -> "…" variants.
    ui_index: HashMap<String, String>,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            language: "en".to_string(),
            catalog: None,
            ui_index: HashMap::new(),
            generation: 0,
        }
    }
}

fn runtime() -> &'static RwLock<Runtime> {
    static RUNTIME: OnceLock<RwLock<Runtime>> = OnceLock::new();
    RUNTIME.get_or_init(|| RwLock::new(Runtime::default()))
}

/// Loads and activates a language catalog.
///
/// Each search directory is checked for `<dir>/<lang>/LC_MESSAGES/audacity.mo`,
/// `<dir>/<lang>.mo`, and `<dir>/<lang>.po`. The exact language is tried before
/// its base language. `"System"` resolves from the standard locale environment
/// variables; `"en"` and the empty string clear the active catalog.
pub fn set_language(lang: &str, search_dirs: &[PathBuf]) -> Result<(), String> {
    let requested = if lang.eq_ignore_ascii_case("system") {
        resolve_system_language()
    } else {
        lang.trim().to_string()
    };

    let normalized = normalize_language(&requested);
    if normalized.is_empty() || normalized.eq_ignore_ascii_case("en") {
        let mut state = runtime().write().unwrap_or_else(|error| error.into_inner());
        state.language = if normalized.is_empty() {
            "en".to_string()
        } else {
            normalized
        };
        state.catalog = None;
        state.ui_index.clear();
        state.generation = state.generation.wrapping_add(1);
        return Ok(());
    }

    let candidates = language_candidates(&normalized);
    let mut last_error = None;

    for candidate in &candidates {
        for directory in search_dirs {
            let paths: [PathBuf; 4] = [
                directory
                    .join(candidate)
                    .join("LC_MESSAGES")
                    .join("audacity.mo"),
                directory.join(candidate).join("LC_MESSAGES").join("audacity.po"),
                directory.join(format!("{candidate}.mo")),
                directory.join(format!("{candidate}.po")),
            ];

            for path in paths {
                if !path.is_file() {
                    continue;
                }
                match load_catalog(&path) {
                    Ok(catalog) => {
                        let mut state =
                            runtime().write().unwrap_or_else(|error| error.into_inner());
                        state.language = candidate.clone();
                        state.ui_index = catalog.ui_index();
                        state.catalog = Some(catalog);
                        state.generation = state.generation.wrapping_add(1);
                        return Ok(());
                    }
                    Err(error) => last_error = Some(format!("{}: {error}", path.display())),
                }
            }
        }
    }

    match last_error {
        Some(error) => Err(format!("could not load language catalog: {error}")),
        None => Err(format!(
            "no Audacity catalog found for language {normalized:?}"
        )),
    }
}

/// Returns the language currently selected by the runtime.
pub fn current_language() -> String {
    runtime()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .language
        .clone()
}

/// Translates a message, falling back to the message id.
pub fn tr(msgid: &str) -> String {
    let state = runtime()
        .read()
        .unwrap_or_else(|error| error.into_inner());
    state
        .catalog
        .as_ref()
        .and_then(|catalog| catalog.get(msgid))
        .unwrap_or(msgid)
        .to_string()
}

/// Translates a UI label via Audacity's msgids with `&` accelerators removed (accelerated
/// menu msgids win over plain ones), then the exact msgid (menus/buttons here show no accelerators); falls back to `label`.
pub fn tr_ui(label: &str) -> String {
    let state = runtime().read().unwrap_or_else(|error| error.into_inner());
    let Some(catalog) = state.catalog.as_ref() else { return label.to_string() };
    if let Some(t) = state.ui_index.get(label) {
        return t.clone();
    }
    catalog.get(label).map(strip_menu_accel).unwrap_or_else(|| label.to_string())
}

/// Translates a contextual message, falling back to the message id.
pub fn trc(context: &str, msgid: &str) -> String {
    let state = runtime()
        .read()
        .unwrap_or_else(|error| error.into_inner());
    state
        .catalog
        .as_ref()
        .and_then(|catalog| catalog.pgettext(context, msgid))
        .unwrap_or(msgid)
        .to_string()
}

/// Translates a singular or plural message, falling back to the supplied
/// English singular or plural text.
pub fn trn(msgid: &str, plural: &str, n: u64) -> String {
    let state = runtime()
        .read()
        .unwrap_or_else(|error| error.into_inner());
    state
        .catalog
        .as_ref()
        .and_then(|catalog| catalog.ngettext(msgid, plural, n))
        .unwrap_or(if n == 1 { msgid } else { plural })
        .to_string()
}

/// Returns the runtime generation counter.
///
/// The counter advances on each successful call to [`set_language`].
pub fn generation() -> u64 {
    runtime()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .generation
}

/// Finds installed catalog languages in the supplied directories.
///
/// The returned language names are sorted and de-duplicated.
pub fn available_languages(search_dirs: &[PathBuf]) -> Vec<String> {
    let mut languages = std::collections::BTreeSet::new();

    for directory in search_dirs {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let language = entry.file_name().to_string_lossy().into_owned();
                if path
                    .join("LC_MESSAGES")
                    .join("audacity.mo")
                    .is_file()
                    || path.join("audacity.mo").is_file()
                    || path.join("audacity.po").is_file()
                {
                    languages.insert(language);
                }
            } else if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
                if extension == "mo" || extension == "po" {
                    if let Some(stem) = path.file_stem().and_then(|value| value.to_str()) {
                        languages.insert(stem.to_string());
                    }
                }
            }
        }
    }

    languages.into_iter().collect()
}

/// Returns the standard locale search directories.
///
/// `AU_LOCALE_DIR` may contain multiple platform-separated paths.
pub fn default_search_dirs() -> Vec<PathBuf> {
    let mut directories = Vec::new();

    if let Some(value) = env::var_os("AU_LOCALE_DIR") {
        directories.extend(env::split_paths(&value));
    }

    if let Ok(executable) = env::current_exe() {
        if let Some(parent) = executable.parent() {
            directories.push(parent.join("../share/locale"));
            directories.push(parent.join("locale"));
        }
    }

    directories.push(PathBuf::from("./locale"));
    directories.push(PathBuf::from("/usr/share/locale"));
    directories.push(PathBuf::from("/usr/local/share/locale"));
    directories
}

/// Removes single menu-accelerator ampersands; doubled ampersands become one.
pub fn strip_menu_accel(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(character) = chars.next() {
        if character == '&' {
            if chars.peek() == Some(&'&') {
                chars.next();
                output.push('&');
            }
        } else {
            output.push(character);
        }
    }

    output
}

fn normalize_language(language: &str) -> String {
    let value = language
        .split([':', ';'])
        .next()
        .unwrap_or_default()
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .trim();

    if value.eq_ignore_ascii_case("C") || value.eq_ignore_ascii_case("POSIX") {
        String::new()
    } else {
        value.to_string()
    }
}

fn resolve_system_language() -> String {
    for variable in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = env::var(variable) {
            let normalized = normalize_language(&value);
            if !normalized.is_empty() {
                return normalized;
            }
        }
    }
    "en".to_string()
}

fn language_candidates(language: &str) -> Vec<String> {
    let mut candidates = vec![language.to_string()];
    if let Some((base, _)) = language.split_once(['_', '-']) {
        if !base.is_empty() && !candidates.iter().any(|value| value == base) {
            candidates.push(base.to_string());
        }
    }
    candidates
}

fn load_catalog(path: &Path) -> Result<Catalog, String> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("mo") => {
            let bytes = fs::read(path).map_err(|error| error.to_string())?;
            Catalog::parse_mo(&bytes)
        }
        Some("po") => {
            let text = fs::read_to_string(path).map_err(|error| error.to_string())?;
            Catalog::parse_po(&text)
        }
        _ => Err("unsupported catalog file extension".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_po_context_plural_escapes_and_fuzzy_entries() {
        let po = r#"
msgid ""
msgstr ""
"Language: pl\n"
"Plural-Forms: nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<12 || n%100>14) ? 1 : 2);\n"

msgctxt "menu"
msgid "Open"
msgstr "Otwórz"

msgid "file"
msgid_plural "files"
msgstr[0] "plik"
msgstr[1] "pliki"
msgstr[2] "plików"

msgid "escaped"
msgstr "line\nquote: \" slash: \\ octal: \101 hex: \x42"

#, fuzzy
msgid "unfinished"
msgstr "niegotowe"
"#;

        let catalog = Catalog::parse_po(po).unwrap();
        assert_eq!(catalog.language(), "pl");
        assert_eq!(catalog.pgettext("menu", "Open"), Some("Otwórz"));
        assert_eq!(catalog.ngettext("file", "files", 1), Some("plik"));
        assert_eq!(catalog.ngettext("file", "files", 2), Some("pliki"));
        assert_eq!(catalog.ngettext("file", "files", 5), Some("plików"));
        assert_eq!(
            catalog.get("escaped"),
            Some("line\nquote: \" slash: \\ octal: A hex: B")
        );
        assert_eq!(catalog.get("unfinished"), None);
    }

    fn make_mo(entries: &[(&str, &str)]) -> Vec<u8> {
        let count = entries.len();
        let originals_table = 28usize;
        let translations_table = originals_table + count * 8;
        let mut data_offset = translations_table + count * 8;
        let mut originals = Vec::new();
        let mut translations = Vec::new();

        for (original, _) in entries {
            originals.push((original.as_bytes().len(), data_offset));
            data_offset += original.as_bytes().len() + 1;
        }
        for (_, translation) in entries {
            translations.push((translation.as_bytes().len(), data_offset));
            data_offset += translation.as_bytes().len() + 1;
        }

        let mut bytes = vec![0u8; translations_table + count * 8];
        bytes[0..4].copy_from_slice(&0x9504_12deu32.to_le_bytes());
        bytes[4..8].copy_from_slice(&0u32.to_le_bytes());
        bytes[8..12].copy_from_slice(&(count as u32).to_le_bytes());
        bytes[12..16].copy_from_slice(&(originals_table as u32).to_le_bytes());
        bytes[16..20].copy_from_slice(&(translations_table as u32).to_le_bytes());
        bytes[20..24].copy_from_slice(&0u32.to_le_bytes());
        bytes[24..28].copy_from_slice(&0u32.to_le_bytes());

        for (index, (length, offset)) in originals.iter().enumerate() {
            let descriptor = originals_table + index * 8;
            bytes[descriptor..descriptor + 4].copy_from_slice(&(*length as u32).to_le_bytes());
            bytes[descriptor + 4..descriptor + 8]
                .copy_from_slice(&(*offset as u32).to_le_bytes());
        }
        for (index, (length, offset)) in translations.iter().enumerate() {
            let descriptor = translations_table + index * 8;
            bytes[descriptor..descriptor + 4].copy_from_slice(&(*length as u32).to_le_bytes());
            bytes[descriptor + 4..descriptor + 8]
                .copy_from_slice(&(*offset as u32).to_le_bytes());
        }

        for (original, _) in entries {
            bytes.extend_from_slice(original.as_bytes());
            bytes.push(0);
        }
        for (_, translation) in entries {
            bytes.extend_from_slice(translation.as_bytes());
            bytes.push(0);
        }

        bytes
    }

    #[test]
    fn parses_mo_catalog() {
        let bytes = make_mo(&[
            ("", "Language: de\nPlural-Forms: nplurals=2; plural=(n != 1);\n"),
            ("Hello", "Hallo"),
        ]);
        let catalog = Catalog::parse_mo(&bytes).unwrap();
        assert_eq!(catalog.language(), "de");
        assert_eq!(catalog.get("Hello"), Some("Hallo"));
    }

    #[test]
    fn language_switch_generation_and_fallback() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = env::temp_dir().join(format!("au-i18n-{}-{unique}", std::process::id()));
        let catalog_dir = root.join("zz_test").join("LC_MESSAGES");
        fs::create_dir_all(&catalog_dir).unwrap();
        fs::write(
            catalog_dir.join("audacity.po"),
            r#"
msgid ""
msgstr ""
"Language: zz_test\n"

msgid "Hello"
msgstr "Hallo"
"#,
        )
        .unwrap();

        let before = generation();
        set_language("zz_test", std::slice::from_ref(&root)).unwrap();
        assert_eq!(generation(), before.wrapping_add(1));
        assert_eq!(current_language(), "zz_test");
        assert_eq!(tr("Hello"), "Hallo");
        assert_eq!(tr("Not translated"), "Not translated");

        let after_load = generation();
        set_language("en", &[]).unwrap();
        assert_eq!(generation(), after_load.wrapping_add(1));
        assert_eq!(tr("Hello"), "Hello");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn strips_menu_accelerators() {
        assert_eq!(strip_menu_accel("&Open && Save..."), "Open & Save...");
    }
}
