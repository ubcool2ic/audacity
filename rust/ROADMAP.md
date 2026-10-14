// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

# Audacity Rust Port Roadmap

Repository: `ubcool2ic/audacity`, under `/rust/`

## Goal

Reach full feature parity with Audacity 3.x/4.x through staged development, preserving Audacity project compatibility and adding plugin hosting for VST3, CLAP, LV2, LADSPA, and—later—Nyquist, alongside a native Rust plugin API.

This roadmap describes intended scope, not a claim that planned features are implemented. Stage 1 is complete; each later stage depends on validation and compatibility work in earlier stages.

## Technology stack

| Area | Choice | Justification |
|---|---|---|
| Audio device I/O | `cpal` | Cross-platform Rust device access for playback and recording. |
| Audio decoding | `symphonia` | Rust-native import foundation for WAV, FLAC, MP3, and Ogg Vorbis. |
| WAV export | `hound` | Straightforward WAV writing, including 16-bit, 24-bit, and 32-bit float output. |
| FLAC export | `flacenc` | Rust-native FLAC encoding. |
| MP3/Ogg/Opus encoding | `mp3lame-encoder`, Vorbis and Opus encoders | Add common compressed export formats; verify their licensing and platform support before adoption. |
| AAC/M4A/WMA and video audio | Optional `ffmpeg-next` feature | Broad format coverage without making FFmpeg mandatory for basic builds. |
| Project database | `rusqlite` | SQLite access for `.aup3` compatibility work. |
| Resampling | `rubato` | Quality resampling for playback, recording, and sample-rate conversion. |
| Spectral processing | `rustfft` | FFT foundation for spectrum analysis and spectral views. |
| GUI | `egui`/`eframe` | Chosen over iced: immediate-mode drawing suits custom waveform and spectrogram painting at 60 fps; it is mature and simple to ship as a single crate. Reconsider iced if accessibility needs demand it. |
| CLAP hosting | `clack-host` | Rust-oriented CLAP host support. |
| VST3 hosting | `vst3-sys` or a suitable `vst3` crate | Bindings for VST3 hosting; validate SDK terms and platform coverage. |
| LV2 hosting | `livi` or `lilv` bindings | LV2 discovery and hosting integration. |
| LADSPA hosting | `libloading` | Load LADSPA shared libraries and resolve plugin entry points. |
| Nyquist | C library FFI, later | Reuse compatible Nyquist implementation where practical; isolate FFI and licensing. |
| Native plugin ABI | `abi_stable` | Version-tolerant ABI for independently built Rust plugins. |

## Stages and acceptance criteria

### Stage 1 — Core editing and I/O — DONE

**Scope**
- Core audio model: `AudioBuffer`, `Track`, `Selection`, and snapshot-based `UndoManager`.
- Editing: cut, copy, paste, delete, silence, and apply effects.
- WAV/FLAC/MP3/Ogg Vorbis import; WAV (16/24/32f) and FLAC export.
- Playback and recording through `cpal`.
- Amplify, Normalize, Fade In, Fade Out, and Reverse.
- Plugin API traits; native host and example gain plugin; VST3/CLAP/LV2/LADSPA scanner stubs return `NotImplemented` on instantiate.
- `.aurs` SQLite save/load; `.aup3` detection only.
- Headless CLI (`--selftest`, `process`) and optional `gui` feature with waveform, drag selection, transport, edit buttons, and shortcuts.

**Crates:** `au-core`, `au-io`, `au-effects`, `au-plugin-api`, `au-project`, `au-app`.

**Acceptance tests**
- `cargo test --workspace` covers core editing, undo/redo, WAV/FLAC I/O, effects, and `.aurs` round trips.
- `au-app --selftest` succeeds; CLI processing has deterministic output for its test fixture.
- Effect golden tests compare expected samples; lossless round trips compare decoded samples within format-appropriate tolerances.

### Stage 2 — Clips and timeline

**Scope**
- Clips, split/join, time-shift, multiple tracks, zoom, snap, and selection toolbar.
- Label tracks, envelopes, sample-accurate draw, clip time-stretch, and sync-lock.

**Crates:** `au-core`, `au-app`, `au-project`, `au-effects`.

**Acceptance tests**
- `cargo test -p au-core timeline_` covers split/join, clip moves, snapping, sync-lock, and sample-accurate edits.
- Golden edit sequences verify undo/redo restores all clip, label, and envelope state.
- Timeline interaction tests verify zoom and selection coordinates at multiple sample rates.

