// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded job queue in front of a rayon work-stealing pool.
use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Cooperative cancellation flag shared between the UI and a job.
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);
impl CancelToken {
    pub fn new() -> Self { Self::default() }
    pub fn cancel(&self) { self.0.store(true, Ordering::Relaxed) }
    pub fn is_cancelled(&self) -> bool { self.0.load(Ordering::Relaxed) }
    /// `Err(JobError::Cancelled)` if cancelled; use with `?` inside jobs.
    pub fn check(&self) -> Result<(), JobError> {
        if self.is_cancelled() { Err(JobError::Cancelled) } else { Ok(()) }
    }
}

/// Lock-free progress counter (`done` of `total` units).
#[derive(Clone, Default, Debug)]
pub struct Progress { done: Arc<AtomicU64>, total: Arc<AtomicU64> }
impl Progress {
    pub fn new(total: u64) -> Self {
        let p = Self::default();
        p.total.store(total, Ordering::Relaxed);
        p
    }
    pub fn set_total(&self, t: u64) { self.total.store(t, Ordering::Relaxed) }
    pub fn add(&self, n: u64) { self.done.fetch_add(n, Ordering::Relaxed); }
    pub fn done(&self) -> u64 { self.done.load(Ordering::Relaxed) }
    pub fn total(&self) -> u64 { self.total.load(Ordering::Relaxed) }
    /// Fraction in `0..=1` (0 when the total is unknown).
    pub fn fraction(&self) -> f32 {
        let t = self.total();
        if t == 0 { 0.0 } else { (self.done() as f64 / t as f64).min(1.0) as f32 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobError {
    Cancelled,
    /// The job panicked; the panic was contained and the message kept.
    Panicked(String),
    /// The queue was full (`try_submit`) or the runtime shut down.
    Busy,
    Failed(String),
}
impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobError::Cancelled => write!(f, "cancelled"),
            JobError::Panicked(m) => write!(f, "worker panicked: {m}"),
            JobError::Busy => write!(f, "job queue full"),
            JobError::Failed(m) => write!(f, "{m}"),
        }
    }
}
impl std::error::Error for JobError {}

pub(crate) fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = p.downcast_ref::<&str>() { (*s).to_owned() }
    else if let Some(s) = p.downcast_ref::<String>() { s.clone() }
    else { "non-string panic payload".to_owned() }
}

/// Runs `f`, converting a panic into `JobError::Panicked`.
pub fn isolate<T>(f: impl FnOnce() -> Result<T, JobError>) -> Result<T, JobError> {
    match crate::crash::with_isolation(|| catch_unwind(AssertUnwindSafe(f))) {
        Ok(r) => r,
        Err(p) => Err(JobError::Panicked(panic_message(&*p))),
    }
}

/// Context passed to every job.
#[derive(Clone)]
pub struct JobCtx { pub cancel: CancelToken, pub progress: Progress, pool: Arc<rayon::ThreadPool> }
impl JobCtx {
    /// Run nested data-parallel work (`par_iter`, `join`) on the same pool.
    pub fn install<R: Send>(&self, f: impl FnOnce() -> R + Send) -> R { self.pool.install(f) }
}

/// Handle to a submitted job. Poll from the UI thread; never block the audio thread on it.
pub struct JobHandle<T> { rx: Receiver<Result<T, JobError>>, pub cancel: CancelToken, pub progress: Progress }
impl<T> JobHandle<T> {
    /// Non-blocking poll: `None` while running.
    pub fn try_result(&self) -> Option<Result<T, JobError>> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(JobError::Busy)),
        }
    }
    /// Blocking wait (tests, CLI, export threads; not the audio thread).
    pub fn wait(self) -> Result<T, JobError> { self.rx.recv().unwrap_or(Err(JobError::Busy)) }
}

type Task = Box<dyn FnOnce() + Send>;

/// Worker pool with a bounded admission queue.
///
/// At most `queue_cap` jobs wait for admission; a single dispatcher thread
/// feeds them to the rayon pool, so bursts of submissions apply backpressure
/// (`submit` blocks, `try_submit` returns `Busy`) instead of growing memory.
pub struct Runtime { pool: Arc<rayon::ThreadPool>, tx: Option<Sender<Task>>, dispatcher: Option<std::thread::JoinHandle<()>> }

impl Runtime {
    /// `threads == 0` uses one thread per logical CPU.
    pub fn new(threads: usize, queue_cap: usize) -> Self {
        let pool = Arc::new(
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|i| format!("au-worker-{i}"))
                .build()
                .expect("rayon pool"),
        );
        let (tx, rx) = bounded::<Task>(queue_cap.max(1));
        let p2 = pool.clone();
        let max_inflight = p2.current_num_threads().max(1);
        let dispatcher = std::thread::Builder::new()
            .name("au-dispatch".into())
            .spawn(move || {
                // Admit at most `threads` jobs into rayon at once so the queue bound is real.
                let (done_tx, done_rx) = bounded::<()>(max_inflight);
                let mut inflight = 0usize;
                for task in rx {
                    if inflight == max_inflight { let _ = done_rx.recv(); inflight -= 1; }
                    while done_rx.try_recv().is_ok() { inflight -= 1; }
                    inflight += 1;
                    let d = done_tx.clone();
                    p2.spawn(move || { task(); let _ = d.send(()); });
                }
                while inflight > 0 { let _ = done_rx.recv(); inflight -= 1; }
            })
            .expect("dispatcher thread");
        Self { pool, tx: Some(tx), dispatcher: Some(dispatcher) }
    }

    pub fn threads(&self) -> usize { self.pool.current_num_threads() }
    pub fn pool(&self) -> &rayon::ThreadPool { &self.pool }

    fn wrap<T: Send + 'static>(
        &self,
        total: u64,
        f: impl FnOnce(&JobCtx) -> Result<T, JobError> + Send + 'static,
    ) -> (Task, JobHandle<T>) {
        let (rtx, rrx) = bounded(1);
        let ctx = JobCtx { cancel: CancelToken::new(), progress: Progress::new(total), pool: self.pool.clone() };
        let handle = JobHandle { rx: rrx, cancel: ctx.cancel.clone(), progress: ctx.progress.clone() };
        let task: Task = Box::new(move || {
            let r = if ctx.cancel.is_cancelled() { Err(JobError::Cancelled) } else { isolate(|| f(&ctx)) };
            if let Err(JobError::Panicked(m)) = &r { crate::crash::log_action(format!("worker job panicked: {m}")); }
            let _ = rtx.send(r);
        });
        (task, handle)
    }

    /// Submits a job, blocking while the queue is full.
    pub fn submit<T: Send + 'static>(
        &self,
        total: u64,
        f: impl FnOnce(&JobCtx) -> Result<T, JobError> + Send + 'static,
    ) -> JobHandle<T> {
        let (task, h) = self.wrap(total, f);
        if let Some(tx) = &self.tx { let _ = tx.send(task); }
        h
    }

    /// Submits without blocking; `Err(Busy)` if the queue is full.
    pub fn try_submit<T: Send + 'static>(
        &self,
        total: u64,
        f: impl FnOnce(&JobCtx) -> Result<T, JobError> + Send + 'static,
    ) -> Result<JobHandle<T>, JobError> {
        let (task, h) = self.wrap(total, f);
        match &self.tx {
            Some(tx) => tx.try_send(task).map(|_| h).map_err(|_| JobError::Busy),
            None => Err(JobError::Busy),
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(d) = self.dispatcher.take() { let _ = d.join(); }
    }
}
