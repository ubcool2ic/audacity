#![cfg(all(unix, feature = "gui"))]

use au_scriptpipe::{PipeClient, PipeNames};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct AppGuard {
    child: Option<Child>,
    dir: PathBuf,
}

impl Drop for AppGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn write_test_wav(path: &Path) -> io::Result<()> {
    const SAMPLE_RATE: u32 = 44_100;
    const DURATION_SECONDS: u32 = 2;
    const CHANNELS: u16 = 1;
    const BITS_PER_SAMPLE: u16 = 16;

    let sample_count = SAMPLE_RATE * DURATION_SECONDS;
    let data_size = sample_count * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE) / 8;
    let byte_rate = SAMPLE_RATE * u32::from(CHANNELS) * u32::from(BITS_PER_SAMPLE) / 8;
    let block_align = CHANNELS * BITS_PER_SAMPLE / 8;

    let mut file = File::create(path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_size).to_le_bytes())?;
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

fn connect_with_retry(names: &PipeNames) -> PipeClient {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let last_error = match PipeClient::connect(names, Duration::from_millis(250)) {
            Ok(client) => return client,
            Err(error) => Some(error),
        };

        if Instant::now() >= deadline {
            panic!(
                "could not connect to script pipes within 20 seconds: {}",
                last_error
                    .map(|error| error.to_string())
                    .unwrap_or_else(|| "no connection attempt was made".to_owned())
            );
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn assert_command_ends_with(client: &mut PipeClient, command: &str, expected: &str) -> String {
    let response = client
        .command(command)
        .unwrap_or_else(|error| panic!("command {command:?} failed: {error}"));
    assert!(
        response.trim_end().ends_with(expected),
        "command {command:?} expected response ending with {expected:?}, got:\n{response}"
    );
    response
}

#[test]
fn gui_spectrogram_rendering_survives_script_commands() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("skipping spectrogram GUI test: DISPLAY is not set");
        return;
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "au-spectrogram-gui-test-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&dir).expect("create temporary test directory");
    let mut guard = AppGuard {
        child: None,
        dir: dir.clone(),
    };

    let wav_path = dir.join("spectrogram_gui_test.wav");
    write_test_wav(&wav_path).expect("write test WAV file");

    let child = Command::new(env!("CARGO_BIN_EXE_audacity-rs"))
        .env("AU_SCRIPT_PIPE", "1")
        .env("AU_SCRIPT_PIPE_DIR", &dir)
            .env("AU_PREFS_PATH", std::path::Path::new(&&dir).join("audacity.cfg"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("launch audacity-rs");
    guard.child = Some(child);

    let names = PipeNames::in_dir(&dir, "audacity_script_pipe");
    let mut client = connect_with_retry(&names);
    let wav_string = wav_path.to_str().expect("WAV path is valid UTF-8");

    assert_command_ends_with(
        &mut client,
        &format!("Import2: Filename={wav_string}"),
        "BatchCommand finished: OK",
    );
    assert_command_ends_with(
        &mut client,
        "SetTrackVisuals: Track=0 Display=Spectrogram",
        "BatchCommand finished: OK",
    );
    assert_command_ends_with(
        &mut client,
        "SetTrackVisuals: Track=5 Display=Spectrogram",
        "BatchCommand finished: Failed!",
    );
    assert_command_ends_with(
        &mut client,
        "SetTrackVisuals: Track=0 Display=Bogus",
        "BatchCommand finished: Failed!",
    );

    thread::sleep(Duration::from_millis(1_500));
    let track_info = assert_command_ends_with(
        &mut client,
        "GetInfo: Type=Tracks",
        "BatchCommand finished: OK",
    );
    let wav_name = wav_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("WAV filename is valid UTF-8");
    assert!(
        track_info.contains(wav_name),
        "track information did not contain {wav_name:?}:\n{track_info}"
    );

    assert_command_ends_with(
        &mut client,
        "SetTrackVisuals: Track=0 Display=Multiview SpecScale=Linear",
        "BatchCommand finished: OK",
    );
    thread::sleep(Duration::from_secs(1));

    let response = client
        .command("Message: Text=alive")
        .expect("send alive message");
    assert!(
        response.starts_with("alive"),
        "expected alive response after rendering, got:\n{response}"
    );
}
