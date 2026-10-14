//! Direct in-process hosting of Audacity's Nyquist interpreter through libnyquist.

use crate::ny::{bindings, default_values, parse, NyType};
use au_plugin_host::nyquist::{NyquistEngine, NyquistJob, NyquistOutput};
use libloading::Library;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Mutex;

type AudioCallback =
    unsafe extern "C" fn(*mut f32, c_int, i64, i64, i64, *mut c_void) -> c_int;
type OutputCallback = unsafe extern "C" fn(c_int, *mut c_void);
type OsCallback = unsafe extern "C" fn(*mut c_void);

type InitFn = unsafe extern "C" fn();
type CleanupFn = unsafe extern "C" fn();
type SetPathFn = unsafe extern "C" fn(*const c_char);
type CaptureOutputFn = unsafe extern "C" fn(Option<OutputCallback>, *mut c_void);
type SetOsCallbackFn = unsafe extern "C" fn(Option<OsCallback>, *mut c_void);
type StopFn = unsafe extern "C" fn();
type BreakFn = unsafe extern "C" fn();
type SetAudioParamsFn = unsafe extern "C" fn(f64, i64);
type SetInputAudioFn =
    unsafe extern "C" fn(AudioCallback, *mut c_void, c_int, i64, f64);
type SetAudioNameFn = unsafe extern "C" fn(*const c_char);
type EvalFn = unsafe extern "C" fn(*const c_char) -> c_int;
type GetAudioChannelsFn = unsafe extern "C" fn() -> c_int;
type GetAudioFn = unsafe extern "C" fn(AudioCallback, *mut c_void) -> c_int;
type GetIntFn = unsafe extern "C" fn() -> c_int;
type GetDoubleFn = unsafe extern "C" fn() -> f64;
type GetStringFn = unsafe extern "C" fn() -> *const c_char;
type GetNumLabelsFn = unsafe extern "C" fn() -> u32;
type GetLabelFn =
    unsafe extern "C" fn(u32, *mut f64, *mut f64, *mut *const c_char);
type GetErrorFn = unsafe extern "C" fn() -> *const c_char;

#[derive(Clone, Copy)]
struct Api {
    init: InitFn,
    cleanup: CleanupFn,
    set_path: SetPathFn,
    capture_output: CaptureOutputFn,
    set_os_callback: SetOsCallbackFn,
    stop: StopFn,
    break_execution: BreakFn,
    set_audio_params: SetAudioParamsFn,
    set_input_audio: SetInputAudioFn,
    set_audio_name: SetAudioNameFn,
    eval: EvalFn,
    get_audio_channels: GetAudioChannelsFn,
    get_audio: GetAudioFn,
    get_int: GetIntFn,
    get_double: GetDoubleFn,
    get_string: GetStringFn,
    get_num_labels: GetNumLabelsFn,
    get_label: GetLabelFn,
    get_error: GetErrorFn,
}

impl Api {
    fn load(library: &Library) -> Result<Self, String> {
        // SAFETY: Each symbol is resolved by its documented nyx.h name and
        // copied as the matching C ABI function-pointer type.
        unsafe {
            Ok(Self {
                init: *library
                    .get::<InitFn>(b"nyx_init\0")
                    .map_err(|e| format!("missing nyx_init: {e}"))?,
                cleanup: *library
                    .get::<CleanupFn>(b"nyx_cleanup\0")
                    .map_err(|e| format!("missing nyx_cleanup: {e}"))?,
                set_path: *library
                    .get::<SetPathFn>(b"nyx_set_xlisp_path\0")
                    .map_err(|e| format!("missing nyx_set_xlisp_path: {e}"))?,
                capture_output: *library
                    .get::<CaptureOutputFn>(b"nyx_capture_output\0")
                    .map_err(|e| format!("missing nyx_capture_output: {e}"))?,
                set_os_callback: *library
                    .get::<SetOsCallbackFn>(b"nyx_set_os_callback\0")
                    .map_err(|e| format!("missing nyx_set_os_callback: {e}"))?,
                stop: *library
                    .get::<StopFn>(b"nyx_stop\0")
                    .map_err(|e| format!("missing nyx_stop: {e}"))?,
                break_execution: *library
                    .get::<BreakFn>(b"nyx_break\0")
                    .map_err(|e| format!("missing nyx_break: {e}"))?,
                set_audio_params: *library
                    .get::<SetAudioParamsFn>(b"nyx_set_audio_params\0")
                    .map_err(|e| format!("missing nyx_set_audio_params: {e}"))?,
                set_input_audio: *library
                    .get::<SetInputAudioFn>(b"nyx_set_input_audio\0")
                    .map_err(|e| format!("missing nyx_set_input_audio: {e}"))?,
                set_audio_name: *library
                    .get::<SetAudioNameFn>(b"nyx_set_audio_name\0")
                    .map_err(|e| format!("missing nyx_set_audio_name: {e}"))?,
                eval: *library
                    .get::<EvalFn>(b"nyx_eval_expression\0")
                    .map_err(|e| format!("missing nyx_eval_expression: {e}"))?,
                get_audio_channels: *library
                    .get::<GetAudioChannelsFn>(b"nyx_get_audio_num_channels\0")
                    .map_err(|e| format!("missing nyx_get_audio_num_channels: {e}"))?,
                get_audio: *library
                    .get::<GetAudioFn>(b"nyx_get_audio\0")
                    .map_err(|e| format!("missing nyx_get_audio: {e}"))?,
                get_int: *library
                    .get::<GetIntFn>(b"nyx_get_int\0")
                    .map_err(|e| format!("missing nyx_get_int: {e}"))?,
                get_double: *library
                    .get::<GetDoubleFn>(b"nyx_get_double\0")
                    .map_err(|e| format!("missing nyx_get_double: {e}"))?,
                get_string: *library
                    .get::<GetStringFn>(b"nyx_get_string\0")
                    .map_err(|e| format!("missing nyx_get_string: {e}"))?,
                get_num_labels: *library
                    .get::<GetNumLabelsFn>(b"nyx_get_num_labels\0")
                    .map_err(|e| format!("missing nyx_get_num_labels: {e}"))?,
                get_label: *library
                    .get::<GetLabelFn>(b"nyx_get_label\0")
                    .map_err(|e| format!("missing nyx_get_label: {e}"))?,
                get_error: *library
                    .get::<GetErrorFn>(b"nyx_get_error_str\0")
                    .map_err(|e| format!("missing nyx_get_error_str: {e}"))?,
            })
        }
    }
}

