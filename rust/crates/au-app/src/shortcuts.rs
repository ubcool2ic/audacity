//! Audacity 3.7.3 keyboard shortcuts, key combinations, and shortcut persistence.

use std::collections::BTreeMap;
use std::fmt;

/// Returns the shortcuts reported by Audacity 3.7.3 for the Standard set.
///
/// Rows, order, and duplicate rows match Audacity's Standard menu shortcut
/// report.
pub fn default_shortcuts() -> &'static [(&'static str, &'static str)] {
    &[
        ("SelectTool", "F1"),
        ("EnvelopeTool", "F2"),
        ("DrawTool", "F3"),
        ("MultiTool", "F6"),
        ("New", "Ctrl+N"),
        ("Open", "Ctrl+O"),
        ("Save", "Ctrl+S"),
        ("Export", "Ctrl+Shift+E"),
        ("ImportAudio", "Ctrl+Shift+I"),
        ("Close", "Ctrl+W"),
        ("Exit", "Ctrl+Q"),
        ("Undo", "Ctrl+Z"),
        ("Redo", "Ctrl+Shift+Z"),
        ("Cut", "Ctrl+X"),
        ("Delete", "Ctrl+K"),
        ("Copy", "Ctrl+C"),
        ("Paste", "Ctrl+V"),
        ("Duplicate", "Ctrl+D"),
        ("SplitCut", "Ctrl+Alt+X"),
        ("SplitDelete", "Ctrl+Alt+K"),
        ("Silence", "Ctrl+L"),
        ("Trim", "Ctrl+T"),
        ("Split", "Ctrl+I"),
        ("Join", "Ctrl+J"),
        ("ChangePitchAndSpeed", "Ctrl+Shift+P"),
        ("PitchDown", "Alt+Down"),
        ("PitchUp", "Alt+Up"),
        ("RenameClip", "Ctrl+F2"),
        ("AddLabel", "Ctrl+B"),
        ("AddLabelPlaying", "Ctrl+M"),
        ("SplitCutLabels", "Alt+Shift+X"),
        ("SplitDeleteLabels", "Alt+Shift+K"),
        ("CopyLabels", "Alt+Shift+C"),
        ("DisjoinLabels", "Alt+Shift+J"),
        ("Preferences", "Ctrl+P"),
        ("SelectAll", "Ctrl+A"),
        ("SelAllTracks", "Ctrl+Shift+K"),
        ("SelSyncLockTracks", "Ctrl+Shift+Y"),
        ("SetLeftSelection", "["),
        ("SetRightSelection", "]"),
        ("SelTrackStartToCursor", "Shift+J"),
        ("SelCursorToTrackEnd", "Shift+K"),
        ("ToggleSpectralSelection", "Q"),
        ("SelPrevClip", "Alt+,"),
        ("SelNextClip", "Alt+."),
        ("ZeroCross", "Z"),
        ("ZoomIn", "Ctrl+1"),
        ("ZoomNormal", "Ctrl+2"),
        ("ZoomOut", "Ctrl+3"),
        ("ZoomSel", "Ctrl+E"),
        ("ZoomToggle", "Shift+Z"),
        ("FitInWindow", "Ctrl+F"),
        ("FitV", "Ctrl+Shift+F"),
        ("CollapseAllTracks", "Ctrl+Shift+C"),
        ("ExpandAllTracks", "Ctrl+Shift+X"),
        ("DefaultPlayStop", "Space"),
        ("PlayStopSelect", "X"),
        ("OncePlayStop", "Shift+Space"),
        ("Pause", "P"),
        ("Record1stChoice", "R"),
        ("Record2ndChoice", "Shift+R"),
        ("TimerRecord", "Shift+T"),
        ("PunchAndRoll", "Shift+D"),
        ("Pause", "P"),
        ("CursTrackStart", "J"),
        ("CursTrackEnd", "K"),
        ("CursProjectStart", "Home"),
        ("CursProjectEnd", "End"),
        ("TogglePlayRegion", "L"),
        ("ClearPlayRegion", "Shift+Alt+L"),
        ("SetPlayRegionToSelection", "Shift+L"),
        ("MuteAllTracks", "Ctrl+U"),
        ("UnmuteAllTracks", "Ctrl+Shift+U"),
        ("MuteTracks", "Ctrl+Alt+U"),
        ("UnmuteTracks", "Ctrl+Alt+Shift+U"),
        ("AddRealtimeEffects", "E"),
        ("RepeatLastEffect", "Ctrl+R"),
    ]
}

