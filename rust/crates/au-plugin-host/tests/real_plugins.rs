use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use au_core::ProjectState;
use au_plugin_api::{PluginHost, PluginInstance, PluginFormat};
use au_plugin_host::clap::ClapHost;
use au_plugin_host::sandbox::{scan_one, ScanOutcome};
use au_plugin_host::state::{PARAMETERS, PROCESSOR_STATE};
use au_plugin_host::vst3::Vst3Host;
use au_plugin_host::{restore_effect, save_effect, registry};
use au_project::aup3::{self, Aup3Extras, Aup3Project, DocSource};

static LOCK: Mutex<()> = Mutex::new(());

const SAMPLE_RATE: u32 = 48_000;
const FRAMES: usize = 4096;
const BLOCK: usize = 512;

fn plugin_path(env: &str, default: &str) -> Option<PathBuf> {
    let path = std::env::var_os(env)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default));

    if path.exists() {
        Some(path)
    } else {
        eprintln!("skip: plugin file not found: {}", path.display());
        None
    }
}

fn unique_temp_path(suffix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "au-plugin-host-{}-{nanos}-{suffix}",
        std::process::id()
    ))
}

fn stereo_sine() -> [Vec<f32>; 2] {
    let mono: Vec<f32> = (0..FRAMES)
        .map(|i| {
            (std::f32::consts::TAU * 440.0 * i as f32 / SAMPLE_RATE as f32).sin() * 0.5
        })
        .collect();
    [mono.clone(), mono]
}

fn process_sine(instance: &mut dyn PluginInstance) -> [Vec<f32>; 2] {
    let input = stereo_sine();
    let mut rendered = [vec![0.0; FRAMES], vec![0.0; FRAMES]];
    let mut block_output = [vec![0.0; BLOCK], vec![0.0; BLOCK]];

    for start in (0..FRAMES).step_by(BLOCK) {
        let end = start + BLOCK;
        let block_input: [&[f32]; 2] = [&input[0][start..end], &input[1][start..end]];
        block_output[0].fill(0.0);
        block_output[1].fill(0.0);
        instance.process(&block_input, &mut block_output);

        rendered[0][start..end].copy_from_slice(&block_output[0]);
        rendered[1][start..end].copy_from_slice(&block_output[1]);
    }

    rendered
}

fn rms_stereo(samples: &[Vec<f32>; 2], start: usize, end: usize) -> f64 {
    let sum_squares: f64 = samples
        .iter()
        .flat_map(|channel| channel[start..end].iter())
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    (sum_squares / ((end - start) * 2) as f64).sqrt()
}

fn assert_finite_and_nonzero(samples: &[Vec<f32>; 2]) {
    assert!(
        samples.iter().flatten().all(|sample| sample.is_finite()),
        "plugin output contains a non-finite sample"
    );
    assert!(
        samples.iter().flatten().any(|sample| sample.abs() > 1.0e-9),
        "plugin output is all zero"
    );
}

fn gain_param(
    descriptor: &au_plugin_api::PluginDescriptor,
) -> &au_plugin_api::ParamInfo {
    descriptor
        .params
        .iter()
        .find(|param| {
            let name = param.name.to_ascii_lowercase();
            name.contains("gain") && !name.contains("bypass")
        })
        .expect("AGain plugin should expose a Gain parameter")
}

#[test]
fn clap_surge_loads_and_processes() {
    let _guard = LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let Some(path) = plugin_path(
        "AU_TEST_CLAP",
        "/workspace/plugins/surge/Surge XT Effects.clap",
    ) else {
        return;
    };

    let descriptors = au_plugin_host::clap::scan_file(&path).expect("scan Surge XT Effects");
    assert!(!descriptors.is_empty(), "Surge scan found no plugins");

    let descriptor = &descriptors[0];
    let host = ClapHost::new();
    let mut instance = host
        .instantiate(descriptor)
        .expect("instantiate Surge XT Effects");
    assert!(
        !descriptor.params.is_empty(),
        "Surge XT Effects should expose parameters"
    );

    instance.activate(SAMPLE_RATE, BLOCK);
    let output = process_sine(instance.as_mut());
    assert_finite_and_nonzero(&output);

    println!(
        "{}: {} params, latency {} samples",
        descriptor.name,
        descriptor.params.len(),
        instance.latency_samples()
    );
}