### Stage 3 — Audio engine and recording

**Scope**
- Real-time mixer graph; per-track gain, pan, mute, solo, and meters.
- Overdub, punch-in, timer recording, sound-activated recording, and Windows loopback.
- Latency compensation, `rubato` resampling, and selectable dither.

**Crates:** `au-io`, `au-core`, `au-app`, `au-project`.

**Acceptance tests**
- `cargo test -p au-io mixer_` checks routing, gain/pan, mute/solo, and clipping behavior.
- Offline mixer renders match reference fixtures within a documented tolerance; resampling tests verify passband and alias rejection.
- Device-independent tests cover punch boundaries, latency compensation, and timer/sound-activation state transitions.

### Stage 4 — Native project format

**Scope**
- `.aup3` read/write using SQLite sampleblocks and Audacity-compatible XML project data, including its binary XML representation.
- Autosave, crash recovery, and compatibility migration.

**Crates:** `au-project`, `au-core`, `au-app`.

**Acceptance tests**
- Read real `.aup3` fixtures created by Audacity 3.x and compare tracks, clips, labels, and sample data.
- Open/save/reopen fixtures in Audacity and the Rust application; compare database integrity and project content.
- Crash-recovery tests restore a valid project from autosave fixtures. Golden-file comparisons document any intentional serialization differences.

### Stage 5 — Built-in effects parity

**Scope**
- Effects: Amplify, Auto Duck, Bass and Treble, Change Pitch/Speed/Tempo, Click Removal, Compressor, Distortion, Echo, Fade, Filter Curve EQ, Graphic EQ, Invert, Limiter, Loudness Normalization, Noise Gate, Noise Reduction, Normalize, Paulstretch, Phaser, Repair, Repeat, Reverb, Reverse, Sliding Stretch, Truncate Silence, Wahwah, Clip Fix, Vocal Reduction, and remaining Audacity built-ins.
- Realtime effect stacks, nondestructive previews, and presets.

**Crates:** `au-effects`, `au-core`, `au-app`, `au-project`.

**Acceptance tests**
- `cargo test -p au-effects` checks each effect’s parameters, boundaries, and repeatability.
- Golden renders compare against C++ Audacity output using documented tolerances (target RMS difference below -90 dBFS where algorithms and parameters are equivalent).
- Preset and effect-stack save/load tests preserve order and parameter values.

### Stage 6 — Generators and analyzers

**Scope**
- Generators: Tone, Chirp, Noise, DTMF, Rhythm Track, and Silence.
- Analyzers: Plot Spectrum, Contrast, Find Clipping, Beat Finder, Label Sounds, and Silence Finder.

**Crates:** `au-effects`, `au-core`, `au-app`, `au-project`.

**Acceptance tests**
- Generator golden files verify duration, frequency, level, and sample counts.
- Analyzer fixtures compare reported measurements and label locations against Audacity reference output.
- Tests cover empty selections, short inputs, and edge cases without panics.

### Stage 7 — Plugin hosting

**Scope**
- CLAP, VST3, LV2, and LADSPA instantiation, processing, parameters, state, and GUI windows.
- Plugin manager, scanning, validation, and out-of-process scanning sandbox.
- Native Rust plugin ABI using `abi_stable`; retain the Stage 1 native example.
- Nyquist remains a later addition in Stage 9.

**Crates:** `au-plugin-api`, `au-io`, `au-core`, `au-app`, `au-project`.

**Acceptance tests**
- Per-format test plugins verify scan, instantiate, process, parameter changes, state round trips, and clean unload.
- Sandbox tests confirm a crashing or timing-out scanner does not crash the application.
- Native ABI compatibility tests load plugins built against supported API versions; golden processing tests compare known plugin output.

### Stage 8 — Import/export completeness

**Scope**
- MP3, Ogg, Opus, WavPack, and AIFF export; metadata tag editor and multi-export.
- Optional FFmpeg-backed AAC/M4A/WMA and video-audio import.
- Raw audio import and MIDI/Note tracks via `midly`.

**Crates:** `au-io`, `au-core`, `au-project`, `au-app`.