/// Returns the Audacity Full-set defaults whose command IDs are absent from
/// the Standard shortcut report.
pub fn full_set_extras() -> &'static [(&'static str, &'static str)] {
    &[
        ("SplitNew", "Ctrl+Alt+I"),
        ("Disjoin", "Ctrl+Alt+J"),
        ("PasteNewLabel", "Ctrl+Alt+V"),
        ("CutLabels", "Alt+X"),
        ("DeleteLabels", "Alt+K"),
        ("SilenceLabels", "Alt+L"),
        ("SplitLabels", "Alt+I"),
        ("JoinLabels", "Alt+J"),
        ("SelectNone", "Ctrl+Shift+A"),
        ("MixAndRenderToNewTrack", "Ctrl+Shift+M"),
        ("SkipSelStart", "Ctrl+["),
        ("SkipSelEnd", "Ctrl+]"),
        ("DeleteKey", "Backspace"),
        ("DeleteKey2", "Delete"),
        ("Compact", "Shift+A"),
        ("PrevWindow", "Alt+Shift+F6"),
        ("NextWindow", "Alt+F6"),
        ("PrevFrame", "Ctrl+Shift+F6"),
        ("NextFrame", "Ctrl+F6"),
        ("PrevTrack", "Up"),
        ("NextTrack", "Down"),
        ("FirstTrack", "Ctrl+Home"),
        ("LastTrack", "Ctrl+End"),
        ("ShiftUp", "Shift+Up"),
        ("ShiftDown", "Shift+Down"),
        ("Toggle", "Return"),
        ("SelStart", "Shift+Home"),
        ("SelEnd", "Shift+End"),
        ("SelExtLeft", "Shift+Left"),
        ("SelExtRight", "Shift+Right"),
        ("SelCntrLeft", "Ctrl+Shift+Right"),
        ("SelCntrRight", "Ctrl+Shift+Left"),
        ("CursorLeft", "Left"),
        ("CursorRight", "Right"),
        ("CursorShortJumpLeft", ","),
        ("CursorShortJumpRight", "."),
        ("CursorLongJumpLeft", "Shift+,"),
        ("CursorLongJumpRight", "Shift+."),
        ("SeekLeftShort", "Left"),
        ("SeekRightShort", "Right"),
        ("SeekLeftLong", "Shift+Left"),
        ("SeekRightLong", "Shift+Right"),
        ("TrackPan", "Shift+P"),
        ("TrackPanLeft", "Alt+Shift+Left"),
        ("TrackPanRight", "Alt+Shift+Right"),
        ("TrackGain", "Shift+G"),
        ("TrackGainInc", "Alt+Shift+Up"),
        ("TrackGainDec", "Alt+Shift+Down"),
        ("TrackMenu", "Shift+M"),
        ("TrackMute", "Shift+U"),
        ("TrackSolo", "Shift+S"),
        ("TrackClose", "Shift+C"),
        ("PlayOneSec", "1"),
        ("PrevTool", "A"),
        ("NextTool", "D"),
        ("KeyboardScrubBackwards", "U"),
        ("KeyboardScrubForwards", "I"),
        ("PlayToSelection", "B"),
        ("PlayBeforeSelectionStart", "Shift+F5"),
        ("PlayAfterSelectionStart", "Shift+F6"),
        ("PlayBeforeSelectionEnd", "Shift+F7"),
        ("PlayAfterSelectionEnd", "Shift+F8"),
        ("PlayBeforeAndAfterSelectionStart", "Ctrl+Shift+F5"),
        ("PlayBeforeAndAfterSelectionEnd", "Ctrl+Shift+F7"),
        ("PlayCutPreview", "C"),
        ("MoveToPrevLabel", "Alt+Left"),
        ("MoveToNextLabel", "Alt+Right"),
    ]
}

