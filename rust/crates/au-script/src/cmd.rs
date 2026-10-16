//! Audacity-style batch command parsing and execution.
//!
//! This module implements the scripting command layer over an abstract
//! [`ScriptHost`]. The host owns project state and provides application-specific
//! operations such as importing audio and dispatching effects.

use au_core::ProjectState;

/// The script-visible time, frequency, and track selection.
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    /// Selection start time in seconds.
    pub t0: f64,
    /// Selection end time in seconds.
    pub t1: f64,
    /// Low frequency in hertz, or `-1.0` when undefined.
    pub f0: f64,
    /// High frequency in hertz, or `-1.0` when undefined.
    pub f1: f64,
    /// Selected tracks, in wave-track then label-track order.
    pub tracks: Vec<bool>,
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            t0: 0.0,
            t1: 0.0,
            f0: -1.0,
            f1: -1.0,
            tracks: Vec::new(),
        }
    }
}

/// Interface between scripting commands and an Audacity project.
pub trait ScriptHost {
    /// Returns mutable project state.
    fn state(&mut self) -> &mut ProjectState;
    /// Returns mutable script-visible selection.
    fn selection(&mut self) -> &mut Selection;
    /// Records an undo state with the given description.
    fn push_undo(&mut self, desc: &str);
    /// Applies an effect by its Audacity command identifier.
    fn apply_effect(
        &mut self,
        id: &str,
        params: &[(String, String)],
    ) -> Result<String, String>;
    /// Returns effect command identifiers and their parameter definitions.
    fn effect_ids(&self) -> Vec<(String, Vec<(String, String, String)>)>;
    /// Imports a file into the project.
    fn import(&mut self, path: &str) -> Result<(), String>;
    /// Exports the project to a file.
    fn export(&mut self, path: &str, channels: u32) -> Result<(), String>;
    /// Runs an application menu command, or returns `None` if it is unknown.
    fn menu_command(&mut self, id: &str) -> Option<Result<(), String>>;
    /// Displays a message to the user.
    fn message(&mut self, text: &str);
    /// Returns the focused track index (wave-track then label-track order), if any.
    fn focused_track(&self) -> Option<usize> {
        None
    }
    /// Moves track focus (SetTrackStatus Focused=1).
    fn set_focused_track(&mut self, _track: Option<usize>) {}
}

/// Parses an Audacity batch command line.
///
/// The command name ends at the first colon or whitespace. Parameter values
/// may be unquoted, double-quoted, or single-quoted; a backslash in a quoted
/// value escapes the following character.
pub fn parse_command(line: &str) -> (String, Vec<(String, String)>) {
    let chars: Vec<char> = line.trim().chars().collect();
    let mut name_end = 0;

    while name_end < chars.len() && chars[name_end] != ':' && !chars[name_end].is_whitespace() {
        name_end += 1;
    }

    let name: String = chars[..name_end].iter().collect();
    let mut pos = name_end;

    while pos < chars.len() && chars[pos].is_whitespace() {
        pos += 1;
    }
    if pos < chars.len() && chars[pos] == ':' {
        pos += 1;
    }

    let mut params = Vec::new();
    while pos < chars.len() {
        while pos < chars.len() && chars[pos].is_whitespace() {
            pos += 1;
        }
        if pos >= chars.len() {
            break;
        }

        let key_start = pos;
        while pos < chars.len() && chars[pos] != '=' && !chars[pos].is_whitespace() {
            pos += 1;
        }

        if pos >= chars.len() || chars[pos] != '=' {
            while pos < chars.len() && !chars[pos].is_whitespace() {
                pos += 1;
            }
            continue;
        }

        let key: String = chars[key_start..pos].iter().collect();
        pos += 1;

        let value = if pos < chars.len() && (chars[pos] == '"' || chars[pos] == '\'') {
            let quote = chars[pos];
            pos += 1;
            let mut value = String::new();

            while pos < chars.len() {
                let current = chars[pos];
                pos += 1;
                if current == quote {
                    break;
                }
                if current == '\\' && pos < chars.len() {
                    value.push(chars[pos]);
                    pos += 1;
                } else {
                    value.push(current);
                }
            }
            value
        } else {
            let value_start = pos;
            while pos < chars.len() && !chars[pos].is_whitespace() {
                pos += 1;
            }
            chars[value_start..pos].iter().collect()
        };

        params.push((key, value));
    }

    (name, params)
}

