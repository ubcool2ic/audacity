# OpenRouter spend log

Model policy: cheapest top-ranked coding model, `openai/gpt-6-luna` ($0.10 / 1M input, $0.50 / 1M output, reasoning low). Escalate a chunk to `openai/gpt-6-sol` only after Luna fails twice on it. Compile/test fixes that are one-liners are done by hand (no spend).

| date | model | file | prompt tok | completion tok | cost |
|---|---|---|---|---|---|
| 2026-10-14 | openai/gpt-6-luna | crates/au-core/src/lib.rs | 1032 | 4918 | $0.00259 |
| 2026-10-14 | openai/gpt-6-luna | crates/au-effects/src/lib.rs | 1135 | 1889 | $0.00109 |
| 2026-10-14 | openai/gpt-6-luna | crates/au-project/src/lib.rs | 1237 | 2560 | $0.00143 |
| 2026-10-14 | openai/gpt-6-luna | crates/au-plugin-api/src/lib.rs | 1657 | 3387 | $0.00190 |
| 2026-10-14 | openai/gpt-6-luna | crates/au-io/src/lib.rs | 1544 | 3615 | $0.00200 |
| 2026-10-14 | openai/gpt-6-luna | crates/au-app/src/main.rs | 2663 | 4809 | $0.00274 |
| 2026-10-14 | openai/gpt-6-luna | ROADMAP.md | 1212 | 2970 | $0.00164 |

**Stage 1 total: $0.0134** (plus ~$0.0001 key smoke test)

## GUI build fix (2026-10-14)

Hand fix, no OpenRouter calls: inner doc comment in `au-app/src/gui.rs` and 8 double-borrow errors (`self.report(self.editor...)`). `cargo build -p au-app --features gui` now succeeds. Spend: $0.00.

## Stage 2 (2026-10-14)

| date | model | file | prompt tok | completion tok | cost |
|---|---|---|---|---|---|
| 2026-10-14 | openai/gpt-6-luna (high) | crates/au-core/src/timeline.rs | 2342 | 17536 | $0.00906 |
| 2026-10-14 | openai/gpt-6-luna (low) | crates/au-project/src/lib.rs | 2975 | 5412 | $0.00308 |
| 2026-10-14 | openai/gpt-6-luna (high) | crates/au-app/src/gui.rs | 5968 | 21177 | $0.01133 |

Hand fixes (no spend): Track/Editor integration in au-core lib.rs, editor tests, 2 wrong test expectations, v1 sample query, 5 GUI type errors, warnings. No Sol escalation needed.

**Stage 2 total: $0.0235**

**Running total: $0.0370**

## Stage 3 (2026-10-14)

| date | model | file | prompt tok | completion tok | cost |
|---|---|---|---|---|---|
| 2026-10-14 | openai/gpt-6-luna (low) | crates/au-io/src/dsp.rs | 967 | 4645 | $0.00242 |
| 2026-10-14 | openai/gpt-6-luna (high) | crates/au-io/src/record.rs | 978 | 10892 | $0.00554 |
| 2026-10-14 | openai/gpt-6-luna (high) | crates/au-io/src/engine.rs | 1411 | 15357 | $0.00785 |
| 2026-10-14 | openai/gpt-6-luna (high) | crates/au-app/src/gui.rs | 15495 | 23721 | $0.01380 |

Hand fixes (no spend): mixer test expectations that exceeded clip range, f32-phase test sine (caused fake -53 dB alias), rubato 0.15 delay trim (output already aligned), cpal Player/Recorder rewrite onto engine + SPSC capture, dither in WAV export, Editor set_track_mix/replace_track_audio/draw_samples, au-app transport.rs (RecordController + headless tests), 6 GUI compile errors. No Sol escalation needed.

**Stage 3 total: $0.0296**

**Running total: $0.0666**

## Stage 4 (2026-10-14)

| date | model | file | prompt tok | completion tok | cost |
|---|---|---|---|---|---|
| 2026-10-14 | openai/gpt-6-luna (high) | crates/au-project/src/binxml.rs | 1149 | 12026 | $0.00616 |
| 2026-10-14 | openai/gpt-6-luna (high) | crates/au-project/src/blocks.rs | 985 | 5638 | $0.00292 |

Both compiled and passed their own tests first time; no Sol escalation. Hand-written (no spend): `aup3.rs` (XML ↔ project mapping, SQLite schema, save/autosave/recover/orphans), integration tests, `aup3_tool` example, GUI File menu + recovery dialog + autosave, CLI `aup3-info`, Audacity fixture/verification scripts.

**Stage 4 total: $0.0091**

**Running total: $0.0757**

## Stages 5–6 — effects, generators, analyzers (openai/gpt-6-luna, no Sol escalation)

