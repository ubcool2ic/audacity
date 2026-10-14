//! Minimal mod-script-pipe bridge for the GUI.
//!
//! `execute` is intentionally a small built-in interpreter used until the full
//! scripting command layer (`au-script`, owned by another agent) lands.

use eframe::egui;
use std::sync::mpsc::{Receiver, TryRecvError};

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

#[cfg(test)]
mod tests {
    use super::parse_command;

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
}
