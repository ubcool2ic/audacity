# Third-party notices — Audacity-RS 1.0.0

Audacity-RS is licensed under the **GNU General Public License v3.0 or later** (GPL-3.0-or-later); see `../LICENSE.txt`.
It is a derivative work of Audacity © Audacity Team and contributors. Many Audacity source files are GPL-2.0-or-later, but Audacity 3.7.3 as a whole is distributed under GPLv3 (see `../LICENSE.txt`), and this port is GPLv3-or-later for the same reason. Several dependencies (Apache-2.0 crates, MPL-2.0 Symphonia, the GPLv3 runtime-exception GCC libraries) are compatible with GPLv3 but not with GPLv2 only.

## Bundled native components (Windows packages)

| Component | License | How it is used |
|---|---|---|
| LAME (libmp3lame) | LGPL-2.0-or-later | MP3 export, dynamically loaded DLL |
| mpg123 | LGPL-2.1 | MP3 decoding helper DLL |
| libsndfile | LGPL-2.1-or-later | DLL |
| libsoxr | LGPL-2.1-or-later | DLL |
| SoundTouch | LGPL-2.1 | DLL |
| libid3tag | GPL-2.0-or-later | ID3 tags, DLL |
| FLAC (libFLAC) | BSD-3-Clause | DLL |
| libogg, libvorbis | BSD-3-Clause | DLL |
| Opus (libopus) | BSD-3-Clause | DLL |
| WavPack | BSD-3-Clause | DLL |
| libgcc, libstdc++, libgomp (MinGW-w64) | GPL-3.0 with GCC Runtime Library Exception | Runtime DLLs |
| winpthreads (MinGW-w64) | MIT / BSD | Runtime DLL |
| SQLite (bundled by rusqlite) | Public domain | Project files |
| egui default fonts (Ubuntu-Light, Hack, Noto Emoji, emoji-icon-font) | Ubuntu Font Licence 1.0, MIT/Bitstream Vera, SIL OFL 1.1 | Embedded in the binary |

The LGPL DLLs are shipped as separate, replaceable files next to `audacity-rs.exe`; their sources are available from MSYS2 (`https://repo.msys2.org/mingw/mingw64`) and upstream.

## Optional components (not bundled)

