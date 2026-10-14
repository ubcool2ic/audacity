// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! `audacity-rs` entry point.
//!
//! * `audacity-rs` (built with `--features gui`) opens the egui editor.
//! * `audacity-rs --selftest` runs a headless edit pipeline (CI-friendly, no audio device).
//! * `audacity-rs process <in> <out.wav|out.flac> [effect...]` applies effects headlessly.
//! * `audacity-rs process <inputs...> --out-dir DIR [--format wav|flac] [--jobs N] [effect...]`
//!   processes multiple files concurrently.
//!   Effects: amplify:<dB>, normalize:<dB>, fadein, fadeout, reverse.

use au_core::{AudioBuffer, Editor, ProjectState, Selection, Track};
use au_effects::{Amplify, Effect, FadeIn, FadeOut, Normalize, Reverse};
use std::path::{Path, PathBuf};

pub mod fx_bridge;
pub mod plugin_bridge;
pub mod transport;
#[cfg(feature = "gui")]
mod gui;
#[cfg(feature = "gui")]
mod script_bridge;
#[cfg(feature = "gui")]
mod plugin_manager;
#[cfg(feature = "gui")]
mod spectrogram_view;
#[cfg(feature = "gui")]
mod fx_ui;
#[cfg(feature = "gui")]
mod analyze_ui;
#[cfg(feature = "gui")]
mod room_noise;
#[cfg(feature = "gui")]
mod export_ui;
#[cfg(feature = "gui")]
mod export_glue;

/// Crash-report folder (`AUDACITY_RS_CRASH_DIR` overrides the per-OS default).
pub fn crash_dir() -> std::path::PathBuf {
    std::env::var_os("AUDACITY_RS_CRASH_DIR")
        .map(Into::into)
        .unwrap_or_else(au_runtime::crash::default_crash_dir)
}

fn main() {
    au_runtime::crash::install(au_runtime::crash::CrashConfig {
        dir: crash_dir(),
        app_name: "audacity-rs".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    });
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("--selftest") => report(selftest()),
        Some("process") => process_command(&args[1..]),
        Some("aup3-info") if args.len() == 2 => report(aup3_info(&args[1])),
        Some("--help") | Some("-h") => {
            println!("usage: audacity-rs [--selftest | aup3-info <file.aup3> | process <in> <out.wav|out.flac> [amplify:<dB>|normalize:<dB>|fadein|fadeout|reverse]...]");
            0
        }
        Some(f) if !f.starts_with('-') && std::path::Path::new(f).exists() => {
            run_gui_with(Some(f.into()))
        }
        _ => run_gui(),
    };
    std::process::exit(code);
}

/// Prints what is in an Audacity `.aup3` project and whether it needs recovery.
fn aup3_info(path: &str) -> Result<String, String> {
    use au_project::aup3;
    let p = std::path::Path::new(path);
    let info = aup3::inspect(p).map_err(|e| e.to_string())?;
    let src = if info.needs_recovery {
        aup3::DocSource::Autosave
    } else {
        aup3::DocSource::Saved
    };
    let proj = aup3::load_from(p, src).map_err(|e| e.to_string())?;
    let mut out = format!(
        "format {}.{}.{}  rate {}  needs_recovery {}  integrity {}\n",
        info.user_version >> 24,
        (info.user_version >> 16) & 0xff,
        (info.user_version >> 8) & 0xff,
        proj.state.sample_rate,
        info.needs_recovery,
        aup3::integrity_check(p).map_err(|e| e.to_string())?
    );
    for (i, t) in proj.state.tracks.iter().enumerate() {
        out += &format!(
            "track {i} {:?}: {} ch, {} clips, {:?}, gain {} pan {} mute {} solo {}\n",
            t.name,
            t.clips.n_channels,
            t.clips.clips.len(),
            proj.extras.track_formats[i],
            t.gain,
            t.pan,
            t.mute,
            t.solo
        );
    }
    for l in &proj.state.labels {
        out += &format!("labels {:?}: {}\n", l.name, l.labels.len());
    }
    Ok(out.trim_end().to_owned())
}

