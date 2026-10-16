use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use au_plugin_api::PluginDescriptor;
use au_plugin_host::editor::{EditorEvent, EditorProcess};

static LOCK: Mutex<()> = Mutex::new(());

fn setup(path: &Path) -> Option<MutexGuard<'static, ()>> {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("AU_PLUGIN_EDITOR", env!("CARGO_BIN_EXE_au-plugin-editor"));

    let Some(display) = std::env::var_os("DISPLAY") else {
        eprintln!("skip: DISPLAY is unset");
        return None;
    };

    if let Ok(status) = std::process::Command::new("xdpyinfo")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        if !status.success() {
            eprintln!(
                "skip: X display {} is not reachable",
                display.to_string_lossy()
            );
            return None;
        }
    }

    if !path.exists() {
        eprintln!("skip: plugin file is missing: {}", path.display());
        return None;
    }

    Some(guard)
}

fn again_path() -> PathBuf {
    PathBuf::from("/workspace/plugins/vst3sdk/build-gui/VST3/Release/again.vst3")
}

fn surge_path() -> PathBuf {
    PathBuf::from("/workspace/plugins/surge/Surge XT Effects.clap")
}

fn again_descriptor(path: &Path) -> PluginDescriptor {
    au_plugin_host::vst3::scan_file(path)
        .unwrap()
        .into_iter()
        .find(|descriptor| descriptor.name == "AGain VST3")
        .expect("AGain VST3 descriptor not found")
}

fn ready(process: &mut EditorProcess, timeout: Duration) -> EditorEvent {
    match process.wait_event(timeout, |event| {
        matches!(
            event,
            EditorEvent::Ready { .. }
                | EditorEvent::Error(_)
                | EditorEvent::Crashed { .. }
                | EditorEvent::Closed
        )
    }) {
        Some(event @ EditorEvent::Ready { .. }) => event,
        Some(event) => panic!("editor failed before ready: {event:?}"),
        None => panic!(
            "editor did not become ready within {timeout:?} (running: {})",
            process.is_running()
        ),
    }
}

fn state(process: &mut EditorProcess, timeout: Duration) -> Vec<(String, String)> {
    match process
        .wait_event(timeout, |event| matches!(event, EditorEvent::State(_)))
        .expect("editor did not return state")
    {
        EditorEvent::State(state) => state,
        _ => unreachable!(),
    }
}

#[test]
fn again_editor_opens_out_of_process() {
    let path = again_path();
    let Some(_guard) = setup(&path) else {
        return;
    };

    let descriptor = again_descriptor(&path);
    let gain_id = descriptor
        .params
        .iter()
        .find(|param| param.name == "Gain")
        .expect("Gain parameter not found")
        .id;

    let mut process =
        EditorProcess::spawn(&descriptor, &[("Parameters".into(), "0=0.500000;".into())])
            .unwrap();

    let event = ready(&mut process, Duration::from_secs(20));
    match &event {
        EditorEvent::Ready {
            width,
            height,
            window,
        } => {
            assert!(*width > 0);
            assert!(*height > 0);
            assert_ne!(*window, 0);
        }
        _ => unreachable!(),
    }

    process.request_state().unwrap();
    let first_state = state(&mut process, Duration::from_secs(10));
    assert!(first_state.iter().any(|(key, _)| key == "ProcessorState"));
    assert!(first_state
        .iter()
        .any(|(key, value)| key == "Parameters" && value.starts_with("0=0.5")));

    process.set_param(gain_id, 0.25).unwrap();
    process.request_state().unwrap();
    let updated_state = state(&mut process, Duration::from_secs(10));
    assert!(updated_state
        .iter()
        .any(|(key, value)| key == "Parameters" && value.starts_with("0=0.25")));

    process.close();
    println!("{event:?}");
}

