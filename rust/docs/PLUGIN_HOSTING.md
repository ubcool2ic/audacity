# Plugin hosting (Stage 7, part 1: CLAP + VST3)

Crate: `crates/au-plugin-host` — one module per format (`clap.rs`, `vst3.rs`; LV2/LADSPA modules are added later by another worker), plus `state.rs` (Audacity-compatible settings), `sandbox.rs` (out-of-process scanning) and the `au-plugin-scan` helper binary. It implements the `au-plugin-api` traits `PluginHost` / `PluginInstance`; `au_plugin_host::registry()` builds a `PluginRegistry` with the real CLAP/VST3 hosts (sandboxed) plus the native and LV2/LADSPA stub hosts.

## What works
| | CLAP (`clap-sys` 0.5) | VST3 (`vst3` 0.3 bindings) |
|---|---|---|
| Discovery | `.clap` files under CLAP spec paths + `CLAP_PATH` | `.vst3` bundles under VST3 spec paths + `VST3_PATH`; Linux `ModuleEntry`/`ModuleExit` |
| Instantiate | `clap_entry` → plugin factory → `create_plugin` + `init`; host exposes log, thread-check, params, latency, state, gui | `IPluginFactory::createInstance` → `IComponent` + `IAudioProcessor` + `IEditController` (single or split, `IConnectionPoint` wired), `IHostApplication`, `IComponentHandler` |
| Parameters | `params` ext; plain values; automation via `CLAP_EVENT_PARAM_VALUE` input events (or `flush` while inactive) | normalized 0..1 values; `setParamNormalized` + `IParameterChanges` queue to the processor each block |
| Audio | 32-bit float, main in/out port, mono↔stereo mapping, block splitting | `kSample32`, `kRealtime` mode, bus arrangement + activation, same mapping |
| Latency | `latency` ext | `getLatencySamples` |
| State | `State` = base64 of `state.save`, plus `Parameters` | Audacity's exact `VST3Wrapper` keys: `ProcessorState`, `ControllerState` (base64 of the `IBStream` blobs), `Parameters` (`"id=value;"`, `%f`) |
| GUI | `gui` ext: embedded (X11/Win32/Cocoa) or floating | `IPlugView` + `IPlugFrame`/`Linux::IRunLoop` host, `X11EmbedWindowID` |