/// A normalized keyboard shortcut.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct KeyCombo {
    /// Whether the Control modifier is pressed.
    pub ctrl: bool,
    /// Whether the Alt modifier is pressed.
    pub alt: bool,
    /// Whether the Shift modifier is pressed.
    pub shift: bool,
    /// Whether the platform Command/Meta modifier is pressed.
    pub meta: bool,
    /// The normalized key name, such as `"Z"` or `"Space"`.
    pub key: String,
}

impl KeyCombo {
    /// Parses an Audacity-style normalized key string, case-insensitively.
    ///
    /// Recognized modifiers are `Ctrl`, `RawCtrl`, `Alt`, `Option`, `Shift`,
    /// `Cmd`, `Command`, and `Meta`. On macOS, `Ctrl`/`Cmd` represent Command
    /// and `RawCtrl` represents Control, as in Audacity's key-string
    /// normalization.
    pub fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().split('+').peekable();
        let mut combo = Self {
            ctrl: false,
            alt: false,
            shift: false,
            meta: false,
            key: String::new(),
        };

        while let Some(part) = parts.next() {
            let token = part.trim();
            if token.is_empty() {
                return None;
            }

            if parts.peek().is_none() {
                combo.key = normalize_key(token)?;
                break;
            }

            match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => {
                    #[cfg(target_os = "macos")]
                    {
                        combo.meta = true;
                    }
                    #[cfg(not(target_os = "macos"))]
                    {
                        combo.ctrl = true;
                    }
                }
                "rawctrl" | "xctrl" => combo.ctrl = true,
                "alt" | "option" => combo.alt = true,
                "shift" => combo.shift = true,
                "cmd" | "command" | "meta" => combo.meta = true,
                _ => return None,
            }
        }

        if combo.key.is_empty() {
            None
        } else {
            Some(combo)
        }
    }

    /// Formats the shortcut using Audacity's normalized modifier order.
    pub fn to_audacity_string(&self) -> String {
        let mut result = String::new();

        #[cfg(target_os = "macos")]
        {
            if self.ctrl {
                result.push_str("RawCtrl+");
            }
            if self.alt {
                result.push_str("Alt+");
            }
            if self.shift {
                result.push_str("Shift+");
            }
            if self.meta {
                result.push_str("Ctrl+");
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            if self.ctrl {
                result.push_str("Ctrl+");
            }
            if self.alt {
                result.push_str("Alt+");
            }
            if self.shift {
                result.push_str("Shift+");
            }
            if self.meta {
                result.push_str("Meta+");
            }
        }

        result.push_str(&self.key);
        result
    }
}

impl fmt::Display for KeyCombo {
    /// Formats the shortcut in Audacity's normalized form.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_audacity_string())
    }
}

fn normalize_key(key: &str) -> Option<String> {
    let lower = key.to_ascii_lowercase();

    let named = match lower.as_str() {
        "space" => Some("Space"),
        "home" => Some("Home"),
        "end" => Some("End"),
        "left" => Some("Left"),
        "right" => Some("Right"),
        "up" => Some("Up"),
        "down" => Some("Down"),
        "pageup" | "page-up" => Some("PageUp"),
        "pagedown" | "page-down" => Some("PageDown"),
        "delete" | "del" => Some("Delete"),
        "backspace" | "back" => Some("Backspace"),
        "return" | "enter" => Some("Return"),
        "escape" | "esc" => Some("Escape"),
        "tab" => Some("Tab"),
        "insert" | "ins" => Some("Insert"),
        _ => None,
    };
    if let Some(name) = named {
        return Some(name.to_string());
    }

    if key.len() == 1 {
        let ch = key.chars().next()?;
        if ch.is_ascii_alphanumeric() {
            return Some(ch.to_ascii_uppercase().to_string());
        }
        if matches!(
            ch,
            ',' | '.' | '[' | ']' | '(' | ')' | '{' | '}' | '/' | '\\' | ';' | '\''
                | '`' | '=' | '-' | '_'
        ) {
            return Some(ch.to_string());
        }
        return None;
    }

    if lower.starts_with('f') && lower.len() > 1 {
        let number = lower[1..].parse::<u8>().ok()?;
        if (1..=24).contains(&number) {
            return Some(format!("F{number}"));
        }
    }

    None
}