**Acceptance tests**
- Codec round trips verify metadata, channel layout, duration, and decoded audio within codec-specific tolerances.
- Golden tag fixtures cover read, edit, and write; multi-export tests verify naming and selection behavior.
- Raw import tests cover format parameters and truncation; MIDI fixtures verify timing, notes, and labels.
- FFmpeg-enabled tests are feature-gated and skipped cleanly when FFmpeg is unavailable.

### Stage 9 — Spectral tools, automation, and Nyquist

**Scope**
- Spectrogram and spectral editing, frequency selection, and pitch view.
- Macros and batch chains, including `.txt` macro compatibility.
- Mod-script-pipe scripting with named pipes and Audacity-compatible command vocabulary.
- Nyquist plugin support through a later C-library FFI integration.

**Crates:** `au-app`, `au-core`, `au-effects`, `au-project`, `au-plugin-api`, `au-io`.

**Acceptance tests**
- Spectral selection/edit fixtures verify frequency/time boundaries and rendered output against golden files.
- Macro compatibility tests run representative Audacity `.txt` chains and compare resulting projects.
- A Python client compatibility test sends commands over named pipes and checks responses and project changes.
- Nyquist fixtures verify plugin discovery and deterministic output; FFI failure tests ensure errors are contained.

### Stage 10 — UI parity and polish

**Scope**
- Remaining toolbars, preferences, themes, keyboard shortcut compatibility, and complete workflows.
- Accessibility via AccessKit, localization, and usability polish.
- audio.com cloud save/share integration.
- Installable packaging for Windows, macOS, and Linux.

**Crates:** `au-app`, `au-core`, `au-io`, `au-project`, plus packaging and integration components.

**Acceptance tests**
- UI automation covers common editing, recording, import/export, preferences, and recovery workflows.
- Accessibility checks exercise keyboard navigation, accessible names, and screen-reader semantics.
- Localization tests verify translated menus and dialogs without layout regressions.
- CI builds and smoke-tests installers on supported operating systems; cloud integration uses mocked-service tests plus an opt-in end-to-end test.

## Feature parity checklist

| Feature | Audacity area | Stage | Status |
|---|---|---:|---|
| Core tracks, selection, undo, cut/copy/paste/delete/silence | Editing | 1 | Done |
| WAV/FLAC export; initial WAV/FLAC/MP3/Ogg import | Import/export | 1 | Done, testing incomplete |
| Basic effects and native plugin API | Effects/plugins | 1 | Done |
| `.aurs` project save/load | Projects | 1 | Done |
| `.aup3` read/write and recovery | Projects | 4 | Planned |
| Clips, labels, envelopes, sync-lock | Timeline | 2 | Done (GUI not verified on a display) |
| Mixer, recording features, resampling, dither | Audio engine | 3 | Done (no real-device test yet) |
| Built-in effects parity and realtime stacks | Effects | 5 | Planned |
| Generators and analyzers | Analyze/generate | 6 | Planned |
| CLAP/VST3/LV2/LADSPA hosting | Plugins | 7 | Scanner stubs only |
| Complete codecs, metadata, raw and MIDI import | Import/export | 8 | Planned |
| Spectral tools, macros, scripting, Nyquist | Analysis/automation | 9 | Planned |
| Full UI, accessibility, cloud, packaging | Application | 10 | Planned |

## Stage 2 status (2026-10-14)

