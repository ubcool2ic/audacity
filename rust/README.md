# Audacity-RS (Rust port of Audacity) — Stage 1

GPL-3.0-or-later. This is a derivative work of [Audacity](https://www.audacityteam.org/) © Audacity Team and contributors; see `../LICENSE.txt` and the original notices in this repository. The port lives entirely in `/rust/`.

## Build & test
```
cd rust
cargo build && cargo test            # headless: core, io, effects, plugin api, project, CLI
cargo run -p au-app -- --selftest    # no audio device needed
cargo run -p au-app -- process in.mp3 out.flac normalize:-1 fadein
cargo run -p au-app --features gui   # egui desktop editor (needs a display + ALSA)
```
Requires Rust ≥ 1.85; Linux needs `pkg-config` and `libasound2-dev` (cpal).

## Crates
| crate | purpose |
|---|---|
| au-core | AudioBuffer, Track, Selection, unlimited snapshot undo/redo, Editor (cut/copy/paste/delete/silence/apply) |
| au-io | Import WAV/FLAC/MP3/Ogg (symphonia), export WAV (hound) / FLAC (flacenc), cpal Player/Recorder |
| au-effects | Amplify, Normalize, Fade In, Fade Out, Reverse |
| au-plugin-api | Plugin host traits, native Rust plugin API + example gain plugin, VST3/CLAP/LV2/LADSPA scanners (instantiate = Stage 7) |
| au-project | Stage-1 `.aurs` SQLite project save/load; `.aup3` detection (read/write = Stage 4) |
| au-app | `audacity-rs` binary: headless CLI + egui GUI (`gui` feature) |

See [ROADMAP.md](ROADMAP.md) for the staged parity plan and [docs/COSTS.md](docs/COSTS.md) for AI-assist spend.
Code was drafted with OpenRouter `openai/gpt-6-luna` and reviewed/fixed by hand.