/// A map of command IDs to their current shortcuts.
///
/// Iteration is in command-ID order and includes known commands with no
/// shortcut assigned.
#[derive(Clone, Debug, Default)]
pub struct KeyMap {
    bindings: BTreeMap<String, Option<KeyCombo>>,
}

impl KeyMap {
    /// Constructs the Audacity Standard shortcut set.
    pub fn standard() -> Self {
        let mut map = Self::with_known_ids();
        for &(id, key) in default_shortcuts() {
            map.bindings.insert(id.to_string(), KeyCombo::parse(key));
        }
        map
    }

    /// Constructs the Audacity Full shortcut set.
    pub fn full() -> Self {
        let mut map = Self::standard();
        for &(id, key) in full_set_extras() {
            map.bindings.insert(id.to_string(), KeyCombo::parse(key));
        }
        map
    }

    /// Gets a command's assigned shortcut, or `None` if it is unbound or
    /// unknown.
    pub fn get(&self, id: &str) -> Option<&KeyCombo> {
        self.bindings.get(id).and_then(Option::as_ref)
    }

    /// Assigns or clears a command's shortcut.
    ///
    /// An ID not already in the map is added.
    pub fn set(&mut self, id: &str, key: Option<KeyCombo>) {
        self.bindings.insert(id.to_string(), key);
    }

    /// Returns the first command, in ID order, assigned to `key`.
    pub fn command_for(&self, key: &KeyCombo) -> Option<&str> {
        self.bindings
            .iter()
            .find_map(|(id, assigned)| (assigned.as_ref() == Some(key)).then_some(id.as_str()))
    }

    /// Returns all other command IDs assigned to `key`, in ID order.
    pub fn conflicts(&self, key: &KeyCombo, except_id: &str) -> Vec<String> {
        self.bindings
            .iter()
            .filter_map(|(id, assigned)| {
                (id != except_id && assigned.as_ref() == Some(key)).then(|| id.clone())
            })
            .collect()
    }

    /// Replaces all bindings with the Standard defaults.
    pub fn reset_standard(&mut self) {
        *self = Self::standard();
    }

    /// Replaces all bindings with the Full defaults.
    pub fn reset_full(&mut self) {
        *self = Self::full();
    }

    /// Iterates over every command and its optional shortcut, sorted by ID.
    pub fn iter(&self) -> impl Iterator<Item = (&str, Option<&KeyCombo>)> {
        self.bindings
            .iter()
            .map(|(id, key)| (id.as_str(), key.as_ref()))
    }

    /// Writes an Audacity keyboard-shortcut XML document.
    ///
    /// Commands are emitted in the order supplied by `all_ids`; IDs absent
    /// from the map are emitted with an empty key.
    pub fn to_xml(&self, all_ids: &[&str]) -> String {
        let mut xml = String::from(
            "<?xml version=\"1.0\" standalone=\"no\" ?>\n\
<audacitykeyboard audacityversion=\"3.7.3\">\n",
        );
        for id in all_ids {
            let key = self
                .get(id)
                .map(KeyCombo::to_audacity_string)
                .unwrap_or_default();
            xml.push_str("  <command name=\"");
            xml.push_str(&escape_xml_attribute(id));
            xml.push_str("\" key=\"");
            xml.push_str(&escape_xml_attribute(&key));
            xml.push_str("\"/>\n");
        }
        xml.push_str("</audacitykeyboard>\n");
        xml
    }

