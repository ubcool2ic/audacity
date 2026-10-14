//! Minimal mod-script-pipe bridge for the GUI.
//!
//! `execute` is intentionally a small built-in interpreter used until the full
//! scripting command layer (`au-script`, owned by another agent) lands.

use eframe::egui;
use std::sync::mpsc::{Receiver, TryRecvError};

use au_core::ProjectState;
use au_scriptpipe::{PipeNames, PipeServer, Request};

/// Owns the script-pipe server and receives incoming commands.
pub struct ScriptPipe {
    server: Option<PipeServer>,
    requests: Receiver<Request>,
}

impl ScriptPipe {
    /// Starts the pipe server and wakes the GUI when a request arrives.
    pub fn start(ctx: &egui::Context) -> std::io::Result<Self> {
        let wake_ctx = ctx.clone();
        // AU_SCRIPT_PIPE_DIR moves the FIFOs (DIR/audacity_script_pipe.to/.from) so tests
        // do not collide with a real Audacity using the default /tmp names.
        let names = match std::env::var_os("AU_SCRIPT_PIPE_DIR") {
            Some(dir) => PipeNames::in_dir(std::path::Path::new(&dir), "audacity_script_pipe"),
            None => PipeNames::audacity_default(),
        };
        let (server, requests) = PipeServer::start(
            names,
            Some(Box::new(move || wake_ctx.request_repaint())),
        )?;

        Ok(Self {
            server: Some(server),
            requests,
        })
    }

    /// Returns the pipe names while the server is running.
    #[allow(dead_code)] // used by diagnostics/tests
    pub fn names(&self) -> Option<&PipeNames> {
        self.server.as_ref().map(PipeServer::names)
    }

