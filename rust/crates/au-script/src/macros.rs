//! Macro file management and batch execution for Audacity-style commands.

use crate::cmd::{execute, ScriptHost};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// One command and its serialized parameter text in a macro.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MacroLine {
    /// Audacity command identifier.
    pub command: String,
    /// Parameters exactly as stored after the colon.
    pub params: String,
}

impl MacroLine {
    /// Creates a macro line.
    pub fn new(command: impl Into<String>, params: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            params: params.into(),
        }
    }

    /// Returns this line in Audacity's macro-file format.
    pub fn serialize(&self) -> String {
        format!("{}:{}", self.command, self.params)
    }

    fn from_file_line(line: &str) -> Option<Self> {
        let (command, params) = line.split_once(':')?;
        Some(Self {
            command: command.trim().to_string(),
            // wxString::Strip(wxString::trailing) preserves leading whitespace.
            params: params.trim_end().to_string(),
        })
    }
}

/// Returns the conventional Audacity macro directory for the current user.
pub fn default_macro_dir() -> PathBuf {
    // Audacity 3.x on Linux keeps macros in the XDG data dir
    // ($XDG_DATA_HOME or ~/.local/share)/audacity/Macros.
    #[cfg(target_os = "linux")]
    {
        if let Some(d) = std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
            return PathBuf::from(d).join("audacity").join("Macros");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".local/share/audacity/Macros");
        }
    }
    {
        // Windows: %APPDATA%\audacity\Macros; macOS: ~/Library/Application Support/audacity/Macros.
        au_core::paths::user_config_base()
            .join("audacity")
            .join("Macros")
    }
}

/// Names of the macros Audacity creates by default.
pub fn default_macro_names() -> Vec<String> {
    vec!["MP3 Conversion".to_string(), "Fade Ends".to_string()]
}

/// Returns the command lines used to restore one of Audacity's built-in macros.
pub fn builtin_macro(name: &str) -> Option<Vec<MacroLine>> {
    // MacroCommands::RestoreMacro. AddToMacro(command) without params stores
    // GetCurrentParamsFor(command): an effect's current settings as an
    // automation string (Audacity defaults on a fresh config) or, for an effect
    // without parameters, its "<Current Settings>" preset reference.
    const CURRENT: &str = "Use_Preset=\"<Current Settings>\"";
    match name {
        "MP3 Conversion" => Some(vec![
            MacroLine::new(
                "Normalize",
                "ApplyVolume=\"1\" PeakLevel=\"-1\" RemoveDcOffset=\"1\" StereoIndependent=\"0\"",
            ),
            MacroLine::new("ExportMP3", ""),
        ]),
        "Fade Ends" => Some(vec![
            MacroLine::new("Select", "Start=\"0\" End=\"1\""),
            MacroLine::new("FadeIn", CURRENT),
            MacroLine::new("Select", "Start=\"0\" End=\"1\" RelativeTo=\"ProjectEnd\""),
            MacroLine::new("FadeOut", CURRENT),
            MacroLine::new("Select", "Start=\"0\" End=\"0\""),
        ]),
        _ => None,
    }
}

/// Manages macro files and the currently loaded sequence of commands.
#[derive(Clone, Debug)]
pub struct MacroCommands {
    macro_dir: PathBuf,
    commands: Vec<MacroLine>,
}

impl Default for MacroCommands {
    fn default() -> Self {
        Self::new(default_macro_dir())
    }
}

impl MacroCommands {
    /// Creates a manager for `macro_dir`, creating it and absent built-in macros.
    pub fn new(macro_dir: impl AsRef<Path>) -> Self {
        let macro_dir = macro_dir.as_ref().to_path_buf();
        let _ = fs::create_dir_all(&macro_dir);
        let mut manager = Self {
            macro_dir,
            commands: Vec::new(),
        };

        for name in default_macro_names() {
            if !manager.macro_path(&name).exists() {
                if manager.add_macro(&name) {
                    if let Some(commands) = builtin_macro(&name) {
                        manager.commands = commands;
                        let _ = manager.write_macro(&name);
                        manager.commands.clear();
                    }
                }
            }
        }

        manager
    }

    /// Returns the directory used for macro files.
    pub fn macro_dir(&self) -> &Path {
        &self.macro_dir
    }

    /// Returns the loaded command sequence.
    pub fn commands(&self) -> &[MacroLine] {
        &self.commands
    }

    /// Replaces the loaded command sequence.
    pub fn set_commands(&mut self, commands: Vec<MacroLine>) {
        self.commands = commands;
    }

    /// Returns the number of commands in the loaded macro.
    pub fn get_count(&self) -> usize {
        self.commands.len()
    }

