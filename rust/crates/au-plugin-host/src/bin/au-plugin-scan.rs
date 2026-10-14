fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(au_plugin_host::sandbox::scanner_main(&a))
}
