// SPDX-License-Identifier: GPL-3.0-or-later
//! Panic hook, autosave flush on crash, local crash reports, and a
//! recent-actions ring log. Nothing is ever uploaded.
//!
//! * Panics inside [`crate::pool::isolate`] (worker jobs) are contained: they
//!   are logged to the ring but do not write a crash report or flush.
//! * Any other panic writes `crash-<ms>-<pid>.txt` into the crash folder
//!   (version, OS, thread, message, location, context, ring log, backtrace),
//!   then runs the registered flush hook (autosave) exactly once.
//! * On next launch, [`pending_reports`] lists unviewed reports; the GUI
//!   shows them and [`dismiss`] moves a report into `viewed/`.
use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const RING_CAP: usize = 64;

type FlushFn = Arc<dyn Fn() + Send + Sync>;

struct State {
    ring: Mutex<VecDeque<String>>,
    context: Mutex<Vec<(String, String)>>,
    flush: Mutex<Option<FlushFn>>,
    config: Mutex<Option<CrashConfig>>,
}
fn state() -> &'static State {
    static S: OnceLock<State> = OnceLock::new();
    S.get_or_init(|| State {
        ring: Mutex::new(VecDeque::with_capacity(RING_CAP)),
        context: Mutex::new(Vec::new()),
        flush: Mutex::new(None),
        config: Mutex::new(None),
    })
}

thread_local! { static ISOLATED: Cell<u32> = const { Cell::new(0) }; }
static IN_HOOK: AtomicBool = AtomicBool::new(false);

/// Marks the current thread as running isolated work for the closure's duration.
pub(crate) fn with_isolation<R>(f: impl FnOnce() -> R) -> R {
    struct Guard;
    impl Drop for Guard { fn drop(&mut self) { ISOLATED.with(|c| c.set(c.get() - 1)) } }
    ISOLATED.with(|c| c.set(c.get() + 1));
    let _g = Guard;
    f()
}

#[derive(Clone, Debug)]
pub struct CrashConfig {
    pub dir: PathBuf,
    pub app_name: String,
    pub version: String,
}

/// Default per-OS crash folder (`$XDG_STATE_HOME/audacity-rs/crashes`,
/// `%LOCALAPPDATA%\audacity-rs\crashes`, `~/Library/Logs/audacity-rs/crashes`).
pub fn default_crash_dir() -> PathBuf {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let base = if cfg!(windows) {
        env("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Logs"))
    } else {
        env("XDG_STATE_HOME").or_else(|| env("HOME").map(|h| h.join(".local/state")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("audacity-rs").join("crashes")
}

/// Appends a line to the recent-actions ring (keeps the last 64).
pub fn log_action(s: impl Into<String>) {
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    if let Ok(mut r) = state().ring.lock() {
        if r.len() == RING_CAP { r.pop_front(); }
        r.push_back(format!("{ms} {}", s.into()));
    }
}

/// Recent actions, oldest first.
pub fn recent_actions() -> Vec<String> {
    state().ring.lock().map(|r| r.iter().cloned().collect()).unwrap_or_default()
}

/// Sets a `key: value` line included in reports (e.g. `project`).
pub fn set_context(key: &str, value: impl Into<String>) {
    if let Ok(mut c) = state().context.lock() {
        let v = value.into();
        match c.iter_mut().find(|(k, _)| k == key) {
            Some(e) => e.1 = v,
            None => c.push((key.to_owned(), v)),
        }
    }
}

pub fn clear_context(key: &str) {
    if let Ok(mut c) = state().context.lock() { c.retain(|(k, _)| k != key); }
}

/// Registers the function run on a fatal panic to flush the autosave.
pub fn set_flush_hook(f: impl Fn() + Send + Sync + 'static) {
    if let Ok(mut s) = state().flush.lock() { *s = Some(Arc::new(f)); }
}

/// Runs the flush hook now (also used by the GUI's top-level unwind guard).
/// Never panics; a panicking flush is swallowed.
pub fn flush_now() {
    let f = state().flush.try_lock().ok().and_then(|g| g.clone());
    if let Some(f) = f {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f()));
    }
}

/// Installs the panic hook. Safe to call more than once (the config is replaced).
pub fn install(config: CrashConfig) {
    let first = state().config.lock().map(|mut c| c.replace(config).is_none()).unwrap_or(false);
    if !first { return; }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = crate::pool::panic_message(info.payload());
        if ISOLATED.with(|c| c.get()) > 0 {
            log_action(format!("contained panic: {msg}"));
            return;
        }
        if IN_HOOK.swap(true, Ordering::SeqCst) {
            prev(info);
            return;
        }
        let loc = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
        let _ = write_report(&msg, &loc);
        flush_now();
        IN_HOOK.store(false, Ordering::SeqCst);
        prev(info);
    }));
}

fn write_report(msg: &str, loc: &str) -> std::io::Result<PathBuf> {
    let cfg = state().config.try_lock().ok().and_then(|c| c.clone()).ok_or_else(|| std::io::Error::other("no config"))?;
    std::fs::create_dir_all(&cfg.dir)?;
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let th = std::thread::current();
    let mut s = format!(
        "{} crash report\nversion: {}\nos: {} {} ({})\ntime_unix_ms: {ms}\nthread: {}\nmessage: {msg}\nlocation: {loc}\n",
        cfg.app_name, cfg.version, std::env::consts::OS, std::env::consts::ARCH, std::env::consts::FAMILY,
        th.name().unwrap_or("<unnamed>"),
    );
    if let Ok(c) = state().context.try_lock() {
        for (k, v) in c.iter() { s += &format!("{k}: {v}\n"); }
    }
    s += "\n-- recent actions (oldest first) --\n";
    if let Ok(r) = state().ring.try_lock() {
        for a in r.iter() { s += a; s.push('\n'); }
    }
    s += &format!("\n-- backtrace --\n{}\n", std::backtrace::Backtrace::force_capture());
    let name = format!("crash-{ms}-{}.txt", std::process::id());
    let tmp = cfg.dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, s)?;
    let path = cfg.dir.join(name);
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

/// One unviewed crash report.
#[derive(Clone, Debug)]
pub struct CrashReport { pub path: PathBuf, pub text: String }
impl CrashReport {
    fn field(&self, key: &str) -> Option<&str> {
        self.text.lines().take_while(|l| !l.is_empty()).find_map(|l| l.strip_prefix(key)?.strip_prefix(": "))
    }
    pub fn message(&self) -> Option<&str> { self.field("message") }
    /// Project that was open at crash time (set via `set_context("project", ..)`).
    pub fn project(&self) -> Option<PathBuf> { self.field("project").map(PathBuf::from) }
}

/// Unviewed reports in `dir`, newest first.
pub fn pending_reports(dir: &Path) -> Vec<CrashReport> {
    let mut v: Vec<CrashReport> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt")))
        .filter_map(|p| Some(CrashReport { text: std::fs::read_to_string(&p).ok()?, path: p }))
        .collect();
    v.sort_by(|a, b| b.path.cmp(&a.path));
    v
}

/// Marks a report viewed (moves it to `<dir>/viewed/`; nothing is deleted).
pub fn dismiss(r: &CrashReport) -> std::io::Result<()> {
    let dir = r.path.parent().unwrap_or(Path::new(".")).join("viewed");
    std::fs::create_dir_all(&dir)?;
    std::fs::rename(&r.path, dir.join(r.path.file_name().unwrap_or_default()))
}
