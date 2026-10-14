//! `audacity-rs a.wav b.wav` imports the files at startup (like Audacity).
#![cfg(feature = "gui")]
use au_scriptpipe::{PipeClient, PipeNames};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn write_wav(path: &std::path::Path, n: usize) {
    let buf = au_core::AudioBuffer::from_channels(44_100, vec![(0..n).map(|i| (i as f32 * 0.01).sin() * 0.3).collect()]);
    au_io::export_wav(path, &buf, 16).unwrap();
}

#[test]
fn wav_arguments_are_imported_at_startup() {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        eprintln!("no display; skipping");
        return;
    }
    let dir = std::env::temp_dir().join(format!("cli-open-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("theme")).unwrap();
    let (a, b) = (dir.join("first.wav"), dir.join("second.wav"));
    write_wav(&a, 44_100);
    write_wav(&b, 22_050);
    let names = PipeNames::in_dir(&dir, "audacity_script_pipe");
    let mut child = Command::new(env!("CARGO_BIN_EXE_audacity-rs"))
        .arg(&a)
        .arg(&b)
        .env("AU_SCRIPT_PIPE", "1")
        .env("AU_SCRIPT_PIPE_DIR", &dir)
        .env("AU_TOOLBARS_CFG", dir.join("toolbars.cfg"))
        .env("AU_THEME_DIR", dir.join("theme"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut client = loop {
        if let Ok(c) = PipeClient::connect(&names, Duration::from_millis(250)) {
            break c;
        }
        assert!(Instant::now() < deadline, "pipe did not come up");
    };
    let mut info = String::new();
    while Instant::now() < deadline {
        info = client.command("GetInfo: Type=Tracks").unwrap();
        if info.contains("first") && info.contains("second") {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(info.contains("first") && info.contains("second"), "{info}");
}