### `.aup3` storage
`save_effect()` turns an instance into an `au_project::aup3::RealtimeEffect` (`<effect id=... active=...><parameters><parameter name= value=/>`), the element Audacity uses for realtime effect stacks; `restore_effect()` re-instantiates it and loads the state. Effect ids are `VST3:<module path>;<class UID>` (Audacity's VST3 plugin path format is `<module path>;<UID>`) and `CLAP:<path>;<clap id>`. Audacity 3.x has no CLAP host, so the CLAP keys are our own; the VST3 keys/encoding match Audacity so VST3 state round-trips with real Audacity projects (not yet cross-tested against Audacity itself).

### Sandboxed scanning
`SandboxedHost` (used by `registry()`) scans each plugin file in a separate `au-plugin-scan` process (located via `AU_PLUGIN_SCANNER`, next to the executable, or its parent dir), one `au-runtime` job per file, with a 30 s timeout. A plugin that segfaults, aborts or hangs only kills the scanner: it is reported as `ScanOutcome::Crashed` / `TimedOut` and logged to the `au-runtime` crash ring; Rust panics inside the scanner are contained by `au_runtime::pool::isolate`. Instantiation for processing still happens in-process (like Audacity).

## Linux X11 editor embedding
- The host passes an X11 window id (`EditorParent::X11(xid)`). CLAP: `gui.create("x11", false)`, `set_parent(clap_window{api:"x11", x11: xid})`, `show`. VST3: `IPlugView::isPlatformTypeSupported("X11EmbedWindowID")`, `setFrame(IPlugFrame)`, `attached(xid as *mut c_void, "X11EmbedWindowID")`.
- VST3 editors on Linux need the host's `Steinberg::Linux::IRunLoop` (queried from the `IPlugFrame`): the host registers the plugin's fd handlers and timers and must call `Vst3Instance::idle_editor()` from the UI loop. CLAP editors can use `posix-fd-support`/`timer-support` (not implemented yet; most plugins run their own thread).
- With eframe/winit, the parent is a child X11 window created under the egui window (raw-window-handle `XlibWindowHandle`/`XcbWindowHandle`); Wayland-only sessions need XWayland (no Wayland embedding in VST3/CLAP yet), or the CLAP floating mode.
- **Not verified yet**: under a bare Xvfb (no window manager) the Steinberg AGain editor segfaults inside `IPlugView::attached`, and Surge XT (JUCE) aborts with an X `BadAtom` error. Needs testing on a real desktop session or Xvfb + a window manager, and a crash-isolated editor host process.

## Licensing
- **VST3 SDK**: since VST 3.8 (released 2025-10-20) Steinberg licenses the SDK under the **MIT License** (official: Steinberg press release 2025-10-29 "VST 3 Now Available Under MIT License"; VST 3 developer portal "VST 3 License" page, which also states the former GPLv3/proprietary dual licence is no longer offered; steinberg.net/developers/vstsdk). The SDK cloned on the box (`vst3sdk`, VST 3.8.1) carries the MIT `LICENSE.txt`. MIT is GPL-compatible, so GPL-3.0-or-later Audacity-RS can host VST3; obligation: keep Steinberg's copyright notice and MIT text in distributions that include SDK-derived code. "VST" logo/trademark use is governed separately by Steinberg's usage guidelines and is optional. VST2 is not covered and is not used.
- **`vst3` crate** (Rust bindings generated from the SDK headers): MIT OR Apache-2.0. **`clap-sys`**: MIT OR Apache-2.0; the CLAP standard itself is MIT. **`libloading`**: ISC. **`base64`**: MIT OR Apache-2.0. All GPLv3-compatible.
- Plugins are separate works loaded at run time; test plugins used: Surge XT 1.3.4 (GPL-3.0), Steinberg SDK examples AGain/ADelay (MIT, built from the SDK). None are committed to the repo.

## Tests (`cargo test -p au-plugin-host`)
Real plugins (each test skips if the file is absent; override with `AU_TEST_CLAP` / `AU_TEST_VST3`): Surge XT Effects `.clap` load + process + state round trip; Steinberg AGain gain 0.5 ⇒ output RMS ratio 0.5, latency 0, state round trip through a saved `.aup3` file; Steinberg ADelay process; sandboxed scan of both; a deliberately segfaulting CLAP built with `cc` is reported as `Crashed` while the test process survives.
Build the test plugins: `git clone --recursive https://github.com/steinbergmedia/vst3sdk && cmake -S vst3sdk -B vst3sdk/build-gui -G Ninja -DCMAKE_BUILD_TYPE=Release -DSMTG_RUN_VST_VALIDATOR=OFF && ninja -C vst3sdk/build-gui again adelay`; Surge XT: `surge-xt-linux-1.3.4-pluginsonly.tar.gz` from github.com/surge-synthesizer/releases-xt.

## Update (Stage 8)
**Effect ids** now use Audacity's PluginID: `Effect_<VST3|CLAP>_<vendor>_<name>_<path>` (VST3 path = `<bundle>;<UID>`). Legacy `VST3:`/`CLAP:` ids still parse. Audacity only matches if the path equals what its pluginregistry.cfg holds.

**Editors** run out of process (`au-plugin-editor`, `au_plugin_host::editor::EditorProcess`), line protocol on stdin/stdout (STATE/LOAD/GETSTATE/SETPARAM/CLOSE → READY/PARAM/STATE/STATEEND/ERROR/CLOSED). A crash in the editor is reported as `Crashed{status}` and the app continues. Fixes that made them work on X11: VST3 needs `IPluginFactory3::setHostContext` with a host context exposing `Linux::IRunLoop` (VSTGUI otherwise null-derefs its RunLoop); JUCE CLAP plugins (Surge) need `timer-support` and `posix-fd-support`. Screenshots: `img/again_editor.png`, `img/surge_editor.png`. Tests: `tests/editor_host.rs` (needs DISPLAY + WM).

**Audacity 3.7.3 interop: PASS.** Debian's build lacks VST3; the official AppImage was used. `examples/aup3_vst3_state.rs` writes a project with AGain (keys ProcessorState/ControllerState/Parameters); `tools/aup3-verify/{run,probe}.sh` open it under Xvfb+openbox and export via mod-script-pipe. Gain 0.5 → peak 0.4001 / rms 0.2828; gain 1.0 → peak 0.8001 (0.8 sine input).

### au-effects change request (for Chief of Staff)
1. `BuiltinEffect::id()` returns `&'static str` → need owned/dynamic ids (`Cow<'static, str>` or `String`); au-app currently interns (leaks) plugin ids.
2. `EffectStack::describe()` wraps `plugin_id(effect.id())` builtin-style → pass through ids that are already full PluginIDs (e.g. starting with `Effect_`), or add a `fn plugin_id(&self) -> String` to the trait with that default.
3. `EffectStack::from_desc`/`push` only call `registry::create` → add `push_effect(Box<dyn BuiltinEffect>)` and a pluggable factory (`from_desc_with(&desc, &dyn Fn(&str)->Option<Box<dyn BuiltinEffect>>)`) so plugin entries restore without app-side slot rebuilding.
4. `get_text(&self)` — plugin state save needs `&mut` (app wraps instance in a Mutex, also taken on the RT process path, uncontended).