    /// Imports an Audacity keyboard-shortcut XML document.
    ///
    /// Unknown command IDs are ignored, as they are by Audacity's command
    /// manager. The return value counts known commands whose assigned key
    /// changed.
    pub fn import_xml(&mut self, text: &str) -> Result<usize, String> {
        let tags = parse_xml_tags(text)?;
        let mut changed = 0;

        for (tag, attributes) in tags {
            if tag != "command" {
                continue;
            }

            let Some(name) = attributes.get("name") else {
                continue;
            };
            if !self.bindings.contains_key(name) {
                continue;
            }

            let key_string = attributes.get("key").map(String::as_str).unwrap_or("");
            let key = if key_string.is_empty() {
                None
            } else {
                Some(
                    KeyCombo::parse(key_string)
                        .ok_or_else(|| au_core::i18n::tr_format("Invalid shortcut key: %s", &[&key_string]))?,
                )
            };

            let current = self.bindings.get(name).and_then(Option::as_ref);
            if current != key.as_ref() {
                changed += 1;
                self.bindings.insert(name.clone(), key);
            }
        }

        Ok(changed)
    }

    /// Saves only values differing from the Standard defaults under
    /// `/NewKeys/<command-id>`.
    pub fn save_to_prefs(&self, prefs: &mut au_core::prefs::Prefs) {
        use au_core::prefs::PrefValue;

        let standard = Self::standard();
        for (id, key) in &self.bindings {
            if standard.get(id) == key.as_ref() {
                prefs.remove(&format!("/NewKeys/{id}"));
            } else {
                let value = key
                    .as_ref()
                    .map(KeyCombo::to_audacity_string)
                    .unwrap_or_default();
                prefs.set(
                    &format!("/NewKeys/{id}"),
                    PrefValue::Str(value),
                );
            }
        }
    }

    /// Loads `/NewKeys/<command-id>` overrides on top of Standard defaults.
    pub fn load_from_prefs(prefs: &au_core::prefs::Prefs) -> Self {
        const MISSING: &str = "\u{0}AUDACITY_SHORTCUT_PREF_MISSING\u{0}";

        let mut map = Self::standard();
        let ids: Vec<String> = map.bindings.keys().cloned().collect();
        for id in ids {
            let path = format!("/NewKeys/{id}");
            let value = prefs.read_string_or(&path, MISSING);
            if value == MISSING {
                continue;
            }

            let key = if value.is_empty() {
                None
            } else {
                KeyCombo::parse(&value)
            };
            map.bindings.insert(id, key);
        }
        map
    }

    fn with_known_ids() -> Self {
        let mut bindings = BTreeMap::new();
        for &(id, _) in default_shortcuts().iter().chain(full_set_extras().iter()) {
            bindings.entry(id.to_string()).or_insert(None);
        }
        Self { bindings }
    }
}

fn escape_xml_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

fn decode_xml_entities(value: &str) -> Result<String, String> {
    let mut decoded = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(amp) = rest.find('&') {
        decoded.push_str(&rest[..amp]);
        let entity_start = amp + 1;
        let Some(end) = rest[entity_start..].find(';') else {
            return Err("unterminated XML entity".to_string());
        };
        let entity = &rest[entity_start..entity_start + end];
        let ch = match entity {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ if entity.starts_with("#x") || entity.starts_with("#X") => {
                let code = u32::from_str_radix(&entity[2..], 16)
                    .map_err(|_| au_core::i18n::tr_format("Invalid XML entity &%s;", &[&entity]))?;
                char::from_u32(code).ok_or_else(|| au_core::i18n::tr_format("Invalid XML entity &%s;", &[&entity]))?
            }
            _ if entity.starts_with('#') => {
                let code = entity[1..]
                    .parse::<u32>()
                    .map_err(|_| au_core::i18n::tr_format("Invalid XML entity &%s;", &[&entity]))?;
                char::from_u32(code).ok_or_else(|| au_core::i18n::tr_format("Invalid XML entity &%s;", &[&entity]))?
            }
            _ => return Err(au_core::i18n::tr_format("Unknown XML entity &%s;", &[&entity])),
        };
        decoded.push(ch);
        rest = &rest[entity_start + end + 1..];
    }

    decoded.push_str(rest);
    Ok(decoded)
}