#[test]
fn surge_editor_opens_out_of_process() {
    let path = surge_path();
    let Some(_guard) = setup(&path) else {
        return;
    };

    let descriptor = au_plugin_host::clap::scan_file(&path)
        .unwrap()
        .into_iter()
        .next()
        .expect("Surge CLAP descriptor not found");

    let mut process = EditorProcess::spawn(&descriptor, &[]).unwrap();
    let event = ready(&mut process, Duration::from_secs(30));
    assert!(matches!(event, EditorEvent::Ready { .. }));

    process.request_state().unwrap();
    let returned_state = state(&mut process, Duration::from_secs(10));
    assert!(returned_state.iter().any(|(key, _)| key == "State"));

    process.close();
}

#[test]
fn crashing_editor_is_contained() {
    let path = again_path();
    let Some(_guard) = setup(&path) else {
        return;
    };

    let descriptor = again_descriptor(&path);
    let mut process = EditorProcess::spawn_with_env(
        &descriptor,
        &[],
        &[("AU_EDITOR_TEST_CRASH_AFTER_MS", "300")],
    )
    .unwrap();

    let _ = ready(&mut process, Duration::from_secs(20));
    let event = process
        .wait_event(Duration::from_secs(15), |event| {
            matches!(event, EditorEvent::Crashed { .. })
        })
        .expect("editor crash was not reported");

    match &event {
        EditorEvent::Crashed { status } => {
            #[cfg(unix)]
            assert!(status.contains("signal 11"), "unexpected crash status: {status}");
        }
        _ => unreachable!(),
    }
    assert!(!process.is_running());
    process.close();

    let mut reopened = EditorProcess::spawn(&descriptor, &[]).unwrap();
    let _ = ready(&mut reopened, Duration::from_secs(20));
    reopened.close();

    println!("{event:?}");
}

/// Parent window for embedding tests: AU_TEST_PARENT_XID if set, else the X root window.
fn parent_xid() -> Option<u64> {
    if let Some(xid) = std::env::var("AU_TEST_PARENT_XID").ok().and_then(|v| {
        v.strip_prefix("0x").map_or_else(|| v.parse().ok(), |h| u64::from_str_radix(h, 16).ok())
    }) {
        return Some(xid);
    }
    let out = std::process::Command::new("xwininfo").arg("-root").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let hex = text.split("Window id: 0x").nth(1)?.split_whitespace().next()?;
    u64::from_str_radix(hex, 16).ok()
}

fn embedded_flag(process: &mut EditorProcess) -> bool {
    match process.wait_event(Duration::from_secs(5), |e| matches!(e, EditorEvent::Embedded(_))) {
        Some(EditorEvent::Embedded(flag)) => flag,
        other => panic!("no EMBED report: {other:?}"),
    }
}

#[test]
fn again_editor_embeds_into_parent_window() {
    let path = again_path();
    let Some(_guard) = setup(&path) else { return };
    let Some(xid) = parent_xid() else { return };
    let descriptor = again_descriptor(&path);
    let mut process = EditorProcess::spawn_embedded(&descriptor, &[], xid, 20, 40).unwrap();
    let _ = ready(&mut process, Duration::from_secs(20));
    assert!(embedded_flag(&mut process));
    process.move_to(30, 50).unwrap();
    assert!(process.is_running());
    process.close();
}

#[test]
fn surge_editor_embeds_into_parent_window() {
    let path = surge_path();
    let Some(_guard) = setup(&path) else { return };
    let Some(xid) = parent_xid() else { return };
    let descriptor = au_plugin_host::clap::scan_file(&path).unwrap().into_iter().next().unwrap();
    let mut process = EditorProcess::spawn_embedded(&descriptor, &[], xid, 0, 0).unwrap();
    let _ = ready(&mut process, Duration::from_secs(30));
    assert!(embedded_flag(&mut process));
    process.close();
}

#[test]
fn invalid_parent_falls_back_to_top_level_window() {
    let path = again_path();
    let Some(_guard) = setup(&path) else { return };
    let descriptor = again_descriptor(&path);
    // Not a window on this display: the child must fall back instead of aborting on BadWindow.
    let mut process = EditorProcess::spawn_embedded(&descriptor, &[], 0x7ff_fff1, 0, 0).unwrap();
    let _ = ready(&mut process, Duration::from_secs(20));
    assert!(!embedded_flag(&mut process));
    process.close();
}