static NYQUIST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(unix)]
mod cstdio {
    use std::os::raw::{c_int, c_void};
    extern "C" {
        pub fn dup(fd: c_int) -> c_int;
        pub fn dup2(fd: c_int, fd2: c_int) -> c_int;
        pub fn close(fd: c_int) -> c_int;
        pub fn fflush(stream: *mut c_void) -> c_int;
    }
}

struct StdoutToStderr {
    #[cfg(unix)]
    saved: std::os::raw::c_int,
}

impl StdoutToStderr {
    fn new() -> Self {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        // SAFETY: plain POSIX fd duplication on the process's own stdio fds.
        #[cfg(unix)]
        unsafe {
            cstdio::fflush(ptr::null_mut());
            let saved = cstdio::dup(1);
            if saved >= 0 {
                cstdio::dup2(2, 1);
            }
            Self { saved }
        }
        #[cfg(not(unix))]
        Self {}
    }
}

impl Drop for StdoutToStderr {
    fn drop(&mut self) {
        // SAFETY: restores the fd saved in new().
        #[cfg(unix)]
        unsafe {
            cstdio::fflush(ptr::null_mut());
            if self.saved >= 0 {
                cstdio::dup2(self.saved, 1);
                cstdio::close(self.saved);
            }
        }
    }
}

/// An engine that evaluates scripts with Audacity's dynamically loaded Nyquist interpreter.
#[derive(Default)]
pub struct AudacityNyquist;

struct Cleanup {
    api: Api,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        // SAFETY: The interpreter was initialized, and callbacks are being
        // detached before the interpreter is cleaned up.
        unsafe {
            (self.api.capture_output)(None, ptr::null_mut());
            (self.api.set_os_callback)(None, ptr::null_mut());
            (self.api.cleanup)();
        }
    }
}

struct RunContext<'a> {
    api: Api,
    cancel: &'a dyn Fn() -> bool,
    cancelled: bool,
    input: Option<&'a au_core::AudioBuffer>,
    captured_output: Vec<u8>,
    output: Vec<Vec<f32>>,
    callback_error: Option<String>,
}

impl RunContext<'_> {
    fn is_cancelled(&mut self) -> bool {
        if (self.cancel)() {
            self.cancelled = true;
        }
        self.cancelled
    }
}