    /// Returns the command at `index`, or an empty string if out of range.
    pub fn get_command(&self, index: isize) -> String {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.commands.get(index))
            .map(|line| line.command.clone())
            .unwrap_or_default()
    }

    /// Returns the parameters at `index`, or an empty string if out of range.
    pub fn get_params(&self, index: isize) -> String {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.commands.get(index))
            .map(|line| line.params.clone())
            .unwrap_or_default()
    }

    /// Clears the loaded macro.
    pub fn reset_macro(&mut self) {
        self.commands.clear();
    }

    /// Loads the built-in definition into the current command sequence.
    pub fn restore_macro(&mut self, name: &str) -> bool {
        if let Some(commands) = builtin_macro(name) {
            self.commands = commands;
            true
        } else {
            self.commands.clear();
            false
        }
    }

    /// Reads `<name>.txt`, replacing the currently loaded command sequence.
    ///
    /// Lines without a colon are ignored. The command name is trimmed on both
    /// ends; parameter text is trimmed only at the end, as in Audacity.
    pub fn read_macro(&mut self, name: &str) -> io::Result<String> {
        self.commands.clear();
        let content = fs::read_to_string(self.macro_path(name))?;
        for line in content.lines() {
            if let Some(command) = MacroLine::from_file_line(line) {
                self.commands.push(command);
            }
        }
        Ok(name.to_string())
    }

    /// Writes the loaded commands to `<name>.txt`, including a trailing newline
    /// after every command.
    pub fn write_macro(&self, name: &str) -> io::Result<String> {
        fs::create_dir_all(&self.macro_dir)?;
        let mut content = String::new();
        for command in &self.commands {
            content.push_str(&command.serialize());
            content.push('\n');
        }
        fs::write(self.macro_path(name), content)?;
        Ok(name.to_string())
    }

    /// Creates an empty `<name>.txt` file, returning false if it already exists
    /// or cannot be created.
    pub fn add_macro(&self, name: &str) -> bool {
        if fs::create_dir_all(&self.macro_dir).is_err() {
            return false;
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.macro_path(name))
            .is_ok()
    }

    /// Deletes `<name>.txt`, returning whether the file was removed.
    pub fn delete_macro(&self, name: &str) -> bool {
        fs::remove_file(self.macro_path(name)).is_ok()
    }

    /// Renames a macro file, returning whether the operation succeeded.
    pub fn rename_macro(&self, old_name: &str, new_name: &str) -> bool {
        fs::rename(self.macro_path(old_name), self.macro_path(new_name)).is_ok()
    }

    /// Inserts a command before `before`; a negative index appends it.
    pub fn add_to_macro(
        &mut self,
        command: impl Into<String>,
        params: impl Into<String>,
        before: isize,
    ) {
        let index = if before < 0 {
            self.commands.len()
        } else {
            (before as usize).min(self.commands.len())
        };
        self.commands
            .insert(index, MacroLine::new(command, params));
    }

    /// Deletes the command at `index`, doing nothing if it is out of range.
    pub fn delete_from_macro(&mut self, index: isize) {
        if let Ok(index) = usize::try_from(index) {
            if index < self.commands.len() {
                self.commands.remove(index);
            }
        }
    }

    /// Returns macro names in sorted order.
    pub fn get_names(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.macro_dir) else {
            return Vec::new();
        };

        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("txt") {
                    path.file_stem()
                        .and_then(|stem| stem.to_str())
                        .map(str::to_string)
                } else {
                    None
                }
            })
            .collect();
        names.sort();
        names
    }

    /// Returns whether `name` is one of Audacity's fixed default macro names.
    pub fn is_fixed(name: &str) -> bool {
        default_macro_names().iter().any(|fixed| fixed == name)
    }

    /// Copies `.txt` files from a legacy Chains directory when no same-named
    /// macro file exists yet.
    pub fn migrate_legacy_chains(&self, legacy_dir: impl AsRef<Path>) -> io::Result<()> {
        fs::create_dir_all(&self.macro_dir)?;
        let entries = match fs::read_dir(legacy_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };

        for entry in entries {
            let entry = entry?;
            let source = entry.path();
            if source.extension().and_then(|ext| ext.to_str()) != Some("txt") {
                continue;
            }
            let Some(file_name) = source.file_name() else {
                continue;
            };
            let destination = self.macro_dir.join(file_name);
            if !destination.exists() {
                fs::copy(source, destination)?;
            }
        }
        Ok(())
    }

    /// Constructs a runner from the currently loaded command sequence.
    pub fn runner(&self) -> MacroRunner {
        MacroRunner::new(self.commands.clone())
    }

    fn macro_path(&self, name: &str) -> PathBuf {
        let mut path = self.macro_dir.join(name);
        path.set_extension("txt");
        path
    }
}