fn report(r: Result<String, String>) -> i32 {
    match r {
        Ok(msg) => {
            println!("{msg}");
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(feature = "gui")]
fn run_gui() -> i32 {
    run_gui_with(None)
}

#[cfg(feature = "gui")]
fn run_gui_with(open: Option<std::path::PathBuf>) -> i32 {
    match gui::run_with(open) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

#[cfg(not(feature = "gui"))]
fn run_gui_with(_open: Option<std::path::PathBuf>) -> i32 {
    run_gui()
}

#[cfg(not(feature = "gui"))]
fn run_gui() -> i32 {
    eprintln!(
        "built without the `gui` feature; rebuild with `cargo run -p au-app --features gui`, or use --selftest / process"
    );
    2
}

fn parse_effect(spec: &str) -> Result<Box<dyn Effect>, String> {
    let (name, arg) = spec.split_once(':').unwrap_or((spec, ""));
    let num = |d: f32| {
        if arg.is_empty() {
            Ok(d)
        } else {
            arg.parse::<f32>().map_err(|e| e.to_string())
        }
    };
    Ok(match name {
        "amplify" => Box::new(Amplify::new(num(0.0)?)),
        "normalize" => Box::new(Normalize {
            peak_db: num(-1.0)?,
            remove_dc: true,
        }),
        "fadein" => Box::new(FadeIn),
        "fadeout" => Box::new(FadeOut),
        "reverse" => Box::new(Reverse),
        other => return Err(format!("unknown effect {other}")),
    })
}

fn process(input: &str, output: &str, effects: &[String]) -> Result<String, String> {
    let buf = au_io::import_file(Path::new(input)).map_err(|e| e.to_string())?;
    let mut editor = Editor::new(ProjectState {
        tracks: vec![],
        sample_rate: buf.sample_rate,
        ..Default::default()
    });
    let frames = buf.len();
    editor.add_track(Track::new("input", buf));
    let sel = Selection {
        track: 0,
        start: 0,
        end: frames,
    };
    for spec in effects {
        let mut fx = parse_effect(spec)?;
        editor
            .apply(sel, spec, |b: &mut AudioBuffer| fx.process(b))
            .map_err(|e| e.to_string())?;
    }
    let out = &editor.state().tracks[0].render();
    let lower = output.to_ascii_lowercase();
    if lower.ends_with(".flac") {
        au_io::export_flac(Path::new(output), out, 16).map_err(|e| e.to_string())?;
    } else {
        au_io::export_wav(Path::new(output), out, 16).map_err(|e| e.to_string())?;
    }
    Ok(format!(
        "wrote {output}: {} frames, {} ch, {} Hz, peak {:.3}",
        out.len(),
        out.n_channels(),
        out.sample_rate,
        out.peak()
    ))
}

fn process_command(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "--out-dir") {
        return match parse_batch_args(args).and_then(run_batch) {
            Ok(result) => {
                println!("{}", result.message);
                if result.failed == 0 {
                    0
                } else {
                    1
                }
            }
            Err(error) => report(Err(error)),
        };
    }

    if args.len() < 2 {
        return report(Err(
            "process requires an input and output, or inputs with --out-dir DIR".into(),
        ));
    }

    report(process(&args[0], &args[1], &args[2..]))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputFormat {
    Wav,
    Flac,
}

impl OutputFormat {
    fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
        }
    }
}

#[derive(Debug)]
struct BatchArgs {
    inputs: Vec<PathBuf>,
    output_dir: PathBuf,
    format: OutputFormat,
    jobs: usize,
    effects: Vec<String>,
}

fn parse_batch_args(args: &[String]) -> Result<BatchArgs, String> {
    let mut inputs = Vec::new();
    let mut output_dir = None;
    let mut format = OutputFormat::Wav;
    let mut jobs = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let mut effects = Vec::new();
    let mut effects_started = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--out-dir" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--out-dir requires a directory".to_owned())?;
                output_dir = Some(PathBuf::from(value));
            }
            "--format" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--format requires wav or flac".to_owned())?;
                format = match value.as_str() {
                    "wav" => OutputFormat::Wav,
                    "flac" => OutputFormat::Flac,
                    _ => return Err(format!("unsupported format {value}; expected wav or flac")),
                };
            }
            "--jobs" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--jobs requires a positive integer".to_owned())?;
                jobs = value
                    .parse::<usize>()
                    .map_err(|_| format!("invalid job count {value}"))?;
                if jobs == 0 {
                    return Err("--jobs must be greater than zero".into());
                }
            }
            "--effects" => effects_started = true,
            "--" => effects_started = true,
            option if option.starts_with("--") => {
                return Err(format!("unknown process option {option}"));
            }
            value => {
                if effects_started || is_effect_spec(value) {
                    effects_started = true;
                    effects.push(value.to_owned());
                } else {
                    inputs.push(PathBuf::from(value));
                }
            }
        }
        index += 1;
    }

    let output_dir = output_dir.ok_or_else(|| "batch process requires --out-dir DIR".to_owned())?;
    if inputs.is_empty() {
        return Err("batch process requires at least one input file or directory".into());
    }

    Ok(BatchArgs {
        inputs,
        output_dir,
        format,
        jobs,
        effects,
    })
}

