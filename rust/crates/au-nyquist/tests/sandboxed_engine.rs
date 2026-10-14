//! End-to-end: BizyBot's crash-contained sandbox running the Audacity-exact engine worker.
use au_plugin_host::nyquist::{run_sandboxed, NyquistJob, NyquistOutput};
use std::path::PathBuf;

#[test]
fn sandbox_runs_shipped_lowpass_through_engine_worker() {
    let script = PathBuf::from("/usr/share/audacity/plug-ins/lowpass.ny");
    if !script.exists() || !std::path::Path::new("/usr/lib/audacity/lib-nyquist-effects.so").exists() {
        eprintln!("Audacity not installed; skipping");
        return;
    }
    std::env::set_var("AU_NYQUIST_WORKER", env!("CARGO_BIN_EXE_au-nyquist-engine"));
    let n = 4410;
    let audio = au_core::AudioBuffer {
        sample_rate: 44_100,
        channels: vec![(0..n).map(|i| ((i as f32) * 0.9).sin() * 0.5).collect()],
    };
    let job = NyquistJob {
        script_path: script,
        params: vec![("FREQUENCY".into(), "500".into()), ("ROLLOFF".into(), "dB12".into())],
        audio: Some(audio.clone()),
        selection_secs: (0.0, 0.1),
    };
    let cancel = au_runtime::pool::CancelToken::default();
    let out = run_sandboxed(&job, std::time::Duration::from_secs(30), &cancel).expect("sandbox");
    let NyquistOutput::Audio(b) = out else { panic!("expected audio, got {out:?}") };
    assert_eq!(b.channels[0].len(), n);
    let rms = |v: &[f32]| (v.iter().map(|x| x * x).sum::<f32>() / v.len() as f32).sqrt();
    // a ~6.3 kHz tone through a 500 Hz low-pass is strongly attenuated
    assert!(rms(&b.channels[0][1000..]) < 0.1 * rms(&audio.channels[0][1000..]));
}