- **Nyquist engine**: Nyquist effects load `lib-nyquist-effects.dll` from an installed Audacity 3.7.x (Nyquist is under a BSD-style license, part of Audacity's GPL distribution). The bundled `.ny` plug-ins and the Nyquist runtime `.lsp` files are copied from Audacity (GPL-2.0-or-later and the Nyquist license).
- **FFmpeg** (LGPL-2.1-or-later or GPL depending on the build): never linked. If the user installs FFmpeg 7.x (`avformat-61.dll`, `avcodec-61.dll`, `avutil-59.dll`, `swresample-5.dll`), Audacity-RS loads it at runtime, like Audacity's FFmpeg support. Without it, M4A/AAC/WMA/AC3 and video audio are unavailable; everything else works.
- **VST3, CLAP, LV2, LADSPA plug-ins**: third-party plug-ins run under their own licenses. The VST3 SDK is MIT-licensed since VST 3.8 (October 2025).
- **Lyria** (Google, `google/lyria-3-pro-preview`, `google/lyria-3-clip-preview`): used through **OpenRouter**. You supply your own OpenRouter API key and are billed by OpenRouter. Lyria output is subject to Google's terms and carries a SynthID watermark.
- **HTDemucs** (Meta, MIT): stem separation runs the user-installed `demucs` Python package as a **local subprocess**. It does not go through OpenRouter, and no audio leaves the machine.

## Rust crates (Windows x64 build, from `cargo metadata`)

| Crate | Version | License | Source |
|---|---|---|---|
| ab_glyph | 0.2.32 | Apache-2.0 | https://github.com/alexheretic/ab-glyph |
| ab_glyph_rasterizer | 0.1.10 | Apache-2.0 | https://github.com/alexheretic/ab-glyph |
| accesskit | 0.16.3 | MIT OR Apache-2.0 | https://github.com/AccessKit/accesskit |
| accesskit_consumer | 0.24.3 | MIT OR Apache-2.0 | https://github.com/AccessKit/accesskit |
| accesskit_windows | 0.23.2 | MIT OR Apache-2.0 | https://github.com/AccessKit/accesskit |
| accesskit_winit | 0.22.4 | Apache-2.0 | https://github.com/AccessKit/accesskit |
| adler2 | 2.0.1 | 0BSD OR MIT OR Apache-2.0 | https://github.com/oyvindln/adler2 |
| ahash | 0.8.12 | MIT OR Apache-2.0 | https://github.com/tkaitchuck/ahash |
| arboard | 3.6.1 | MIT OR Apache-2.0 | https://github.com/1Password/arboard |
| arrayvec | 0.7.8 | MIT OR Apache-2.0 | https://github.com/bluss/arrayvec |
| ash | 0.38.0+1.3.281 | MIT OR Apache-2.0 | https://github.com/ash-rs/ash |
| autocfg | 1.5.1 | Apache-2.0 OR MIT | https://github.com/cuviper/autocfg |
| base64 | 0.22.1 | MIT OR Apache-2.0 | https://github.com/marshallpierce/rust-base64 |
| bit-set | 0.6.0 | MIT/Apache-2.0 | https://github.com/contain-rs/bit-set |
| bit-vec | 0.7.0 | MIT/Apache-2.0 | https://github.com/contain-rs/bit-vec |
| bitflags | 1.3.2 | MIT/Apache-2.0 | https://github.com/bitflags/bitflags |
| bitflags | 2.13.2 | MIT OR Apache-2.0 | https://github.com/bitflags/bitflags |
| block-buffer | 0.10.4 | MIT OR Apache-2.0 | https://github.com/RustCrypto/utils |
| built | 0.7.7 | MIT | https://github.com/lukaslueg/built |
| bytemuck | 1.25.2 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| bytemuck_derive | 1.12.1 | Zlib OR Apache-2.0 OR MIT | https://github.com/Lokathor/bytemuck |
| byteorder | 1.5.0 | Unlicense OR MIT | https://github.com/BurntSushi/byteorder |
| byteorder-lite | 0.1.0 | Unlicense OR MIT | https://github.com/image-rs/byteorder-lite |
| cc | 1.8.0 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| cfg-if | 1.0.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/cfg-if |
| cfg_aliases | 0.1.1 | MIT | https://github.com/katharostech/cfg_aliases |
| cfg_aliases | 0.2.2 | MIT | https://github.com/katharostech/cfg_aliases |
| clap-sys | 0.5.0 | MIT/Apache-2.0 | https://github.com/micahrj/clap-sys |
| clipboard-win | 5.4.1 | BSL-1.0 | https://github.com/DoumanAsh/clipboard-win |
| codespan-reporting | 0.11.1 | Apache-2.0 | https://github.com/brendanzab/codespan |
| com | 0.6.0 | MIT | https://github.com/microsoft/com-rs |
| com-scrape-types | 0.1.1 | MIT OR Apache-2.0 | https://github.com/coupler-rs/vst3-rs |
| com_macros | 0.6.0 | MIT | https://github.com/microsoft/com-rs |
| com_macros_support | 0.6.0 | MIT | https://github.com/microsoft/com-rs |
| cpal | 0.15.3 | Apache-2.0 | https://github.com/rustaudio/cpal |
| crc | 2.1.0 | MIT OR Apache-2.0 | https://github.com/mrhooray/crc-rs.git |
| crc-catalog | 1.1.1 | MIT OR Apache-2.0 | https://github.com/akhilles/crc-catalog.git |
| crc32fast | 1.5.2 | MIT OR Apache-2.0 | https://github.com/srijs/rust-crc32fast |
| crossbeam-channel | 0.5.17 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crossbeam-deque | 0.8.8 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crossbeam-epoch | 0.9.21 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crossbeam-utils | 0.8.23 | MIT OR Apache-2.0 | https://github.com/crossbeam-rs/crossbeam |
| crypto-common | 0.1.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| cursor-icon | 1.2.0 | MIT OR Apache-2.0 OR Zlib | https://github.com/rust-windowing/cursor-icon |
| dasp_sample | 0.11.0 | MIT OR Apache-2.0 | https://github.com/rustaudio/sample.git |
| digest | 0.10.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/traits |
| displaydoc | 0.2.7 | MIT OR Apache-2.0 | https://github.com/yaahc/displaydoc |
| document-features | 0.2.12 | MIT OR Apache-2.0 | https://github.com/slint-ui/document-features |
| dpi | 0.1.2 | Apache-2.0 AND MIT | https://github.com/rust-windowing/winit |
| ecolor | 0.29.1 | MIT OR Apache-2.0 | https://github.com/emilk/egui |
| eframe | 0.29.1 | MIT OR Apache-2.0 | https://github.com/emilk/egui/tree/master/crates/eframe |
| egui | 0.29.1 | MIT OR Apache-2.0 | https://github.com/emilk/egui |
| egui-wgpu | 0.29.1 | MIT OR Apache-2.0 | https://github.com/emilk/egui/tree/master/crates/egui-wgpu |
| egui-winit | 0.29.1 | MIT OR Apache-2.0 | https://github.com/emilk/egui/tree/master/crates/egui-winit |
| egui_glow | 0.29.1 | MIT OR Apache-2.0 | https://github.com/emilk/egui/tree/master/crates/egui_glow |
| either | 1.19.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/either |
| emath | 0.29.1 | MIT OR Apache-2.0 | https://github.com/emilk/egui/tree/master/crates/emath |
| encoding_rs | 0.8.35 | (Apache-2.0 OR MIT) AND BSD-3-Clause | https://github.com/hsivonen/encoding_rs |
| epaint | 0.29.1 | MIT OR Apache-2.0 | https://github.com/emilk/egui/tree/master/crates/epaint |
| epaint_default_fonts | 0.29.1 | (MIT OR Apache-2.0) AND OFL-1.1 AND LicenseRef-UFL-1.0 | https://github.com/emilk/egui/tree/master/crates/epaint_default_fonts |
| equivalent | 1.0.3 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/equivalent |
| error-code | 3.4.0 | BSL-1.0 | https://github.com/DoumanAsh/error-code |
| extended | 0.1.0 | MIT | https://github.com/depp/extended-rs |
| fallible-iterator | 0.3.0 | MIT/Apache-2.0 | https://github.com/sfackler/rust-fallible-iterator |
| fallible-streaming-iterator | 0.1.9 | MIT/Apache-2.0 | https://github.com/sfackler/fallible-streaming-iterator |
| fastrand | 2.5.0 | Apache-2.0 OR MIT | https://github.com/smol-rs/fastrand |
| fdeflate | 0.3.7 | MIT OR Apache-2.0 | https://github.com/image-rs/fdeflate |
| find-msvc-tools | 0.1.14 | MIT OR Apache-2.0 | https://github.com/rust-lang/cc-rs |
| flacenc | 0.4.0 | Apache-2.0 | https://github.com/yotarok/flacenc-rs/ |
| flate2 | 1.1.10 | MIT OR Apache-2.0 | https://github.com/rust-lang/flate2-rs |
| foldhash | 0.1.5 | Zlib | https://github.com/orlp/foldhash |
| form_urlencoded | 1.2.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| generic-array | 0.14.7 | MIT | https://github.com/fizyk20/generic-array.git |
| getrandom | 0.2.17 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.3.4 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| getrandom | 0.4.3 | MIT OR Apache-2.0 | https://github.com/rust-random/getrandom |
| gl_generator | 0.14.0 | Apache-2.0 | https://github.com/brendanzab/gl-rs/ |
| glow | 0.13.1 | MIT OR Apache-2.0 OR Zlib | https://github.com/grovesNL/glow |
| glow | 0.14.2 | MIT OR Apache-2.0 OR Zlib | https://github.com/grovesNL/glow |
| glutin | 0.32.3 | Apache-2.0 | https://github.com/rust-windowing/glutin |
| glutin-winit | 0.5.0 | MIT | https://github.com/rust-windowing/glutin |
| glutin_egl_sys | 0.7.1 | Apache-2.0 | https://github.com/rust-windowing/glutin |
| glutin_wgl_sys | 0.6.1 | Apache-2.0 | https://github.com/rust-windowing/glutin |
| gpu-alloc | 0.6.2 | MIT OR Apache-2.0 | https://github.com/zakarumych/gpu-alloc |
| gpu-alloc-types | 0.3.1 | MIT OR Apache-2.0 | https://github.com/zakarumych/gpu-alloc |
| gpu-allocator | 0.26.0 | MIT OR Apache-2.0 | https://github.com/Traverse-Research/gpu-allocator |
| gpu-descriptor | 0.3.2 | MIT OR Apache-2.0 | https://github.com/zakarumych/gpu-descriptor |
| gpu-descriptor-types | 0.2.0 | MIT OR Apache-2.0 | https://github.com/zakarumych/gpu-descriptor |
| hash32 | 0.3.1 | MIT OR Apache-2.0 | https://github.com/japaric/hash32 |
| hashbrown | 0.14.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hashbrown | 0.15.5 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hashbrown | 0.17.1 | MIT OR Apache-2.0 | https://github.com/rust-lang/hashbrown |
| hashlink | 0.9.1 | MIT OR Apache-2.0 | https://github.com/kyren/hashlink |
| hassle-rs | 0.11.0 | MIT | https://github.com/Traverse-Research/hassle-rs |
| heapless | 0.8.0 | MIT OR Apache-2.0 | https://github.com/rust-embedded/heapless |
| hexf-parse | 0.2.1 | CC0-1.0 | https://github.com/lifthrasiir/hexf |
| hound | 3.5.1 | Apache-2.0 | https://github.com/ruuda/hound |
| icu_collections | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_locale_core | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_normalizer_data | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties | 2.1.2 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_properties_data | 2.1.2 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| icu_provider | 2.1.1 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| idna | 1.1.0 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| idna_adapter | 1.2.1 | Apache-2.0 OR MIT | https://github.com/hsivonen/idna_adapter |
| image | 0.25.9 | MIT OR Apache-2.0 | https://github.com/image-rs/image |
| immutable-chunkmap | 2.1.4 | Apache-2.0 OR MIT | https://github.com/estokes/immutable-chunkmap |
| indexmap | 2.14.2 | Apache-2.0 OR MIT | https://github.com/indexmap-rs/indexmap |
| itoa | 1.0.18 | MIT OR Apache-2.0 | https://github.com/dtolnay/itoa |
| jobserver | 0.1.35 | MIT OR Apache-2.0 | https://github.com/rust-lang/jobserver-rs |
| khronos-egl | 6.0.0 | MIT/Apache-2.0 | https://github.com/timothee-haudebourg/khronos-egl |
| khronos_api | 3.1.0 | Apache-2.0 | https://github.com/brendanzab/gl-rs/ |
| lazy_static | 1.5.1 | MIT OR Apache-2.0 | https://github.com/rust-lang-nursery/lazy-static.rs |
| libc | 0.2.190 | MIT OR Apache-2.0 | https://github.com/rust-lang/libc |
| libloading | 0.8.9 | ISC | https://github.com/nagisa/rust_libloading/ |
| libsqlite3-sys | 0.28.0 | MIT | https://github.com/rusqlite/rusqlite |
| litemap | 0.8.3 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| litrs | 1.0.1 | MIT OR Apache-2.0 | https://github.com/LukasKalbertodt/litrs |
| lock_api | 0.4.14 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| log | 0.4.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/log |
| md-5 | 0.10.6 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| memchr | 2.8.3 | Unlicense OR MIT | https://github.com/BurntSushi/memchr |
| memoffset | 0.9.1 | MIT | https://github.com/Gilnaa/memoffset |
| midly | 0.5.3 | Unlicense | https://github.com/negamartin/midly |
| miniz_oxide | 0.8.9 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| miniz_oxide | 0.9.1 | MIT OR Zlib OR Apache-2.0 | https://github.com/Frommi/miniz_oxide/tree/master/miniz_oxide |
| moxcms | 0.7.11 | BSD-3-Clause OR Apache-2.0 | https://github.com/awxkee/moxcms.git |
| naga | 22.1.0 | MIT OR Apache-2.0 | https://github.com/gfx-rs/wgpu/tree/trunk/naga |
| nohash-hasher | 0.2.0 | Apache-2.0 OR MIT | https://github.com/paritytech/nohash-hasher |
| num-complex | 0.4.6 | MIT OR Apache-2.0 | https://github.com/rust-num/num-complex |
| num-integer | 0.1.47 | MIT OR Apache-2.0 | https://github.com/rust-num/num-integer |
| num-traits | 0.2.19 | MIT OR Apache-2.0 | https://github.com/rust-num/num-traits |
| once_cell | 1.21.4 | MIT OR Apache-2.0 | https://github.com/matklad/once_cell |
| owned_ttf_parser | 0.25.1 | Apache-2.0 | https://github.com/alexheretic/owned-ttf-parser |
| parking_lot | 0.12.5 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| parking_lot_core | 0.9.12 | MIT OR Apache-2.0 | https://github.com/Amanieu/parking_lot |
| paste | 1.0.15 | MIT OR Apache-2.0 | https://github.com/dtolnay/paste |
| percent-encoding | 2.3.2 | MIT OR Apache-2.0 | https://github.com/servo/rust-url/ |
| pin-project-lite | 0.2.17 | Apache-2.0 OR MIT | https://github.com/taiki-e/pin-project-lite |
| pkg-config | 0.3.34 | MIT OR Apache-2.0 | https://github.com/rust-lang/pkg-config-rs |
| png | 0.18.1 | MIT OR Apache-2.0 | https://github.com/image-rs/image-png |
| potential_utf | 0.1.6 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| presser | 0.3.1 | MIT OR Apache-2.0 | https://github.com/EmbarkStudios/presser |
| primal-check | 0.3.4 | MIT OR Apache-2.0 | https://github.com/huonw/primal |
| proc-macro2 | 1.0.107 | MIT OR Apache-2.0 | https://github.com/dtolnay/proc-macro2 |
| profiling | 1.0.18 | MIT OR Apache-2.0 | https://github.com/aclysma/profiling |
| pxfm | 0.1.30 | BSD-3-Clause OR Apache-2.0 | https://github.com/awxkee/pxfm |
| quote | 1.0.47 | MIT OR Apache-2.0 | https://github.com/dtolnay/quote |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 OR Zlib | https://github.com/rust-windowing/raw-window-handle |
| rayon | 1.12.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/rayon |
| rayon-core | 1.13.0 | MIT OR Apache-2.0 | https://github.com/rayon-rs/rayon |
| realfft | 3.5.0 | MIT | https://github.com/HEnquist/realfft |
| renderdoc-sys | 1.1.0 | MIT OR Apache-2.0 | https://github.com/ebkalderon/renderdoc-rs |
| rfd | 0.15.4 | MIT | https://github.com/PolyMeilex/rfd |
| ring | 0.17.14 | Apache-2.0 AND ISC | https://github.com/briansmith/ring |
| rtrb | 0.3.5 | MIT OR Apache-2.0 | https://github.com/mgeier/rtrb |
| rubato | 0.15.0 | MIT | https://github.com/HEnquist/rubato |
| rusqlite | 0.31.0 | MIT | https://github.com/rusqlite/rusqlite |
| rustc-hash | 1.1.0 | Apache-2.0/MIT | https://github.com/rust-lang-nursery/rustc-hash |
| rustc-hash | 2.1.3 | Apache-2.0 OR MIT | https://github.com/rust-lang/rustc-hash |
| rustfft | 6.4.1 | MIT OR Apache-2.0 | https://github.com/ejmahler/RustFFT |
| rustls | 0.23.46 | Apache-2.0 OR ISC OR MIT | https://github.com/rustls/rustls |
| rustls-pki-types | 1.15.1 | MIT OR Apache-2.0 | https://github.com/rustls/pki-types |
| rustls-webpki | 0.103.15 | ISC | https://github.com/rustls/webpki |
| rustversion | 1.0.23 | MIT OR Apache-2.0 | https://github.com/dtolnay/rustversion |
| scopeguard | 1.2.0 | MIT OR Apache-2.0 | https://github.com/bluss/scopeguard |
| seq-macro | 0.3.6 | MIT OR Apache-2.0 | https://github.com/dtolnay/seq-macro |
| serde | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_core | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_derive | 1.0.229 | MIT OR Apache-2.0 | https://github.com/serde-rs/serde |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | https://github.com/serde-rs/json |
| shlex | 2.0.1 | MIT OR Apache-2.0 | https://github.com/comex/rust-shlex |
| simd-adler32 | 0.3.10 | MIT | https://github.com/mcountryman/simd-adler32 |
| smallvec | 1.16.3 | MIT OR Apache-2.0 | https://github.com/servo/rust-smallvec |
| smol_str | 0.2.2 | MIT OR Apache-2.0 | https://github.com/rust-analyzer/smol_str |
| spirv | 0.3.0+sdk-1.3.268.0 | Apache-2.0 | https://github.com/gfx-rs/rspirv |
| stable_deref_trait | 1.2.1 | MIT OR Apache-2.0 | https://github.com/storyyeller/stable_deref_trait |
| static_assertions | 1.1.0 | MIT OR Apache-2.0 | https://github.com/nvzqz/static-assertions-rs |
| strength_reduce | 0.2.4 | MIT OR Apache-2.0 | http://github.com/ejmahler/strength_reduce |
| subtle | 2.6.1 | BSD-3-Clause | https://github.com/dalek-cryptography/subtle |
| symphonia | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-bundle-flac | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-bundle-mp3 | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-codec-adpcm | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-codec-pcm | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-codec-vorbis | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-core | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-format-mkv | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-format-ogg | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-format-riff | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-metadata | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| symphonia-utils-xiph | 0.5.5 | MPL-2.0 | https://github.com/pdeljanov/Symphonia |
| syn | 1.0.109 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| syn | 2.0.119 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| syn | 3.0.9 | MIT OR Apache-2.0 | https://github.com/dtolnay/syn |
| synstructure | 0.14.0 | MIT | https://github.com/mystor/synstructure |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | https://github.com/Stebalien/tempfile |
| termcolor | 1.4.1 | Unlicense OR MIT | https://github.com/BurntSushi/termcolor |
| thiserror | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror | 2.0.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror-impl | 1.0.69 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| thiserror-impl | 2.0.21 | MIT OR Apache-2.0 | https://github.com/dtolnay/thiserror |
| tinystr | 0.8.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| tracing | 0.1.44 | MIT | https://github.com/tokio-rs/tracing |
| tracing-attributes | 0.1.31 | MIT | https://github.com/tokio-rs/tracing |
| tracing-core | 0.1.36 | MIT | https://github.com/tokio-rs/tracing |
| transpose | 0.2.3 | MIT OR Apache-2.0 | https://github.com/ejmahler/transpose |
| ttf-parser | 0.25.1 | MIT OR Apache-2.0 | https://github.com/harfbuzz/ttf-parser |
| type-map | 0.5.1 | MIT/Apache-2.0 | https://github.com/kardeiz/type-map |
| typenum | 1.20.1 | MIT OR Apache-2.0 | https://github.com/paholg/typenum |
| unicode-ident | 1.0.27 | (MIT OR Apache-2.0) AND Unicode-3.0 | https://github.com/dtolnay/unicode-ident |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 | https://github.com/unicode-rs/unicode-segmentation |
| unicode-width | 0.1.14 | MIT OR Apache-2.0 | https://github.com/unicode-rs/unicode-width |
| unicode-xid | 0.2.6 | MIT OR Apache-2.0 | https://github.com/unicode-rs/unicode-xid |
| untrusted | 0.9.0 | ISC | https://github.com/briansmith/untrusted |
| ureq | 2.12.1 | MIT OR Apache-2.0 | https://github.com/algesten/ureq |
| url | 2.5.8 | MIT OR Apache-2.0 | https://github.com/servo/rust-url |
| utf8_iter | 1.0.4 | Apache-2.0 OR MIT | https://github.com/hsivonen/utf8_iter |
| vcpkg | 0.2.15 | MIT/Apache-2.0 | https://github.com/mcgoo/vcpkg-rs |
| version_check | 0.9.5 | MIT/Apache-2.0 | https://github.com/SergioBenitez/version_check |
| vst3 | 0.3.0 | MIT OR Apache-2.0 | https://github.com/coupler-rs/vst3-rs |
| web-time | 1.1.0 | MIT OR Apache-2.0 | https://github.com/daxpedda/web-time |
| webbrowser | 1.2.4 | MIT OR Apache-2.0 | https://github.com/amodm/webbrowser-rs |
| webpki-roots | 0.26.11 | CDLA-Permissive-2.0 | https://github.com/rustls/webpki-roots |
| webpki-roots | 1.0.9 | CDLA-Permissive-2.0 | https://github.com/rustls/webpki-roots |
| wgpu | 22.1.0 | MIT OR Apache-2.0 | https://github.com/gfx-rs/wgpu |
| wgpu-core | 22.1.0 | MIT OR Apache-2.0 | https://github.com/gfx-rs/wgpu |
| wgpu-hal | 22.0.0 | MIT OR Apache-2.0 | https://github.com/gfx-rs/wgpu |
| wgpu-types | 22.0.0 | MIT OR Apache-2.0 | https://github.com/gfx-rs/wgpu |
| widestring | 1.2.1 | MIT OR Apache-2.0 | https://github.com/VoidStarKat/widestring-rs |
| winapi | 0.3.9 | MIT/Apache-2.0 | https://github.com/retep998/winapi-rs |
| winapi-util | 0.1.11 | Unlicense OR MIT | https://github.com/BurntSushi/winapi-util |
| winapi-x86_64-pc-windows-gnu | 0.4.0 | MIT/Apache-2.0 | https://github.com/retep998/winapi-rs |
| windows | 0.52.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows | 0.54.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows | 0.58.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-core | 0.52.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-core | 0.54.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-core | 0.58.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-implement | 0.58.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-interface | 0.58.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-link | 0.2.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-result | 0.1.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-result | 0.2.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-strings | 0.1.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.52.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.59.0 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.60.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-sys | 0.61.2 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-targets | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows-targets | 0.53.5 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnu | 0.52.6 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| windows_x86_64_gnu | 0.53.1 | MIT OR Apache-2.0 | https://github.com/microsoft/windows-rs |
| winit | 0.30.13 | Apache-2.0 | https://github.com/rust-windowing/winit |
| writeable | 0.6.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| xml-rs | 0.8.29 | MIT | https://github.com/kornelski/xml-rs |
| yoke | 0.8.3 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| yoke-derive | 0.8.4 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerocopy | 0.8.63 | BSD-2-Clause OR Apache-2.0 OR MIT | https://github.com/google/zerocopy |
| zerofrom | 0.1.8 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerofrom-derive | 0.1.8 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zeroize | 1.9.1 | Apache-2.0 OR MIT | https://github.com/RustCrypto/utils |
| zerotrie | 0.2.5 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerovec | 0.11.8 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zerovec-derive | 0.11.6 | Unicode-3.0 | https://github.com/unicode-org/icu4x |
| zlib-rs | 0.6.8 | Zlib | https://github.com/trifectatechfoundation/zlib-rs |
| zmij | 1.0.23 | MIT | https://github.com/dtolnay/zmij |