fn is_effect_spec(value: &str) -> bool {
    let name = value.split_once(':').map_or(value, |(name, _)| name);
    matches!(name, "amplify" | "normalize" | "fadein" | "fadeout" | "reverse")
}

#[derive(Debug)]
struct BatchResult {
    message: String,
    failed: usize,
}

#[derive(Debug)]
struct FileOutcome {
    line: String,
    succeeded: bool,
}

fn run_batch(args: BatchArgs) -> Result<BatchResult, String> {
    let files = expand_inputs(&args.inputs)?;
    let limiter = JobLimiter::new(args.jobs);
    let output_dir = args.output_dir;
    let format = args.format;
    let effects = args.effects;

    let outcomes = if files.is_empty() {
        Vec::new()
    } else {
        let exec = if args.jobs == 1 {
            au_runtime::compute::Exec::Serial
        } else {
            au_runtime::compute::Exec::Parallel
        };
        let cancel = au_runtime::pool::CancelToken::new();
        let progress = au_runtime::pool::Progress::new(files.len() as u64);

        au_effects::offline::runtime()
            .pool()
            .install(|| {
                au_runtime::compute::map_items(
                    exec,
                    &files,
                    &cancel,
                    &progress,
                    |input| {
                        let _permit = limiter.acquire();
                        Ok::<FileOutcome, au_runtime::pool::JobError>(process_batch_file(
                            input,
                            &output_dir,
                            format,
                            &effects,
                        ))
                    },
                )
            })
            .map_err(|error| error.to_string())?
    };

    let failed = outcomes.iter().filter(|outcome| !outcome.succeeded).count();
    let succeeded = outcomes.len() - failed;
    let mut message = outcomes
        .into_iter()
        .map(|outcome| outcome.line)
        .collect::<Vec<_>>()
        .join("\n");
    if !message.is_empty() {
        message.push('\n');
    }
    message.push_str(&format!(
        "summary: {succeeded} succeeded, {failed} failed"
    ));

    Ok(BatchResult { message, failed })
}

fn expand_inputs(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for input in inputs {
        if input.is_dir() {
            let entries = std::fs::read_dir(input)
                .map_err(|error| format!("cannot read directory {}: {error}", input.display()))?;
            let mut directory_files = Vec::new();
            for entry in entries {
                let entry = entry.map_err(|error| error.to_string())?;
                let path = entry.path();
                if path.is_file() {
                    directory_files.push(path);
                }
            }
            directory_files.sort();
            files.extend(directory_files);
        } else {
            files.push(input.clone());
        }
    }
    Ok(files)
}

fn process_batch_file(
    input: &Path,
    output_dir: &Path,
    format: OutputFormat,
    effects: &[String],
) -> FileOutcome {
    let output = output_dir.join(format!(
        "{}.{}",
        input
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("output"),
        format.extension()
    ));

    let result = (|| -> Result<AudioBuffer, String> {
        let mut buffer = au_io::import_file(input).map_err(|error| error.to_string())?;

        for spec in effects {
            let effect = parse_isolated_effect(spec)?;
            let mut context = au_effects::core::EffectContext::default();
            au_effects::offline::process_isolated(&mut IsolatedCliEffect { inner: effect }, &mut buffer, &mut context)
                .map_err(|error| format!("effect {spec}: {error}"))?;
        }

        std::fs::create_dir_all(output_dir).map_err(|error| error.to_string())?;
        match format {
            OutputFormat::Wav => au_io::export_wav(&output, &buffer, 16),
            OutputFormat::Flac => au_io::export_flac(&output, &buffer, 16),
        }
        .map_err(|error| error.to_string())?;

        Ok(buffer)
    })();

    match result {
        Ok(buffer) => FileOutcome {
            line: format!(
                "wrote {}: {} frames, {} ch, {} Hz, peak {:.3}",
                output.display(),
                buffer.len(),
                buffer.n_channels(),
                buffer.sample_rate,
                buffer.peak()
            ),
            succeeded: true,
        },
        Err(error) => FileOutcome {
            line: format!("failed {}: {error}", input.display()),
            succeeded: false,
        },
    }
}

