# AI Music: generation, stem separation, instrumental remake

Status: research + prototype (`crates/au-ai`), Oct 14 2026. Prices verified on vendor pages that day.

## 1. Text-to-music generation

| Option | Vocals | Price (Oct 2026) | License / commercial | Notes |
|---|---|---|---|---|
| **Google Lyria 3 Pro** via OpenRouter (`google/lyria-3-pro-preview`) | yes + instrumental mode | **$0.08 / song** (≤~3 min) | Google terms, SynthID watermark, preview | Same OpenRouter key/billing we already use. **Top pick.** |
| Lyria 3 Clip (`google/lyria-3-clip-preview`) | yes | $0.04 / 30 s clip | same | loops / previews |
| **ACE-Step 1.5** hosted on fal.ai | yes (lyrics, 50+ langs) or `[inst]` | $0.0002 / s → ~$0.04 for 3.5 min | **MIT** weights, trained on licensed/royalty-free data | Cheapest; also does cover / repaint / extract / "lego" (add track) |
| ACE-Step on Replicate | yes | ~$0.02 / run | MIT | |
| ElevenLabs Eleven Music API | yes | $0.15 / min (~$0.50 per 3.5-min song) | broad commercial on paid plans; film/TV = Enterprise | best licensing story, priciest |
| Suno / Udio | yes | **no public API** (Suno partner intake only, Jul 2026) | — | skip; unofficial wrappers violate ToS |
| OpenAI `gpt-audio(-mini)` (OpenRouter) | speech only | token priced | — | not music |

OpenRouter `list-models` (audio output) returned exactly: lyria-3-clip-preview, lyria-3-pro-preview, gpt-audio, gpt-audio-mini.
`generate-speech` is TTS only. Live Lyria call not executed: the box's `OPENROUTER_API_KEY` env var returned **401 "API key expired"**.

### Local open models
| Model | Vocals | License | Box (8 CPU, 15 GB RAM, no GPU) |
|---|---|---|---|
| ACE-Step 1.5 (2B turbo) | yes | MIT | GPU ≥4 GB VRAM recommended; CPU mode exists but expect many minutes/song |
| YuE | yes | Apache-2.0 | needs 24 GB+ GPU; not feasible |
| DiffRhythm | yes | Apache-2.0 (check weights) | GPU ~8 GB |
| MusicGen (AudioCraft) | no | code MIT, **weights CC-BY-NC** (non-commercial) | small model runs on CPU, slow |
| Stable Audio Open 1.0 | no (≤47 s) | Stability Community License (free < $1M revenue) | GPU recommended |

Recommendation: hosted (Lyria Pro for quality, fal ACE-Step for cost/edit features); local ACE-Step for users with a GPU.

## 2. Stem separation

| Model | Stems | License | Notes |
|---|---|---|---|
| **HTDemucs (v4)** / htdemucs_ft / htdemucs_6s | vocals, drums, bass, other (+guitar, piano in 6s) | **MIT** | **chosen**; CPU-friendly |
| BS-RoFormer / Mel-RoFormer (UVR / python-audio-separator) | vocals/instrumental SOTA | MIT code; community weights vary | best vocal SDR; heavier |
| UVR MDX-Net | vocals/inst | MIT app; model licenses vary | |
| ACE-Step `extract` task | per-track | MIT | needs GPU |

**Box test (real):** htdemucs, CPU, 8 cores, 30 s stereo 44.1 kHz clip (synthetic test tone — no copyrighted audio): **~12 s, ~2.9× realtime** (Python), 14.9 s through the Rust `au-ai` example including model load. Estimate: **~70–90 s per 3.5-min song** on this box; htdemucs_ft is ~4× slower.

Rust/ONNX path: HTDemucs exports to ONNX (community exports exist; STFT must be done outside the graph) and runs via the `ort` crate on CPU; RoFormer ONNX models already ship with python-audio-separator/UVR. Plan: `OrtSeparator` implementing `StemSeparator`, removing the Python dependency. Prototype uses a Demucs subprocess today.

Reference: Audacity's official **OpenVINO AI plugins** (Intel, Windows/Linux) provide Music Separation (Demucs v4), MusicGen generation, noise suppression, Whisper transcription — same feature set we're targeting natively.

## 3. Instrumental + "remake"
1. Separate → instrumental = drums+bass+other (`Stems::instrumental`, `export_instrumental`) — free, local.
2. Remake: send the instrumental or the vocal melody to **ACE-Step `cover`/`repaint`/`lego`** (fal or local) with a style prompt; or Lyria with a descriptive prompt (no audio conditioning).
3. Output lands as new tracks in the project.

**Copyright:** remaking copyrighted songs is fine for personal use; distributing a remake/cover needs rights (mechanical license for covers; master rights for anything derived from the original recording). Lyria/ElevenLabs filter recitation & voice likeness.

## 4. Architecture (`crates/au-ai`)
- `trait MusicGenerator { estimate_cost_usd, generate }` → backends: `ProceduralGenerator` (offline, free, tests), `OpenRouterLyria` (clip/pro), `FalAceStep`; planned `LocalAceStep` (python subprocess), `ElevenMusic`.
- `trait StemSeparator { separate }` → `DemucsSeparator` (subprocess, working); planned `OrtSeparator` (ONNX).
- App integration through `au-plugin-api`: generators appear under **Generate ▸ AI Music…** (prompt, lyrics, instrumental toggle, duration, backend, **cost shown before run**); separation under **Effect ▸ AI ▸ Separate Stems / Make Instrumental** creating 4 new tracks. Network calls run off the UI thread; API keys from env/settings, never in project files.

## 5. Cost per song (3.5 min)
| Task | Cost |
|---|---|
| Stems + instrumental (local Demucs) | $0 (≈80 s CPU) |
| Generate via fal ACE-Step | ≈$0.04 |
| Generate via Lyria 3 Pro | $0.08 |
| Generate via ElevenLabs | ≈$0.53 |
| Full remake (local stems + ACE-Step cover) | ≈$0.04 |

## 6. Live test results (Oct 14 2026)
- **Lyria 3 Clip via OpenRouter works.** OpenRouter requires `"stream": true` for audio output; audio arrives as base64 MP3 in SSE `delta.audio.data` chunks, cost in the final `usage.cost`. `OpenRouterLyria` now streams and parses this (`openrouter::parse_lyria_sse`).
- Key loading: `load_openrouter_key()` reads `~/.config/openrouter/env`, falling back to `$OPENROUTER_API_KEY`.
- 2 calls: instrumental lo-fi clip + indie-pop clip with lyrics; each 30.8 s 48 kHz MP3, **$0.04 each ($0.08 total)**, ~8–9 s latency.
- HTDemucs on the vocal clip (Rust `instrumental` example, CPU): 13.9 s for 30.8 s (~2.2× realtime). Stem levels: vocals −20.5 dB, drums −22.1, bass −23.5, other −18.9 (mix −14.3); vocals active in 35/61 half-second windows. On the instrumental clip the vocals stem was −45 dB (no false vocals).