/// Executes one scripting command, returning its output text and success flag.
pub fn execute(host: &mut dyn ScriptHost, line: &str) -> (String, bool) {
    let (command, params) = parse_command(line);

    match command.as_str() {
        "Help" => execute_help(host, &params),
        "Message" => {
            let text = param(&params, "Text").unwrap_or("Some message");
            host.message(text);
            (text.to_owned(), true)
        }
        "GetInfo" => execute_get_info(host, &params),
        "Select" | "SelectTime" | "SelectFrequencies" | "SelectTracks" => {
            execute_select(host, &command, &params)
        }
        "SetLabel" => execute_set_label(host, &params),
        "AddLabel" => execute_add_label(host),
        "SetClip" => execute_set_clip(host, &params),
        "SetTrack" | "SetTrackStatus" | "SetTrackAudio" => {
            execute_set_track(host, &command, &params)
        }
        "Import2" => {
            let Some(path) = param(&params, "Filename") else {
                return ("Missing required parameter Filename.".to_owned(), false);
            };
            match host.import(path) {
                Ok(()) => (String::new(), true),
                Err(error) => (error, false),
            }
        }
        "Export2" => {
            let Some(path) = param(&params, "Filename") else {
                return ("Missing required parameter Filename.".to_owned(), false);
            };
            let channels = param(&params, "NumChannels")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(2);
            match host.export(path, channels) {
                Ok(()) => (String::new(), true),
                Err(error) => (error, false),
            }
        }
        _ => {
            if is_builtin_command(&command) {
                if let Some(result) = host.menu_command(&command) {
                    return match result {
                        Ok(()) => (String::new(), true),
                        Err(error) => (error, false),
                    };
                }
            }

            if let Some((id, definition)) = host
                .effect_ids()
                .into_iter()
                .find(|(id, _)| id == &command)
            {
                return execute_effect(host, &id, &definition, &params);
            }

            if let Some(result) = host.menu_command(&command) {
                return match result {
                    Ok(()) => (String::new(), true),
                    Err(error) => (error, false),
                };
            }

            (
                format!("Your batch command of {command} was not recognized."),
                false,
            )
        }
    }
}

/// Returns known scripting command identifiers, including host-provided effects.
pub fn command_ids(host: &dyn ScriptHost) -> Vec<String> {
    let mut ids: Vec<String> = builtin_ids()
        .iter()
        .map(|id| (*id).to_owned())
        .collect();
    ids.extend(menu_ids().iter().map(|id| (*id).to_owned()));
    ids.extend(host.effect_ids().into_iter().map(|(id, _)| id));
    ids.sort();
    ids.dedup();
    ids
}

fn execute_help(host: &dyn ScriptHost, params: &[(String, String)]) -> (String, bool) {
    let format = param(params, "Format").unwrap_or("JSON");
    if !valid_format(format) {
        return (format!("Unsupported Help format: {format}."), false);
    }

    let command = param(params, "Command").unwrap_or("Help");
    let json = if command == "*" || param(params, "Command").is_none() {
        let mut definitions = Vec::new();
        for id in builtin_ids() {
            definitions.push(command_definition(host, id));
        }
        for (id, _) in host.effect_ids() {
            definitions.push(effect_definition(host, &id));
        }
        format!("[{}]", definitions.join(","))
    } else if is_builtin_command(command) {
        command_definition(host, command)
    } else if host.effect_ids().iter().any(|(id, _)| id == command) {
        effect_definition(host, command)
    } else {
        return (format!("No help available for command {command}."), true);
    };

    (format_output(&json, format), true)
}