enum CliEffect {
    Amplify(Amplify),
    Normalize(Normalize),
    FadeIn(FadeIn),
    FadeOut(FadeOut),
    Reverse(Reverse),
}

impl CliEffect {
    fn process(&mut self, buffer: &mut AudioBuffer) -> Result<(), String> {
        match self {
            Self::Amplify(effect) => effect.process(buffer),
            Self::Normalize(effect) => effect.process(buffer),
            Self::FadeIn(effect) => effect.process(buffer),
            Self::FadeOut(effect) => effect.process(buffer),
            Self::Reverse(effect) => effect.process(buffer),
        }
        Ok(())
    }
}

fn parse_isolated_effect(spec: &str) -> Result<CliEffect, String> {
    let (name, arg) = spec.split_once(':').unwrap_or((spec, ""));
    let num = |default: f32| {
        if arg.is_empty() {
            Ok(default)
        } else {
            arg.parse::<f32>().map_err(|error| error.to_string())
        }
    };

    Ok(match name {
        "amplify" => CliEffect::Amplify(Amplify::new(num(0.0)?)),
        "normalize" => CliEffect::Normalize(Normalize {
            peak_db: num(-1.0)?,
            remove_dc: true,
        }),
        "fadein" => CliEffect::FadeIn(FadeIn),
        "fadeout" => CliEffect::FadeOut(FadeOut),
        "reverse" => CliEffect::Reverse(Reverse),
        other => return Err(format!("unknown effect {other}")),
    })
}

struct IsolatedCliEffect {
    inner: CliEffect,
}

impl au_effects::core::BuiltinEffect for IsolatedCliEffect {
    fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed("BatchCliEffect") }

    fn params(&self) -> Vec<au_effects::core::ParamDef> {
        Vec::new()
    }

    fn get(&self, _key: &str) -> Option<f64> {
        None
    }

    fn set(&mut self, _key: &str, _value: f64) -> bool {
        false
    }

    fn process(
        &mut self,
        buffer: &mut AudioBuffer,
        _context: &mut au_effects::core::EffectContext,
    ) -> Result<(), au_effects::core::EffectError> {
        self.inner
            .process(buffer)
            .map_err(au_effects::core::EffectError::Invalid)
    }
}

struct JobLimiter {
    maximum: usize,
    active: std::sync::Mutex<usize>,
    available: std::sync::Condvar,
}

impl JobLimiter {
    fn new(maximum: usize) -> Self {
        Self {
            maximum: maximum.max(1),
            active: std::sync::Mutex::new(0),
            available: std::sync::Condvar::new(),
        }
    }

    fn acquire(&self) -> JobPermit<'_> {
        let mut active = self.active.lock().unwrap_or_else(|error| error.into_inner());
        while *active >= self.maximum {
            active = self
                .available
                .wait(active)
                .unwrap_or_else(|error| error.into_inner());
        }
        *active += 1;
        JobPermit { limiter: self }
    }
}

struct JobPermit<'a> {
    limiter: &'a JobLimiter,
}

impl Drop for JobPermit<'_> {
    fn drop(&mut self) {
        let mut active = self
            .limiter
            .active
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        *active -= 1;
        self.limiter.available.notify_one();
    }
}

