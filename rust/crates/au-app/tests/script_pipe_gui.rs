#![cfg(all(unix, feature = "gui"))]

use std::fs::{self, File};
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct AppGuard {
    child: Child,
}

impl Drop for AppGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn script_pipe_paths() -> (PathBuf, PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = std::env::temp_dir().join(format!(
        "au-script-pipe-test-{}-{nonce}",
        std::process::id()
    ));

    let file = File::create(&temp_path).expect("create temporary file to read uid");
    let uid = file.metadata().expect("read temporary file metadata").uid();
    drop(file);
    let _ = fs::remove_file(temp_path);

    (
        PathBuf::from(format!("/tmp/audacity_script_pipe.to.{uid}")),
        PathBuf::from(format!("/tmp/audacity_script_pipe.from.{uid}")),
    )
}

fn wait_for_pipes(to_server: &PathBuf, from_server: &PathBuf, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        if to_server.exists() && from_server.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }

    to_server.exists() && from_server.exists()
}

fn wait_with_timeout(mut child: Child, timeout: Duration) -> Option<Output> {
    let deadline = Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[test]
fn gui_script_pipe_runs_python_batch() {
    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("skipping script-pipe GUI test: DISPLAY is not set");
        return;
    }

    if Command::new("python3").arg("--version").output().is_err() {
        eprintln!("skipping script-pipe GUI test: python3 is unavailable");
        return;
    }

    let Some(script_path) = std::env::var_os("AU_PIPE_TEST_PY").map(PathBuf::from) else {
        eprintln!("skipping script-pipe GUI test: AU_PIPE_TEST_PY is not set");
        return;
    };
    if !script_path.is_file() {
        eprintln!("skipping script-pipe GUI test: AU_PIPE_TEST_PY is not an existing file");
        return;
    }

    let (to_server, from_server) = script_pipe_paths();
    // Stale FIFOs from a killed instance would make the client connect too early.
    let _ = std::fs::remove_file(&to_server);
    let _ = std::fs::remove_file(&from_server);
    let app = Command::new(env!("CARGO_BIN_EXE_audacity-rs"))
        .env("AU_SCRIPT_PIPE", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("launch audacity-rs");
    let _app_guard = AppGuard { child: app };

    assert!(
        wait_for_pipes(&to_server, &from_server, Duration::from_secs(30)),
        "script-pipe endpoints did not appear within 30 seconds"
    );

    let python = Command::new("python3")
        .arg(script_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("launch python3 script");

    let output = wait_with_timeout(python, Duration::from_secs(30))
        .expect("python script timed out after 30 seconds");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "python script failed: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.matches("BatchCommand finished: OK").count() >= 2,
        "expected at least two successful batch responses; stdout was:\n{stdout}"
    );
}
