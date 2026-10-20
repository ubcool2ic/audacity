# SPEND.md - OpenRouter spend for the Rust port (BizyBot5)

Model: `deepseek/deepseek-v4.1-flash-1017` only, via the OpenRouter HTTP API.

| # | Date (UTC-4) | Purpose | Prompt tok | Completion tok | Cost (USD) | Running total (USD) |
|---|---|---|---|---|---|---|
| 1 | 2026-10-20 04:25 | Scaffold au3/rust crate + CMake glue; port au3-math Biquad | 7608 | 20000 | 0.01053 | 0.01053 |
| 2 | 2026-10-20 04:28 | Retry (prev reply all reasoning, cut off): scaffold au3/rust + port Biquad | 7608 | 30229 | 0.01565 | 0.02618 |
| 3 | 2026-10-20 04:30 | Fix Biquad cheby1 1e-10 coefficient mismatch | 3052 | 3620 | 0.00202 | 0.02820 |
| 4 | 2026-10-20 04:32 | Fix Biquad cheby2/butterworth f32 sqrt mismatch | 3767 | 15485 | 0.00801 | 0.03621 |
