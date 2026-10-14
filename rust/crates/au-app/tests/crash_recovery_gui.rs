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
        let last_error = match PipeClient::connect(names, Duration::from_millis(250)) {
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
fn crash_dialog_and_recovery_reopen() {
    let Some(temp) = test_dir_or_skip("crash dialog and recovery reopen") else {
        return;
    };
    let wav = temp.path.join("recovery.wav");
    let project = temp.path.join("p.aup3");
    write_test_wav(&wav, 2.0).expect("write test WAV");

    let mut app = App::launch(&temp.path);
    app.ok(&format!(
        "Import2: Filename={}",
        filename_argument(&wav)
    ));
    app.ok(&format!(
        "SaveProject2: Filename={}",
        filename_argument(&project)
    ));
    assert!(project.exists(), "saved project file does not exist");

    app.ok("Select: Start=0 End=0.5 Track=0");
    app.ok("Delete");
    assert_track_ends(&mut app, &[1.5]);

    send_ticks_for(&mut app, Duration::from_secs(3));
    let _ = app.cmd_may_die("CrashTest");
    let status = app
        .wait_exit(Duration::from_secs(10))
        .expect("audacity-rs did not exit after CrashTest");
    assert!(
        !status.success(),
        "CrashTest process exited successfully: {status}"
    );

    let crash_dir = temp.path.join("crash");
    let crash_files = fs::read_dir(&crash_dir)
        .expect("crash directory exists")
        .flatten()
        .filter(|entry| entry.path().is_file())
        .count();
    assert!(crash_files > 0, "crash directory contains no report files");

    let mut app = App::launch(&temp.path);
    let dialogs = app.ok("GetDialogs");
    assert!(
        has_dialog_line(&dialogs, "crash_report\t"),
        "expected a crash_report dialog line; response:\n{dialogs}"
    );
    let recovery_line = dialogs
        .lines()
        .find(|line| line.starts_with("recovery\t"))
        .expect("expected a recovery dialog line");
    assert!(
        recovery_line.contains("p.aup3"),
        "recovery dialog does not name p.aup3: {recovery_line}"
    );

    app.ok("Recover: Action=Recover");
    assert_track_ends(&mut app, &[1.5]);
    app.ok("DismissCrashReport");
    let dialogs = app.ok("GetDialogs");
    assert!(
        !has_dialog_line(&dialogs, "crash_report\t"),
        "crash_report dialog remains after dismissal; response:\n{dialogs}"
    );
    drop(app);

    let mut app = App::launch(&temp.path);
    let dialogs = app.ok("GetDialogs");
    assert!(
        !has_dialog_line(&dialogs, "crash_report\t"),
        "dismissed crash report reappeared after relaunch; response:\n{dialogs}"
    );
}

#[test]
fn progress_dialog_cancel() {
    let Some(temp) = test_dir_or_skip("progress dialog cancel") else {
        return;
    };
    let mut app = App::launch(&temp.path);

    app.ok("ProgressTest: Seconds=20 Title=LongJob");
    send_ticks_for(&mut app, Duration::from_secs(1));

    let progress = app.ok("GetProgress");
    let (percent, status) =
        progress_status(&progress, "LongJob").expect("LongJob progress line is missing");
    assert_eq!(status, "Running", "expected LongJob to be running: {progress}");
    assert!(
        percent > 0.0 && percent < 100.0,
        "expected partial LongJob progress, got {percent}: {progress}"
    );

    let dialogs = app.ok("GetDialogs");
    assert!(
        has_dialog_line(&dialogs, "progress\tLongJob"),
        "expected a LongJob progress dialog; response:\n{dialogs}"
    );

    app.ok("CancelProgress");
    wait_for_progress_none(&mut app, Duration::from_secs(3));
}

#[test]
fn progress_job_completes() {
    let Some(temp) = test_dir_or_skip("progress job completes") else {
        return;
    };
    let mut app = App::launch(&temp.path);

    app.ok("ProgressTest: Seconds=1 Title=Short");
    wait_for_progress_none(&mut app, Duration::from_secs(5));
}