fn command_definition(host: &dyn ScriptHost, command: &str) -> String {
    let params: &[(&str, &str)] = match command {
        "Help" => &[("Command", "String"), ("Format", "String")],
        "Message" => &[("Text", "String")],
        "GetInfo" => &[("Type", "String"), ("Format", "String")],
        "Select" => &[
            ("Start", "Number"),
            ("End", "Number"),
            ("RelativeTo", "String"),
            ("High", "Number"),
            ("Low", "Number"),
            ("Track", "Number"),
            ("TrackCount", "Number"),
            ("Mode", "String"),
        ],
        "SelectTime" => &[
            ("Start", "Number"),
            ("End", "Number"),
            ("RelativeTo", "String"),
        ],
        "SelectFrequencies" => &[("High", "Number"), ("Low", "Number")],
        "SelectTracks" => &[
            ("Track", "Number"),
            ("TrackCount", "Number"),
            ("Mode", "String"),
        ],
        "SetClip" => &[("At", "Number"), ("Color", "String"), ("Start", "Number")],
        "SetLabel" => &[
            ("Label", "Number"),
            ("Text", "String"),
            ("Start", "Number"),
            ("End", "Number"),
            ("Selected", "Bool"),
        ],
        "SetTrack" | "SetTrackStatus" | "SetTrackAudio" => &[
            ("Track", "Number"),
            ("Name", "String"),
            ("Selected", "Bool"),
            ("Focused", "Bool"),
            ("Mute", "Bool"),
            ("Solo", "Bool"),
            ("Gain", "Number"),
            ("Volume", "Number"),
            ("Pan", "Number"),
        ],
        "Import2" => &[("Filename", "String")],
        "Export2" => &[("Filename", "String"), ("NumChannels", "Number")],
        _ if menu_ids().contains(&command) => &[],
        _ => &[],
    };

    let mut entries = Vec::new();
    for (name, ty) in params {
        entries.push(format!(
            "{{\"name\":{},\"type\":{}}}",
            json_string(name),
            json_string(ty)
        ));
    }
    let _ = host;
    format!(
        "{{\"id\":{},\"name\":{},\"params\":[{}]}}",
        json_string(command),
        json_string(command),
        entries.join(",")
    )
}

