// Builds the SoundTouch C shim and links the same system libsoxr / libSoundTouch that Audacity 3.7.3 uses.
fn main() {
    let shim = "csrc/speedpitch.cpp";
    println!("cargo:rerun-if-changed={shim}");
    if std::path::Path::new(shim).exists() {
        cc::Build::new().cpp(true).file(shim).flag_if_supported("-std=c++17").compile("aust_shim");
    }
    println!("cargo:rustc-link-lib=SoundTouch");
    println!("cargo:rustc-link-lib=soxr");
}
