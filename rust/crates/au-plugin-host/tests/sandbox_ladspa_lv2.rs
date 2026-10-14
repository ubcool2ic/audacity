use au_plugin_api::{PluginFormat, PluginHost};
use au_plugin_host::ladspa::LadspaHost;
use au_plugin_host::lv2::Lv2Host;
use au_plugin_host::sandbox::{scan_one, ScanOutcome, SandboxedHost};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn set_scanner_env() {
    std::env::set_var(
        "AU_PLUGIN_SCANNER",
        env!("CARGO_BIN_EXE_au-plugin-scan"),
    );
}


struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let unique = format!(
            "au-plugin-host-ladspa-crash-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct CrashEnvGuard;

impl Drop for CrashEnvGuard {
    fn drop(&mut self) {
        std::env::remove_var("AU_SCAN_TEST_CRASH");
        std::env::remove_var("AU_SCAN_TEST_CRASH_FORMAT");
    }
}

#[test]
fn ladspa_scan_crash_is_contained() {
    let _env_lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_scanner_env();

    if Command::new("cc").arg("--version").output().is_err() {
        eprintln!("skipping LADSPA crash test: cc is not available");
        return;
    }

    let temp_dir = TempDir::new();
    let source = temp_dir.0.join("crasher.c");
    let so = temp_dir.0.join("crasher.so");
    std::fs::write(
        &source,
        r#"
const void *ladspa_descriptor(unsigned long index) {
    (void)index;
    *(volatile int *)16 = 1;
    return 0;
}
"#,
    )
    .unwrap();

    let compile = Command::new("cc")
        .args(["-shared", "-fPIC", "-o"])
        .arg(&so)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "failed to compile LADSPA crasher: {}",
        String::from_utf8_lossy(&compile.stderr)
    );

    assert!(matches!(
        scan_one(
            PluginFormat::Ladspa,
            &so,
            Duration::from_secs(30),
        ),
        ScanOutcome::Crashed { .. }
    ));

    let system_dir = PathBuf::from("/usr/lib/ladspa");
    let host = SandboxedHost::new(LadspaHost::new());
    let plugins = host.scan(&[temp_dir.0.clone(), system_dir.clone()]);

    assert!(
        host.last_outcomes().iter().any(|outcome| {
            matches!(
                outcome,
                ScanOutcome::Crashed { path, .. }
                    if path == &temp_dir.0 || path == &so
            )
        }),
        "sandbox outcomes did not contain the crashed LADSPA scanner"
    );

    if system_dir.exists() {
        assert!(
            !plugins.is_empty(),
            "expected plugins in {}",
            system_dir.display()
        );
        host.instantiate(&plugins[0])
            .expect("failed to instantiate the first LADSPA plugin");
    }
}

#[test]
fn lv2_scan_runs_out_of_process() {
    let _env_lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_scanner_env();

    let lv2_dir = PathBuf::from("/usr/lib/lv2");
    if !lv2_dir.exists() {
        eprintln!("skipping LV2 scan test: {} does not exist", lv2_dir.display());
        return;
    }

    let host = SandboxedHost::new(Lv2Host::new());
    let plugins = host.scan(&[lv2_dir.clone()]);

    assert!(!plugins.is_empty(), "sandbox scan found no LV2 plugins");
    for plugin in &plugins {
        assert_eq!(plugin.format, PluginFormat::Lv2);
        assert_eq!(plugin.path.as_ref(), Some(&lv2_dir));
    }

    let to_instantiate = plugins
        .iter()
        .find(|plugin| plugin.name.contains("Amp"))
        .unwrap_or(&plugins[0]);
    host.instantiate(to_instantiate)
        .expect("failed to instantiate an LV2 plugin");

    let outcomes = host.last_outcomes();
    assert!(!outcomes.is_empty(), "sandbox scan produced no outcomes");
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, ScanOutcome::Found { .. })),
        "expected every LV2 scan outcome to be Found"
    );
}

#[test]
fn lv2_scan_crash_is_contained() {
    let _env_lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_scanner_env();

    std::env::set_var("AU_SCAN_TEST_CRASH", "1");
    std::env::set_var("AU_SCAN_TEST_CRASH_FORMAT", "lv2");
    let _crash_env_guard = CrashEnvGuard;

    let lv2_dir = PathBuf::from("/usr/lib/lv2");
    if lv2_dir.exists() {
        assert!(
            matches!(
                scan_one(
                    PluginFormat::Lv2,
                    &lv2_dir,
                    Duration::from_secs(30),
                ),
                ScanOutcome::Crashed { .. }
            ),
            "expected the LV2 scanner to crash under the test crash environment"
        );
    }

    let ladspa_dir = PathBuf::from("/usr/lib/ladspa");
    if ladspa_dir.exists() {
        let ladspa_so = std::fs::read_dir(&ladspa_dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "so")
            });

        if let Some(ladspa_so) = ladspa_so {
            assert!(
                !matches!(
                    scan_one(
                        PluginFormat::Ladspa,
                        &ladspa_so,
                        Duration::from_secs(30),
                    ),
                    ScanOutcome::Crashed { .. }
                ),
                "the LV2 crash setting must not crash the LADSPA scanner"
            );
        }
    }
}
