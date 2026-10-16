# Changelog

## 1.0.0 — unreleased (release candidate, 2026-10-15)

First complete release of the Rust rewrite of Audacity 3.7.3.

### Added
- Multi-track editor (egui) with clips, labels, envelopes (linear and dB view), Time Shift, snapping, history, and Extra-menu commands 1–67.
- Clips that start before time 0 and non-destructive clip trims, with clip edge dragging in the GUI.
- Native `.aup3` read/write that Audacity 3.7.3 opens: trims, negative offsets, tags, realtime effect stacks, note/time tracks, and autosave with crash recovery.
- Audio engine: playback and recording, resampling, Windows loopback, punch-and-roll, play-at-speed, keyboard scrub, and mouse scrub/seek through the stutter queue.
- Built-in effects, generators and analyzers ported from 3.7.3; Noise Reduction is bit-identical.
- Plug-in hosting: CLAP, VST3, LV2, LADSPA (sandboxed), Nyquist (sandboxed worker), out-of-process scanning, Plugin Manager.
- Import/export: WAV, AIFF, FLAC, MP3, Ogg, Opus and WavPack; metadata; Export Multiple; optional FFmpeg 7; MIDI.
- mod-script-pipe compatible scripting and macros.
- AI music: Lyria generation through OpenRouter and local HTDemucs stem separation.
- Room noise profile: per device and per project, used for live and offline reduction.
- Windows packages: MSI and portable zip (`packaging/windows`).
- Release docs: README, THIRD_PARTY_NOTICES, release notes and known issues.

### Fixed (since 0.1.0 development builds)
- New projects are stamped as version 3.7, so Audacity no longer shows a modal "older version" notice.
- Delete collapses the selection; selections may extend below 0.
- Snap-To/GridMove now matches Audacity's pixel-offset logic (132/132 comparisons).
