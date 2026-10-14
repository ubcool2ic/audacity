//! Standalone worker process for crash-contained Nyquist execution.

fn main() {
    std::process::exit(au_plugin_host::nyquist::worker_main())
}

