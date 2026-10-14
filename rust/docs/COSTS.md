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
