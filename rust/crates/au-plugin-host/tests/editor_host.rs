use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use au_plugin_api::PluginDescriptor;
use au_plugin_host::editor::{EditorEvent, EditorProcess};

static LOCK: Mutex<()> = Mutex::new(());

fn setup(path: &Path) -> Option<MutexGuard<'static, ()>> {
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("AU_PLUGIN_EDITOR", env!("CARGO_BIN_EXE_au-plugin-editor"));

    if std::env::var_os("DISPLAY").is_none() {
        eprintln!("skip: DISPLAY is unset");
        return None;
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
    process
        .wait_event(timeout, |event| matches!(event, EditorEvent::Ready { .. }))
        .expect("editor did not become ready")
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
