// Builds the export codec shims and links the same system libraries Audacity 3.7.3's
// export modules use (LAME, libvorbis/libogg, libopus, libwavpack, libsndfile, libFLAC, libid3tag).
fn main() {
    let mut b = cc::Build::new();
    b.cpp(true).flag_if_supported("-std=c++17").warnings(false);
    for f in ["csrc/export_mp3.cpp", "csrc/export_ogg_opus.cpp", "csrc/export_intfmt.cpp"] {
        println!("cargo:rerun-if-changed={f}");
        b.file(f);
    }
    b.compile("au_export_shim");
    for l in ["mp3lame", "vorbisenc", "vorbis", "ogg", "opus", "wavpack", "sndfile", "FLAC", "id3tag"] {
        println!("cargo:rustc-link-lib={l}");
    }
}
