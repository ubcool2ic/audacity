# Stage 9 split — Spectral tools, automation, Nyquist

Owner of the left column: ubcool2ic agent (core / effects / IO / app logic).
Right column: hand to **BizyBot** (runtime, plugin host, GUI hardening, packaging).

## ubcool2ic agent (core, effects, IO, app)
- Spectral edit model in `au-core`: time+frequency selection (f0/f1, centre/width as Audacity's SelectedRegion), saved in `.aup3` like Audacity (`selLow`/`selHigh`).
- Spectral edit effects (`au-effects`): Spectral Delete, Spectral Edit Multi Tool, Spectral Edit Parametric EQ, Spectral Edit Shelves — ports compared against Audacity 3.7.3 (mod-script-pipe, exact where deterministic).
- Spectrogram computation matching Audacity's SpectrogramSettings (window types, sizes, zero padding, scales: linear/log/mel/bark/ERB/period, gain/range/frequency gain) — golden pixels-in-dB arrays against lib-fft reference.
- Pitch (EAC) view algorithm port.
- Macros: `.txt` macro file read/write compatible with Audacity (`~/.config/audacity/Macros`), the command vocabulary for built-in effects/edits/select/export, Apply Macro to project and to files (batch, on au-runtime). Tests run Audacity's shipped macros (MP3 Conversion, Fade Ends) and compare outputs.
- Scripting command layer (`au-app` / new `au-script` module): Audacity command vocabulary (Select, SelectTime, SelectTracks, Import2, Export2, GetInfo (Tracks/Clips/Labels/Commands, JSON/LISP/Brief), SetTrack, SetClip, SetLabel, Help, Message, effects by id with parameters), response format "BatchCommand finished: OK".
- Nyquist: Nyquist interpreter FFI integration (C library build + `.ny` plugin header parsing, controls → dialog params, $control types, `;type process/generate/analyze/tool`), discovery from Audacity plug-in dirs, deterministic-output fixtures vs Audacity; error containment via `process_isolated`.

## BizyBot (runtime, plugin host, GUI hardening, packaging)
- mod-script-pipe transport: named pipes `/tmp/audacity_script_pipe.to/from.<uid>` (Linux/macOS) and Windows `\\.\pipe\ToSrvPipe`/`FromSrvPipe`, threaded server on au-runtime, command queue to the UI thread, Python client compatibility test (Audacity's `pipe_test.py`).
- Nyquist sandboxing: running Nyquist plugins in a crash-contained worker (like `au-plugin-scan`), timeouts/cancel.
- Nyquist plugins as a `PluginFormat` in au-plugin-host / registry / plugin manager UI (enable/disable, rescan).
- Spectrogram view rendering in the GUI (GPU texture upload, zoom, frequency ruler), spectral selection tool interactions, GUI performance (waveform/spectrogram caches on au-runtime).
- Macros manager dialog and Apply Macro menu GUI hardening; UI automation tests under Xvfb.