unsafe extern "C" fn input_callback(
    buffer: *mut f32,
    channel: c_int,
    start: i64,
    len: i64,
    _total_len: i64,
    userdata: *mut c_void,
) -> c_int {
    if userdata.is_null() || start < 0 || len < 0 || channel < 0 {
        return -1;
    }

    // SAFETY: userdata points to the live RunContext passed to the synchronous
    // Nyquist call; the C callback does not retain it.
    let context = unsafe { &mut *(userdata.cast::<RunContext<'_>>()) };
    if context.is_cancelled() {
        return -1;
    }

    let Some(audio) = context.input else {
        return -1;
    };
    let Some(samples) = audio.channels.get(channel as usize) else {
        return -1;
    };

    let start = start as usize;
    let len = len as usize;
    let Some(end) = start.checked_add(len) else {
        return -1;
    };
    if end > samples.len() || (len > 0 && buffer.is_null()) {
        return -1;
    }

    if len > 0 {
        // SAFETY: Nyquist supplies a writable buffer for `len` float samples;
        // the input range was checked to be within the source channel.
        unsafe {
            ptr::copy_nonoverlapping(samples.as_ptr().add(start), buffer, len);
        }
    }
    0
}

unsafe extern "C" fn output_callback(
    buffer: *mut f32,
    channel: c_int,
    start: i64,
    len: i64,
    total_len: i64,
    userdata: *mut c_void,
) -> c_int {
    if userdata.is_null() || channel < 0 || start < 0 || len < 0 {
        return -1;
    }

    // SAFETY: userdata points to the live RunContext for the synchronous
    // nyx_get_audio call.
    let context = unsafe { &mut *(userdata.cast::<RunContext<'_>>()) };
    if context.is_cancelled() {
        return -1;
    }

    let channel = channel as usize;
    let start = start as usize;
    let len = len as usize;
    let Some(end) = start.checked_add(len) else {
        context.callback_error = Some("Nyquist returned an invalid audio range".into());
        return -1;
    };

    const MAX_OUTPUT_FRAMES: usize = 64 * 1024 * 1024;
    if end > MAX_OUTPUT_FRAMES
        || (total_len > 0 && total_len as u64 > MAX_OUTPUT_FRAMES as u64)
    {
        context.callback_error = Some("Nyquist audio output exceeds the frame limit".into());
        return -1;
    }
    if len > 0 && buffer.is_null() {
        context.callback_error = Some("Nyquist returned a null audio buffer".into());
        return -1;
    }

    if context.output.len() <= channel {
        context.output.resize_with(channel + 1, Vec::new);
    }
    let samples = &mut context.output[channel];
    if samples.len() < end {
        samples.resize(end, 0.0);
    }
    if len > 0 {
        // SAFETY: Nyquist supplies a readable buffer for `len` float samples;
        // the destination range was resized to fit.
        unsafe {
            ptr::copy_nonoverlapping(buffer, samples.as_mut_ptr().add(start), len);
        }
    }
    0
}

unsafe extern "C" fn capture_callback(byte: c_int, userdata: *mut c_void) {
    if userdata.is_null() {
        return;
    }

    // SAFETY: userdata points to the live RunContext for the synchronous
    // interpreter evaluation.
    let context = unsafe { &mut *(userdata.cast::<RunContext<'_>>()) };
    context.captured_output.push(byte as u8);
}

unsafe extern "C" fn os_callback(userdata: *mut c_void) {
    if userdata.is_null() {
        return;
    }

    // SAFETY: userdata points to the live RunContext for the synchronous
    // interpreter evaluation.
    let context = unsafe { &mut *(userdata.cast::<RunContext<'_>>()) };
    if context.is_cancelled() {
        // SAFETY: Nyquist documents these functions for interrupting the
        // currently executing interpreter from its OS callback.
        unsafe {
            (context.api.stop)();
            (context.api.break_execution)();
        }
    }
}

impl NyquistEngine for AudacityNyquist {
    fn run(
        &self,
        job: &NyquistJob,
        cancel: &dyn Fn() -> bool,
    ) -> Result<NyquistOutput, String> {
        if cancel() {
            return Err("Nyquist execution cancelled".into());
        }

        let script = std::fs::read_to_string(&job.script_path).map_err(|e| {
            format!(
                "could not read Nyquist script {}: {e}",
                job.script_path.display()
            )
        })?;
        let program = parse(&script).map_err(|e| format!("Nyquist parse error: {e}"))?;

        let mut values = default_values(&program);
        for (key, value) in &job.params {
            if let Some((_, current)) = values.iter_mut().find(|(name, _)| name == key) {
                *current = value.clone();
            } else {
                values.push((key.clone(), value.clone()));
            }
        }

        let input = job.audio.as_ref().filter(|audio| !audio.channels.is_empty());
        if let Some(audio) = input {
            let frames = audio.channels[0].len();
            if audio.channels.iter().any(|channel| channel.len() != frames) {
                return Err("Nyquist input audio channels have different lengths".into());
            }
            if audio.channels.len() > 2 {
                return Err("Nyquist supports at most two input channels".into());
            }
        }

        let sample_rate = input
            .map(|audio| audio.sample_rate)
            .filter(|rate| *rate > 0)
            .unwrap_or(44_100);
        let input_len = input.map_or(0usize, |audio| audio.channels[0].len());
        let input_len_i64 =
            i64::try_from(input_len).map_err(|_| "Nyquist input is too long".to_owned())?;
        let input_channels = input.map_or(0usize, |audio| audio.channels.len());

        let _guard = NYQUIST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // libnyquist printf()s banners/diagnostics to stdout; keep stdout clean
        // (the sandbox worker protocol uses it) by routing fd 1 to stderr.
        let _stdout = StdoutToStderr::new();
        let library = shared_library()?;
        let api = Api::load(library)?;

        let mut context = RunContext {
            api,
            cancel,
            cancelled: false,
            input,
            captured_output: Vec::new(),
            output: Vec::new(),
            callback_error: None,
        };

        let plugin_dir = job
            .script_path
            .parent()
            .unwrap_or_else(|| Path::new("/usr/share/audacity/plug-ins"));
        let search_path = format!(
            "/usr/share/audacity/nyquist:{}:/usr/share/audacity/plug-ins",
            plugin_dir.display()
        );
        let search_path =
            CString::new(search_path).map_err(|_| "Nyquist search path contains NUL".to_owned())?;

        // SAFETY: The library remains loaded for the complete interpreter
        // lifetime, and all strings/callback userdata remain live through each
        // synchronous FFI call.
        // Audacity sets the XLisp path before nyx_init so init.lsp is found.
        unsafe {
            (api.set_path)(search_path.as_ptr());
            (api.init)();
        }
        let _cleanup = Cleanup { api };

        // SAFETY: The context outlives the interpreter run.
        unsafe {
            (api.capture_output)(Some(capture_callback), (&mut context as *mut RunContext<'_>).cast());
            (api.set_os_callback)(Some(os_callback), (&mut context as *mut RunContext<'_>).cast());
        }

        let is_tool = matches!(&program.kind, NyType::Tool);
        let is_generate = matches!(&program.kind, NyType::Generate);
        let audio_name = if program.version >= 4 && !is_tool {
            "*TRACK*"
        } else {
            "S"
        };
        let audio_name_c =
            CString::new(audio_name).expect("the fixed Nyquist audio name has no NUL");
        // SAFETY: Nyquist copies this name; the CString remains live for the call.
        unsafe {
            (api.set_audio_name)(audio_name_c.as_ptr());
        }

        let input_rate = f64::from(sample_rate);
        if input.is_some() && !is_generate && !is_tool {
            // nyx_set_input_audio also sets rate/length, matching NyquistBase.
            // SAFETY: `context` and the input buffer remain live until Nyquist
            // has finished evaluating and reading the input sound.
            unsafe {
                (api.set_input_audio)(
                    input_callback,
                    (&mut context as *mut RunContext<'_>).cast(),
                    input_channels as c_int,
                    input_len_i64,
                    input_rate,
                );
            }
        } else if is_tool {
            // SAFETY: This configures Nyquist's default tool invocation.
            unsafe {
                (api.set_audio_params)(44_100.0, 0);
            }
        } else {
            // Generators have no input sound; NyquistBase supplies LEN = 0.
            // SAFETY: The sample rate is finite and the selection length fits i64.
            unsafe {
                (api.set_audio_params)(input_rate, 0);
            }
        }

        let mut command = build_prologue(
            &program,
            job,
            input,
            sample_rate,
            input_len,
            input_channels,
        );
        command.push_str(&bindings(&program, &values));
        if !command.ends_with('\n') {
            command.push('\n');
        }

        if program.is_sal {
            let mut sal = program.code.clone();
            sal.push_str("\nset aud:result = main()\n");
            command.push_str("(setf *sal-call-stack* nil)\n");
            command.push_str("(setf aud:result nil)\n");
            command.push_str("(sal-compile-audacity \"");
            command.push_str(&escape_lisp_string(&sal));
            command.push_str("\" t t nil)\n");
            command.push_str("(prog1 aud:result (setf aud:result nil))\n");
        } else {
            command.push_str(&program.code);
            if !command.ends_with('\n') {
                command.push('\n');
            }
        }

        if let Some(path) = std::env::var_os("AU_NY_DUMP") {
            let _ = std::fs::write(path, &command);
        }
        let command_c =
            CString::new(command).map_err(|_| "Nyquist command contains NUL".to_owned())?;

        // SAFETY: The complete expression is NUL-terminated and lives through
        // the synchronous evaluation call.
        let result_type = unsafe { (api.eval)(command_c.as_ptr()) };

        if context.cancelled || cancel() {
            return Err("Nyquist execution cancelled".into());
        }

        match result_type {
            0 => {
                // nyx_get_error_str is commonly NULL; captured interpreter
                // output is Audacity's useful diagnostic in that case.
                let error = unsafe {
                    let ptr = (api.get_error)();
                    if ptr.is_null() {
                        None
                    } else {
                        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
                    }
                };
                let captured = String::from_utf8_lossy(&context.captured_output)
                    .trim()
                    .to_owned();
                if let Some(error) = error.filter(|s| !s.trim().is_empty()) {
                    Err(error)
                } else if !captured.is_empty() {
                    Err(format!("Nyquist evaluation failed:\n{captured}"))
                } else {
                    Err("Nyquist evaluation failed".into())
                }
            }
            1 => read_audio_result(&api, &mut context, sample_rate, input_channels),
            2 => {
                // SAFETY: The result type confirms the Nyquist integer getter
                // is valid until cleanup.
                Ok(NyquistOutput::Text(format!(
                    "Nyquist returned the value: {}",
                    unsafe { (api.get_int)() }
                )))
            }
            3 => {
                // SAFETY: The result type confirms the getter is valid; the
                // returned string remains owned by Nyquist until cleanup.
                let value = unsafe { (api.get_double)() };
                Ok(NyquistOutput::Text(format!(
                    "Nyquist returned the value: {value:.6}"
                )))
            }
            4 => {
                // SAFETY: The result type confirms the getter is valid; copy
                // the string before Nyquist cleanup.
                let ptr = unsafe { (api.get_string)() };
                if ptr.is_null() {
                    Ok(NyquistOutput::Text(String::new()))
                } else {
                    let value = unsafe { CStr::from_ptr(ptr) }
                        .to_string_lossy()
                        .into_owned();
                    Ok(NyquistOutput::Text(value))
                }
            }
            5 => {
                let count = unsafe { (api.get_num_labels)() };
                let mut labels = Vec::with_capacity(count as usize);
                for index in 0..count {
                    let mut start = 0.0;
                    let mut end = 0.0;
                    let mut text_ptr: *const c_char = ptr::null();
                    // SAFETY: Nyquist owns the indexed label and writes the
                    // supplied scalar outputs; its result remains live here.
                    unsafe {
                        (api.get_label)(index, &mut start, &mut end, &mut text_ptr);
                    }
                    let text = if text_ptr.is_null() {
                        String::new()
                    } else {
                        unsafe { CStr::from_ptr(text_ptr) }
                            .to_string_lossy()
                            .into_owned()
                    };
                    labels.push((
                        start + job.selection_secs.0,
                        end + job.selection_secs.0,
                        text,
                    ));
                }
                // LabelTrack::AddLabel keeps labels ordered by start time.
                labels.sort_by(|a, b| a.0.total_cmp(&b.0));
                Ok(NyquistOutput::Labels(labels))
            }
            6 => Ok(NyquistOutput::Nothing),
            other => Err(format!("Nyquist returned unknown result type {other}")),
        }
    }
}

fn read_audio_result(
    api: &Api,
    context: &mut RunContext<'_>,
    sample_rate: u32,
    input_channels: usize,
) -> Result<NyquistOutput, String> {
    // SAFETY: This getter reports the channel count for the current audio result.
    let output_channels = unsafe { (api.get_audio_channels)() };
    if output_channels == -1 {
        return Err("Nyquist returned one audio channel as an array".into());
    }
    if output_channels <= 0 {
        return Err("Nyquist returned an empty audio array".into());
    }

    let allowed_channels = if input_channels == 0 {
        2
    } else {
        input_channels
    };
    if output_channels as usize > allowed_channels {
        return Err("Nyquist returned too many audio channels".into());
    }

    // SAFETY: The callback and userdata remain valid for this synchronous
    // evaluation; Nyquist invokes it only while producing the current result.
    let success = unsafe {
        (api.get_audio)(
            output_callback,
            (context as *mut RunContext<'_>).cast(),
        )
    };
    if context.cancelled {
        return Err("Nyquist execution cancelled".into());
    }
    if let Some(error) = context.callback_error.take() {
        return Err(error);
    }
    if success == 0 {
        return Err("Nyquist failed while returning audio".into());
    }

    context.output.truncate(output_channels as usize);
    if context.output.len() < output_channels as usize {
        context
            .output
            .resize_with(output_channels as usize, Vec::new);
    }
    let frames = context.output.iter().map(Vec::len).max().unwrap_or(0);
    for channel in &mut context.output {
        channel.resize(frames, 0.0);
    }

    // Audacity duplicates a mono result when replacing a stereo channel group.
    if input_channels == 2 && output_channels == 1 {
        context.output.push(context.output[0].clone());
    }

    if frames == 0 {
        return Err("Nyquist returned nil audio".into());
    }

    Ok(NyquistOutput::Audio(au_core::AudioBuffer {
        sample_rate,
        channels: std::mem::take(&mut context.output),
    }))
}

/// The interpreter library is loaded once per process (like Audacity, which
/// links it) so XLisp's function table and our SUBR registration persist.
fn shared_library() -> Result<&'static Library, String> {
    static LIB: std::sync::OnceLock<Result<Library, String>> = std::sync::OnceLock::new();
    let lib = LIB
        .get_or_init(|| {
            let lib = load_library()?;
            register_functions(&lib)?;
            Ok(lib)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    Ok(lib)
}

// --- Audacity's RegisterFunctions(): _, _C, NGETTEXT, NGETTEXTC SUBRs ---
// Layout from xldmem.h/xlisp.h (LP64): node { char n_type; char n_flags; union at +8 },
// FIXNUM n_fixnum is `long`. SUBR = 1, FIXNUM = 5, STRING = 7.
type Lval = *mut u8;
#[repr(C)]
struct FunDef {
    name: *const c_char,
    kind: c_int,
    subr: unsafe extern "C" fn() -> Lval,
}
struct XlArgs {
    argc: *mut c_int,
    argv: *mut *mut Lval,
    toofew: unsafe extern "C" fn() -> Lval,
    badtype: unsafe extern "C" fn(Lval) -> Lval,
    toomany: unsafe extern "C" fn() -> Lval,
}
unsafe impl Send for XlArgs {}
unsafe impl Sync for XlArgs {}
// SAFETY: the table holds pointers to static C strings and fns only.
unsafe impl Send for FunDef {}
unsafe impl Sync for FunDef {}
static XL: std::sync::OnceLock<XlArgs> = std::sync::OnceLock::new();

/// SAFETY: only called by XLisp while evaluating a SUBR, with xlargc/xlargv valid.
unsafe fn xl_arg(kind: u8) -> Lval {
    let x = XL.get().expect("registered");
    if *x.argc <= 0 {
        return (x.toofew)();
    }
    let v = **x.argv;
    if v.is_null() || *v != kind {
        return (x.badtype)(v);
    }
    *x.argc -= 1;
    *x.argv = (*x.argv).add(1);
    v
}
unsafe fn xl_skip_optional() {
    let x = XL.get().expect("registered");
    if *x.argc > 0 {
        *x.argc -= 1;
        *x.argv = (*x.argv).add(1);
    }
}
unsafe fn xl_last() {
    let x = XL.get().expect("registered");
    if *x.argc > 0 {
        (x.toomany)();
    }
}
// English catalogue: translations are the identity (GetCustomTranslation/wxGetTranslation).
unsafe extern "C" fn subr_gettext() -> Lval {
    let s = xl_arg(7);
    xl_skip_optional();
    xl_last();
    s
}
unsafe extern "C" fn subr_ngettext() -> Lval {
    let s1 = xl_arg(7);
    let s2 = xl_arg(7);
    let n = xl_arg(5);
    let num = *(n.add(8) as *const i64);
    xl_skip_optional();
    xl_last();
    if num == 1 { s1 } else { s2 }
}

fn register_functions(lib: &Library) -> Result<(), String> {
    // SAFETY: symbol types match xlisp.h declarations in this library.
    unsafe {
        let argc = *lib.get::<*mut c_int>(b"xlargc\0").map_err(|e| e.to_string())?;
        let argv = *lib.get::<*mut *mut Lval>(b"xlargv\0").map_err(|e| e.to_string())?;
        let toofew = *lib.get::<unsafe extern "C" fn() -> Lval>(b"xltoofew\0").map_err(|e| e.to_string())?;
        let toomany = *lib.get::<unsafe extern "C" fn() -> Lval>(b"xltoomany\0").map_err(|e| e.to_string())?;
        let badtype = *lib.get::<unsafe extern "C" fn(Lval) -> Lval>(b"xlbadtype\0").map_err(|e| e.to_string())?;
        let bind = *lib
            .get::<unsafe extern "C" fn(*const FunDef, usize) -> c_int>(b"xlbindfunctions\0")
            .map_err(|e| e.to_string())?;
        let _ = XL.set(XlArgs { argc, argv, toofew, badtype, toomany });
        static TABLE: std::sync::OnceLock<Vec<FunDef>> = std::sync::OnceLock::new();
        let t = TABLE.get_or_init(|| {
            vec![
                FunDef { name: c"_".as_ptr(), kind: 1, subr: subr_gettext },
                FunDef { name: c"_C".as_ptr(), kind: 1, subr: subr_gettext },
                FunDef { name: c"NGETTEXT".as_ptr(), kind: 1, subr: subr_ngettext },
                FunDef { name: c"NGETTEXTC".as_ptr(), kind: 1, subr: subr_ngettext },
            ]
        });
        bind(t.as_ptr(), t.len());
    }
    Ok(())
}

fn load_library() -> Result<Library, String> {
    if let Some(path) = std::env::var_os("AU_NYQUIST_LIB") {
        let path = PathBuf::from(path);
        // SAFETY: Loading the user-selected shared library is the explicit
        // runtime configuration requested for the Nyquist engine.
        return unsafe { Library::new(&path) }
            .map_err(|e| format!("could not load Nyquist library {}: {e}", path.display()));
    }

    let candidates = [
        PathBuf::from("/usr/lib/audacity/lib-nyquist-effects.so"),
        PathBuf::from("/usr/lib/x86_64-linux-gnu/audacity/lib-nyquist-effects.so"),
        PathBuf::from("/usr/local/lib/audacity/lib-nyquist-effects.so"),
    ];
    let mut errors = Vec::new();
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        // SAFETY: This is the Audacity Nyquist shared library at a standard
        // installation path; symbols are validated by Api::load.
        match unsafe { Library::new(&path) } {
            Ok(library) => return Ok(library),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }

    if errors.is_empty() {
        Err("Nyquist library not found; set AU_NYQUIST_LIB".into())
    } else {
        Err(format!(
            "could not load Nyquist library: {}",
            errors.join("; ")
        ))
    }
}

fn build_prologue(
    program: &crate::ny::NyProgram,
    job: &NyquistJob,
    input: Option<&au_core::AudioBuffer>,
    sample_rate: u32,
    input_len: usize,
    input_channels: usize,
) -> String {
    let mut command = String::from(
        "(snd-set-latency  0.1)",
    );
    let is_tool = matches!(&program.kind, NyType::Tool);

    if is_tool {
        command.push_str("(setf S 0.25)\n");
    } else if program.version >= 4 {
        command.push_str("(setf S 0.25)\n");
    } else {
        command.push_str("(setf *TRACK* '*unbound*)\n");
    }

    if program.version >= 4 {
        command.push_str(&version_four_properties(
            job,
            input,
            sample_rate,
            input_len,
            input_channels,
        ));
    }

    if !is_tool && program.version >= 4 {
        command.push_str("(putprop '*TRACK* 1 'INDEX)\n");
        command.push_str("(putprop '*TRACK* \"Audio\" 'NAME)\n");
        command.push_str("(putprop '*TRACK* \"wave\" 'TYPE)\n");
        command.push_str("(putprop '*TRACK* \"Waveform\" 'VIEW)\n");
        command.push_str(&format!(
            "(putprop '*TRACK* {} 'CHANNELS)\n",
            input_channels.max(1)
        ));
        command.push_str("(putprop '*TRACK* NIL 'SPECTRAL-EDIT-ENABLED)\n");
        command.push_str(&format!(
            "(putprop '*TRACK* (float {}) 'START-TIME)\n",
            lisp_number(job.selection_secs.0)
        ));
        command.push_str(&format!(
            "(putprop '*TRACK* (float {}) 'END-TIME)\n",
            lisp_number(job.selection_secs.1)
        ));
        command.push_str("(putprop '*TRACK* (float 1.0) 'GAIN)\n");
        command.push_str("(putprop '*TRACK* (float 0.0) 'PAN)\n");
        command.push_str(&format!(
            "(putprop '*TRACK* (float {}) 'RATE)\n",
            lisp_number(f64::from(sample_rate))
        ));
        command.push_str("(putprop '*TRACK* 32.0 'FORMAT)\n");

        let start = job.selection_secs.0;
        let end = job.selection_secs.1;
        // GetClipBoundaries + per-channel wrapping exactly as NyquistBase.
        let bounds = format!(
            "(list (float {}) (float {}))",
            lisp_number(start),
            lisp_number(end)
        );
        let clip = if input_channels > 1 {
            let per = format!("(list {bounds} )");
            format!("(vector {} )", per.repeat(input_channels))
        } else {
            format!("(list {bounds} )")
        };
        command.push_str(&format!("(putprop '*TRACK* {clip} 'INCLIPS)\n"));
        command.push_str(&format!("(putprop '*TRACK* {clip} 'CLIPS)\n"));

        let (peak, rms) = audio_stats(input);
        if input_channels > 1 {
            command.push_str(&format!(
                "(putprop '*SELECTION* (vector (float {}) (float {})) 'PEAK)\n",
                lisp_number(peak.0),
                lisp_number(peak.1)
            ));
            command.push_str(&format!(
                "(putprop '*SELECTION* (vector (float {}) (float {})) 'RMS)\n",
                lisp_number(rms.0),
                lisp_number(rms.1)
            ));
        } else {
            command.push_str(&format!(
                "(putprop '*SELECTION* (float {}) 'PEAK)\n",
                lisp_number(peak.0)
            ));
            command.push_str(&format!(
                "(putprop '*SELECTION* (float {}) 'RMS)\n",
                lisp_number(rms.0)
            ));
        }
        command.push_str(&format!(
            "(putprop '*SELECTION* (float {}) 'PEAK-LEVEL)\n",
            lisp_number(peak.0.max(peak.1))
        ));
    }

    if matches!(&program.kind, NyType::Generate) {
        command.push_str("(setf s 0.25)\n");
    }

    command.push_str("(setf *tracenable* NIL)\n");
    command
}

fn version_four_properties(
    job: &NyquistJob,
    input: Option<&au_core::AudioBuffer>,
    sample_rate: u32,
    input_len: usize,
    input_channels: usize,
) -> String {
    let selection_start = job.selection_secs.0;
    let selection_end = job.selection_secs.1;
    let channels = if input_channels == 0 {
        0
    } else {
        input_channels
    };
    let frames = if input_len == 0 {
        ((selection_end - selection_start).max(0.0) * f64::from(sample_rate)).round() as usize
    } else {
        input_len
    };
    let end = if selection_end > selection_start {
        selection_end
    } else {
        selection_start + frames as f64 / f64::from(sample_rate)
    };

    let mut out = String::new();
    out.push_str("(putprop '*AUDACITY* (list 3 7 3) 'VERSION)\n");
    out.push_str("(putprop '*AUDACITY* \"en\" 'LANGUAGE)\n");
    out.push_str("(setf *DECIMAL-SEPARATOR* #\\.)\n");

    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let temp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    for (property, value) in [
        ("BASE", "/usr"),
        ("DATA", "/usr/share/audacity"),
        ("HELP", "/usr/share/doc/audacity"),
        ("TEMP", temp.as_str()),
        ("SYS-TEMP", temp.as_str()),
        ("DOCUMENTS", home.as_str()),
        ("HOME", home.as_str()),
        ("USER-PLUG-IN", ""),
    ] {
        out.push_str(&format!(
            "(putprop '*SYSTEM-DIR* \"{}\" '{property})\n",
            escape_lisp_string(value)
        ));
    }

    let script_dir = job
        .script_path
        .parent()
        .unwrap_or_else(|| Path::new("/usr/share/audacity/plug-ins"));
    let paths = format!(
        "\"/usr/share/audacity/nyquist\" \"{}\" \"/usr/share/audacity/plug-ins\"",
        escape_lisp_string(&script_dir.to_string_lossy())
    );
    out.push_str(&format!(
        "(putprop '*SYSTEM-DIR* (list {paths}) 'PLUGIN)\n"
    ));
    out.push_str(&format!(
        "(putprop '*SYSTEM-DIR* (list {paths}) 'PLUG-IN)\n"
    ));

    let (year, day_of_year, month, day, hour, minute, second, weekday) = utc_parts();
    out.push_str(&format!(
        "(setf *SYSTEM-TIME* (list {year} {day_of_year} {hour} {minute} {second}))\n"
    ));
    out.push_str(&format!(
        "(putprop '*SYSTEM-TIME* \"{month:02}/{day:02}/{year}\" 'DATE)\n"
    ));
    out.push_str(&format!(
        "(putprop '*SYSTEM-TIME* \"{hour:02}:{minute:02}:{second:02}\" 'TIME)\n"
    ));
    out.push_str(&format!(
        "(putprop '*SYSTEM-TIME* \"{year:04}-{month:02}-{day:02}\" 'ISO-DATE)\n"
    ));
    out.push_str(&format!(
        "(putprop '*SYSTEM-TIME* \"{hour:02}:{minute:02}:{second:02}\" 'ISO-TIME)\n"
    ));
    out.push_str(&format!("(putprop '*SYSTEM-TIME* {year} 'YEAR)\n"));
    out.push_str(&format!("(putprop '*SYSTEM-TIME* {day} 'DAY)\n"));
    out.push_str(&format!("(putprop '*SYSTEM-TIME* {} 'MONTH)\n", month - 1));
    out.push_str(&format!(
        "(putprop '*SYSTEM-TIME* \"{}\" 'MONTH-NAME)\n",
        month_name(month)
    ));
    out.push_str(&format!(
        "(putprop '*SYSTEM-TIME* \"{}\" 'DAY-NAME)\n",
        weekday_name(weekday)
    ));

    out.push_str("(putprop '*PROJECT* 1 'PROJECTS)\n");
    out.push_str("(putprop '*PROJECT* \"\" 'NAME)\n");
    out.push_str(&format!(
        "(putprop '*PROJECT* (float {}) 'RATE)\n",
        lisp_number(f64::from(sample_rate))
    ));
    out.push_str(&format!("(putprop '*PROJECT* {} 'TRACKS)\n", usize::from(channels > 0)));
    out.push_str(&format!(
        "(putprop '*PROJECT* {} 'WAVETRACKS)\n",
        usize::from(channels > 0)
    ));
    out.push_str("(putprop '*PROJECT* 0 'LABELTRACKS)\n");
    out.push_str("(putprop '*PROJECT* 0 'MIDITRACKS)\n");
    out.push_str("(putprop '*PROJECT* 0 'TIMETRACKS)\n");
    out.push_str("(putprop '*PROJECT* (float 6.0) 'PREVIEW-DURATION)\n");
    out.push_str("(setf *PREVIEWP* NIL)\n");

    if channels > 0 {
        out.push_str("(putprop '*SELECTION* (list 1) 'TRACKS)\n");
    } else {
        out.push_str("(putprop '*SELECTION* NIL 'TRACKS)\n");
    }
    out.push_str(&format!(
        "(putprop '*SELECTION* {channels} 'CHANNELS)\n"
    ));

    let rate = f64::from(sample_rate);
    let snapped_start = (selection_start * rate).round() / rate;
    let snapped_end = (end * rate).round() / rate;
    let (f0, f1) = spectral_bounds(&job.params);
    let low = f0.map_or_else(|| "NIL".to_owned(), |v| format!("(float {})", lisp_number(v)));
    let high = f1.map_or_else(|| "NIL".to_owned(), |v| format!("(float {})", lisp_number(v)));
    let center = match (f0, f1) {
        (Some(a), Some(b)) => format!("(float {})", lisp_number((a * b).sqrt())),
        _ => "NIL".to_owned(),
    };
    let bandwidth = match (f0, f1) {
        (Some(a), Some(b)) if a > 0.0 && b >= a => {
            let value = (b / a).ln() / 2.0_f64.ln();
            if value.is_finite() {
                format!("(float {})", lisp_number(value))
            } else {
                "NIL".to_owned()
            }
        }
        _ => "NIL".to_owned(),
    };
    out.push_str(&format!(
        "(putprop '*SELECTION* {low} 'LOW-HZ)\n\
         (putprop '*SELECTION* {center} 'CENTER-HZ)\n\
         (putprop '*SELECTION* {high} 'HIGH-HZ)\n\
         (putprop '*SELECTION* {bandwidth} 'BANDWIDTH)\n"
    ));
    out.push_str(&format!(
        "(putprop '*SELECTION* (float {}) 'START)\n",
        lisp_number(snapped_start)
    ));
    out.push_str(&format!(
        "(putprop '*SELECTION* (float {}) 'END)\n",
        lisp_number(snapped_end)
    ));
    let _ = input;
    out
}

fn audio_stats(input: Option<&au_core::AudioBuffer>) -> ((f64, f64), (f64, f64)) {
    let Some(audio) = input else {
        return ((0.0, 0.0), (0.0, 0.0));
    };
    let stats = |channel: &[f32]| {
        let mut peak = 0.0_f32;
        let mut sum = 0.0_f64;
        for &sample in channel {
            peak = peak.max(sample.abs());
            sum += f64::from(sample) * f64::from(sample);
        }
        let rms = if channel.is_empty() {
            0.0
        } else {
            (sum / channel.len() as f64).sqrt()
        };
        // NyquistBase stores both in `float` before formatting.
        (f64::from(peak), f64::from(rms as f32))
    };
    let first = audio.channels.first().map_or((0.0, 0.0), |c| stats(c));
    let second = audio.channels.get(1).map_or(first, |c| stats(c));
    // ((peak L, peak R), (rms L, rms R))
    ((first.0, second.0), (first.1, second.1))
}

fn spectral_bounds(params: &[(String, String)]) -> (Option<f64>, Option<f64>) {
    let parse = |key: &str| {
        params
            .iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| value.parse::<f64>().ok())
            .filter(|value| *value >= 0.0)
    };
    (parse("__f0"), parse("__f1"))
}

fn lisp_number(value: f64) -> String {
    crate::ny::internat_to_string(value, -1)
}

fn escape_lisp_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn utc_parts() -> (i32, u32, u32, u32, u32, u32, u32, u32) {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let days = (seconds / 86_400) as i64;
    let within_day = seconds % 86_400;
    let hour = (within_day / 3_600) as u32;
    let minute = ((within_day % 3_600) / 60) as u32;
    let second = (within_day % 60) as u32;

    // Gregorian civil-from-days conversion, with days counted from 1970-01-01.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let january_first_days = days - (doy - day + 1);
    let day_of_year = (days - january_first_days + 1) as u32;
    let weekday = (days + 4).rem_euclid(7) as u32;

    (
        year as i32,
        day_of_year,
        month as u32,
        day as u32,
        hour,
        minute,
        second,
        weekday,
    )
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        _ => "December",
    }
}

fn weekday_name(weekday: u32) -> &'static str {
    match weekday {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        _ => "Saturday",
    }
}

/// Installs this engine as the process-wide Nyquist engine.
pub fn install() {
    au_plugin_host::nyquist::set_engine(Box::new(AudacityNyquist::default()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

    fn library_is_available() -> bool {
        if let Some(path) = std::env::var_os("AU_NYQUIST_LIB") {
            return PathBuf::from(path).is_file();
        }
        [
            "/usr/lib/audacity/lib-nyquist-effects.so",
            "/usr/lib/x86_64-linux-gnu/audacity/lib-nyquist-effects.so",
            "/usr/local/lib/audacity/lib-nyquist-effects.so",
        ]
        .iter()
        .any(|path| Path::new(path).is_file())
    }

    fn make_script(kind: &str, code: &str) -> PathBuf {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "audacity-nyquist-engine-{}-{id}.ny",
            std::process::id()
        ));
        let text = format!(";nyquist plug-in\n;version 4\n;type {kind}\n{code}\n");
        std::fs::write(&path, text).expect("write temporary Nyquist script");
        path
    }

    fn run_script(
        kind: &str,
        code: &str,
        audio: Option<au_core::AudioBuffer>,
        selection_secs: (f64, f64),
    ) -> Result<NyquistOutput, String> {
        let path = make_script(kind, code);
        let result = AudacityNyquist::default().run(
            &NyquistJob {
                script_path: path.clone(),
                params: Vec::new(),
                audio,
                selection_secs,
            },
            &|| false,
        );
        let _ = std::fs::remove_file(path);
        result
    }

    #[test]
    fn generator_returns_audio() {
        if !library_is_available() {
            return;
        }
        let output = run_script("generate", "(osc 60 1)", None, (0.0, 1.0)).unwrap();
        let NyquistOutput::Audio(audio) = output else {
            panic!("generator did not return audio");
        };
        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.channels.len(), 1);
        assert_eq!(audio.channels[0].len(), 44_100);
    }

    #[test]
    fn process_multiplies_input() {
        if !library_is_available() {
            return;
        }
        let input = au_core::AudioBuffer {
            sample_rate: 44_100,
            channels: vec![vec![0.25; 256], vec![-0.5; 256]],
        };
        let output = run_script(
            "process",
            "(mult *track* 0.5)",
            Some(input),
            (0.0, 256.0 / 44_100.0),
        )
        .unwrap();
        let NyquistOutput::Audio(audio) = output else {
            panic!("process did not return audio");
        };
        assert_eq!(audio.channels.len(), 2);
        assert!(audio.channels[0].iter().all(|sample| (*sample - 0.125).abs() < 1e-6));
        assert!(audio.channels[1].iter().all(|sample| (*sample + 0.25).abs() < 1e-6));
    }

    #[test]
    fn analyzer_returns_labels() {
        if !library_is_available() {
            return;
        }
        let output = run_script(
            "analyze",
            "(list (list 0.1 0.2 \"label\"))",
            None,
            (3.0, 4.0),
        )
        .unwrap();
        let NyquistOutput::Labels(labels) = output else {
            panic!("analyzer did not return labels");
        };
        assert_eq!(labels, vec![(3.1, 3.2, "label".to_owned())]);
    }

    #[test]
    fn syntax_error_returns_error() {
        if !library_is_available() {
            return;
        }
        assert!(run_script("process", "(this is not valid", None, (0.0, 1.0)).is_err());
    }
}
