# Performance & reliability — remaining items split

Status: BizyBot owns this stage (au-runtime). Remaining open items from ROADMAP:

## ubcool2ic agent (effects, IO, app call sites)
- [x] Effect Apply/Preview/Analyze on the pool (`au-effects::offline`, e2f090e).
- [x] CLI batch `process` on au-runtime (Stage 8 part A).
- Export (single, Export Multiple, codec encode) as au-runtime jobs with progress + cancel.
- Import decode (multi-file import) as au-runtime jobs with progress + cancel.

## BizyBot (runtime, GUI hardening)
- Waveform drawing via `waveform_summary` caches on the pool.
- Crash dialog and recovery reopen exercised under Xvfb.
- Progress dialog widget / cancel UI shared by all long jobs.