| file | USD |
|---|---|
| fx/autoduck.rs | 0.0035 |
| fx/analyzers.rs | 0.0056 |
| fx/loudness.rs | 0.0057 |
| fx/eq.rs | 0.0081 |
| fx/dynamics.rs | 0.0086 |
| fx/repair.rs | 0.0027 |
| fx/silence.rs | 0.0040 |
| fx/analyzers.rs | 0.0053 |
| fx/generators.rs | 0.0051 |
| fx/reverb.rs | 0.0057 |
| fx/loudness.rs | 0.0056 |
| fx/noisered.rs | 0.0059 |
| fx/dynamics.rs | 0.0077 |
| fx/stretch.rs | 0.0087 |
| fx/filters.rs | 0.0101 |
| fx/simple.rs | 0.0101 |
| fx/eq.rs | 0.0100 |
| fx/modulation.rs | 0.0115 |
| fx/modulation.rs | 0.0128 |

Includes $0.0035 for Auto Duck. Repeated rows are regenerations after 402/compile failures.

**Stages 5–6 total: $0.1366**

**Running total: $0.2123**

## Stages 5–6 parity fixes (Audacity 3.7.3 mismatches)

| file | model | USD |
|---|---|---|
| fx/fix_loudness.rs | openai/gpt-6-luna | 0.0053 |
| fx/fix_nyqfilters.rs | openai/gpt-6-luna | 0.0067 |
| fx/fix_analyzers.rs | openai/gpt-6-luna | 0.0071 |
| fx/fix_pluck.rs | openai/gpt-6-luna | 0.0040 |
| fx/fix_delay.rs | openai/gpt-6-luna | 0.0057 |
| fx/fix_tremolo.rs | openai/gpt-6-luna | 0.0058 |
| fx/fix_distortion.rs | openai/gpt-6-luna | 0.0067 |
| fx/fix_studiofade.rs | openai/gpt-6-luna | 0.0087 |
| fx/fix_rhythm.rs | openai/gpt-6-luna | 0.0110 |
| fx/fix_paulstretch.rs | openai/gpt-6-luna | 0.0072 |
| fx/fix_dynamics.rs | openai/gpt-6-luna | 0.0094 |
| fx/fix_nyqfilters.rs | openai/gpt-6-luna | 0.0074 |
| fx/fix_analyzers.rs | openai/gpt-6-luna | 0.0079 |
| fx/fix_dynamics.rs | openai/gpt-6-luna | 0.0089 |
| fx/fix_distortion.rs | openai/gpt-6-luna | 0.0090 |
| fx/fix_paulstretch.rs | openai/gpt-6-luna | 0.0087 |
| fx/fix_speedpitch.rs | openai/gpt-6-luna | 0.0075 |
| fx/fix_rhythm.rs | openai/gpt-6-luna | 0.0179 |
| fx/fix_nyqfilters.rs | openai/gpt-6-sol | 0.1023 |
| fx/fix_rhythm.rs | openai/gpt-6-sol | 0.2122 |
| fx/fix_dynamics.rs | openai/gpt-6-sol | 0.1830 |
| fx/fix_dynamics.rs | openai/gpt-6-sol | 0.1292 |
| fx/fix_rhythm.rs | openai/gpt-6-sol | 0.2281 |
| fx/fix_findclipping.rs | openai/gpt-6-luna | 0.0027 |
| fx/fix_dynamics.rs | openai/gpt-6-luna | 0.0091 |

Luna (openai/gpt-6-luna): 20 calls, $0.1566. Sol (openai/gpt-6-sol): 5 calls, $0.8547 (Notch, Compressor/Limiter x2, Rhythm Track x2, only after two Luna failures).

**Parity fixes total: $1.0113**

**Running total: $1.2236**

## Stage 6: generators and analyzers (2026-10-14)
| file | model | cost $ |
|---|---|---|
| fx/fix_rissetdrum.rs | openai/gpt-6-luna | 0.0122 |
| fx/fix_rissetdrum.rs (retry) | openai/gpt-6-luna | 0.0103 |
| fx/fix_rissetdrum.rs (review after 2 Luna passes) | openai/gpt-6-sol | 0.2132 |
| fx/fix_plotspectrum.rs | openai/gpt-6-luna | 0.0083 |
| fx/fix_plotspectrum.rs (RealFFTf port) | openai/gpt-6-luna | 0.0092 |
| fx/fix_contrast.rs | openai/gpt-6-luna | 0.0059 |
| fx/fix_sampledataexport.rs | openai/gpt-6-luna | 0.0071 |
| au-app/src/analyze_ui.rs | openai/gpt-6-luna | 0.0084 |

Stage 6: Luna 7 calls, $0.0614. Sol 1 call, $0.2132. **Stage 6 total: $0.2746**

**Running total: $1.4982**