fn parse_xml_tags(text: &str) -> Result<Vec<(String, BTreeMap<String, String>)>, String> {
    let mut tags = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = text[cursor..].find('<') {
        let start = cursor + relative_start;

        if text[start..].starts_with("<!--") {
            let Some(end) = text[start + 4..].find("-->") else {
                return Err("unterminated XML comment".to_string());
            };
            cursor = start + 4 + end + 3;
            continue;
        }
        if text[start..].starts_with("<![CDATA[") {
            let Some(end) = text[start + 9..].find("]]>") else {
                return Err("unterminated XML CDATA section".to_string());
            };
            cursor = start + 9 + end + 3;
            continue;
        }
        if text[start..].starts_with("<?") {
            let Some(end) = text[start + 2..].find("?>") else {
                return Err("unterminated XML processing instruction".to_string());
            };
            cursor = start + 2 + end + 2;
            continue;
        }

        let mut quote = None;
        let mut end = None;
        for (offset, ch) in text[start + 1..].char_indices() {
            if let Some(q) = quote {
                if ch == q {
                    quote = None;
                }
            } else if ch == '"' || ch == '\'' {
                quote = Some(ch);
            } else if ch == '>' {
                end = Some(start + 1 + offset);
                break;
            }
        }
        let Some(end) = end else {
            return Err("unterminated XML tag".to_string());
        };

        let body = text[start + 1..end].trim();
        cursor = end + 1;

        if body.is_empty() || body.starts_with('/') || body.starts_with('!') {
            continue;
        }

        let (tag, attributes) = parse_xml_start_tag(body)?;
        tags.push((tag, attributes));
    }

    Ok(tags)
}

fn parse_xml_start_tag(body: &str) -> Result<(String, BTreeMap<String, String>), String> {
    let body = body.trim_end_matches('/').trim_end();
    let mut index = 0;

    skip_ascii_whitespace(body, &mut index);
    let name_start = index;
    while index < body.len() && !body.as_bytes()[index].is_ascii_whitespace() {
        index += 1;
    }
    if name_start == index {
        return Err("empty XML tag name".to_string());
    }
    let tag = body[name_start..index].to_string();
    let mut attributes = BTreeMap::new();

    loop {
        skip_ascii_whitespace(body, &mut index);
        if index >= body.len() {
            break;
        }

        let attr_start = index;
        while index < body.len()
            && !body.as_bytes()[index].is_ascii_whitespace()
            && body.as_bytes()[index] != b'='
        {
            index += 1;
        }
        if attr_start == index {
            return Err("invalid XML attribute".to_string());
        }
        let attr = &body[attr_start..index];

        skip_ascii_whitespace(body, &mut index);
        if index >= body.len() || body.as_bytes()[index] != b'=' {
            return Err(au_core::i18n::tr_format("Missing '=' after XML attribute %s", &[&attr]));
        }
        index += 1;
        skip_ascii_whitespace(body, &mut index);

        if index >= body.len() {
            return Err(au_core::i18n::tr_format("Missing value for XML attribute %s", &[&attr]));
        }
        let quote = body.as_bytes()[index];
        if quote != b'"' && quote != b'\'' {
            return Err(au_core::i18n::tr_format("Unquoted XML attribute %s", &[&attr]));
        }
        index += 1;
        let value_start = index;
        while index < body.len() && body.as_bytes()[index] != quote {
            index += 1;
        }
        if index >= body.len() {
            return Err(au_core::i18n::tr_format("Unterminated XML attribute %s", &[&attr]));
        }
        let value = decode_xml_entities(&body[value_start..index])?;
        index += 1;
        attributes.insert(attr.to_string(), value);
    }

    Ok((tag, attributes))
}

