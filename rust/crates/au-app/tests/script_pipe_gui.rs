#![cfg(all(unix, feature = "gui"))]

use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct TempDirGuard {
    path: PathBuf,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct AppGuard {
    child: Child,
}

impl Drop for AppGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn escaped_python_path(path: &std::path::Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('\'', "\\'")
}

fn rewrite_pipe_test_script(script_path: &std::path::Path, to_path: &std::path::Path, from_path: &std::path::Path) -> String {
    let script = fs::read_to_string(script_path).expect("read pipe_test.py");
    let to_path = escaped_python_path(to_path);
    let from_path = escaped_python_path(from_path);
    let mut replaced_to = false;
    let mut replaced_from = false;
    let mut rewritten = String::with_capacity(script.len());

    for line in script.split_inclusive('\n') {
        let body = line.strip_suffix('\n').unwrap_or(line);
        let indentation = body.len() - body.trim_start().len();
        let trimmed = body.trim_start();
        let line_ending = if line.ends_with('\n') { "\n" } else { "" };

        if trimmed.starts_with("TONAME = '/tmp/") {
            rewritten.push_str(&body[..indentation]);
            rewritten.push_str("TONAME = '");
            rewritten.push_str(&to_path);
            rewritten.push('\'');
            rewritten.push_str(line_ending);
            replaced_to = true;
        } else if trimmed.starts_with("FROMNAME = '/tmp/") {
            rewritten.push_str(&body[..indentation]);
            rewritten.push_str("FROMNAME = '");
            rewritten.push_str(&from_path);
            rewritten.push('\'');
            rewritten.push_str(line_ending);
            replaced_from = true;
        } else {
            rewritten.push_str(line);
        }
    }

    assert!(replaced_to, "pipe_test.py TONAME assignment was not replaced");
    assert!(
        replaced_from,
        "pipe_test.py FROMNAME assignment was not replaced"
    );

    rewritten
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

    let script_path: PathBuf = std::env::var_os("AU_PIPE_TEST_PY")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../au3/scripts/piped-work/pipe_test.py"
            ))
        });
    if !script_path.is_file() {
        panic!("script-pipe GUI test script is not a file: {}", script_path.display());
    }

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_dir = std::env::temp_dir().join(format!(
        "au-script-pipe-test-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&temp_dir).expect("create temporary script-pipe directory");
    let _temp_dir_guard = TempDirGuard {
        path: temp_dir.clone(),
    };

    let to_server = temp_dir.join("audacity_script_pipe.to");
    let from_server = temp_dir.join("audacity_script_pipe.from");

    let app = Command::new(env!("CARGO_BIN_EXE_audacity-rs"))
        .env("AU_SCRIPT_PIPE", "1")
        .env("AU_SCRIPT_PIPE_DIR", &temp_dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("launch audacity-rs");
    let _app_guard = AppGuard { child: app };

    assert!(
        wait_for_pipes(&to_server, &from_server, Duration::from_secs(30)),
        "script-pipe endpoints did not appear within 30 seconds"
    );

    let rewritten_script = rewrite_pipe_test_script(&script_path, &to_server, &from_server);
    let copied_script_path = temp_dir.join("pipe_test.py");
    fs::write(&copied_script_path, rewritten_script).expect("write rewritten pipe_test.py");

    let python = Command::new("python3")
        .arg(copied_script_path)
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
