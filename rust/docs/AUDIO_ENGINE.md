# Audio engine (Stage 3)

## Threads
- **UI thread** owns the `Editor`, builds `MixTrack`s (`au-app::transport::mix_tracks`), and talks to the engine only through `EngineHandle` (an `rtrb` SPSC command ring + atomics).
- **Audio callback** (`AudioCallback::render`) drains commands, mixes preallocated track buffers, writes meters/position to `SharedMeters` atomics. No locks, no allocation, no I/O.
- **Capture callback** pushes interleaved `f32` into an SPSC ring and updates an atomic peak; the UI calls `Recorder::poll` each frame and feeds `RecordController` (timer / sound-activation gate, latency compensation, placement).

## Sample rates
If the output device rate differs from the project, each track is resampled with `rubato` (sinc 256 taps, cutoff 0.95, BlackmanHarris2) before the stream starts; positions are converted with `Player::rate_ratio`.

## Testing without hardware
`engine::NullOutput::pump` and `engine::NullInput` drive the exact callback/record code paths in tests (`cargo test -p au-io`, `cargo test -p au-app`).

## System-audio (loopback) recording per OS
| OS | Support | How |
|---|---|---|
| Windows | Native | `InputKind::Loopback` opens an input stream on the default *output* device (WASAPI loopback via cpal). |
| Linux | Indirect | Not via `InputKind::Loopback` (returns `Unsupported`). Select the PulseAudio/PipeWire "Monitor of <sink>" source as the default input. |
| macOS | Unsupported | Needs a virtual device (BlackHole, Loopback) set as input; ScreenCaptureKit capture is future work. |

## Known limits
- Not yet run on real audio hardware (build box has no sound card).
- Latency compensation is a user-entered value (ms); no automatic round-trip measurement yet.
- Scrubbing gesture not wired (engine supports variable/negative speed).