fn skip_ascii_whitespace(text: &str, index: &mut usize) {
    while *index < text.len() && text.as_bytes()[*index].is_ascii_whitespace() {
        *index += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_shortcuts() {
        assert_eq!(
            KeyCombo::parse("ctrl+shift+z")
                .unwrap()
                .to_audacity_string(),
            "Ctrl+Shift+Z"
        );
        assert_eq!(
            KeyCombo::parse("Alt+PageDown")
                .unwrap()
                .to_audacity_string(),
            "Alt+PageDown"
        );
    }

    #[test]
    fn standard_shortcuts_and_conflicts() {
        let mut map = KeyMap::standard();
        let undo = KeyCombo::parse("Ctrl+Z").unwrap();
        assert_eq!(map.command_for(&undo), Some("Undo"));

        map.set("AnotherUndo", Some(undo.clone()));
        assert_eq!(map.conflicts(&undo, "Undo"), vec!["AnotherUndo"]);
        assert_eq!(KeyCombo::parse("Space").unwrap().key, "Space");
        assert_eq!(
            map.get("DefaultPlayStop"),
            Some(&KeyCombo::parse("Space").unwrap())
        );
    }

    #[test]
    fn xml_round_trip_and_hand_written_xml() {
        let ids = ["Undo", "DefaultPlayStop", "Unknown&Command"];
        let mut map = KeyMap::standard();
        let xml = map.to_xml(&ids);
        assert!(xml.contains("name=\"Unknown&amp;Command\""));

        let mut imported = KeyMap::standard();
        assert_eq!(imported.import_xml(&xml).unwrap(), 0);

        let hand_written = r#"
            <?xml version="1.0"?>
            <audacitykeyboard audacityversion="3.7.3">
              <command key="Ctrl+Shift+Z" name="Undo" />
              <command name="DefaultPlayStop" key=""/>
              <command name="NotAKnownCommand" key="Ctrl+A"/>
            </audacitykeyboard>
        "#;
        assert_eq!(map.import_xml(hand_written).unwrap(), 2);
        assert_eq!(
            map.get("Undo"),
            Some(&KeyCombo::parse("Ctrl+Shift+Z").unwrap())
        );
        assert_eq!(map.get("DefaultPlayStop"), None);
    }

    #[test]
    fn resets_and_iteration_are_sorted() {
        let mut map = KeyMap::standard();
        map.set("Undo", None);
        map.reset_standard();
        assert_eq!(
            map.get("Undo"),
            Some(&KeyCombo::parse("Ctrl+Z").unwrap())
        );

        let ids: Vec<_> = map.iter().map(|(id, _)| id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted);

        map.reset_full();
        assert_eq!(
            map.get("SplitNew"),
            Some(&KeyCombo::parse("Ctrl+Alt+I").unwrap())
        );
    }

    #[test]
    fn prefs_persistence_round_trip() {
        let mut prefs = au_core::prefs::Prefs::default();
        let mut map = KeyMap::standard();
        map.set("Undo", Some(KeyCombo::parse("Ctrl+Alt+Z").unwrap()));
        map.set("DefaultPlayStop", None);
        map.save_to_prefs(&mut prefs);

        let loaded = KeyMap::load_from_prefs(&prefs);
        assert_eq!(
            loaded.get("Undo"),
            Some(&KeyCombo::parse("Ctrl+Alt+Z").unwrap())
        );
        assert_eq!(loaded.get("DefaultPlayStop"), None);
        assert_eq!(
            loaded.get("Redo"),
            Some(&KeyCombo::parse("Ctrl+Shift+Z").unwrap())
        );
    }
}