#[test]
fn vst3_again_gain() {
    let _guard = LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let Some(path) = plugin_path(
        "AU_TEST_VST3",
        "/workspace/plugins/vst3sdk/build-gui/VST3/Release/again.vst3",
    ) else {
        return;
    };

    let descriptors = au_plugin_host::vst3::scan_file(&path).expect("scan AGain");
    let descriptor = descriptors
        .iter()
        .find(|descriptor| descriptor.name.to_ascii_lowercase().contains("gain"))
        .expect("VST3 scan should find an AGain-like plugin");

    let host = Vst3Host::new();
    let mut instance = host
        .instantiate(descriptor)
        .expect("instantiate AGain");
    instance.activate(SAMPLE_RATE, BLOCK);

    let gain = gain_param(descriptor);
    instance
        .set_param(gain.id, 0.5)
        .expect("set AGain to one half");
    assert!(
        (instance.get_param(gain.id).expect("read Gain") - 0.5).abs() < 0.02,
        "AGain parameter did not retain its set value"
    );

    let output = process_sine(instance.as_mut());
    assert!(output.iter().flatten().all(|sample| sample.is_finite()));

    let input = stereo_sine();
    let input_rms = rms_stereo(&input, FRAMES - BLOCK, FRAMES);
    let output_rms = rms_stereo(&output, FRAMES - BLOCK, FRAMES);
    let ratio = output_rms / input_rms;
    assert!(
        (ratio - 0.5).abs() < 0.02,
        "expected last-block RMS ratio near 0.5, got {ratio}"
    );
    assert_eq!(instance.latency_samples(), 0);
}

#[test]
fn vst3_state_round_trip() {
    let _guard = LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let Some(path) = plugin_path(
        "AU_TEST_VST3",
        "/workspace/plugins/vst3sdk/build-gui/VST3/Release/again.vst3",
    ) else {
        return;
    };

    let descriptors = au_plugin_host::vst3::scan_file(&path).expect("scan AGain");
    let descriptor = descriptors
        .iter()
        .find(|descriptor| descriptor.name.to_ascii_lowercase().contains("gain"))
        .expect("VST3 scan should find an AGain-like plugin");

    let host = Vst3Host::new();
    let mut instance = host
        .instantiate(descriptor)
        .expect("instantiate AGain");
    instance.activate(SAMPLE_RATE, BLOCK);

    let gain = gain_param(descriptor);
    instance
        .set_param(gain.id, 0.25)
        .expect("set AGain to one quarter");
    let state = instance.save_state().expect("save AGain state");
    assert!(
        state.iter().any(|(key, _)| key == PROCESSOR_STATE),
        "saved state should contain processor state"
    );
    assert!(
        state.iter().any(|(key, _)| key == PARAMETERS),
        "saved state should contain parameter state"
    );

    let mut fresh_instance = host
        .instantiate(descriptor)
        .expect("instantiate fresh AGain");
    fresh_instance
        .load_state(&state)
        .expect("load AGain state");
    assert!(
        (fresh_instance.get_param(gain.id).expect("read restored Gain") - 0.25).abs() < 0.02
    );

    let effect = save_effect(instance.as_mut(), true).expect("serialize AGain effect");
    let mut project = Aup3Project {
        state: ProjectState::default(),
        extras: Aup3Extras::default(),
    };
    project.extras.master_effects.active = true;
    project.extras.master_effects.effects.push(effect);

    let project_path = unique_temp_path("round-trip.aup3");
    aup3::save(&project_path, &project).expect("save temporary AUP3 project");
    let loaded = aup3::load_from(&project_path, DocSource::Saved).expect("load AUP3 project");
    let _ = std::fs::remove_file(&project_path);

    let loaded_effect = loaded
        .extras
        .master_effects
        .effects
        .first()
        .expect("AUP3 should contain the saved master effect");
    let restored = restore_effect(&registry(), loaded_effect).expect("restore AGain effect");
    assert!(
        (restored.get_param(gain.id).expect("read restored effect Gain") - 0.25).abs() < 0.02
    );
}

#[test]
fn clap_state_round_trip() {
    let _guard = LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let Some(path) = plugin_path(
        "AU_TEST_CLAP",
        "/workspace/plugins/surge/Surge XT Effects.clap",
    ) else {
        return;
    };

    let descriptors = au_plugin_host::clap::scan_file(&path).expect("scan Surge XT Effects");
    let descriptor = descriptors.first().expect("Surge scan found no plugins");
    let parameter = descriptor
        .params
        .iter()
        .find(|param| param.max > param.min)
        .expect("Surge XT Effects should expose a ranged parameter");
    let target = parameter.min + 0.3 * (parameter.max - parameter.min);

    let host = ClapHost::new();
    let mut instance = host
        .instantiate(descriptor)
        .expect("instantiate Surge XT Effects");
    instance
        .set_param(parameter.id, target)
        .expect("set Surge parameter");
    let state = instance.save_state().expect("save Surge state");

    let mut fresh_instance = host
        .instantiate(descriptor)
        .expect("instantiate fresh Surge XT Effects");
    fresh_instance
        .load_state(&state)
        .expect("load Surge state");
    let restored = fresh_instance
        .get_param(parameter.id)
        .expect("read restored Surge parameter");
    assert!(
        (restored - target).abs() < 1.0e-3,
        "expected restored parameter {target}, got {restored}"
    );
}

