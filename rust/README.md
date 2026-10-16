# Audacity-RS 1.0.0

A Rust rewrite of [Audacity](https://www.audacityteam.org/) 3.7.3: a multi-track audio editor and recorder that reads and writes real Audacity `.aup3` projects. Behaviour is checked against Audacity 3.7.3 itself, through its scripting pipe and through reference programs built on Audacity's own code.

Windows x64 is the primary release target. Linux builds and runs too.

## Features

- **Editing:** multi-track waveform editing, clips (including clips before time 0 and non-destructive trims), envelopes in linear and dB view, labels, Time Shift, snapping, undo/redo history, and the Extra menu commands (see [Known issues](docs/KNOWN_ISSUES.md) for the ones still missing).
- **Projects:** reads and writes Audacity 3.x `.aup3` (SQLite) files that Audacity 3.7.3 opens, with autosave and crash recovery. Saved data includes clip offsets and trims, tags, realtime effect stacks, note and time tracks, track colours and clip pitch.
- **Audio engine:** playback and recording through cpal, playback resampling, loopback recording on Windows (WASAPI), punch-and-roll, play-at-speed, and keyboard and mouse scrubbing/seeking.
- **Effects, generators, analyzers:** ports of Audacity's built-in effects. Many are compared sample by sample against Audacity 3.7.3, and Noise Reduction is bit-identical.
- **Plug-ins:** CLAP, VST3, LV2, LADSPA (sandboxed hosts) and Nyquist. See [Plug-ins](#plug-ins).
- **Import/export:** WAV/AIFF/FLAC/MP3/Ogg/Opus/WavPack and more, metadata, and Export Multiple. FFmpeg formats are available when FFmpeg is installed.
- **Scripting:** a mod-script-pipe compatible command pipe (`au-scriptpipe`) and macros.
- **AI music:** text-to-music with Google Lyria and local stem separation with HTDemucs. See [AI music](#ai-music).
- **Room noise profile:** learn a noise profile per input device and use it for live or offline noise reduction. See [Noise profile](#noise-profile).
- **Localization:** UI strings use Audacity's own translations (`locale/*/audacity.mo`).

## Install (Windows)

Download one of these from the release:

- **`Audacity-RS-1.0.0-x64.msi`**: the installer. It puts the app under `Program Files\Audacity-RS` and adds a Start-menu entry.
- **`Audacity-RS-1.0.0-win64-portable.zip`**: unzip anywhere and run `Audacity-RS\audacity-rs.exe`.

Settings are stored in `%APPDATA%\audacity\audacity.cfg` (the same keys as Audacity) and in `%APPDATA%\audacity-rs`.

Optional extras:

- **FFmpeg 7.x** for M4A/AAC/WMA/AC3 and audio from video files: put `avformat-61.dll`, `avcodec-61.dll`, `avutil-59.dll` and `swresample-5.dll` next to the exe, or install "FFmpeg for Audacity" to `C:\Program Files\FFmpeg for Audacity`.
- **Nyquist effects** use `lib-nyquist-effects.dll` from an installed Audacity 3.7.x.

## Install (Linux)

Build from source (below), then run `target/release/audacity-rs`. You need ALSA or PulseAudio/PipeWire development headers.

## Building

You need Rust stable (edition 2021).

```sh
cd rust
cargo build --release -p au-app --features gui   # GUI app: target/release/audacity-rs
cargo test --workspace                           # tests
```

Without `--features gui` you get the headless CLI, which includes `audacity-rs aup3-info <file>` and the script pipe.

**Windows packages from Linux:** `packaging/windows/build-cross.sh` cross-builds with mingw-w64, downloads the codec DLLs from MSYS2, and writes the portable zip and the MSI (the MSI needs `wixl`) to `target/packages/`.

**Building on Windows:** `packaging/windows/build-msi.ps1` builds the release binaries on Windows and packages them with WiX.

## Plug-ins

| Format | Notes |
|---|---|
| CLAP | Realtime and offline processing. Editors are embedded in the app window (X11/Win32). |
| VST3 | Effect IDs and saved state match Audacity's (`VST3:<module>;<UID>`). Editors are embedded in the app window (X11/Win32). |
| LV2, LADSPA | Run in a sandboxed host process. |
| Nyquist | `.ny` plug-ins run in a sandboxed worker (`au-nyquist-worker`). The bundled Audacity Nyquist plug-ins are included. |

Scanning runs in a separate process (`au-plugin-scan`), so a plug-in that crashes cannot take down the app. Use Effect ▸ Plugin Manager to enable or disable plug-ins. VST2 is not supported (its SDK licence is GPL-incompatible). Details: [docs/PLUGIN_HOSTING.md](docs/PLUGIN_HOSTING.md).

## AI music

- **Generate music** with Google Lyria 3 (`google/lyria-3-pro-preview` for full songs, `google/lyria-3-clip-preview` for 30 s clips). Requests go through **OpenRouter** using your own API key, and OpenRouter bills you directly (about $0.08 per song or $0.04 per clip, October 2026 prices). The app looks for the key in `~/.config/openrouter/env` (`OPENROUTER_API_KEY=...`) and then in the `OPENROUTER_API_KEY` environment variable. On Windows, set the environment variable. Output carries Google's SynthID watermark and is subject to Google's terms.
- **Separate stems** (vocals, drums, bass, other) with HTDemucs. This runs **locally**: install Python and `pip install demucs`. No audio is uploaded. A 3.5-minute song takes about 1–1.5 minutes on an 8-core CPU.

Details: [docs/AI_MUSIC.md](docs/AI_MUSIC.md).

## Noise profile

Under Recording options, **Capture Noise Profile (10 s)** records room tone from the current input. Profiles are saved per input device (`~/.config/audacity-rs/noise-profiles/`) and per project, inside the project file. Audacity ignores this extra data.

- **Offline:** Effect ▸ Noise Reduction uses the captured profile. It is a bit-identical port of Audacity's Noise Reduction.
- **Live:** turn on suppression while recording. The raw take is kept, and a time-aligned "(noise suppressed)" track is added next to it.

## Documentation

- [CHANGELOG.md](CHANGELOG.md)
- [Release notes 1.0.0](docs/RELEASE_NOTES_1.0.0.md)
- [Known issues](docs/KNOWN_ISSUES.md)
- [ROADMAP.md](ROADMAP.md)
- [.aup3 format](docs/AUP3_FORMAT.md)
- [Audio engine](docs/AUDIO_ENGINE.md)

## License

GPL-3.0-or-later. Audacity-RS is a derived work of Audacity © the Audacity Team, and Audacity 3.7.3 is distributed as a whole under GPLv3 (`../LICENSE.txt`). Third-party components and their licences are listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