/// Runs a sequence of macro commands against a scripting host.
#[derive(Clone, Debug, Default)]
pub struct MacroRunner {
    commands: Vec<MacroLine>,
    aborted: bool,
}

impl MacroRunner {
    /// Creates a runner for `commands`.
    pub fn new(commands: Vec<MacroLine>) -> Self {
        Self {
            commands,
            aborted: false,
        }
    }

    /// Returns the runner's command sequence.
    pub fn commands(&self) -> &[MacroLine] {
        &self.commands
    }

    /// Requests that execution stop after the current command.
    pub fn abort(&mut self) {
        self.aborted = true;
    }

    /// Applies the macro, stopping and returning the first command failure.
    pub fn apply(&mut self, host: &mut dyn ScriptHost) -> Result<(), String> {
        self.aborted = false;
        host.push_undo("Applied Macro");

        for command in &self.commands {
            let (output, success) = execute(host, &command.serialize());
            if !success {
                let error = if output.is_empty() {
                    format!(
                        "Your batch command of {} was not recognized.",
                        command.command
                    )
                } else {
                    output
                };
                host.message(&error);
                return Err(error);
            }
            if self.aborted {
                break;
            }
        }
        Ok(())
    }

    /// Imports each input file, selects its contents, applies the macro, and
    /// exports it into `output_dir/macro-output`.
    ///
    /// Export commands in the macro are handled as batch exports. The output
    /// basename comes from the input file; the export command determines the
    /// extension. When a macro contains no export command, `extension` is used.
    pub fn apply_to_files(
        &mut self,
        host: &mut dyn ScriptHost,
        files: &[PathBuf],
        output_dir: impl AsRef<Path>,
        extension: &str,
        channels: u32,
    ) -> Result<Vec<PathBuf>, String> {
        let output_directory = output_dir.as_ref().join("macro-output");
        fs::create_dir_all(&output_directory)
            .map_err(|error| format!("Could not create output directory: {error}"))?;

        let mut exported = Vec::with_capacity(files.len());
        for (file_index, input) in files.iter().enumerate() {
            if file_index > 0 {
                run_menu_command(host, "Close")?;
                run_menu_command(host, "New")?;
            }

            host.import(&input.to_string_lossy())?;
            run_menu_command(host, "SelectAll")?;

            host.push_undo("Applied Macro");
            let basename = input
                .file_stem()
                .or_else(|| input.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("output");

            for command in &self.commands {
                if let Some(export_extension) = export_extension(&command.command) {
                    let path = output_directory.join(format!(
                        "{}.{}",
                        basename, export_extension
                    ));
                    host.export(&path.to_string_lossy(), channels)?;
                    exported.push(path);
                    continue;
                }

                let (output, success) = execute(host, &command.serialize());
                if !success {
                    let error = if output.is_empty() {
                        format!(
                            "Your batch command of {} was not recognized.",
                            command.command
                        )
                    } else {
                        output
                    };
                    host.message(&error);
                    return Err(error);
                }
                if self.aborted {
                    break;
                }
            }

            if !self.commands.iter().any(|command| {
                export_extension(&command.command).is_some()
            }) {
                let ext = extension.trim_start_matches('.');
                let ext = if ext.is_empty() { "wav" } else { ext };
                let path = output_directory.join(format!("{basename}.{ext}"));
                host.export(&path.to_string_lossy(), channels)?;
                exported.push(path);
            }
        }

        self.aborted = false;
        Ok(exported)
    }
}

/// Applies `commands` to a host, stopping on the first failure.
pub fn apply_macro(
    host: &mut dyn ScriptHost,
    commands: &[MacroLine],
) -> Result<(), String> {
    MacroRunner::new(commands.to_vec()).apply(host)
}

/// Applies a macro to multiple files, exporting to `output_dir/macro-output`.
pub fn apply_macro_to_files(
    host: &mut dyn ScriptHost,
    commands: &[MacroLine],
    files: &[PathBuf],
    output_dir: impl AsRef<Path>,
    extension: &str,
    channels: u32,
) -> Result<Vec<PathBuf>, String> {
    MacroRunner::new(commands.to_vec()).apply_to_files(
        host,
        files,
        output_dir,
        extension,
        channels,
    )
}

fn run_menu_command(host: &mut dyn ScriptHost, id: &str) -> Result<(), String> {
    match host.menu_command(id) {
        Some(Ok(())) | None => Ok(()),
        Some(Err(error)) => Err(error),
    }
}

fn export_extension(command: &str) -> Option<&'static str> {
    match command {
        "ExportMp3_56k_before" | "ExportMP3" | "ExportMp3" => Some("mp3"),
        "ExportWAV" | "ExportWav" => Some("wav"),
        "ExportOgg" | "ExportOGG" => Some("ogg"),
        "ExportFLAC" | "ExportFlac" => Some("flac"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::{ScriptHost, Selection};
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

    fn temp_dir(label: &str) -> PathBuf {
        let id = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "au-script-macros-{label}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[derive(Default)]
    struct MockHost {
        state: au_core::ProjectState,
        selection: Selection,
        undo: Vec<String>,
        messages: Vec<String>,
        imports: Vec<String>,
        exports: Vec<(String, u32)>,
        calls: Vec<String>,
        fail_command: Option<String>,
    }

    impl ScriptHost for MockHost {
        fn state(&mut self) -> &mut au_core::ProjectState {
            &mut self.state
        }

        fn selection(&mut self) -> &mut Selection {
            &mut self.selection
        }

        fn push_undo(&mut self, desc: &str) {
            self.undo.push(desc.to_string());
        }

        fn apply_effect(
            &mut self,
            id: &str,
            _params: &[(String, String)],
        ) -> Result<String, String> {
            self.calls.push(id.to_string());
            if self.fail_command.as_deref() == Some(id) {
                Err(format!("{id} failed"))
            } else {
                Ok(String::new())
            }
        }

        fn effect_ids(&self) -> Vec<(String, Vec<(String, String, String)>)> {
            Vec::new()
        }

        fn import(&mut self, path: &str) -> Result<(), String> {
            self.imports.push(path.to_string());
            Ok(())
        }

        fn export(&mut self, path: &str, channels: u32) -> Result<(), String> {
            self.exports.push((path.to_string(), channels));
            Ok(())
        }

        fn menu_command(&mut self, id: &str) -> Option<Result<(), String>> {
            self.calls.push(id.to_string());
            if self.fail_command.as_deref() == Some(id) {
                return Some(Err(format!("{id} failed")));
            }
            Some(Ok(()))
        }

        fn message(&mut self, text: &str) {
            self.messages.push(text.to_string());
        }
    }

    #[test]
    fn macro_file_round_trip_has_trailing_newline() {
        let dir = temp_dir("round-trip");
        let mut macros = MacroCommands::new(&dir);
        macros.set_commands(vec![
            MacroLine::new("Select", " Start=\"0\" End=\"1\""),
            MacroLine::new("FadeIn", ""),
        ]);

        macros.write_macro("RoundTrip").unwrap();
        let contents = fs::read_to_string(dir.join("RoundTrip.txt")).unwrap();
        assert!(contents.ends_with('\n'));

        macros.set_commands(Vec::new());
        assert_eq!(macros.read_macro("RoundTrip").unwrap(), "RoundTrip");
        assert_eq!(
            macros.commands(),
            &[
                MacroLine::new("Select", " Start=\"0\" End=\"1\""),
                MacroLine::new("FadeIn", "")
            ]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn builtins_match_audacity_macro_definitions() {
        assert_eq!(
            builtin_macro("MP3 Conversion").unwrap()[1],
            MacroLine::new("ExportMP3", "")
        );
        assert_eq!(builtin_macro("Fade Ends").unwrap().len(), 5);
        assert_eq!(MacroCommands::is_fixed("Fade Ends"), true);
        assert_eq!(MacroCommands::is_fixed("Other"), false);
    }

    #[test]
    fn apply_stops_at_first_failure() {
        let mut host = MockHost::default();
        host.fail_command = Some("Bad".to_string());
        let commands = vec![
            MacroLine::new("Good", ""),
            MacroLine::new("Bad", ""),
            MacroLine::new("Never", ""),
        ];

        let error = apply_macro(&mut host, &commands).unwrap_err();
        assert_eq!(error, "Bad failed");
        assert_eq!(host.calls, vec!["Good", "Bad"]);
        assert_eq!(host.undo, vec!["Applied Macro"]);
    }

    #[test]
    fn apply_to_files_exports_into_macro_output() {
        let dir = temp_dir("batch");
        let file = PathBuf::from("input.wav");
        let mut host = MockHost::default();
        let commands = vec![MacroLine::new("ExportWAV", "")];

        let exported = apply_macro_to_files(
            &mut host,
            &commands,
            &[file],
            &dir,
            "mp3",
            2,
        )
        .unwrap();

        let expected = dir.join("macro-output").join("input.wav");
        assert_eq!(exported, vec![expected.clone()]);
        assert_eq!(host.exports, vec![(expected.to_string_lossy().into_owned(), 2)]);
        assert_eq!(host.imports, vec!["input.wav"]);
        assert!(host.calls.contains(&"SelectAll".to_string()));
        let _ = fs::remove_dir_all(dir);
    }
}