- [x] Multiple clips per track with gaps (rendered as silence); split (sample-accurate), join (gaps filled with silence), time-shift with overlap rejection
- [x] Clip-aware cut/copy/paste/delete/apply; sync-lock ripples cuts/pastes across wave and label tracks
- [x] Zoom (`ZoomInfo`, default 86.13 px/s like Audacity, anchored zoom, fit) and snap (grid, samples, zero crossings, labels)
- [x] Label tracks: point/region labels, add/edit/move/delete, Audacity label `.txt` import (skips spectral `\` lines) and export
- [x] Volume envelope (0..2 linear, linear interpolation) applied in render, playback and export
- [x] Clip time-stretch, pitch preserved, via in-house WSOLA (Hann window ~23 ms, 50% overlap, ±10 ms cross-correlation search; linear-interp fallback for very short clips). `rubato` was not used because it resamples (changes pitch) rather than stretching. A phase vocoder / Audacity's "Staffpad" stretcher is a future quality upgrade.
- [x] All of the above are undoable and persisted in `.aurs` format v2 (v1 files still load)
- [x] egui GUI: Selection / Envelope / Time Shift tools, ruler, zoom buttons and Ctrl+wheel, snap combo, sync-lock, Split (Ctrl+I), Join (Ctrl+J), Change Clip Speed, label track drawing/editing (Ctrl+B), label import/export
- [ ] GUI interaction verified on a real display (launches and renders under Xvfb; clicks not exercised)
- [x] Selection toolbar (Start/End/Length typed `hh:mm:ss.mmm` fields) and Draw tool (sample draw at ≥1 px/sample, one undo per stroke; refused on stretched clips) — added in Stage 3 pass

## Stage 3 status (2026-10-14)

- [x] Lock-free engine (`au-io::engine`): audio callback drains an `rtrb` SPSC command ring, mixes preallocated buffers, publishes meters/position via atomics; no locks or allocation in the callback
- [x] Multitrack mixer: per-track gain, constant-power pan (mono) / balance (stereo), mute, solo, master gain, clamp + clip counter (`mixer_*` tests)
- [x] Peak/RMS meters per track and master; record input peak meter
- [x] Offline render matches reference fixtures (tolerance 1e-6; 1e-5 where constant-power pan rounding applies)
- [x] Playback resampling with `rubato` (sinc 256, BlackmanHarris2) when device rate ≠ project rate; tests: passband within 0.1 dB, alias rejection ≥ 60 dB (23 kHz → 32 kHz), impulse alignment ±2 frames. Tracks are resampled before the stream starts; `StreamResampler` exists for streaming use
- [x] Dither on export bit-depth reduction: None / Rectangle / Triangle (default) / Shaped (1st-order error feedback); WAV export uses it, FLAC export still truncates via its own path
- [x] Recording: new track, overdub (plays mix while recording), latency compensation, punch-and-roll with preroll + crossfade, timer record, sound-activated record (threshold + hold); capture via SPSC ring
- [x] System-audio loopback: Windows WASAPI only; Linux = choose a PulseAudio/PipeWire "Monitor of…" input; macOS unsupported (needs BlackHole/ScreenCaptureKit). See `docs/AUDIO_ENGINE.md`
- [x] Play-at-speed (0.1–3×, linear interpolation, negative = reverse) and loop playback
- [ ] Scrubbing UI (engine supports negative/variable speed + seek; no mouse-scrub gesture yet)
- [x] Headless null backends (`NullOutput`, `NullInput`) — playback and record paths tested without hardware (`au-app::transport` tests)
- [x] Undo/project: mixer changes (`Track Mix`), recordings (`Add Track`), punch (`Punch and Roll Record`), Draw; gain/pan/mute/solo persist in `.aurs`
- [x] egui: transport (Play/Pause/Stop/Loop/speed/Record), meters, mixer side panel, record options, export dither/bit-depth
- [ ] Not verified on real audio hardware (box has no sound device): cpal output/input streams, loopback, real latency values
- [ ] Punch-and-roll flattens that track's clips into one clip; per-clip punch is a later refinement

## Stage 4 status (2026-10-14)

Verified against **real Audacity 3.7.3** (Debian package, run headlessly under Xvfb via mod-script-pipe). See `docs/AUP3_FORMAT.md`.

- [x] `.aup3` SQLite layout identical to Audacity 3.x: `application_id` = `AUDY`, `user_version` packed version, `project`/`autosave`/`sampleblocks` tables (schema SQL compared byte-for-byte with an Audacity-made file)
- [x] ProjectSerializer binary XML: dict + doc decode/encode, char sizes 1/2/4, all field types incl. Push/Pop and Blob (`au-project::binxml`)
- [x] Sample blocks: int16 / int24 / float, Audacity block sizes, min/max/rms + summary256 + summary64k — summaries **byte-identical** to Audacity's for every block in the fixtures (`au-project::blocks`)
- [x] Tracks (mono + linked stereo), clips (offset, trims applied on read, name), per-clip envelopes (relative times ↔ track envelope), labels (UTF-8), project rate, track rate, gain/pan, mute/solo, selection, sample format per track, clip `clipStretchRatio` (writes format 3.4 when used), silent blocks (negative ids)
- [x] Real Audacity fixtures (`tests/fixtures/aup3`, regenerate with `tools/make_aup3_fixtures.sh`): int16 mono 2 clips + envelope + labels + selection, float stereo 48 kHz muted/panned, int24 7 s multi-block, killed-session file with an orphan block
- [x] Round trip: Audacity file → Rust save → Rust reopen equals; stored sample bytes identical; `PRAGMA integrity_check` ok
- [x] Reopen in Audacity (`tools/verify_in_audacity.sh`): for every fixture, Audacity's WAV export of the Rust-saved copy is byte-identical to its export of the original, and Audacity's GetInfo (clips, labels, envelopes) is identical
- [x] Rust-built project (stereo, int24, gain 0.5, pan −0.3, stretched clip, envelope point, UTF-8 label) opens in Audacity with the right values; Audacity re-saves it and Rust reads back an identical state
- [x] Autosave journal into the `autosave` table (saved doc untouched, orphan blocks pruned); temporary (never-saved) projects
- [x] Crash recovery: `inspect()` reports pending autosave and orphan blocks; recover / discard; real Audacity 3.7.3 opens a Rust-written autosave and shows "recovered to the last snapshot" with the recovered content
- [x] GUI: File ▸ Open (.aup3/.aurs), Save Project, Save Project As (.aup3 default); autosave every edit (≤ once / 2 s) for .aup3 projects; Automatic Crash Recovery dialog (Recover / Discard / Cancel) — exercised under Xvfb with a real click
- [x] CLI: `audacity-rs <file>` opens a project; `audacity-rs aup3-info <file>`
- [x] `.aurs` v2 path kept (chosen by extension)
- [ ] Not modelled (dropped on save): tags/metadata, realtime effect stacks, time/note tracks, track colors/heights, spectrogram settings, clip pitch (`centShift`), `rawAudioTempo`, project tempo/time signature
- [ ] Trims are applied destructively on read (Audacity's hidden trimmed audio is not kept)
- [ ] Audacity's own autosave rows were not captured in a fixture (scripted edits did not journal before SIGKILL); autosave fixtures are Rust-written and verified by opening them in Audacity
- [ ] Older formats: pre-3.0 `.aup` + `_data` folders not imported; files > 3.7 rejected
- [ ] Not tested against Audacity on Windows/macOS (char size 2 is supported by the decoder but no real Windows file was tested)

## Known Stage 1 gaps

- The GUI has not been built or verified on the current CI box because it has no display and insufficient disk space.
- ~~Playback resampling is still TODO.~~ Done in Stage 3.
- MP3 and Ogg import compile, but automated coverage currently exercises only WAV and FLAC.
- ~~`.aup3` is detected but is not yet readable or writable.~~ Done in Stage 4.

## License

This project is GPL-3.0-or-later and is a derived work of Audacity. Preserve Audacity copyright notices and applicable attribution in derived files and distributions. Review every third-party dependency and SDK for GPL compatibility before adoption; avoid GPL-incompatible components such as the VST2 SDK. The VST3 SDK is GPLv3-dual-licensed and is acceptable when used under its GPLv3 terms. Keep optional dependencies and their license obligations documented.

## Stage 5/6 parity status (2026-10-14)
Audacity 3.7.3 comparison (`tools/compare_effects.py`, results in `docs/effects-compare-3.7.3.jsonl`): **55/55 cases pass**; 52 are bit-exact (−240 dB RMS diff).
- Change Speed/Pitch/Tempo link the same system libsoxr 0.1.3 / libSoundTouch 2.4.0 as Debian's Audacity 3.7.3 (C shim in `crates/au-effects/csrc`).
- Classic Filters: Audacity 3.7.3 segfaults when it is scripted (null `mpBiquad` in `ScienFilterBase::ProcessInitialize`), so the reference is Audacity's own `lib-math.so` run through `tools/aud_ref/classic_filters_ref.cpp`.
- DTMF: when scripted, 3.7.3 ignores Sequence/Duty Cycle (stale `Recalculate`), so the harness uses the real dialog via xdotool, with default settings.
- Tone: when scripted, 3.7.3 skips `PostSet`, so the reference is Chirp with start = end, which is the same code path.
- Not yet exact: Rhythm Track (−57 dB RMS, passes −40 dB threshold), Pluck (−167 dB), Studio Fade Out (−119 dB), Beat Finder labels within 1 ms.
- Not yet compared: Noise Reduction (needs a GUI-captured noise profile), Repair, Vocoder, Sliding Stretch, Vocal Reduction, Risset Drum, Contrast, Plot Spectrum.
