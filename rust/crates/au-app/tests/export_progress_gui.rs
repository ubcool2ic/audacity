//! Xvfb: the export progress bar advances while File > Export Audio runs.
#![allow(dead_code)]
#![cfg(all(unix, feature = "gui"))]

use au_scriptpipe::{PipeClient, PipeNames};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_DIR_ID: AtomicU64 = AtomicU64::new(0);

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let id = NEXT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "au-ui-automation-{}-{nonce}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create temporary test directory");
        fs::create_dir(path.join("theme")).expect("create temporary theme directory");
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct App {
    child: Child,
    client: PipeClient,
}

impl App {
    fn launch(dir: &Path) -> Self {
        remove_stale_pipes(dir);

        let names = PipeNames::in_dir(dir, "audacity_script_pipe");
        let mut child = Command::new(env!("CARGO_BIN_EXE_audacity-rs"))
            .env("AU_SCRIPT_PIPE", "1")
            .env("AU_SCRIPT_PIPE_DIR", dir)
            .env("AU_PREFS_PATH", std::path::Path::new(&dir).join("audacity.cfg"))
            .env("AU_TOOLBARS_CFG", dir.join("toolbars.cfg"))
            .env("AU_THEME_DIR", dir.join("theme"))
            .env("AUDACITY_RS_CRASH_DIR", dir.join("crash"))
            .env("AU_ALLOW_TEST_HOOKS", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("launch audacity-rs");

        let client = connect_with_retry(&names, &mut child);
        Self { child, client }
    }

    fn cmd(&mut self, command: &str) -> String {
        self.client
            .command(command)
            .unwrap_or_else(|error| panic!("command {command:?} failed: {error}"))
    }

    fn cmd_may_die(&mut self, command: &str) -> Option<String> {
        self.client.command(command).ok()
    }

    fn ok(&mut self, command: &str) -> String {
        let response = self.cmd(command);
        assert!(
            response
                .trim_end()
                .ends_with("BatchCommand finished: OK"),
            "command {command:?} did not succeed; response:\n{response}"
        );
        response
    }

    fn wait_exit(&mut self, timeout: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .expect("check whether audacity-rs exited")
            {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn remove_stale_pipes(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("audacity_script_pipe") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn connect_with_retry(names: &PipeNames, child: &mut Child) -> PipeClient {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let last_error = match PipeClient::connect(names, Duration::from_secs(120)) {
            Ok(client) => return client,
            Err(error) => error.to_string(),
        };

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("could not connect to script pipes within 20 seconds: {last_error}");
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn test_dir_or_skip(test_name: &str) -> Option<TestDir> {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("skipping {test_name}: DISPLAY is not set");
        None
    } else {
        Some(TestDir::new())
    }
}

fn filename_argument(path: &Path) -> String {
    let path = path.to_str().expect("path is valid UTF-8");
    if path.contains(' ') {
        format!("\"{path}\"")
    } else {
        path.to_owned()
    }
}

fn write_test_wav(path: &Path, seconds: f64) -> io::Result<()> {
    const SAMPLE_RATE: u32 = 44_100;
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;

    let sample_count = (f64::from(SAMPLE_RATE) * seconds).round() as u32;
    let data_size = sample_count * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE) / 8;
    let byte_rate = SAMPLE_RATE * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE) / 8;
    let block_align = CHANNELS * BITS_PER_SAMPLE / 8;

    let mut file = File::create(path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36_u32 + data_size).to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16_u32.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&CHANNELS.to_le_bytes())?;
    file.write_all(&SAMPLE_RATE.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&BITS_PER_SAMPLE.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_size.to_le_bytes())?;

    for sample_index in 0..sample_count {
        let phase = std::f64::consts::TAU * 1_000.0 * f64::from(sample_index)
            / f64::from(SAMPLE_RATE);
        let sample = (phase.sin() * 16_000.0).round() as i16;
        file.write_all(&sample.to_le_bytes())?;
    }

    Ok(())
}

fn json_string_field<'a>(object: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!("\"{key}\"");
    let value = object.split_once(&marker)?.1.split_once(':')?.1.trim_start();
    let value = value.strip_prefix('"')?;
    value.split('"').next()
}

fn json_number_field(object: &str, key: &str) -> Option<f64> {
    let marker = format!("\"{key}\"");
    let value = object.split_once(&marker)?.1.split_once(':')?.1.trim_start();
    let number: String = value
        .chars()
        .take_while(|character| matches!(character, '-' | '+' | '.' | '0'..='9' | 'e' | 'E'))
        .collect();
    number.parse().ok()
}

fn track_ends(app: &mut App) -> Vec<f64> {
    let response = app.ok("GetInfo: Type=Tracks");
    response
        .split('{')
        .skip(1)
        .filter_map(|chunk| chunk.split_once('}').map(|(object, _)| object))
        .filter(|object| json_string_field(object, "kind") == Some("wave"))
        .filter_map(|object| json_number_field(object, "end"))
        .collect()
}

fn assert_track_ends(app: &mut App, expected: &[f64]) {
    let actual = track_ends(app);
    assert_eq!(
        actual.len(),
        expected.len(),
        "expected track ends {expected:?}, got {actual:?}"
    );
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(
            (actual - expected).abs() <= 0.01,
            "expected track end {expected:.6}, got {actual:.6}; all ends: {actual:?}"
        );
    }
}

