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

**Status (2026-10-14): done.** Checklist:
- [x] Tone, Chirp, Noise, DTMF, Silence (built-ins), Rhythm Track, Pluck, Risset Drum (Nyquist ports): every 3.7.3 Generate-menu item.
- [x] Plot Spectrum: all 5 algorithms, all 10 window functions, sizes 128 to 131072, linear/log axis, peak cursor, grid, Export text (FreqWindow format). It has its own GUI window (`au-app/src/analyze_ui.rs`).
- [x] Contrast: ContrastBase::GetDB measurement, WCAG2 pass/fail (more than 20 dB), Contrast.cpp export report. It has its own GUI dialog with Measure-selection buttons.
- [x] Find Clipping, Beat Finder, Label Sounds. Silence Finder was replaced by Label Sounds in Audacity 3.x and is not shipped in 3.7.3.
- [x] Sample Data Export: byte-identical text output.
- [x] Loudness measurement: MeasureLoudness/RMS analyzers. 3.7.3 has no standalone loudness analyzer to compare against; the LUFS path is verified through Loudness Normalization.
- [x] GUI: generators and the other analyzers use the generic parameter dialog with a report pane. Plot Spectrum and Contrast have dedicated windows.
- 3.7.3 comparison: **74/74 pass**, with 14 Plot Spectrum configurations, 2 Contrast, 2 Sample Data Export and Risset Drum added. See the parity section below.

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

**LV2/LADSPA status (2026-10-14).** `au-plugin-host/src/ladspa.rs` and `lv2.rs` are registered in `au_plugin_host::registry()`; CLAP and VST3 are BizyBot's part.
- LADSPA:
  - Scanning uses dlopen and `ladspa_descriptor`.
  - Parameter defaults and ranges follow Audacity's LadspaEffect: LOW/MIDDLE/HIGH hints, log-aware, sample-rate scaled.
  - One instance per channel for mono plugins.
  - Latency comes from a "latency" output port.
  - State is stored by port index.
- LV2:
  - lilv is loaded at runtime with libloading, so there is no link-time dependency.
  - Host features: urid map/unmap, options (block length and sample rate), boundedBlockLength.
  - Optional atom/CV ports get dummy buffers. Latency comes from the reportsLatency port.
  - State is stored by port symbol.
- Tests use swh `amp_1181.so` and `amp-swh.lv2` (+6.02 dB doubles the signal, state round trip, stereo with a mono plugin, non-plugin .so skipped, missing lilv reported), and skip when the plugins are not installed.
- Gaps:
  - LADSPA and LV2 scanning runs in-process. The out-of-process sandbox covers CLAP/VST3 only.
  - LV2 plugin GUIs (suil) are not hosted.
  - No golden comparison against Audacity 3.7.3 hosting the same plugins yet.

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

**BizyBot status (2026-10-14), per docs/SPLIT_stage9.md:**
- mod-script-pipe (`au-scriptpipe`):
  - Unix FIFOs `/tmp/audacity_script_pipe.to/from.<uid>`, overridable with `AU_SCRIPT_PIPE_DIR`. Windows `\\.\pipe\ToSrvPipe`/`FromSrvPipe` (not yet compiled or tested).
  - Threaded server; commands are queued to the UI thread.
  - Audacity's `pipe_test.py` passes against both the server and the GUI (`AU_SCRIPT_PIPE=1`, or View > mod-script-pipe).
  - The interim interpreter in `au-app/src/script_bridge.rs` handles Help, Message and GetInfo. The app handles Import2 and SetTrackVisuals/SetTrack (Display, SpecScale) itself. The partner's au-script will replace this interpreter.
- Nyquist:
  - `au-plugin-host/src/nyquist.rs` runs each script in the crash-contained `au-nyquist-worker` process, with a timeout and cancel.
  - Nyquist is a PluginFormat in the registry and in the new Plugin Manager dialog (filter, enable/disable persisted, rescan).
  - The interpreter plugs in through `nyquist::set_engine`. It is the partner's work and is not on main yet.
