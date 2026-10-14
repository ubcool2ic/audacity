//! Integration tests for the Nyquist worker protocol and sandbox.

use au_plugin_api::{PluginDescriptor, PluginFormat, PluginHost};
use au_plugin_host::nyquist::{
    read_header, run_sandboxed, NyquistHost, NyquistJob, NyquistOutput, SandboxError,
};
use au_runtime::pool::CancelToken;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn configure_worker() {
    std::env::set_var(
        "AU_NYQUIST_WORKER",
        env!("CARGO_BIN_EXE_au-nyquist-worker"),
    );
}

fn job(params: Vec<(String, String)>, audio: Option<au_core::AudioBuffer>) -> NyquistJob {
    NyquistJob {
        script_path: PathBuf::from("test.ny"),
        params,
        audio,
        selection_secs: (0.0, 1.0),
    }
}

fn stereo_buffer() -> au_core::AudioBuffer {
    au_core::AudioBuffer {
        sample_rate: 48_000,
        channels: vec![vec![0.25, -0.5, 1.0], vec![-1.0, 0.5, 0.125]],
    }
}

#[test]
fn echo_round_trips_stereo_audio_with_gain() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    configure_worker();

    let input = stereo_buffer();
    let output = run_sandboxed(
        &job(
            vec![
                ("__au_test".into(), "echo".into()),
                ("__au_gain".into(), "0.5".into()),
            ],
            Some(input),
        ),
        Duration::from_secs(3),
        &CancelToken::new(),
    )
    .unwrap();

    assert_eq!(
        output,
        NyquistOutput::Audio(au_core::AudioBuffer {
            sample_rate: 48_000,
            channels: vec![vec![0.125, -0.25, 0.5], vec![-0.5, 0.25, 0.0625]],
        })
    );
}

#[test]
fn labels_are_returned() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    configure_worker();

    let output = run_sandboxed(
        &job(vec![("__au_test".into(), "labels".into())], None),
        Duration::from_secs(3),
        &CancelToken::new(),
    )
    .unwrap();

    assert_eq!(
        output,
        NyquistOutput::Labels(vec![(0.0, 1.0, "test".into())])
    );
}

#[test]
fn crashing_worker_does_not_crash_the_test_process() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    configure_worker();

    let result = run_sandboxed(
        &job(vec![("__au_test".into(), "crash".into())], None),
        Duration::from_secs(3),
        &CancelToken::new(),
    );
    assert!(matches!(result, Err(SandboxError::Crashed(_))));

    let following = run_sandboxed(
        &job(vec![("__au_test".into(), "echo".into())], None),
        Duration::from_secs(3),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(following, NyquistOutput::Text("generated".into()));
}

#[test]
fn hanging_worker_times_out() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    configure_worker();

    let started = Instant::now();
    let result = run_sandboxed(
        &job(vec![("__au_test".into(), "hang".into())], None),
        Duration::from_millis(500),
        &CancelToken::new(),
    );
    assert_eq!(result, Err(SandboxError::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn hanging_worker_can_be_cancelled() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    configure_worker();

    let cancel = CancelToken::new();
    let cancel_from_thread = cancel.clone();
    let cancellation = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        cancel_from_thread.cancel();
    });

    let result = run_sandboxed(
        &job(vec![("__au_test".into(), "hang".into())], None),
        Duration::from_secs(3),
        &cancel,
    );
    cancellation.join().unwrap();
    assert_eq!(result, Err(SandboxError::Cancelled));
}

#[test]
fn worker_panics_are_contained() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    configure_worker();

    let result = run_sandboxed(
        &job(vec![("__au_test".into(), "panic".into())], None),
        Duration::from_secs(3),
        &CancelToken::new(),
    );
    assert!(matches!(
        result,
        Err(SandboxError::Crashed(_)) | Err(SandboxError::Failed(_))
    ));
}

#[test]
fn missing_engine_is_reported_by_worker() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    configure_worker();

    let result = run_sandboxed(
        &job(Vec::new(), None),
        Duration::from_secs(3),
        &CancelToken::new(),
    );
    assert!(matches!(
        result,
        Err(SandboxError::Failed(message)) if message.contains("not available")
    ));
}

#[test]
fn scans_nyquist_scripts_and_ignores_other_files() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    configure_worker();

    static NEXT_DIR: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    let unique = NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "au-nyquist-scan-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("amp.ny"),
        "$name (_ \"My Amp\")\n$type process\n$author \"Me\"\n",
    )
    .unwrap();
    fs::write(
        dir.join("generator.ny"),
        ";name \"Gen\"\n;type generate\n",
    )
    .unwrap();
    fs::write(dir.join("ignored.txt"), ";name \"Ignored\"\n").unwrap();

    let descriptors = NyquistHost::new().scan(std::slice::from_ref(&dir));
    assert_eq!(descriptors.len(), 2);

    let amp = descriptors.iter().find(|descriptor| descriptor.name == "My Amp").unwrap();
    assert_eq!(amp.vendor, "Me");
    assert_eq!(amp.n_inputs, 2);
    assert_eq!(amp.n_outputs, 2);

    let generator = descriptors
        .iter()
        .find(|descriptor| descriptor.name == "Gen")
        .unwrap();
    assert_eq!(generator.n_inputs, 0);
    assert_eq!(generator.format, PluginFormat::Nyquist);

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn reads_nyquist_header_variants() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());

    assert_eq!(
        read_header(";name \"Amp\"\n;type process\n;author \"A\"\n;version 3\n"),
        Some(au_plugin_host::nyquist::NyquistHeader {
            name: "Amp".into(),
            kind: "process".into(),
            author: "A".into(),
            version: 3,
        })
    );
    assert_eq!(
        read_header("$name (_ \"My Amp\")\n$type generate\n$author Me\n"),
        Some(au_plugin_host::nyquist::NyquistHeader {
            name: "My Amp".into(),
            kind: "generate".into(),
            author: "Me".into(),
            version: 0,
        })
    );
    assert_eq!(read_header(";type process\n"), None);
}

#[test]
fn instance_process_buffer_runs_the_whole_buffer() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    configure_worker();

    let descriptor = PluginDescriptor {
        id: "test-nyquist".into(),
        name: "Echo".into(),
        vendor: String::new(),
        format: PluginFormat::Nyquist,
        path: Some(PathBuf::from("test.ny")),
        params: Vec::new(),
        n_inputs: 2,
        n_outputs: 2,
    };
    let mut instance = NyquistHost::new()
        .with_timeout(Duration::from_secs(3))
        .instantiate(&descriptor)
        .unwrap();
    instance
        .load_state(&[
            ("__au_test".into(), "echo".into()),
            ("__au_gain".into(), "0.5".into()),
        ])
        .unwrap();

    let mut buffer = stereo_buffer();
    instance.process_buffer(&mut buffer);
    assert_eq!(
        buffer,
        au_core::AudioBuffer {
            sample_rate: 48_000,
            channels: vec![vec![0.125, -0.25, 0.5], vec![-0.5, 0.25, 0.0625]],
        }
    );
}
