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
| Clips, labels, envelopes, sync-lock | Timeline | 2 | Planned |
| Mixer, recording features, resampling, dither | Audio engine | 3 | Planned |
| Built-in effects parity and realtime stacks | Effects | 5 | Planned |
| Generators and analyzers | Analyze/generate | 6 | Planned |
| CLAP/VST3/LV2/LADSPA hosting | Plugins | 7 | Scanner stubs only |
| Complete codecs, metadata, raw and MIDI import | Import/export | 8 | Planned |
| Spectral tools, macros, scripting, Nyquist | Analysis/automation | 9 | Planned |
| Full UI, accessibility, cloud, packaging | Application | 10 | Planned |

## Known Stage 1 gaps

- The GUI has not been built or verified on the current CI box because it has no display and insufficient disk space.
- Playback resampling is still TODO.
- MP3 and Ogg import compile, but automated coverage currently exercises only WAV and FLAC.
- `.aup3` is detected but is not yet readable or writable.

## License

This project is GPL-3.0-or-later and is a derived work of Audacity. Preserve Audacity copyright notices and applicable attribution in derived files and distributions. Review every third-party dependency and SDK for GPL compatibility before adoption; avoid GPL-incompatible components such as the VST2 SDK. The VST3 SDK is GPLv3-dual-licensed and is acceptable when used under its GPLv3 terms. Keep optional dependencies and their license obligations documented.
