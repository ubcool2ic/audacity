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