    /// Receives up to `budget` pending requests without blocking.
    pub fn drain(&mut self, budget: usize) -> Vec<Request> {
        let mut requests = Vec::with_capacity(budget);

        for _ in 0..budget {
            match self.requests.try_recv() {
                Ok(request) => requests.push(request),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }

        requests
    }
}

impl Drop for ScriptPipe {
    fn drop(&mut self) {
        if let Some(server) = self.server.take() {
            server.stop();
        }
    }
}

/// Parses Audacity's `Name: key=value` command syntax.
///
/// The command name ends at the first colon or whitespace. Parameter keys are
/// preserved exactly, and values may be unquoted, double-quoted, or
/// single-quoted. A backslash inside a quoted value escapes the following
/// character.
pub fn parse_command(line: &str) -> (String, Vec<(String, String)>) {
    let chars: Vec<char> = line.trim().chars().collect();
    let mut name_end = 0;

    while name_end < chars.len() && chars[name_end] != ':' && !chars[name_end].is_whitespace()
    {
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

/// Executes the deliberately minimal built-in command set.
///
/// This interpreter is a temporary bridge until the full scripting command
/// layer (`au-script`, owned by another agent) lands. Every response is
/// finalized through `au_scriptpipe::finished`.
pub fn execute(state: &ProjectState, line: &str) -> String {
    let (command, params) = parse_command(line);

    let response = match command.as_str() {
        "Help" => execute_help(&params),
        "Message" => {
            let text = param(&params, "Text").unwrap_or_default();
            (text.to_owned(), true)
        }
        "GetInfo" => execute_get_info(state, &params),
        _ => (
            format!("Your batch command of {command} was not recognized."),
            false,
        ),
    };

    au_scriptpipe::finished(&response.0, response.1)
}

fn execute_help(params: &[(String, String)]) -> (String, bool) {
    const HELP: &str = r#"{ "id":"Help", "name":"Help", "params":[{"name":"Command","type":"String"}] }"#;
    const MESSAGE: &str =
        r#"{ "id":"Message", "name":"Message", "params":[{"name":"Text","type":"String"}] }"#;
    const GET_INFO: &str = r#"{ "id":"GetInfo", "name":"GetInfo", "params":[{"name":"Type","type":"String"},{"name":"Format","type":"String"}] }"#;

    match param(params, "Command") {
        None => (
            format!("[ {HELP}, {MESSAGE}, {GET_INFO} ]"),
            true,
        ),
        Some("Help") => (HELP.to_owned(), true),
        Some("Message") => (MESSAGE.to_owned(), true),
        Some("GetInfo") => (GET_INFO.to_owned(), true),
        Some(other) => (
            format!("No help available for command {other}."),
            true,
        ),
    }
}

fn execute_get_info(state: &ProjectState, params: &[(String, String)]) -> (String, bool) {
    let info_type = param(params, "Type").unwrap_or("Tracks");
    let format = param(params, "Format").unwrap_or("JSON");

    if format != "JSON" && format != "Brief" {
        return (format!("Unsupported GetInfo format: {format}."), false);
    }

    match info_type {
        "Tracks" => {
            if format == "JSON" {
                let mut output = String::from("[");
                let mut needs_comma = false;

                for track in &state.tracks {
                    if needs_comma {
                        output.push(',');
                    }
                    needs_comma = true;
                    output.push_str(&format!(
                        r#"{{"name":{},"kind":"wave","rate":{},"clips":{}}}"#,
                        json_string(&track.name),
                        track.sample_rate(),
                        track.clips.clips.len()
                    ));
                }

                for track in &state.note_tracks {
                    if needs_comma {
                        output.push(',');
                    }
                    needs_comma = true;
                    output.push_str(&format!(
                        r#"{{"name":{},"kind":"note"}}"#,
                        json_string(&track.name)
                    ));
                }

                for track in &state.labels {
                    if needs_comma {
                        output.push(',');
                    }
                    needs_comma = true;
                    output.push_str(&format!(
                        r#"{{"name":{},"kind":"label"}}"#,
                        json_string(&track.name)
                    ));
                }

                output.push(']');
                (output, true)
            } else {
                let mut output = String::new();
                for track in &state.tracks {
                    output.push_str(&format!(
                        "{}: wave, {} frames, {} Hz, {} clips\n",
                        track.name,
                        track.len(),
                        track.sample_rate(),
                        track.clips.clips.len()
                    ));
                }
                for track in &state.note_tracks {
                    output.push_str(&format!(
                        "{}: note, {} notes\n",
                        track.name,
                        track.notes.len()
                    ));
                }
                for track in &state.labels {
                    output.push_str(&format!(
                        "{}: label, {} labels\n",
                        track.name,
                        track.labels.len()
                    ));
                }
                (output, true)
            }
        }
        "Labels" => {
            if format == "JSON" {
                let mut output = String::from("[");
                for (track_index, track) in state.labels.iter().enumerate() {
                    if track_index > 0 {
                        output.push(',');
                    }
                    output.push_str(&format!("[{track_index},["));

                    for (label_index, label) in track.labels.iter().enumerate() {
                        if label_index > 0 {
                            output.push(',');
                        }
                        output.push_str(&format!(
                            "[{},{},{}]",
                            label.start,
                            label.end,
                            json_string(&label.text)
                        ));
                    }

                    output.push_str("]]");
                }
                output.push(']');
                (output, true)
            } else {
                let mut output = String::new();
                for (track_index, track) in state.labels.iter().enumerate() {
                    output.push_str(&format!("Track {track_index}: {}\n", track.name));
                    for label in &track.labels {
                        output.push_str(&format!(
                            "{} - {}: {}\n",
                            label.start, label.end, label.text
                        ));
                    }
                }
                (output, true)
            }
        }
        other => (
            format!("Unsupported GetInfo type: {other}."),
            false,
        ),
    }
}

fn param<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
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

#[cfg(test)]
mod tests {
    use super::{execute, parse_command};

    #[test]
    fn parses_command_and_quoted_values() {
        let (name, params) = parse_command(
            r#"Message: Text="quoted value" Single='single value' Plain=hello"#,
        );

        assert_eq!(name, "Message");
        assert_eq!(
            params,
            vec![
                ("Text".to_owned(), "quoted value".to_owned()),
                ("Single".to_owned(), "single value".to_owned()),
                ("Plain".to_owned(), "hello".to_owned()),
            ]
        );
    }

    #[test]
    fn parses_command_without_colon() {
        let (name, params) = parse_command("Help");
        assert_eq!(name, "Help");
        assert!(params.is_empty());
    }

    #[test]
    fn executes_message_and_rejects_unknown_commands() {
        let state = au_core::ProjectState::default();

        assert!(execute(&state, "Message: Text=hello").contains("hello"));
        assert!(execute(&state, "NotACommand").contains(
            "Your batch command of NotACommand was not recognized."
        ));
    }

    #[test]
    fn help_lists_supported_commands() {
        let state = au_core::ProjectState::default();
        let response = execute(&state, "Help");

        assert!(response.contains(r#""id":"Help""#));
        assert!(response.contains(r#""id":"Message""#));
        assert!(response.contains(r#""id":"GetInfo""#));
    }
}