fn send_ticks_for(app: &mut App, duration: Duration) {
    let deadline = Instant::now() + duration;
    let mut next_tick = Instant::now();

    while Instant::now() < deadline {
        app.ok("Message: Text=tick");
        next_tick += Duration::from_millis(250);
        let now = Instant::now();
        if now < deadline && now < next_tick {
            thread::sleep((next_tick - now).min(deadline - now));
        }
    }
}

fn has_dialog_line(response: &str, prefix: &str) -> bool {
    response
        .lines()
        .any(|line| line.starts_with(prefix))
}

fn progress_status<'a>(response: &'a str, title: &str) -> Option<(f64, &'a str)> {
    let prefix = format!("{title}\t");
    let line = response.lines().map(str::trim).find(|line| line.starts_with(&prefix))?;
    let mut columns = line.split('\t');
    columns.next()?;
    let percent = columns.next()?.parse().ok()?;
    let status = columns.next()?.trim();
    Some((percent, status))
}

fn wait_for_progress_none(app: &mut App, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        app.ok("Message: Text=tick");
        let response = app.ok("GetProgress");
        if response.lines().any(|line| line.trim() == "none") {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "progress job was not removed within {timeout:?}; response:\n{response}"
        );
        thread::sleep(Duration::from_millis(250).min(deadline.saturating_duration_since(Instant::now())));
    }
}

#[test]
fn export_progress_bar_moves() {
    let Some(temp) = test_dir_or_skip("export progress bar moves") else {
        return;
    };
    let wav = temp.path.join("long.wav");
    write_test_wav(&wav, 240.0).expect("write test WAV");
    let mut app = App::launch(&temp.path);
    app.ok(&format!("Import2: Filename={}", filename_argument(&wav)));
    wait_for_progress_none(&mut app, Duration::from_secs(120));

    let out = temp.path.join("out.mp3");
    app.ok(&format!("ExportAudioTest: Filename={} Format=MP3", filename_argument(&out)));
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        app.ok("Message: Text=tick");
        let response = app.ok("GetProgress");
        if response.lines().any(|l| l.trim() == "none") {
            break;
        }
        if let Some((percent, status)) = progress_status(&response, "Export Audio") {
            assert_eq!(status, "Running", "{response}");
            seen.push(percent);
        }
        assert!(Instant::now() < deadline, "export did not finish: {seen:?}");
        thread::sleep(Duration::from_millis(100));
    }
    assert!(seen.windows(2).all(|w| w[1] >= w[0]), "progress went backwards: {seen:?}");
    let distinct: Vec<f64> = seen.iter().copied().filter(|p| *p > 0.0 && *p < 100.0).collect::<Vec<_>>();
    let mut uniq = distinct.clone();
    uniq.dedup();
    assert!(uniq.len() >= 3, "expected the bar to move through several partial values: {seen:?}");
    assert!(std::fs::metadata(&out).map(|m| m.len() > 0).unwrap_or(false), "no MP3 written");
}
