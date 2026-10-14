//! Nyquist worker with the Audacity engine installed.
fn main() {
    au_nyquist::install();
    std::process::exit(au_plugin_host::nyquist::worker_main())
}
