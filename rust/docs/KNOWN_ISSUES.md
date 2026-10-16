# Known issues — Audacity-RS 1.0.0

This list collects every open item from ROADMAP.md and the docs, as of 2026-10-15. **Bold** items block a 1.0.0 release.

## Parity gaps

- **22 Extra-menu commands are not implemented.** They answer "not recognized" on the pipe: CursorShortJumpLeft/Right, CursorLongJumpLeft/Right, TrackPan, TrackPanLeft/Right, TrackGain, TrackGainInc/Dec, TrackMenu, TrackMute, TrackSolo, TrackClose, TrackMoveUp/Down/Top/Bottom, SetEnvelope, SetProject, Drag, CompareAudio (docs/SPLIT_extra.md, items 68–89).
- Some effects are not exact: Rhythm Track, Pluck and Studio Fade Out differ from 3.7.3. Some other effects have not been compared.
- Applying a macro has not been compared against Audacity.
- Hidden audio in a stretched clip that extends before 0 is converted approximately.
- The RTL layout workaround is always on, because egui cannot mirror layouts.
- Punch-and-roll merges the track's clips into one clip (Audacity punches per clip).

## Projects and formats

- Pre-3.0 `.aup` projects (with `_data` folders) cannot be imported. Files newer than 3.7 are rejected.
- Spectrogram settings and `rawAudioTempo` are dropped on save. Project tempo/time signature round-trip is not verified.
- `.aup3` files written by Audacity on Windows or macOS have not been tested, and neither has VST3 state against real Audacity projects.
- Autosave fixtures come from our own writer. Audacity's own autosave rows were not captured.
- Only FFmpeg 7.x is supported (struct offsets differ per major version). There is no FFmpeg export options dialog.
- MIDI: there is no MIDI output device and no note editing. On export, only the first tempo of an Allegro tempo map is used.

## Platform and testing

- **Not tested on real audio hardware.** The build box has no sound card, so cpal input/output, loopback and real latency values are unverified.
- **GUI clicks are mostly not automated.** The app renders under Xvfb, but clip edge drag, crash-dialog reopen and most mouse paths have no automated GUI test.
- **The Windows build, packages and the scripting pipe on Windows are being rechecked separately (verify worker). 1.0 waits on that result.**
- Loopback recording is native on Windows only. On Linux, choose a "Monitor of" source. macOS needs a virtual device.
- Not tested with a real screen reader.
- The CI workflow is not enabled.

## Features

- Plug-in editors are embedded (X11, Win32), but embedding has only been tested under Xvfb and Wine. The realtime plug-in path still takes a Mutex.
- Long jobs (effect apply, export, import, waveform drawing) still run serially instead of as `Runtime` jobs with progress bars.
- audio.com cloud saving is deferred.
- Some format strings are English only.
- AI music: on Windows the OpenRouter key is read from the `OPENROUTER_API_KEY` environment variable only, because `~/.config/openrouter/env` is found through `HOME`, which Windows usually doesn't set. Stem separation needs a local Python install with `demucs`.
- Nyquist effects need `lib-nyquist-effects.dll` from an installed Audacity 3.7.x.
- The CLAP host reports its version as `0.1.0` (`au-plugin-host/src/clap.rs`).

## Licensing

- The project is GPL-3.0-or-later, **not GPLv2+**: Audacity 3.7.3 is distributed under GPLv3 as a whole, and several dependencies (Apache-2.0, the MPL-2.0 Symphonia, GCC runtime libraries) are not GPLv2-compatible. See THIRD_PARTY_NOTICES.md.