fn effect_definition(host: &dyn ScriptHost, command: &str) -> String {
    let Some((_, params)) = host.effect_ids().into_iter().find(|(id, _)| id == command) else {
        return command_definition(host, command);
    };
    let entries = params
        .iter()
        .map(|(key, ty, default)| {
            format!(
                "{{\"name\":{},\"type\":{},\"default\":{}}}",
                json_string(key),
                json_string(ty),
                json_string(default)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"id\":{},\"name\":{},\"params\":[{}]}}",
        json_string(command),
        json_string(command),
        entries
    )
}

fn execute_effect(
    host: &mut dyn ScriptHost,
    id: &str,
    definition: &[(String, String, String)],
    params: &[(String, String)],
) -> (String, bool) {
    let mut forwarded = Vec::new();
    for (key, value) in params {
        if key == "Use_Preset" {
            if value == "<Current Settings>" || value == "<Factory Defaults>" {
                continue;
            } else {
                return (format!("Unknown preset {value} for command {id}."), false);
            }
        }

        let Some((_, ty, _)) = definition.iter().find(|(name, _, _)| name == key) else {
            return (format!("Unknown parameter {key} for command {id}."), false);
        };
        if !valid_effect_value(ty, value) {
            return (
                format!("Invalid value for parameter {key}: {value}."),
                false,
            );
        }
        forwarded.push((key.clone(), value.clone()));
    }

    match host.apply_effect(id, &forwarded) {
        Ok(output) => {
            host.push_undo(id);
            (output, true)
        }
        Err(error) => (error, false),
    }
}

fn valid_effect_value(ty: &str, value: &str) -> bool {
    match ty.to_ascii_lowercase().as_str() {
        "bool" | "boolean" => parse_bool(value).is_some(),
        "number" | "float" | "double" | "int" | "integer" => {
            value.parse::<f64>().is_ok()
        }
        "enum" | "choice" | "string" | "text" => true,
        _ => true,
    }
}

fn execute_select(
    host: &mut dyn ScriptHost,
    command: &str,
    params: &[(String, String)],
) -> (String, bool) {
    if command == "Select" || command == "SelectTime" {
        if param(params, "Start").is_some() || param(params, "End").is_some() {
            let t0 = param(params, "Start")
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(0.0);
            let t1 = param(params, "End")
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(0.0);
            let relative = param(params, "RelativeTo").unwrap_or("ProjectStart");
            let (project_end, old_t0, old_t1) = {
                let state = host.state();
                let end = state
                    .tracks
                    .iter()
                    .map(|track| {
                        let rate = track.sample_rate();
                        if rate == 0 {
                            0.0
                        } else {
                            track.len() as f64 / rate as f64
                        }
                    })
                    .fold(0.0_f64, f64::max);
                let selection = host.selection();
                (end, selection.t0, selection.t1)
            };
            let (start, end) = match relative {
                "ProjectStart" => (t0, t1),
                "Project" => (t0, project_end + t1),
                "ProjectEnd" => (project_end - t0, project_end - t1),
                "SelectionStart" => (old_t0 + t0, old_t0 + t1),
                "Selection" => (old_t0 + t0, old_t1 + t1),
                "SelectionEnd" => (old_t1 - t0, old_t1 - t1),
                _ => return (format!("Invalid RelativeTo value: {relative}."), false),
            };
            // SelectedRegion::setTimes orders the pair (e.g. ProjectEnd Start=0 End=1).
            let (start, end) = if start > end { (end, start) } else { (start, end) };
            let selection = host.selection();
            selection.t0 = start;
            selection.t1 = end;
        }
    }

    if command == "Select" || command == "SelectFrequencies" {
        if param(params, "High").is_some() || param(params, "Low").is_some() {
            let high = param(params, "High")
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(0.0);
            let low = param(params, "Low")
                .and_then(|value| value.parse::<f64>().ok())
                .unwrap_or(0.0);
            let selection = host.selection();
            selection.f0 = low;
            selection.f1 = high;
        }
    }

    if command == "Select" || command == "SelectTracks" {
        let first = param(params, "Track")
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(0.0);
        let count = param(params, "TrackCount")
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(1.0);
        let mode = param(params, "Mode").unwrap_or("Set");
        if !matches!(mode, "Set" | "Add" | "Remove") {
            return (format!("Invalid Mode value: {mode}."), false);
        }

        let track_count = {
            let state = host.state();
            state.tracks.len() + state.labels.len()
        };
        let selection = host.selection();
        selection.tracks.resize(track_count, false);
        let last = first + count;
        for index in 0..track_count {
            let selected = first <= index as f64 && (index as f64) < last;
            match mode {
                "Set" => selection.tracks[index] = selected,
                "Add" if selected => selection.tracks[index] = true,
                "Remove" if selected => selection.tracks[index] = false,
                _ => {}
            }
        }
    }

    (String::new(), true)
}

fn execute_set_label(
    host: &mut dyn ScriptHost,
    params: &[(String, String)],
) -> (String, bool) {
    let index = param(params, "Label")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);

    let (label_track_index, label_index) = {
        let state = host.state();
        let mut remaining = index;
        let mut found = None;
        for (track_index, track) in state.labels.iter().enumerate() {
            if remaining < track.labels.len() {
                found = Some((track_index, remaining));
                break;
            }
            remaining -= track.labels.len();
        }
        let Some(found) = found else {
            return ("LabelIndex was invalid.".to_owned(), false);
        };
        found
    };

    {
        let state = host.state();
        let label = &mut state.labels[label_track_index].labels[label_index];
        if let Some(text) = param(params, "Text") {
            label.text = text.to_owned();
        }
        if let Some(start) = param(params, "Start").and_then(|value| value.parse::<f64>().ok()) {
            label.start = start;
        }
        if let Some(end) = param(params, "End").and_then(|value| value.parse::<f64>().ok()) {
            label.end = end;
        }
        if label.start > label.end {
            std::mem::swap(&mut label.start, &mut label.end);
        }
        state.labels[label_track_index]
            .labels
            .sort_by(|a, b| a.start.total_cmp(&b.start));
    }

    if let Some(selected) = param(params, "Selected").and_then(parse_bool) {
        if selected {
            let (start, end, n_wave, n_labels) = {
                let state = host.state();
                let label = &state.labels[label_track_index].labels[label_index];
                (label.start, label.end, state.tracks.len(), state.labels.len())
            };
            let selection = host.selection();
            selection.t0 = start;
            selection.t1 = end;
            selection.tracks.resize(n_wave + n_labels, false);
            selection.tracks[n_wave + label_track_index] = true;
        }
    }

    host.push_undo("Edited Label");
    (String::new(), true)
}

fn execute_set_clip(
    host: &mut dyn ScriptHost,
    params: &[(String, String)],
) -> (String, bool) {
    let at = param(params, "At").and_then(|value| value.parse::<f64>().ok());
    let new_start = param(params, "Start").and_then(|value| value.parse::<f64>().ok());

    if new_start.is_none() {
        return (String::new(), true);
    }
    let new_start = new_start.unwrap();

    let selected = host.selection().tracks.clone();
    let state = host.state();
    let mut changed = false;
    for (track_index, track) in state.tracks.iter_mut().enumerate() {
        if !selected.get(track_index).copied().unwrap_or(false) {
            continue;
        }
        let rate = track.sample_rate();
        if rate == 0 {
            continue;
        }
        let target = (new_start * rate as f64).round() as i64;
        let clip_indices: Vec<usize> = track
            .clips
            .clips
            .iter()
            .enumerate()
            .filter_map(|(index, clip)| {
                let start = clip.start as f64 / rate as f64;
                let duration = clip.buffer.len() as f64
                    * clip.stretch_ratio
                    / rate as f64;
                let end = start + duration;
                if at.map_or(true, |time| start <= time && time <= end) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect();

        for index in clip_indices {
            if track.clips.move_clip(index, target).is_err() {
                return ("Could not move clip to the requested start time.".to_owned(), false);
            }
            changed = true;
        }
    }
    if changed {
        host.push_undo("Set Clip");
    }
    (String::new(), true)
}

fn execute_set_track(
    host: &mut dyn ScriptHost,
    _command: &str,
    params: &[(String, String)],
) -> (String, bool) {
    let index = param(params, "Track")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let track_count = host.state().tracks.len();
    if index >= track_count {
        return (format!("Track not found: {index}."), false);
    }

    let mut changed = false;
    {
        let track = &mut host.state().tracks[index];
        if let Some(name) = param(params, "Name") {
            track.name = name.to_owned();
            changed = true;
        }
        if let Some(mute) = param(params, "Mute").and_then(parse_bool) {
            track.mute = mute;
            changed = true;
        }
        if let Some(solo) = param(params, "Solo").and_then(parse_bool) {
            track.solo = solo;
            changed = true;
        }
        if let Some(gain) = param(params, "Gain").and_then(|value| value.parse::<f32>().ok()) {
            track.gain = gain.max(0.0);
            changed = true;
        }
        // SetTrackAudioCommand (3.7.3): Volume in dB (-36..36), Pan in percent (-100..100).
        if let Some(db) = param(params, "Volume").and_then(|value| value.parse::<f32>().ok()) {
            track.gain = 10f32.powf(db.clamp(-36.0, 36.0) / 20.0);
            changed = true;
        }
        if let Some(pan) = param(params, "Pan").and_then(|value| value.parse::<f32>().ok()) {
            track.pan = (pan / 100.0).clamp(-1.0, 1.0);
            changed = true;
        }
    }

    let total = host.state().tracks.len() + host.state().labels.len();
    if let Some(selected) = param(params, "Selected").and_then(parse_bool) {
        let selection = host.selection();
        selection.tracks.resize(total, false);
        selection.tracks[index] = selected;
    }
    if param(params, "Focused").and_then(parse_bool) == Some(true) {
        host.set_focused_track(Some(index));
    }

    if changed {
        host.push_undo("Track Mix");
    }
    (String::new(), true)
}

fn execute_get_info(host: &mut dyn ScriptHost, params: &[(String, String)]) -> (String, bool) {
    let info_type = param(params, "Type").unwrap_or("Tracks");
    let format = param(params, "Format").unwrap_or("JSON");
    if !valid_format(format) {
        return (format!("Unsupported GetInfo format: {format}."), false);
    }

    let json = match info_type {
        "Tracks" => get_info_tracks(host),
        "Clips" => get_info_clips(host),
        "Labels" => get_info_labels(host),
        "Selection" => {
            let selection = host.selection();
            format!(
                "{{\"Start\":{},\"End\":{}}}",
                number(selection.t0),
                number(selection.t1)
            )
        }
        "Envelopes" | "Boxes" | "Preferences" => "[]".to_owned(),
        "Menus" => get_info_menus(),
        "Commands" => get_info_commands(host),
        _ => return (format!("Command options not recognised: {info_type}"), false),
    };

    (format_output(&json, format), true)
}

/// Edit > Labels > Add Label at Selection (LabelMenus.cpp DoAddLabel): the first selected
/// label track gets the label, else a new label track is created; that track ends selected.
fn execute_add_label(host: &mut dyn ScriptHost) -> (String, bool) {
    let n_wave = host.state().tracks.len();
    let n_label = host.state().labels.len();
    let (t0, t1) = (host.selection().t0, host.selection().t1);
    let selected = host.selection().tracks.clone();
    let target = (0..n_label).find(|i| selected.get(n_wave + i).copied().unwrap_or(false));
    let target = match target {
        Some(i) => i,
        None => {
            host.state().labels.push(au_core::timeline::LabelTrack::new("Label"));
            n_label
        }
    };
    host.state().labels[target].add(t0.min(t1), t0.max(t1), "");
    let total = n_wave + host.state().labels.len();
    let selection = host.selection();
    selection.tracks.resize(total, false);
    selection.tracks[n_wave + target] = true;
    host.push_undo("Added Label");
    (String::new(), true)
}

fn get_info_tracks(host: &mut dyn ScriptHost) -> String {
    let focus = host.focused_track();
    let selected = host.selection().tracks.clone();
    let state = host.state();
    let mut entries = Vec::new();
    let mut index = 0usize;

    for track in &state.tracks {
        let rate = track.sample_rate();
        let start = track
            .clips
            .clips
            .iter()
            .map(|clip| clip.start as f64 / rate.max(1) as f64)
            .fold(0.0_f64, f64::min);
        let end = if rate == 0 {
            0.0
        } else {
            track.len() as f64 / rate as f64
        };
        let channels = track
            .clips
            .clips
            .first()
            .map(|clip| clip.buffer.n_channels())
            .unwrap_or(1);
        entries.push(format!(
            "{{\"name\":{},\"focused\":{},\"selected\":{},\"kind\":\"wave\",\"start\":{},\"end\":{},\"pan\":{},\"volume\":{},\"channels\":{},\"solo\":{},\"mute\":{},\"VZoomMin\":0,\"VZoomMax\":1}}",
            json_string(&track.name),
            focus == Some(index),
            selected.get(index).copied().unwrap_or(false),
            number(start),
            number(end),
            number(track.pan as f64),
            number(track.gain as f64),
            channels,
            track.solo,
            track.mute
        ));
        index += 1;
    }

    for track in &state.note_tracks {
        entries.push(format!(
            "{{\"name\":{},\"focused\":false,\"selected\":false,\"kind\":\"note\"}}",
            json_string(&track.name)
        ));
    }
    for track in &state.labels {
        entries.push(format!(
            "{{\"name\":{},\"focused\":false,\"selected\":{},\"kind\":\"label\"}}",
            json_string(&track.name),
            selected.get(index).copied().unwrap_or(false)
        ));
        index += 1;
    }
    format!("[{}]", entries.join(","))
}

fn get_info_clips(host: &mut dyn ScriptHost) -> String {
    let state = host.state();
    let mut entries = Vec::new();
    for (track_index, track) in state.tracks.iter().enumerate() {
        let rate = track.sample_rate();
        for (clip_index, clip) in track.clips.clips.iter().enumerate() {
            let start = if rate == 0 {
                0.0
            } else {
                track.clips.clip_start_frames(clip_index) as f64 / rate as f64
            };
            let end = start
                + if rate == 0 {
                    0.0
                } else {
                    (clip.buffer.len() as f64 * clip.stretch_ratio
                        + (clip.start as i64 - track.clips.clip_start_frames(clip_index)) as f64)
                        / rate as f64
                };
            entries.push(format!(
                "{{\"track\":{},\"start\":{},\"end\":{},\"color\":0,\"name\":\"Clip\"}}",
                track_index,
                number(start),
                number(end)
            ));
        }
    }
    format!("[{}]", entries.join(","))
}

fn get_info_labels(host: &mut dyn ScriptHost) -> String {
    let state = host.state();
    let mut entries = Vec::new();
    let track_offset = state.tracks.len() + state.note_tracks.len();
    for (index, track) in state.labels.iter().enumerate() {
        let labels = track
            .labels
            .iter()
            .map(|label| {
                format!(
                    "[{},{},{}]",
                    number(label.start),
                    number(label.end),
                    json_string(&label.text)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        entries.push(format!("[{},[{}]]", track_offset + index, labels));
    }
    format!("[{}]", entries.join(","))
}

fn get_info_menus() -> String {
    let mut entries = Vec::new();
    for id in menu_ids() {
        entries.push(format!(
            "{{\"depth\":1,\"flags\":0,\"label\":{},\"accel\":\"\",\"id\":{}}}",
            json_string(id),
            json_string(id)
        ));
    }
    format!("[{}]", entries.join(","))
}

fn get_info_commands(host: &dyn ScriptHost) -> String {
    let mut entries = Vec::new();
    for id in builtin_ids() {
        entries.push(command_definition(host, id));
    }
    for (id, _) in host.effect_ids() {
        entries.push(effect_definition(host, &id));
    }
    format!("[{}]", entries.join(","))
}

fn format_output(json: &str, format: &str) -> String {
    match format {
        "LISP" => json_to_lisp(json).unwrap_or_else(|| json.to_owned()),
        // Brief output is deliberately kept compact, as with the JSON target.
        "Brief" | "JSON" => json.to_owned(),
        _ => json.to_owned(),
    }
}

fn json_to_lisp(input: &str) -> Option<String> {
    struct Parser<'a> {
        input: &'a [u8],
        pos: usize,
    }

    impl<'a> Parser<'a> {
        fn whitespace(&mut self) {
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_whitespace() {
                self.pos += 1;
            }
        }

        fn string(&mut self) -> Option<String> {
            if self.input.get(self.pos) != Some(&b'"') {
                return None;
            }
            self.pos += 1;
            let mut out = String::from("\"");
            while self.pos < self.input.len() {
                let ch = self.input[self.pos];
                self.pos += 1;
                match ch {
                    b'"' => {
                        out.push('"');
                        return Some(out);
                    }
                    b'\\' => {
                        let escaped = *self.input.get(self.pos)?;
                        self.pos += 1;
                        match escaped {
                            b'"' => out.push_str("\\\""),
                            b'\\' => out.push_str("\\\\"),
                            b'n' => out.push_str("\\n"),
                            b'r' => out.push_str("\\r"),
                            b't' => out.push_str("\\t"),
                            other => {
                                out.push('\\');
                                out.push(other as char);
                            }
                        }
                    }
                    other => out.push(other as char),
                }
            }
            None
        }

        fn value(&mut self) -> Option<String> {
            self.whitespace();
            match *self.input.get(self.pos)? {
                b'[' => {
                    self.pos += 1;
                    let mut items = Vec::new();
                    loop {
                        self.whitespace();
                        if self.input.get(self.pos) == Some(&b']') {
                            self.pos += 1;
                            break;
                        }
                        items.push(self.value()?);
                        self.whitespace();
                        if self.input.get(self.pos) == Some(&b',') {
                            self.pos += 1;
                        } else if self.input.get(self.pos) != Some(&b']') {
                            return None;
                        }
                    }
                    Some(format!("({})", items.join(" ")))
                }
                b'{' => {
                    self.pos += 1;
                    let mut fields = Vec::new();
                    loop {
                        self.whitespace();
                        if self.input.get(self.pos) == Some(&b'}') {
                            self.pos += 1;
                            break;
                        }
                        let key = self.string()?;
                        self.whitespace();
                        if self.input.get(self.pos) != Some(&b':') {
                            return None;
                        }
                        self.pos += 1;
                        let value = self.value()?;
                        let key = key.trim_matches('"');
                        fields.push(format!("({key} . {value})"));
                        self.whitespace();
                        if self.input.get(self.pos) == Some(&b',') {
                            self.pos += 1;
                        } else if self.input.get(self.pos) != Some(&b'}') {
                            return None;
                        }
                    }
                    Some(format!("({})", fields.join(" ")))
                }
                b'"' => self.string(),
                _ => {
                    let start = self.pos;
                    while self.pos < self.input.len()
                        && !self.input[self.pos].is_ascii_whitespace()
                        && !matches!(self.input[self.pos], b',' | b']' | b'}')
                    {
                        self.pos += 1;
                    }
                    let token = std::str::from_utf8(&self.input[start..self.pos]).ok()?;
                    Some(match token {
                        "true" => "T".to_owned(),
                        "false" | "null" => "NIL".to_owned(),
                        _ => token.to_owned(),
                    })
                }
            }
        }
    }

    let mut parser = Parser {
        input: input.as_bytes(),
        pos: 0,
    };
    parser.value()
}

fn valid_format(format: &str) -> bool {
    matches!(format, "JSON" | "LISP" | "Brief")
}

fn param<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn number(value: f64) -> String {
    if value.is_finite() {
        value.to_string()
    } else {
        "0".to_owned()
    }
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => output.push(character),
        }
    }
    output.push('"');
    output
}

fn is_builtin_command(id: &str) -> bool {
    builtin_ids().contains(&id) || menu_ids().contains(&id)
}

fn builtin_ids() -> &'static [&'static str] {
    &[
        "Export2",
        "GetInfo",
        "Help",
        "Import2",
        "Message",
        "Select",
        "SelectFrequencies",
        "SelectTime",
        "SelectTracks",
        "SetClip",
        "SetLabel",
        "SetTrack",
        "SetTrackAudio",
        "SetTrackStatus",
    ]
}

fn menu_ids() -> &'static [&'static str] {
    &[
        "Close",
        "Copy",
        "Cut",
        "Delete",
        "New",
        "Paste",
        "Redo",
        "Save",
        "SelectAll",
        "Undo",
        "ZoomIn",
        "ZoomNormal",
        "ZoomOut",
        "ZoomToFit",
        "ZoomToSelection",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use au_core::{AudioBuffer, Track};

    #[derive(Default)]
    struct MockHost {
        state: ProjectState,
        selection: Selection,
        undo: Vec<String>,
        messages: Vec<String>,
        effects: Vec<(String, Vec<String>)>,
    }

    impl ScriptHost for MockHost {
        fn state(&mut self) -> &mut ProjectState {
            &mut self.state
        }

        fn selection(&mut self) -> &mut Selection {
            &mut self.selection
        }

        fn push_undo(&mut self, desc: &str) {
            self.undo.push(desc.to_owned());
        }

        fn apply_effect(
            &mut self,
            id: &str,
            params: &[(String, String)],
        ) -> Result<String, String> {
            self.effects.push((
                id.to_owned(),
                params.iter().map(|(key, _)| key.clone()).collect(),
            ));
            Ok(String::new())
        }

        fn effect_ids(&self) -> Vec<(String, Vec<(String, String, String)>)> {
            vec![(
                "Amplify".to_owned(),
                vec![("Ratio".to_owned(), "number".to_owned(), "1".to_owned())],
            )]
        }

        fn import(&mut self, _path: &str) -> Result<(), String> {
            Ok(())
        }

        fn export(&mut self, _path: &str, _channels: u32) -> Result<(), String> {
            Ok(())
        }

        fn menu_command(&mut self, _id: &str) -> Option<Result<(), String>> {
            None
        }

        fn message(&mut self, text: &str) {
            self.messages.push(text.to_owned());
        }
    }

    fn mock() -> MockHost {
        let mut host = MockHost::default();
        host.state.sample_rate = 10;
        host.state.tracks.push(Track::new(
            "Track A",
            AudioBuffer::from_channels(10, vec![vec![0.0; 10]]),
        ));
        host.selection.tracks = vec![false];
        host
    }

    #[test]
    fn select_then_get_info_tracks_json() {
        let mut host = mock();
        assert!(execute(&mut host, "Select: Start=1 End=2 Track=0").1);
        assert_eq!(host.selection.t0, 1.0);
        assert_eq!(host.selection.t1, 2.0);
        assert!(execute(
            &mut host,
            "GetInfo: Type=Tracks Format=JSON"
        )
        .0
        .contains(r#""name":"Track A""#));
    }

    #[test]
    fn set_label_edits_label_text_and_times() {
        let mut host = mock();
        host.state.labels.push(au_core::timeline::LabelTrack::new("Labels"));
        host.state.labels[0].add(0.0, 1.0, "before");
        let (output, success) = execute(
            &mut host,
            r#"SetLabel: Label=0 Text="after text" Start=2 End=3"#,
        );
        assert!(success, "{output}");
        assert_eq!(host.state.labels[0].labels[0].text, "after text");
        assert_eq!(host.state.labels[0].labels[0].start, 2.0);
    }

    #[test]
    fn help_and_unknown_command() {
        let mut host = mock();
        let (help, success) = execute(&mut host, "Help: Command=Message");
        assert!(success);
        assert!(help.contains(r#""id":"Message""#));

        let (error, success) = execute(&mut host, "NotACommand");
        assert!(!success);
        assert_eq!(
            error,
            "Your batch command of NotACommand was not recognized."
        );
    }

    #[test]
    fn effect_dispatch_validates_and_records_undo() {
        let mut host = mock();
        let (output, success) = execute(&mut host, "Amplify: Ratio=2");
        assert!(success, "{output}");
        assert_eq!(host.effects[0].0, "Amplify");
        assert_eq!(host.undo, vec!["Amplify"]);

        let (_, success) = execute(&mut host, "Amplify: NotAParam=2");
        assert!(!success);
    }

    #[test]
    fn parses_quoted_values_and_messages() {
        let (name, params) = parse_command(r#"Message: Text="a quoted value" Other='x y'"#);
        assert_eq!(name, "Message");
        assert_eq!(params[0].1, "a quoted value");
        assert_eq!(params[1].1, "x y");

        let mut host = mock();
        assert_eq!(execute(&mut host, "Message: Text=hello"), ("hello".into(), true));
        assert_eq!(host.messages, vec!["hello"]);
    }
}