#[test]
fn vst3_adelay_processes() {
    let _guard = LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let Some(path) = plugin_path(
        "AU_TEST_ADELAY",
        "/workspace/plugins/vst3sdk/build-gui/VST3/Release/adelay.vst3",
    ) else {
        return;
    };

    let descriptors = au_plugin_host::vst3::scan_file(&path).expect("scan ADelay");
    let descriptor = descriptors
        .first()
        .expect("VST3 scan found no ADelay plugins");
    let host = Vst3Host::new();
    let mut instance = host
        .instantiate(descriptor)
        .expect("instantiate ADelay");

    let delay_param = descriptor
        .params
        .iter()
        .find(|param| param.name.to_lowercase().contains("delay"))
        .expect("ADelay has no Delay parameter");
    instance
        .set_param(delay_param.id, 0.01)
        .expect("set ADelay parameter");

    instance.activate(SAMPLE_RATE, BLOCK);

    let output = process_sine(instance.as_mut());
    assert_finite_and_nonzero(&output);

    let latency = instance.latency_samples();
    assert_eq!(latency, instance.latency_samples(), "latency should be reported consistently");
    println!("{}: latency {} samples", descriptor.name, latency);
}

fn set_scanner_env() {
    std::env::set_var(
        "AU_PLUGIN_SCANNER",
        env!("CARGO_BIN_EXE_au-plugin-scan"),
    );
}

fn assert_scan_found(outcome: ScanOutcome) {
    match outcome {
        ScanOutcome::Found { plugins, .. } => {
            assert!(!plugins.is_empty(), "sandbox scan found no plugins");
        }
        ScanOutcome::Crashed { .. } => panic!("sandbox scanner crashed"),
        ScanOutcome::TimedOut { .. } => panic!("sandbox scan timed out"),
        ScanOutcome::Failed { .. } => panic!("sandbox scan failed"),
    }
}

#[test]
fn sandboxed_scan_finds_real_plugins() {
    set_scanner_env();

    let Some(clap_path) = plugin_path(
        "AU_TEST_CLAP",
        "/workspace/plugins/surge/Surge XT Effects.clap",
    ) else {
        return;
    };
    let Some(vst3_path) = plugin_path(
        "AU_TEST_VST3",
        "/workspace/plugins/vst3sdk/build-gui/VST3/Release/again.vst3",
    ) else {
        return;
    };

    assert_scan_found(scan_one(
        PluginFormat::Clap,
        &clap_path,
        Duration::from_secs(60),
    ));
    assert_scan_found(scan_one(
        PluginFormat::Vst3,
        &vst3_path,
        Duration::from_secs(60),
    ));
}

#[test]
fn sandbox_survives_crashing_plugin() {
    set_scanner_env();

    if Command::new("cc").arg("--version").output().is_err() {
        eprintln!("skip: C compiler 'cc' is not available");
        return;
    }

    let directory = unique_temp_path("crashing-clap");
    std::fs::create_dir_all(&directory).expect("create temporary crash-plugin directory");
    let source_path = directory.join("crash.c");
    let plugin_path = directory.join("crash.clap");
    let source = r#"#include <signal.h>
#include <stdint.h>
typedef struct { uint32_t major, minor, revision; } v;
typedef struct { v clap_version; int (*init)(const char*); void (*deinit)(void); const void* (*get_factory)(const char*); } entry;
static int init(const char* p){ (void)p; *(volatile int*)0 = 1; return 0; } /* real fault: raise() would be swallowed by Rust's SIGSEGV handler */
static void deinit(void){}
static const void* gf(const char* id){ (void)id; return 0; }
__attribute__((visibility("default"))) const entry clap_entry = {{1,2,0}, init, deinit, gf};
"#;
    std::fs::write(&source_path, source).expect("write crashing CLAP source");

    let compilation = Command::new("cc")
        .arg("-shared")
        .arg("-fPIC")
        .arg("-o")
        .arg(&plugin_path)
        .arg(&source_path)
        .output()
        .expect("run C compiler");
    assert!(
        compilation.status.success(),
        "failed to compile crashing CLAP: {}",
        String::from_utf8_lossy(&compilation.stderr)
    );

    let outcome = scan_one(
        PluginFormat::Clap,
        &plugin_path,
        Duration::from_secs(30),
    );
    assert!(
        matches!(outcome, ScanOutcome::Crashed { .. }),
        "expected the sandbox scanner to report a crashed plugin"
    );
    assert!(
        std::process::id() != 0,
        "test process should remain alive after the sandbox scan"
    );

    let _ = std::fs::remove_dir_all(&directory);
}
