#![cfg(all(unix, feature = "gui"))]

use au_scriptpipe::{PipeClient, PipeNames};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
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
        let tmp = dir.join("tmp");
        let _ = fs::create_dir_all(&tmp);
        // Each launch gets its own Xvfb server (xvfb-run -a picks a free display) and its
        // own TMPDIR/XDG/crash dirs, so parallel tests never share an X display or state.
        let mut child = Command::new("xvfb-run")
            .args(["-a", "-s", "-screen 0 1280x1024x24 -nolisten tcp"])
            .arg(env!("CARGO_BIN_EXE_audacity-rs"))
            .process_group(0)
            .env_remove("DISPLAY")
            .env("TMPDIR", &tmp)
            .env("XDG_RUNTIME_DIR", &tmp)
            .env("XDG_CACHE_HOME", dir.join("cache"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_DATA_HOME", dir.join("data"))
            .env("AUDACITY_RS_CRASH_DIR", dir.join("crash"))
            .env("AU_SCRIPT_PIPE", "1")
            .env("AU_SCRIPT_PIPE_DIR", dir)
            .env("AU_PREFS_PATH", std::path::Path::new(&dir).join("audacity.cfg"))
            .env("AU_TOOLBARS_CFG", dir.join("toolbars.cfg"))
            .env("AU_THEME_DIR", dir.join("theme"))
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

    fn kill9(mut self) {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.child.id())])
            .status();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.child.id())])
            .status();
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
        let last_error = match PipeClient::connect(names, Duration::from_secs(10)) {
            Ok(client) => return client,
            Err(error) => error.to_string(),
        };

        if Instant::now() >= deadline {
            let _ = Command::new("kill")
                .args(["-KILL", "--", &format!("-{}", child.id())])
                .status();
            let _ = child.kill();
            let _ = child.wait();
            panic!("could not connect to script pipes within 20 seconds: {last_error}");
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn test_dir_or_skip(test_name: &str) -> Option<TestDir> {
    let have_xvfb = Command::new("sh")
        .args(["-c", "command -v xvfb-run >/dev/null"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !have_xvfb {
        eprintln!("skipping {test_name}: xvfb-run is not installed");
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
        .take_while(|character| {
            matches!(
                character,
                '-' | '+' | '.' | '0'..='9' | 'e' | 'E'
            )
        })
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

fn assert_failed(app: &mut App, command: &str) -> String {
    let response = app.cmd(command);
    assert!(
        response
            .trim_end()
            .ends_with("BatchCommand finished: Failed!"),
        "command {command:?} did not fail; response:\n{response}"
    );
    response
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

#[test]
fn editing_workflow() {
    let Some(temp) = test_dir_or_skip("editing workflow") else {
        return;
    };
    let wav = temp.path.join("editing.wav");
    write_test_wav(&wav, 2.0).expect("write test WAV");
    let mut app = App::launch(&temp.path);

    app.ok(&format!(
        "Import2: Filename={}",
        filename_argument(&wav)
    ));
    assert_track_ends(&mut app, &[2.0]);

    app.ok("Select: Start=0.5 End=1.0 Track=0");
    app.ok("Cut");
    assert_track_ends(&mut app, &[1.5]);

    app.ok("Undo");
    assert_track_ends(&mut app, &[2.0]);

    app.ok("Redo");
    assert_track_ends(&mut app, &[1.5]);

    app.ok("SelectAll");
    app.ok("Copy");
    app.ok("Select: Start=0 End=0");
    app.ok("Paste");
    assert_track_ends(&mut app, &[3.0]);

    app.ok("Select: Start=0 End=1 Track=0");
    app.ok("Delete");
    assert_track_ends(&mut app, &[2.0]);
}

#[test]
fn import_export_workflow() {
    let Some(temp) = test_dir_or_skip("import/export workflow") else {
        return;
    };
    let wav = temp.path.join("source.wav");
    let out_wav = temp.path.join("out.wav");
    let out_flac = temp.path.join("out.flac");
    let missing = temp.path.join("missing.wav");
    write_test_wav(&wav, 1.0).expect("write test WAV");

    let mut app = App::launch(&temp.path);
    app.ok(&format!(
        "Import2: Filename={}",
        filename_argument(&wav)
    ));
    app.ok(&format!(
        "Export2: Filename={}",
        filename_argument(&out_wav)
    ));
    app.ok(&format!(
        "Export2: Filename={}",
        filename_argument(&out_flac)
    ));

    for path in [&out_wav, &out_flac] {
        let metadata = fs::metadata(path).expect("exported file exists");
        assert!(
            metadata.len() > 44,
            "exported file {path:?} is unexpectedly small: {} bytes",
            metadata.len()
        );
    }

    app.ok(&format!(
        "Import2: Filename={}",
        filename_argument(&out_wav)
    ));
    let ends = track_ends(&mut app);
    assert_eq!(ends.len(), 2, "expected two tracks, got {ends:?}");
    for end in ends {
        assert!(
            (end - 1.0).abs() <= 0.01,
            "expected imported track duration 1.0 s, got {end:.6} s"
        );
    }

    assert_failed(
        &mut app,
        &format!(
            "Import2: Filename={}",
            filename_argument(&missing)
        ),
    );
}

#[test]
fn save_and_crash_recovery_workflow() {
    let Some(temp) = test_dir_or_skip("save and crash recovery workflow") else {
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

    send_ticks_for(&mut app, Duration::from_millis(3_500));
    app.kill9();

    let mut app = App::launch(&temp.path);
    let response = app.cmd(&format!(
        "OpenProject2: Filename={}",
        filename_argument(&project)
    ));
    assert!(
        response.contains("Recovery available"),
        "expected recovery availability in response, got:\n{response}"
    );
    app.ok("Recover: Action=Recover");
    assert_track_ends(&mut app, &[1.5]);
}

#[test]
fn recording_workflow() {
    let Some(temp) = test_dir_or_skip("recording workflow") else {
        return;
    };
    let mut app = App::launch(&temp.path);

    let response = app.cmd("Record1stChoice");
    if !response
        .trim_end()
        .ends_with("BatchCommand finished: OK")
    {
        eprintln!("skipping recording workflow: audio input unavailable: {response}");
        return;
    }

    send_ticks_for(&mut app, Duration::from_secs(1));
    app.ok("Stop");
    let ends = track_ends(&mut app);
    assert!(
        !ends.is_empty(),
        "recording succeeded but no wave tracks were reported"
    );
}

#[test]
fn script_errors_are_reported() {
    let Some(temp) = test_dir_or_skip("script error workflow") else {
        return;
    };
    let mut app = App::launch(&temp.path);

    assert_failed(&mut app, "Bogus: X=1");
    assert_failed(&mut app, "Select: Start=0 End=1 Track=7");
    app.ok("Message: Text=still-alive");
}
