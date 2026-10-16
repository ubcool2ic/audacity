# Audacity-RS 1.0.0 — release notes (draft)

Audacity-RS is a Rust rewrite of Audacity 3.7.3. It opens and saves real Audacity `.aup3` projects, and its behaviour is checked against Audacity 3.7.3 itself.

## Highlights
- **Full editor:** clips, including clips before 0 and trims you can drag at a clip's edge; envelopes; labels; snapping; scrubbing; history.
- **Real `.aup3` projects** that round-trip with Audacity 3.7.3, with autosave and crash recovery.
- **Plug-ins:** CLAP, VST3, LV2, LADSPA and Nyquist, with crash-isolated scanning.
- **Bit-identical Noise Reduction**, plus a room noise profile for live recording.
- **AI music:** generate songs with Google Lyria (through OpenRouter, using your own key) and split stems locally with HTDemucs.
- **Windows first:** MSI installer and portable zip.

## Verification
- Exports: 27/27 bit-identical with Audacity 3.7.3.
- Noise Reduction: 6/6 parameter sets bit-identical.
- Extra commands: 24/24 and 35/35 match through Audacity's own scripting pipe.
- Snap/GridMove: 132/132 match.
- Effects menu: 15/15 match.

## Install
See README.md. FFmpeg 7 is optional (M4A/AAC/WMA/AC3). Nyquist effects need an installed Audacity 3.7.x.

## Known issues
See docs/KNOWN_ISSUES.md. The main ones: 22 Extra-menu commands are missing, nothing has been tested on real audio hardware and most GUI clicks have no automated test.

## License
GPL-3.0-or-later. See THIRD_PARTY_NOTICES.md.
