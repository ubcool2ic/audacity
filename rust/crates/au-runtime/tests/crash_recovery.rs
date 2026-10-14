//! Injected-panic recovery: a child process installs the hook, registers an
//! autosave flush, makes an "edit" that was NOT yet autosaved, and panics on
//! the main thread. The parent checks the crash report and that the Stage 4
//! recovery path restores the edit.
use au_core::{AudioBuffer, ProjectState, Track};
use au_project::aup3::{self, Aup3Extras, Aup3Project};
use au_runtime::crash;
use std::sync::{Arc, Mutex};

fn project(gain: f32) -> Aup3Project {
    let mut t = Track::new("T", AudioBuffer::from_channels(44_100, vec![vec![0.25; 1000]]));
    t.gain = gain;
    Aup3Project { state: ProjectState { tracks: vec![t], sample_rate: 44_100, ..Default::default() }, extras: Aup3Extras::default() }
}

#[test]
fn injected_panic_writes_report_and_flushes_autosave() {
    if let Ok(dir) = std::env::var("AU_RT_CHILD") {
        let dir = std::path::PathBuf::from(dir);
        let file = dir.join("p.aup3");
        crash::install(crash::CrashConfig { dir: dir.join("crashes"), app_name: "audacity-rs".into(), version: "test-1".into() });
        crash::set_context("project", file.display().to_string());
        let live = Arc::new(Mutex::new(project(1.0)));
        aup3::save(&file, &live.lock().unwrap()).unwrap();
        let l2 = live.clone();
        let f2 = file.clone();
        crash::set_flush_hook(move || { if let Ok(p) = l2.try_lock() { let _ = aup3::autosave(&f2, &p); } });
        crash::log_action("Amplify");
        live.lock().unwrap().state.tracks[0].gain = 0.5; // unsaved edit
        // A contained worker panic must not produce a report.
        let rt = au_runtime::Runtime::new(2, 2);
        assert!(rt.submit(0, |_| -> Result<(), _> { panic!("worker boom") }).wait().is_err());
        assert!(crash::pending_reports(&dir.join("crashes")).is_empty());
        panic!("injected fatal panic");
    }
    let tmp = tempfile::tempdir().unwrap();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "injected_panic_writes_report_and_flushes_autosave", "--nocapture"])
        .env("AU_RT_CHILD", tmp.path())
        .output()
        .unwrap();
    assert!(!out.status.success(), "child should have panicked");
    let reports = crash::pending_reports(&tmp.path().join("crashes"));
    assert_eq!(reports.len(), 1, "{}", String::from_utf8_lossy(&out.stderr));
    let r = &reports[0];
    assert_eq!(r.message(), Some("injected fatal panic"));
    assert_eq!(r.project(), Some(tmp.path().join("p.aup3")));
    for needle in ["version: test-1", "os: ", "Amplify", "contained panic: worker boom", "-- backtrace --"] {
        assert!(r.text.contains(needle), "missing {needle}:\n{}", r.text);
    }
    // Next launch: Stage 4 recovery sees the flushed autosave with the edit.
    let file = r.project().unwrap();
    assert!(aup3::inspect(&file).unwrap().needs_recovery);
    assert_eq!(aup3::recover(&file).unwrap().state.tracks[0].gain, 0.5);
    assert!(!aup3::inspect(&file).unwrap().needs_recovery);
    crash::dismiss(r).unwrap();
    assert!(crash::pending_reports(&tmp.path().join("crashes")).is_empty());
}
