# `.aup3` support (Stage 4)

Code: `crates/au-project/src/{aup3,binxml,blocks}.rs`. Reference: `au3/libraries/au3-project-file-io/{ProjectFileIO,ProjectSerializer,SqliteSampleBlock}.cpp`.

## Format summary
- SQLite file. `PRAGMA application_id = 0x41554459` ("AUDY"), `PRAGMA user_version` = packed Audacity version (`3.7.0.0` = `0x03070000`).
- `project(id, dict, doc)` — last saved document. `autosave(id, dict, doc)` — journal of unsaved edits; non-empty on open ⇒ the last session ended uncleanly.
- `sampleblocks(blockid, sampleformat, summin, summax, sumrms, summary256, summary64k, samples)`. Formats: int16 `0x00020001` (2 bytes), int24 `0x00040001` (stored as 4-byte i32), float `0x0004000F`. Default block = 1 MiB of samples. Summaries are `(min,max,rms)` f32 triples per 256 and per 65536 samples, computed exactly like `SqliteSampleBlock::CalcSummary` (padding frames FLT_MAX/−FLT_MAX/0).
- Document = ProjectSerializer binary XML: dict starts `[0, charsize]` + `Name(id,u16 len,chars)` records; doc = header `Raw` records then tags/attributes with typed values (`Double` carries a digits field). Linux/macOS builds use charsize 4 (UTF-32), Windows 2.
- Envelope `controlpoint t` is relative to the clip's offset (confirmed: a point written at t=0.5 in a clip at 1.0 s is reported by Audacity at 1.5 s).
- Negative `blockid` = silent block of `-blockid` samples.

## How compatibility was verified
1. Installed Audacity 3.7.3 (`apt install audacity xvfb`), enabled mod-script-pipe, generated fixtures with `tools/make_aup3_fixtures.sh` (real Audacity files, WAL folded in).
2. `cargo test -p au-project --test aup3_fixtures`: content checks, summaries byte-identical to Audacity's, lossless round trip, schema identical, crash recovery.
3. `tools/verify_in_audacity.sh`: Audacity opens each Rust-saved copy; its WAV export and GetInfo output match those of the original.
4. Manual: a Rust-built project opens in Audacity with the right gain/pan/stretch/envelope/labels, Audacity re-saves it, Rust reads an identical state; a Rust-written autosave is recovered by Audacity.

## Intentional differences from Audacity's own writer (golden notes)
- Not written: `selectionformat`, `frequencyformat`, `bandwidthformat`, `time_signature_*`, `preferred_export_rate`, `<effects>`, `height1/minimized1`, `rawAudioTempo`. Audacity uses defaults when they are absent.
- `user_version`: the version of the file that was read is kept (so Audacity 3.7 does not show its "created with an older version" notice); new projects are written as 3.0.0.0, or 3.4.0.0 when a clip is time-stretched. Audacity 3.7 shows a one-time "older version" notice for those.
- Stereo is written as two `wavetrack` elements with `linked="3"` on the first (as 3.7 does). Mono uses `channel="0"`.
- The file is written to `<name>.aup3.saving` and renamed over the target (Audacity writes in place); journal mode is SQLite's default rather than WAL.
- Trims are applied on read, so trimmed-away audio is not preserved; saved clips always have `trimLeft/trimRight = 0`.

## Scripting tips (headless Audacity)
- The sample-format preference applies only to new projects, so set it in `audacity.cfg` before launching.
- `SetTrackAudio` (not `SetTrack`) sets gain/pan/mute/solo; pan is −100…100.
- A killed Audacity leaves `[ActiveProjects]` entries and `/var/tmp/audacity-$USER/*` temp projects, which bring up a startup recovery dialog that blocks the pipe; the scripts clear both.
- Audacity marks a file with orphan sample blocks as "not saved properly" (modal notice).