fn selftest() -> Result<String, String> {
    let rate = 44_100;
    let tone: Vec<f32> = (0..rate)
        .map(|i| {
            (i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.25
        })
        .collect();
    let mut editor = Editor::new(ProjectState {
        tracks: vec![],
        sample_rate: rate as u32,
        ..Default::default()
    });
    editor.add_track(Track::new(
        "tone",
        AudioBuffer::from_channels(rate as u32, vec![tone]),
    ));
    let e = |x: au_core::CoreError| x.to_string();
    editor
        .cut(Selection {
            track: 0,
            start: 0,
            end: 1000,
        })
        .map_err(e)?;
    editor.paste(0, 0).map_err(e)?;
    editor
        .apply(
            Selection {
                track: 0,
                start: 0,
                end: rate,
            },
            "Normalize",
            |b| {
                Normalize {
                    peak_db: -1.0,
                    remove_dc: false,
                }
                .process(b)
            },
        )
        .map_err(e)?;
    editor.undo().map_err(e)?;
    editor.redo().map_err(e)?;
    let dir = std::env::temp_dir().join(format!("au-selftest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|x| x.to_string())?;
    let wav = dir.join("t.wav");
    au_io::export_wav(&wav, &editor.state().tracks[0].render(), 16)
        .map_err(|x| x.to_string())?;
    let back = au_io::import_file(&wav).map_err(|x| x.to_string())?;
    let proj = dir.join("t.aurs");
    au_project::save_project(&proj, editor.state()).map_err(|x| x.to_string())?;
    let loaded = au_project::load_project(&proj).map_err(|x| x.to_string())?;
    let _ = std::fs::remove_dir_all(&dir);
    if back.len() != rate || loaded.tracks.len() != 1 {
        return Err("selftest mismatch".into());
    }
    Ok(format!(
        "selftest ok: {} frames, peak {:.3} (expected ~0.891)",
        back.len(),
        back.peak()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_batch_arguments() {
        let args = [
            "one.wav",
            "two.wav",
            "--out-dir",
            "rendered",
            "--format",
            "flac",
            "--jobs",
            "2",
            "amplify:3",
            "reverse",
        ]
        .map(str::to_owned);

        let parsed = parse_batch_args(&args).unwrap();
        assert_eq!(
            parsed.inputs,
            vec![PathBuf::from("one.wav"), PathBuf::from("two.wav")]
        );
        assert_eq!(parsed.output_dir, PathBuf::from("rendered"));
        assert_eq!(parsed.format, OutputFormat::Flac);
        assert_eq!(parsed.jobs, 2);
        assert_eq!(parsed.effects, vec!["amplify:3", "reverse"]);
    }

    #[test]
    fn batch_wav_matches_single_file_output() {
        let directory = std::env::temp_dir().join(format!(
            "audacity-rs-batch-test-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();

        let input = directory.join("input.wav");
        let single_output = directory.join("single.wav");
        let batch_output_dir = directory.join("batch");
        let source = AudioBuffer::from_channels(
            44_100,
            vec![
                (0..257)
                    .map(|frame| (frame as f32 * 0.071).sin() * 0.5)
                    .collect(),
                (0..257)
                    .map(|frame| (frame as f32 * 0.053).cos() * 0.25)
                    .collect(),
            ],
        );
        au_io::export_wav(&input, &source, 16).unwrap();

        let effects = vec!["reverse".to_owned(), "fadein".to_owned()];
        process(
            input.to_str().unwrap(),
            single_output.to_str().unwrap(),
            &effects,
        )
        .unwrap();

        let batch_args = BatchArgs {
            inputs: vec![input.clone()],
            output_dir: batch_output_dir.clone(),
            format: OutputFormat::Wav,
            jobs: 2,
            effects,
        };
        let batch_result = run_batch(batch_args).unwrap();
        assert_eq!(batch_result.failed, 0);

        let single = au_io::import_file(&single_output).unwrap();
        let batch = au_io::import_file(&batch_output_dir.join("input.wav")).unwrap();
        assert_eq!(single.sample_rate, batch.sample_rate);
        assert_eq!(single.channels.len(), batch.channels.len());
        for (single_channel, batch_channel) in single.channels.iter().zip(&batch.channels) {
            assert_eq!(single_channel.len(), batch_channel.len());
            for (single_sample, batch_sample) in single_channel.iter().zip(batch_channel) {
                assert_eq!(single_sample.to_bits(), batch_sample.to_bits());
            }
        }

        let _ = std::fs::remove_dir_all(&directory);
    }
}