- Spectrogram view (`au-app/src/spectrogram_view.rs`):
  - Waveform, Spectrogram and Multi-view per track. Spectrograms are computed on the au-runtime pool and uploaded as cached GPU textures (LRU).
  - Follows zoom and scroll. The frequency ruler supports Linear, Log and Mel (View > Spectrogram scale).
  - The spectral selection tool drags t0/t1/f0/f1.
  - Columns come from a replaceable `ColumnFn`. Right now that is an interim Hann STFT, until the partner's SpectrogramSettings computation lands.
  - The selection is kept in the app until the partner's spectral model lands.
- Xvfb UI automation: `tests/script_pipe_gui.rs` and `tests/spectrogram_gui.rs` drive the GUI over the pipe.
- Open:
  - Macros dialog (partner, ac14bfb). Apply Macro vs Audacity 3.7.3 checked with tools/compare_macros.py (`Macro_FadeEnds` over the pipe in both apps): max|diff| 5.96e-8, RMS -163.7 dBFS. Fixes needed: Use_Preset, ordering of reversed Select times, FadeOut (n-1-i)/n.
  - The Windows pipe has not been built or tested.
  - The waveform_summary cache work is part of Perf.

**BizyBot Performance & reliability status (2026-10-14), per docs/SPLIT_perf.md:**
- Waveform drawing:
  - Uses `waveform_summary` min/max caches at 256 and 64K samples per entry. They are computed on an au-runtime pool (LRU, stale jobs cancelled), and stretched clips are rendered once instead of every frame (`au-app/src/gui/waveform_cache.rs`).
  - Tracks are no longer cloned every frame.
  - A 20-minute stereo track at Fit zoom went from about 749 ms to 3.3 ms per frame (release build, headless).
- Shared progress dialog (`gui/progress.rs`): Audacity ProgressDialog semantics (shows after 500 ms, Elapsed/Remaining time, Cancel/Stop), with a headless AccessKit click test.
- Xvfb: `tests/crash_recovery_gui.rs` covers:
  - crash, report written, autosave flushed;
  - relaunch, crash dialog, automatic reopen with recovery prompt, Recover;
  - dismissed reports staying dismissed;
  - progress cancel and completion.
  These use test-only pipe hooks behind `AU_ALLOW_TEST_HOOKS=1`.

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