## Noise Reduction + room noise profile (2026-10-14)
| file | model | cost $ |
|---|---|---|
| fx/fix_noisereduction.rs (first pass) | openai/gpt-6-luna | 0.0096 |
| fx/fix_noisereduction.rs (review: Luna file + C++ + diff numbers) | openai/gpt-6-sol | 0.1975 |
| tools/aud_ref/noise_reduction_ref.cpp (3 calls; first 2 returned no C++) | openai/gpt-6-luna | 0.0217 |
| au-project/src/noise_profiles.rs | openai/gpt-6-luna | 0.0017 |
| au-app/src/room_noise.rs (+ test fix) | openai/gpt-6-luna | 0.0105 |

Luna 7 calls, $0.0435. Sol 1 call, $0.1975. **Total: $0.2410**

**Running total: $1.7392**

## Offline render on au-runtime pool + LV2/LADSPA hosting (2026-10-14)
| file | model | cost $ |
|---|---|---|
| au-effects/src/offline.rs | openai/gpt-6-luna | 0.0065 |
| au-plugin-host/src/ladspa.rs | openai/gpt-6-luna | 0.0116 |
| au-plugin-host/src/lv2.rs | openai/gpt-6-luna | 0.0157 |
| au-plugin-host/src/lv2.rs (fix: shared in/out buffer) | openai/gpt-6-luna | 0.0113 |

Luna 4 calls, $0.0451. Sol: none. **Total: $0.0451**

**Running total: $1.7843**

## BizyBot — Performance & reliability (2026-10-14)
- au-runtime crate, crash hook/dialog, tests: written by hand, no OpenRouter calls. Spend: $0.00

## BizyBot — Stage 7 CLAP/VST3 hosting (2026-10-14)
`openai/gpt-6-luna` only (no Sol escalation needed): 11 calls — lib+state 1, clap.rs 2 (first was a skeleton, retried at high effort) + 1 compile-fix pass, vst3.rs 2 + 1 compile-fix pass, sandbox.rs 1, tests 1, two small logic/test fixes. Hand edits: import paths, Option/Result mismatches, `Symbol<*const clap_plugin_entry>`, test fixture paths/segfault line.
**Spend: $0.0588** (per-call `usage.cost`). Earlier Performance & reliability stage: $0.00 (hand-written before the Luna requirement).

## BizyBot — Stage 8 plugins in app / editor host / Audacity check (2026-10-14)
openai/gpt-6-luna only (no Sol escalation): vst3 run loop, clap timer/fd, state ids, editor host, editor tests, aup3 example (2 tries), plugin_bridge, GUI integration. Total ≈ $0.043.

## BizyBot — Stage 8B FFmpeg / MIDI / LADSPA-LV2 sandbox (2026-10-14)
openai/gpt-6-luna only (no Sol): sandbox.rs (+1 wasted call from a shell-quoting mistake in the prompt), LADSPA/LV2 tests, ffmpeg.rs, FFmpeg tests, notes.rs, midi.rs + aup3/aurs + tests, MIDI velocity fix, Audacity fixture tests, GUI wiring. Total ≈ $0.044.

## BizyBot — Stage 9 script pipe / Nyquist host / spectrogram view (2026-10-14)
openai/gpt-6-luna only (no Sol): au-scriptpipe + fix pass, app bridge, Nyquist sandbox host, plugin manager, spectrogram view, app pipe commands, Xvfb spectrogram test. Total ≈ $0.040.

## Stage 8 part A + au-effects API change (2026-10-14)
| file | model | cost $ |
|---|---|---|
| au-io export/mp3.rs + csrc/export_mp3.cpp | openai/gpt-6-luna | 0.0090 |
| au-io export/ogg_opus.rs + csrc/export_ogg_opus.cpp | openai/gpt-6-luna | 0.0090 |
| au-io export/intfmt.rs + csrc/export_intfmt.cpp (PCM/FLAC/WavPack + Dither) | openai/gpt-6-luna | 0.0132 |
| intfmt WavPack fix (2nd pass after comparison) | openai/gpt-6-luna | 0.0124 |
| au-core/src/tags.rs | openai/gpt-6-luna | 0.0050 |
| au-app/src/export_ui.rs (Metadata Editor, Export Multiple) | openai/gpt-6-luna | 0.0073 |
| au-app/src/export_glue.rs (format options, mixdown, parallel Export Multiple) | openai/gpt-6-luna | 0.0080 |
| au-app/src/main.rs (batch CLI on au-runtime) | openai/gpt-6-luna | 0.0082 |
| au-effects/src/stack.rs (API change) | openai/gpt-6-luna | 0.0040 |
| au-app/src/plugin_bridge.rs (API change) | openai/gpt-6-luna | 0.0049 |

Luna 10 calls, $0.0810 (API change part: $0.0089). Sol: none. **Total: $0.0810**

Hand edits: trivial compile fixes (FLAC vorbiscomment helper, WavPack writer ids, two corrupted literals in generated Rust, egui borrow, includes), the glibc rand() seeding in the dither port, the mechanical `id()`→`Cow` signature migration across effect files (script), gui.rs menu wiring.

**Running total (ubcool2ic): $1.8653**
