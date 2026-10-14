#![cfg(all(unix, feature = "gui"))]
//! Preferences over the pipe (GetPreference/SetPreference) and a runtime switch to
//! German: the rendered menu bar titles come from Audacity's de.po.

use au_scriptpipe::{PipeClient, PipeNames};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn po_dir() -> PathBuf {
    std::env::var_os("AU_TEST_PO_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/workspace/loc"))
}

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
            .env("AU_PREFS_PATH", dir.join("audacity.cfg"))
            .env("AU_LOCALE_DIR", po_dir())
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

    #[allow(dead_code)]
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

    #[allow(dead_code)]
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


fn titles(app: &mut App) -> String {
    app.ok("GetMenuTitles:")
}

fn wait_titles(app: &mut App, want: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let t = titles(app);
        if t.contains(want) || Instant::now() > deadline {
            return t;
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn preferences_and_runtime_german() {
    if !po_dir().join("de.po").exists() {
        eprintln!("skipping: no de.po in {}", po_dir().display());
        return;
    }
    let dir = TestDir::new();
    let mut app = App::launch(&dir.path);
    let t = wait_titles(&mut app, "File");
    assert!(t.contains("File") && t.contains("Edit"), "{t}");

    app.ok("SetPreference: Name=/Locale/Language Value=de");
    assert!(app.ok("GetPreference: Name=/Locale/Language").contains("de"));
    let t = wait_titles(&mut app, "Datei");
    assert!(t.contains("Datei") && t.contains("Bearbeiten") && t.contains("Ansicht"), "{t}");
    assert!(!t.contains("File"), "{t}");
    let cfg = fs::read_to_string(dir.path.join("audacity.cfg")).unwrap();
    assert!(cfg.contains("[Locale]") && cfg.contains("Language=de"), "{cfg}");

    app.ok("SetPreference: Name=/Locale/Language Value=en");
    let t = wait_titles(&mut app, "File");
    assert!(t.contains("File") && t.contains("Edit"), "{t}");
}