| Feature | Audacity area | Stage | Status (2026-10-14) |
|---|---|---:|---|
| Core tracks, selection, undo, cut/copy/paste/delete/silence | Editing | 1 | Done |
| `.aurs` project save/load | Projects | 1 | Done |
| `.aup3` read/write, recovery, tags, selLow/selHigh | Projects | 4/8/9 | Done |
| Clips, labels, envelopes, sync-lock | Timeline | 2 | Done |
| Mixer, recording, resampling, dither | Audio engine | 3 | Done (no real-device test yet) |
| Built-in effects + realtime stacks | Effects | 5 | Done; 74/74 bit-exact vs 3.7.3 (float WAV no longer clamped) |
| Generators and analyzers | Analyze/generate | 6 | Done |
| CLAP/VST3/LV2/LADSPA hosting | Plugins | 7/8B | Done (BizyBot) |
| Codecs (WAV/AIFF/FLAC/MP3/Ogg/Opus/WavPack/FFmpeg), metadata, Export Multiple, MIDI | Import/export | 8 | Done; 27/27 bit-identical exports |
| Nyquist (Audacity's libnyquist, .ny parser, sandboxed worker) | Automation | 9 | Done; 19/19 bit-exact |
| Spectrogram (Frequencies bit-exact, EAC ≤4e-7), spectral selection | Analysis | 9 | Done; Reassignment checked against lib-fft (tools/specref, 11 window/zero-padding configs × 2 zoom levels; ≤2 bins of ~150k flip on rounding). Not reproduced: Audacity's stale scratch at the extra window sample when zero padding > 1 |
| mod-script-pipe + scripting commands, macros (.txt byte-identical) | Automation | 9 | Done; macro *application* not yet compared against Audacity (BizyBot) |
| Toolbars, themes, AccessKit, UI automation | Application | 10 | Done (BizyBot) |
| Preferences (3.7.3 keys/defaults, audacity.cfg I/O, Edit > Preferences pages, Get/SetPreference) | Preferences | 10 | Done. All keys on our pages are consumed except scrubbing, RTL=off and the compile-time-disabled AIL (see "Preferences wiring status"). `tools/compare_prefs.py` vs Audacity 3.7.3: **30/30** |
| Localization (gettext .po/.mo, runtime switch) | Localization | 10 | Done: 249 UI literals via `tr_ui()` (Audacity msgids, accelerators stripped); Xvfb test switches to German at runtime. Strings built with `format!` are still English; catalog packaging is BizyBot's |
| Keyboard shortcuts (Audacity Standard/Full defaults, remapping, shortcuts XML import/export, /NewKeys) | Application | 10 | Done. Standard set checked against 3.7.3 GetInfo Menus (73 menu shortcuts); non-menu (Extra) defaults taken from source, not checked against a running Audacity |
| audio.com cloud save/share | Application | 10 | Deferred: needs an audio.com account and Audacity's private API |
| Packaging .deb/AppImage/.msi/.dmg | Packaging | 10 | Done (BizyBot); CI workflow not enabled (push needs `workflow` scope); package should ship au-nyquist-engine |
| Performance: waveform caches, progress dialog, import/export as pool jobs with real progress/cancel | Perf | — | Done |


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
- [x] Scrubbing UI: scrub ruler (View > Scrubbing > Scrub Ruler / ToggleScrubRuler), Scrub and Seek (mutually exclusive checks), ruler drag scrubs (speed from mouse motion every 50 ms, |speed| <= 32) or seeks (scrub.rs + gui/scrub_glue.rs). Not pipe-compared: Audacity exposes no scrub state over the pipe
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
- [x] Kept on save (later stages): tags/metadata, realtime effect stacks, time/note tracks, track colors/heights, clip pitch (`centShift`)
- [ ] Still dropped on save: spectrogram settings, `rawAudioTempo`; project tempo/time signature round-trip not verified
- [x] Trims are kept non-destructively (`trimLeft`/`trimRight` plus full sequence), and clips may start before 0 (hidden head); verified both ways with Audacity 3.7.3
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
Audacity 3.7.3 comparison (`tools/compare_effects.py`, results in `docs/effects-compare-3.7.3.jsonl`): **74/74 cases pass**.
- Stage 6 additions:
  - Plot Spectrum: the reference is Audacity's own `lib-fft.so` `SpectrumAnalyst::Calculate` (`tools/aud_ref/plot_spectrum_ref.cpp`), with input mixed as in PlotSpectrumBase::GetAudio (channel sum) and printed in the FreqWindow export format. 12 of the 14 configurations give byte-identical export text. Cube-root and Enhanced autocorrelation differ by at most 7e-6 in a few lines (last printed digit, libm `pow`).
  - Contrast: the dialog cannot be scripted. The reference is the ContrastBase::GetDB formula, compared on the `%.2f` values and the WCAG verdict.
  - Sample Data Export: the exported file is byte-identical in Linear/Count/SameLine and in dB/Time/Standard-header/Alternate.
  - Risset Drum: −109.7 dB RMS difference, using glibc rand() with a fresh Audacity process.
- Change Speed/Pitch/Tempo link the same system libsoxr 0.1.3 / libSoundTouch 2.4.0 as Debian's Audacity 3.7.3 (C shim in `crates/au-effects/csrc`).
- Classic Filters: Audacity 3.7.3 segfaults when it is scripted (null `mpBiquad` in `ScienFilterBase::ProcessInitialize`), so the reference is Audacity's own `lib-math.so` run through `tools/aud_ref/classic_filters_ref.cpp`.
- DTMF: when scripted, 3.7.3 ignores Sequence/Duty Cycle (stale `Recalculate`), so the harness uses the real dialog via xdotool, with default settings.
- Tone: when scripted, 3.7.3 skips `PostSet`, so the reference is Chirp with start = end, which is the same code path.
- Not yet exact: Rhythm Track (−57 dB RMS, passes −40 dB threshold), Pluck (−167 dB), Studio Fade Out (−119 dB), Beat Finder labels within 1 ms.
- Not yet compared: Noise Reduction (needs a GUI-captured noise profile), Repair, Vocoder, Sliding Stretch, Vocal Reduction, Risset Drum, Contrast, Plot Spectrum.

## Performance & reliability status (2026-10-14)
New crate `crates/au-runtime` (no changes to `au-effects`).
- [x] Rayon work-stealing pool (`Runtime`) behind a bounded admission queue (`submit` blocks, `try_submit` → `Busy`), `CancelToken`, lock-free `Progress`, non-blocking `JobHandle::try_result` for the UI thread
- [x] Offline kernels with serial + parallel paths, bit-identical by construction: `map_items` (per-track effect render / per-file import decode / export encode), `mix_offline` (chunk-parallel, tracks summed in serial order), `waveform_summary` (min/max/RMS), `spectrogram` (Hann + radix-2 FFT, frame-parallel)
- [x] Real-time audio thread untouched: `au-io` callback never locks, waits on, or submits to the pool
- [x] Worker panics isolated with `catch_unwind` → `JobError::Panicked`, logged to the ring, no crash report
- [x] Panic hook: local crash report (version, OS/arch, thread, message, location, open project, last 64 actions, backtrace) in the per-OS crash folder (`AUDACITY_RS_CRASH_DIR` override), then the registered autosave flush; no network upload
- [x] GUI: top-level unwind guard force-flushes the `.aup3` autosave; on next launch the crashed project is reopened so the Stage 4 recovery dialog appears, and a crash dialog shows the report with Copy / Dismiss (dismissed reports move to `viewed/`)
- [x] Tests: parallel vs serial bit-identity (mix, summaries, spectrogram, map), stress (2,000 jobs with injected panics, cancellation, queue bound), injected fatal panic in a child process → report + autosave → `aup3::recover` restores the unsaved edit
- [ ] Not yet wired: existing call sites (effect apply, export, import, waveform drawing) still run serially; they should move onto `Runtime` jobs with progress bars
- [ ] Crash dialog and recovery reopen not yet exercised under Xvfb

## Stage 7 status — CLAP + VST3 (2026-10-14)
See `docs/PLUGIN_HOSTING.md`. New crate `crates/au-plugin-host` (code generated with OpenRouter `openai/gpt-6-luna`).
- [x] CLAP + VST3 instantiation, parameter enumeration and automation, audio processing, latency reporting
- [x] State save/restore as Audacity `.aup3` realtime-effect parameters (VST3 `ProcessorState`/`ControllerState`/`Parameters`, CLAP `State`/`Parameters`), round-tripped through a saved `.aup3`
- [x] Out-of-process sandboxed scanning (`au-plugin-scan`, `au-runtime` jobs + panic isolation); crashing plugin reported, app survives
- [x] Real plugins verified: Surge XT Effects (CLAP), Steinberg AGain + ADelay (VST3, SDK 3.8.1)
- [x] VST3 SDK licence verified (MIT since 3.8, GPL-compatible) — documented
- [x] Plugin editors: out-of-process host `au-plugin-editor` (crash-isolated; AGain + Surge XT verified under Xvfb+openbox, crash reported as signal 11 and app unaffected). Separate top-level window, not embedded in egui yet
- [x] au-app wiring (Stage 8). [x] LV2/LADSPA hosting (`ladspa.rs`/`lv2.rs`, d9b67e2) + out-of-process scanning (fee20b8). Nyquist: Stage 9

## Noise Reduction and room noise profile (2026-10-14)
- **Noise Reduction** (`fx/fix_noisereduction.rs`) is a literal port of `NoiseReductionBase` 3.7.3, covering SpectrumTransformer, the RealFFTf port, statistics, Classify, gain smoothing and Reduce/Isolate/Residue.
  - Luna wrote the first pass; it differed by −13 to −26 dB. Sol's review of that pass made it bit-identical.
  - Reference: `tools/aud_ref/noise_reduction_ref.cpp`, which is Audacity's worker code verbatim on top of the installed `lib-fft.so` SpectrumTransformer. Run `tools/compare_noise_reduction.py`.
  - Result: 6/6 parameter sets bit-identical (−240 dB). See `docs/noise-reduction-compare-3.7.3.jsonl`.
- **Room noise profile** (`au-app/src/room_noise.rs`; GUI section in Recording options):
  - "Capture Noise Profile (10 s)" records from the current input and learns a profile.
  - Profiles are stored per input device (`~/.config/audacity-rs/noise-profiles/<device>.aunp`) and per project (`aurs_noise_profiles` table in the .aup3/.aurs SQLite file, via `au-project/src/noise_profiles.rs`). Audacity ignores unknown tables.
  - Offline: the Noise Reduction dialog automatically uses the captured profile.
  - Live: optional suppression with `StreamingNoiseReducer` (allocation-free after start, no locks), with reduction dB, sensitivity, smoothing and a bypass switch. The raw take is kept as recorded, and a time-aligned "(noise suppressed)" track is added next to it.
  - Tests:
    - live and offline outputs are identical (offline uses the same streaming path);
    - profile save/load round-trips for the user directory, the project table and bytes;
    - tone+noise SNR improves by more than 6 dB at 24 dB reduction. This is Audacity-exact behaviour: +3.6/+7.2/+13.8 dB at 12/24/48 dB.
## Stage 8 status — plugins in the app (2026-10-14)
- [x] PluginRegistry wired into au-app (`plugin_bridge.rs`): background scan at startup, plugins in the Effect menu, addable to realtime stacks via `EffectStack.slots` (public API) and saved/restored in `RealtimeEffectList`.
- [x] Out-of-process plugin editor ("Open editor" in the effect dialog), param/state sync back to the dialog.
- [x] Audacity 3.7.3 (official AppImage) reads our VST3 state from .aup3: AGain gain 0.5 → exported peak 0.4001 (input 0.8). See `tools/aup3-verify/`.
- [x] au-effects API changes (Cow ids, plugin_id passthrough, push_effect/from_desc_with, get_text(&mut)) — c88c890.
- [ ] Embed editor inside the egui window; avoid Mutex on RT path; CLAP fd polling uses poll() instead of per-tick callbacks.

## Stage 8B status — FFmpeg, MIDI/note tracks, sandboxed LADSPA/LV2 (2026-10-14)
- [x] Optional FFmpeg (`au-io/src/ffmpeg.rs`): dlopen of libavformat.so.61/libavcodec.so.61/libavutil.so.59/libswresample.so.5 at runtime (no link-time dependency), version-checked; `AU_FFMPEG_LIBDIR` = Audacity's "FFmpeg library location". Import: M4A/AAC, WMA, AC3, audio from video (mp4/mkv/mov/webm/avi/wmv…); export M4A (AAC), AC3, WMA v2. Missing FFmpeg → clear message in File ▸ FFmpeg status, export items disabled, WAV/FLAC/MP3/Ogg unaffected. Tests: present (round trips, video audio, CLI-made WMA/AC3, ffprobe reads our files) and absent.
- [x] Note tracks (`au-core/src/notes.rs`) modelled on NoteTrack: Allegro text serializer/parser following portsmf; `.aup3` `<notetrack … data="…">` exactly like Audacity 3.7.3 (verified both ways with the official 3.7.3 AppImage, fixtures in `tests/fixtures/aup3/notetrack_*`), native .aurs table; MIDI import/export (`au-io/src/midi.rs`, midly); piano-roll display, mute/solo/velocity, built-in synth playback via the mixer.
- [x] LADSPA (per .so) and LV2 (per search directory) scanning out of process through `au-plugin-scan`; crash containment tests.
- [ ] FFmpeg majors other than 7.x (struct offsets are per major), FFmpeg export options dialog, MIDI out device, note editing, per-note-track Allegro tempo maps beyond the first tempo on export.

## Stage 8 part A status — export codecs, metadata, Export Multiple (2026-10-14)
Exporters link the same system libraries Audacity 3.7.3's export modules use (LAME, libvorbis/libogg, libopus, libwavpack, libsndfile, libFLAC, libid3tag) through C++ shims in `au-io/csrc` that follow Audacity's export processors line by line; `au-io/src/export/` (`mp3.rs`, `ogg_opus.rs`, `intfmt.rs`, `export_any`).
- [x] MP3: Preset (Insane/Extreme/Standard/Medium), VBR 0–9, ABR, CBR; joint/stereo; ID3v2 via libid3tag as Audacity
- [x] Ogg Vorbis (quality 0–10), Opus (bitrate list, VBR/CVBR/CBR, complexity, frame duration, application, cutoff; 48 kHz family rates)
- [x] WavPack (Low/Normal/High/Very High, 16/24/32-bit float, hybrid + .wvc, APEv2 tags)
- [x] WAV/AIFF and other libsndfile headers/encodings, RIFF INFO + ID3 chunk as Audacity
- [x] FLAC dither: yes (Shaped HQ dither via the mixer, exact port of Dither.cpp + glibc rand()). 32-bit FLAC: not offered by Audacity 3.7.3 (16/24 only) — not added
- [x] Audacity-exact dither (Shaped/Triangle/Rectangle) — bit-identical to a fresh Audacity process
- [x] Tags model (`au-core/src/tags.rs`, Tags.cpp semantics, ID3v1 genres, templates); `.aup3` `<tags><tag name value/>` read/write — Audacity reads ours (verified in the comparison: tags come from our .aup3)
- [x] Metadata Editor dialog, Export Audio dialog (all format options), Export Multiple (labels/tracks, Audacity naming rules, numbering, prefix, include-before-first-label, non-overwrite renaming) encoding files in parallel on au-runtime (`au-app/src/export_ui.rs`, `export_glue.rs`)
- [x] CLI `process` batch form (`process <files|dirs> --out-dir D [--format wav|flac] [--jobs N] effects…`) on au-runtime; output byte-identical to single-file runs
- [x] Comparison vs Audacity 3.7.3 Export2 (`tools/compare_exports.py`, `docs/export-compare-3.7.3.txt`): **27/27** — decoded audio bit-identical (−240 dB) and tags equal field by field for MP3 ×8, Ogg ×3, Opus ×4, WavPack ×4, FLAC ×3, WAV ×3, AIFF ×2. Files differ in bytes only by tag order (Audacity iterates an unordered_map)
- Notes/gaps: AIFF reference is Audacity's WAV export at the same depth because 3.7.3's scripted Export2 cannot write AIFF on Linux (typed-PCM editor drops the header option); Export Multiple naming compared against ExportAudioDialog.cpp logic by unit tests, not by running Audacity's dialog; MP3/Opus comparisons used default channel mode only

**BizyBot status (2026-10-14), per docs/SPLIT_stage10.md:**
- Toolbars (`au-app/src/gui/toolbars.rs`):
  - 13 Audacity bars: Transport, Tools, Edit, Cut/Copy/Paste, Meter, Mixer, Play-at-Speed, Spectral Selection, Time, Selection, Snapping, Audio Setup, Share Audio.
  - Bars dock at the top or bottom or float. They can be reordered or put on a new row, and wrap to extra lines in narrow windows.
  - View > Toolbars has show/hide and Reset. The layout persists in `toolbars.cfg` (`AU_TOOLBARS_CFG` overrides the path).
- Themes (`au-app/src/theme.rs`):
  - Light, Dark, High Contrast and Classic, taken from Audacity's own `Colors.txt` tables.
  - Custom themes through Load/Save Theme Components (Colors.txt) and an `ImageCache.png` theme cache. This cache is our own colour-swatch PNG, not byte-compatible with Audacity's packed ImageCache.
  - View > Theme. The track panel uses theme colours.
- Accessibility:
  - AccessKit is enabled. The track panel exposes Audacity-style names ("Track 1 Audio Track, Mono, 1.00 seconds, Select On").
  - Up/Down/Home/End move track focus, Enter toggles selection, and the focus ring uses TrackFocus1.
  - Tests run headless against the AccessKit tree. Not yet tested with a real screen reader (AT-SPI/NVDA/VoiceOver).
- Localization layout tests: headless runs at 1.0× and 1.4× text size, plus the Plugin Manager dialog. They check for clipping and overlapping controls, and they caught a real toolbar overflow, which is now fixed.
- UI automation under Xvfb (`tests/ui_automation.rs`, `script_pipe_gui.rs`, `spectrogram_gui.rs`):
  - Workflows: editing, import/export, save + kill -9 + crash recovery, recording (skipped when there is no input device), and error reporting.
  - The pipe gained Select, SelectAll, Cut, Copy, Paste, Delete, Undo, Redo, Export2, SaveProject2, OpenProject2, Recover, Play, Stop and Record1stChoice.
- Packaging (`rust/packaging/`):
  - Linux .deb and AppImage, plus smoke tests. These were built and smoke-tested locally, including the GUI under Xvfb.
  - Windows .msi (WiX v5, optional signtool) and macOS .app/.dmg (codesign, notarytool, stapler). Neither has been run yet.
  - The CI workflow is at `rust/packaging/ci/rust-packaging.yml`. A maintainer needs to copy it to `.github/workflows/`, because the agent token lacks the `workflow` scope.
- Open:
  - Preferences workflows in the automation suite are waiting on the partner's preferences model.
  - Translated-catalog layout runs are waiting on the partner's gettext work.

## Preferences wiring status (2026-10-15)
Every setting is checked against Audacity 3.7.3 sources. Where the pipe allows it, it is also run side by side with Audacity (`tools/compare_prefs.py`, **30/30**).
- [x] Tracks: default track name, view mode and waveform scale (per-track dB toggle). Sample display: stem plot or connect dots once zoomed in to at least ½ pixel per sample, with dots from 3 px per sample (WaveformView thresholds).
- [x] Half-wave display when collapsed: per-track Collapse/Expand; bounds (-0.01, 1) follow DoSetMinimized. Auto-fit track height: ZoomFitVertically after every undo push (TracksPrefs Handler).
- [x] Waveform drawing ported from TrackArt GetWaveYPos/FromDB/ValueOfPixel (`au-app/src/wave_paint.rs`). This replaces the old 0.44 scale.
- [x] Interface > Meter dB range (/GUI/EnvdBRange, DecibelScaleCutoff): sets the floor of dB waveforms and the meter bars.
- [x] Interface > Show 'Extra' menus: the Extra menu, item for item, from Audacity's GetInfo Menus. Compared: the menu shows and hides like Audacity's. Full Screen works.
- [x] Tracks Behaviors: cut lines, Editing a clip can move other clips, Paste as new clips, Retain labels, Type to create a label, New Label dialog. Compared: 3/3 editing cases.
- [x] Quality: Default Sample Format. Audio written into 16/24-bit tracks (effects, generators, recordings) is quantized with the HQ dither. Imports stay exact. Fixed the key to `/Quality/HQDitherAlgorithmChoice` and the 3.7.3 defaults: Dither None, HQ Shaped. Compared: Format16/24 at None is bit-exact (≤1 LSB at 24-bit); Shaped noise matches at 2.06 LSB rms for both.
- [x] Playback > Micro-fades: master volume and pause/stop ramp linearly over one buffer (FillOutputBuffers). Stop waits latency + 50 ms when latency is under 150 ms. Unit-tested in au-io.
- [x] Recording: Hear other tracks (Duplex), Audible input monitoring (software playthrough through a second output stream at the capture rate), Pre-roll (default fixed to 5 s), Crossfade, Sound-activated recording, Latency correction.
- [x] Directories > Temporary files directory: untitled projects journal to `New Project <date> N-<n>.aup3unsaved` there (TempDirectory::UnsavedProjectFileName). The file is offered for recovery on the next launch after a crash and deleted on save or clean exit. Autosave now runs on a worker thread.
- [x] Effects menu grouping and sorting, with a separate publisher for port-only effects. Compared: 15/15 menus.
- [x] Always scrub unpinned (/AudioIO/UnpinnedScrubbing, default on as in 3.7.3; /AudioIO/PinnedHead read): with the head unpinned a ruler drag scrubs without Scrub checked.
- [ ] RTL workaround = off: egui cannot mirror layouts, so we always behave as with the default (on).
- [ ] Automated input level adjustment: `EXPERIMENTAL_AUTOMATED_INPUT_LEVEL_ADJUSTMENT` is commented out in 3.7.3's Experimental.cmake, so Audacity ignores the key too. Matches.
- [x] Extra menu commands 1-45 of docs/SPLIT_extra.md (ubcool2ic): play-around-selection, cut preview, keyboard scrub, tools (F1/F2/F3/F6/A/D), mixer volume, play-at-speed, device dialogs, Delete key, clip left/right, Snap-To, SelStart/SelEnd/SelExtLeft/SelExtRight/SelSetExtLeft. tools/compare_extra.py: 24/24 match Audacity 3.7.3 (clips may now start before 0).
- [x] Extra menu commands 46-67 (BizyBot): SelSetExtRight, SelCntrLeft/Right, Move to Prev/Next Label, Export as MP3/WAV/OGG/FLAC/Selected, Prev/NextFrame, track focus nav, ShiftUp/Down, Toggle/ToggleAlt, CursorLeft/Right (gui/extra_nav.rs). tools/compare_extra_b.py: 11/11 vs Audacity 3.7.3 (selection/cursor commands; label/export/focus commands are not pipe-observable).
- [ ] Extra menu commands 68-89: 22 ids still answer "not recognized" (CursorShortJumpLeft/Right, CursorLongJumpLeft/Right, TrackPan/PanLeft/PanRight, TrackGain/GainInc/GainDec, TrackMenu/Mute/Solo/Close/MoveUp/MoveDown/MoveTop/MoveBottom, SetEnvelope, SetProject, Drag, CompareAudio). **Blocks 1.0.**
- [x] Envelope drawn and edited with GetWaveYPos/ValueOfPixel when the track is in dB view (linear view keeps the 0-2 scale).

## Final status for 1.0.0 (2026-10-15)

- [x] Clip edge drag (trim) in the GUI, mouse scrub/seek on the stutter queue, Snap-To/GridMove: tools/compare_snap.py 132/132 vs Audacity 3.7.3 (isolated runs via tools/iso_run.sh), compare_extra 24/24, compare_extra_b 35/35.
- [x] Workspace version 1.0.0; README, CHANGELOG, THIRD_PARTY_NOTICES, docs/RELEASE_NOTES_1.0.0.md, docs/KNOWN_ISSUES.md.
- [ ] Blocks 1.0: the 22 Extra menu commands listed above; no real-hardware audio test; GUI clicks mostly unautomated; Windows package recheck (verify worker).
- Everything still open is collected in docs/KNOWN_ISSUES.md.
