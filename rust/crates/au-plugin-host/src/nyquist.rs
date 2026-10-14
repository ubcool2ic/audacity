//! Crash-contained hosting and discovery for offline Nyquist plug-ins.

use au_plugin_api::{
    PluginDescriptor, PluginError, PluginFormat, PluginHost, PluginInstance,
};
use au_runtime::pool::CancelToken;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

const MAGIC: &[u8; 5] = b"AUNY1";
const MAX_STRING_LEN: usize = 64 * 1024 * 1024;
const MAX_ITEMS: usize = 1_000_000;
const MAX_AUDIO_SAMPLES: usize = 16_000_000;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Engine that evaluates Nyquist code inside the worker process.
pub trait NyquistEngine: Send + Sync {
    /// Evaluates a job. The callback returns whether execution should be cancelled.
    fn run(
        &self,
        job: &NyquistJob,
        cancel: &dyn Fn() -> bool,
    ) -> Result<NyquistOutput, String>;
}

/// A Nyquist script invocation and its optional input audio.
#[derive(Clone, Debug, PartialEq)]
pub struct NyquistJob {
    /// Path to the `.ny` script.
    pub script_path: PathBuf,
    /// Free-form string parameters.
    pub params: Vec<(String, String)>,
    /// Input audio, or `None` for generators.
    pub audio: Option<au_core::AudioBuffer>,
    /// Start and end of the selection, in seconds.
    pub selection_secs: (f64, f64),
}

/// A result produced by a Nyquist script.
#[derive(Clone, Debug, PartialEq)]
pub enum NyquistOutput {
    /// Processed or generated audio.
    Audio(au_core::AudioBuffer),
    /// Label track entries.
    Labels(Vec<(f64, f64, String)>),
    /// Text output.
    Text(String),
    /// No output.
    Nothing,
}

static ENGINE: OnceLock<Box<dyn NyquistEngine>> = OnceLock::new();

/// Installs the worker's Nyquist engine. Subsequent calls are ignored.
pub fn set_engine(engine: Box<dyn NyquistEngine>) {
    let _ = ENGINE.set(engine);
}

/// Minimal metadata parsed from the comment header of a Nyquist script.
#[derive(Clone, Debug, PartialEq)]
pub struct NyquistHeader {
    /// Display name of the plug-in.
    pub name: String,
    /// Plug-in type, such as `process` or `generate`.
    pub kind: String,
    /// Plug-in author.
    pub author: String,
    /// Plug-in version number, or zero if absent or invalid.
    pub version: u32,
}

/// Reads the minimal name, type, author, and version header from a Nyquist script.
pub fn read_header(text: &str) -> Option<NyquistHeader> {
    let mut name = None;
    let mut kind = None;
    let mut author = String::new();
    let mut version = 0;

    for line in text.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix(';').or_else(|| line.strip_prefix('$')) else {
            continue;
        };
        let rest = rest.trim_start();
        let split_at = rest
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let key = rest[..split_at].trim().to_ascii_lowercase();
        let value = rest[split_at..].trim_start().trim_start_matches('=').trim();
        let value = clean_header_value(value);

        match key.as_str() {
            "name" if !value.is_empty() => name = Some(value),
            "type" if !value.is_empty() => kind = Some(value.to_ascii_lowercase()),
            "author" => author = value,
            "version" => version = value.parse().unwrap_or(0),
            _ => {}
        }
    }

    name.map(|name| NyquistHeader {
        name,
        kind: kind.unwrap_or_default(),
        author,
        version,
    })
}

fn clean_header_value(value: &str) -> String {
    let mut value = value.trim();

    if let Some(inner) = value.strip_prefix("_(") {
        value = inner.trim();
        if let Some(inner) = value.strip_suffix(')') {
            value = inner.trim();
        }
    } else if let Some(inner) = value.strip_prefix("(_") {
        value = inner.trim();
        if let Some(inner) = value.strip_suffix(')') {
            value = inner.trim();
        }
    }

    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        let last = value.as_bytes()[value.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            value = &value[1..value.len() - 1];
        }
    }

    value.trim().to_owned()
}

/// Host configuration for scanning and running Nyquist plug-ins.
pub struct NyquistHost {
    timeout: Duration,
}

impl NyquistHost {
    /// Creates a host with a 30-second worker timeout.
    pub fn new() -> Self {
        Self {
            timeout: Duration::from_secs(30),
        }
    }

    /// Sets the maximum time allowed for a worker invocation.
    pub fn with_timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }
}

impl Default for NyquistHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost for NyquistHost {
    fn format(&self) -> PluginFormat {
        PluginFormat::Nyquist
    }

    fn default_search_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            let home = PathBuf::from(home);
            paths.push(home.join(".audacity-files/plug-ins"));
            paths.push(home.join(".local/share/audacity/plug-ins"));
        }
        paths.push(PathBuf::from("/usr/share/audacity/plug-ins"));
        paths.push(PathBuf::from("/usr/local/share/audacity/plug-ins"));
        if let Some(extra) = std::env::var_os("AUDACITY_PATH") {
            paths.extend(std::env::split_paths(&extra));
        }
        paths
    }

    fn scan(&self, paths: &[PathBuf]) -> Vec<PluginDescriptor> {
        let mut descriptors = Vec::new();

        for dir in paths {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file()
                    || !path
                        .extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("ny"))
                {
                    continue;
                }

                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                let Some(header) = read_header(&text) else {
                    continue;
                };

                let kind = header.kind.split([',', ';', ' ', '\t']).next().unwrap_or("");
                let n_inputs = if kind == "generate" { 0 } else { 2 };
                let id = crate::state::effect_id(
                    PluginFormat::Nyquist,
                    &path,
                    "",
                    &header.author,
                    &header.name,
                );

                descriptors.push(PluginDescriptor {
                    id,
                    name: header.name,
                    vendor: header.author,
                    format: PluginFormat::Nyquist,
                    path: Some(path),
                    params: Vec::new(),
                    n_inputs,
                    n_outputs: 2,
                });
            }
        }

        descriptors
    }

    fn instantiate(
        &self,
        descriptor: &PluginDescriptor,
    ) -> Result<Box<dyn PluginInstance>, PluginError> {
        let path = descriptor
            .path
            .clone()
            .ok_or_else(|| PluginError::LoadFailed("Nyquist plug-in has no path".into()))?;
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ny"))
        {
            return Err(PluginError::LoadFailed(
                "Nyquist plug-in path is not a .ny file".into(),
            ));
        }

        Ok(Box::new(NyquistInstance {
            descriptor: descriptor.clone(),
            params: Vec::new(),
            timeout: self.timeout,
            last_output: None,
            last_error: None,
        }))
    }
}

/// Finds the worker executable from `AU_NYQUIST_WORKER` or beside the current executable.
pub fn worker_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AU_NYQUIST_WORKER") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }

    let current_exe = std::env::current_exe().ok()?;
    let parent = current_exe.parent()?;
    let worker_name = if cfg!(windows) {
        "au-nyquist-worker.exe"
    } else {
        "au-nyquist-worker"
    };

    let direct = parent.join(worker_name);
    if direct.is_file() {
        return Some(direct);
    }

    if parent.file_name().is_some_and(|name| name == "deps") {
        let profile_worker = parent.parent()?.join(worker_name);
        if profile_worker.is_file() {
            return Some(profile_worker);
        }
    }

    None
}

/// Errors produced while executing a Nyquist job in the worker process.
#[derive(Clone, Debug, PartialEq)]
pub enum SandboxError {
    /// No worker executable could be found.
    WorkerMissing,
    /// The worker exited without a complete reply.
    Crashed(String),
    /// The worker exceeded its time limit.
    TimedOut,
    /// The job was cancelled.
    Cancelled,
    /// The interpreter returned an error.
    Failed(String),
    /// The worker sent an invalid or incomplete protocol message.
    Protocol(String),
}

/// Runs a job in a separate worker process.
pub fn run_sandboxed(
    job: &NyquistJob,
    timeout: Duration,
    cancel: &CancelToken,
) -> Result<NyquistOutput, SandboxError> {
    let executable = worker_path().ok_or(SandboxError::WorkerMissing)?;
    let mut child = Command::new(executable)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                SandboxError::WorkerMissing
            } else {
                SandboxError::Crashed(format!("could not start worker: {error}"))
            }
        })?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SandboxError::Protocol("worker stdout was not piped".into()))?;
    let reader = thread::spawn(move || read_response(stdout));

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| SandboxError::Protocol("worker stdin was not piped".into()))?;
    let job = job.clone();
    let writer = thread::spawn(move || write_job(stdin, &job));

    let started = Instant::now();
    let status = loop {
        if cancel.is_cancelled() {
            terminate_worker(&mut child);
            let _ = writer.join();
            let _ = reader.join();
            return Err(SandboxError::Cancelled);
        }
        if started.elapsed() >= timeout {
            terminate_worker(&mut child);
            let _ = writer.join();
            let _ = reader.join();
            return Err(SandboxError::TimedOut);
        }

        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(POLL_INTERVAL),
            Err(error) => {
                terminate_worker(&mut child);
                let _ = writer.join();
                let _ = reader.join();
                return Err(SandboxError::Crashed(format!(
                    "could not wait for worker: {error}"
                )));
            }
        }
    };

    let _ = writer.join();
    match reader.join() {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(_error)) if !status.success() => {
            Err(SandboxError::Crashed(describe_exit_status(status)))
        }
        Ok(Err(error)) => Err(error),
        Err(_) if !status.success() => Err(SandboxError::Crashed(describe_exit_status(status))),
        Err(_) => Err(SandboxError::Protocol(
            "worker response reader panicked".into(),
        )),
    }
}

fn terminate_worker(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn describe_exit_status(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("signal {signal}");
        }
    }

    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "terminated without an exit code".into(),
    }
}

fn write_job(mut writer: impl Write, job: &NyquistJob) -> io::Result<()> {
    writer.write_all(MAGIC)?;
    write_string(&mut writer, &job.script_path.to_string_lossy())?;
    write_u32(&mut writer, checked_u32(job.params.len())?)?;
    for (key, value) in &job.params {
        write_string(&mut writer, key)?;
        write_string(&mut writer, value)?;
    }
    writer.write_all(&job.selection_secs.0.to_le_bytes())?;
    writer.write_all(&job.selection_secs.1.to_le_bytes())?;
    match &job.audio {
        Some(audio) => {
            writer.write_all(&[1])?;
            write_audio(&mut writer, audio)?;
        }
        None => writer.write_all(&[0])?,
    }
    writer.flush()
}

fn read_job(mut reader: impl Read) -> io::Result<NyquistJob> {
    read_magic(&mut reader)?;
    let script_path = PathBuf::from(read_string(&mut reader)?);
    let param_count = read_count(&mut reader, MAX_ITEMS)?;
    let mut params = Vec::with_capacity(param_count);
    for _ in 0..param_count {
        params.push((read_string(&mut reader)?, read_string(&mut reader)?));
    }
    let selection_secs = (read_f64(&mut reader)?, read_f64(&mut reader)?);
    let mut has_audio = [0u8; 1];
    reader.read_exact(&mut has_audio)?;
    let audio = match has_audio[0] {
        0 => None,
        1 => Some(read_audio(&mut reader)?),
        _ => return Err(invalid_data("invalid audio-present flag")),
    };

    Ok(NyquistJob {
        script_path,
        params,
        audio,
        selection_secs,
    })
}

fn write_response(mut writer: impl Write, output: Result<NyquistOutput, String>) -> io::Result<()> {
    writer.write_all(MAGIC)?;
    match output {
        Ok(NyquistOutput::Nothing) => writer.write_all(&[0])?,
        Ok(NyquistOutput::Audio(audio)) => {
            writer.write_all(&[1])?;
            write_audio(&mut writer, &audio)?;
        }
        Ok(NyquistOutput::Labels(labels)) => {
            writer.write_all(&[2])?;
            write_u32(&mut writer, checked_u32(labels.len())?)?;
            for (start, end, text) in labels {
                writer.write_all(&start.to_le_bytes())?;
                writer.write_all(&end.to_le_bytes())?;
                write_string(&mut writer, &text)?;
            }
        }
        Ok(NyquistOutput::Text(text)) => {
            writer.write_all(&[3])?;
            write_string(&mut writer, &text)?;
        }
        Err(error) => {
            writer.write_all(&[255])?;
            write_string(&mut writer, &error)?;
        }
    }
    writer.flush()
}

fn read_response(mut reader: impl Read) -> Result<NyquistOutput, SandboxError> {
    read_magic(&mut reader).map_err(protocol_error)?;
    let mut tag = [0u8; 1];
    reader.read_exact(&mut tag).map_err(protocol_error)?;
    match tag[0] {
        0 => Ok(NyquistOutput::Nothing),
        1 => read_audio(&mut reader)
            .map(NyquistOutput::Audio)
            .map_err(protocol_error),
        2 => {
            let count = read_count(&mut reader, MAX_ITEMS).map_err(protocol_error)?;
            let mut labels = Vec::with_capacity(count);
            for _ in 0..count {
                let start = read_f64(&mut reader).map_err(protocol_error)?;
                let end = read_f64(&mut reader).map_err(protocol_error)?;
                let text = read_string(&mut reader).map_err(protocol_error)?;
                labels.push((start, end, text));
            }
            Ok(NyquistOutput::Labels(labels))
        }
        3 => read_string(&mut reader)
            .map(NyquistOutput::Text)
            .map_err(protocol_error),
        255 => {
            let message = read_string(&mut reader).map_err(protocol_error)?;
            Err(SandboxError::Failed(message))
        }
        other => Err(SandboxError::Protocol(format!(
            "unknown worker response tag {other}"
        ))),
    }
}

fn protocol_error(error: io::Error) -> SandboxError {
    SandboxError::Protocol(error.to_string())
}

fn write_audio(writer: &mut impl Write, audio: &au_core::AudioBuffer) -> io::Result<()> {
    let channels = checked_u32(audio.channels.len())?;
    let frames = audio.channels.first().map_or(0, Vec::len);
    if audio.channels.iter().any(|channel| channel.len() != frames) {
        return Err(invalid_input("audio channels have different frame counts"));
    }

    write_u32(writer, audio.sample_rate)?;
    write_u32(writer, channels)?;
    writer.write_all(
        &u64::try_from(frames)
            .map_err(|_| invalid_input("audio frame count is too large"))?
            .to_le_bytes(),
    )?;
    for channel in &audio.channels {
        for sample in channel {
            writer.write_all(&sample.to_le_bytes())?;
        }
    }
    Ok(())
}

fn read_audio(reader: &mut impl Read) -> io::Result<au_core::AudioBuffer> {
    let sample_rate = read_u32(reader)?;
    let channels = read_count(reader, 256)?;
    let frames_u64 = read_u64(reader)?;
    let frames = usize::try_from(frames_u64)
        .map_err(|_| invalid_data("audio frame count does not fit this platform"))?;
    let sample_count = channels
        .checked_mul(frames)
        .ok_or_else(|| invalid_data("audio sample count overflow"))?;
    if sample_count > MAX_AUDIO_SAMPLES {
        return Err(invalid_data("audio response exceeds the sample limit"));
    }

    let mut audio_channels = Vec::with_capacity(channels);
    for _ in 0..channels {
        let mut channel = Vec::with_capacity(frames);
        for _ in 0..frames {
            let mut bytes = [0u8; 4];
            reader.read_exact(&mut bytes)?;
            channel.push(f32::from_le_bytes(bytes));
        }
        audio_channels.push(channel);
    }

    Ok(au_core::AudioBuffer {
        sample_rate,
        channels: audio_channels,
    })
}

fn read_magic(reader: &mut impl Read) -> io::Result<()> {
    let mut magic = [0u8; 5];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(invalid_data("invalid Nyquist worker protocol magic"));
    }
    Ok(())
}

fn write_string(writer: &mut impl Write, value: &str) -> io::Result<()> {
    let bytes = value.as_bytes();
    if bytes.len() > MAX_STRING_LEN {
        return Err(invalid_input("protocol string is too large"));
    }
    write_u32(writer, checked_u32(bytes.len())?)?;
    writer.write_all(bytes)
}

fn read_string(reader: &mut impl Read) -> io::Result<String> {
    let len = read_count(reader, MAX_STRING_LEN)?;
    let mut bytes = vec![0u8; len];
    reader.read_exact(&mut bytes)?;
    String::from_utf8(bytes).map_err(|error| invalid_data(error.to_string()))
}

fn write_u32(writer: &mut impl Write, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn read_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_f64(reader: &mut impl Read) -> io::Result<f64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(f64::from_le_bytes(bytes))
}

fn read_count(reader: &mut impl Read, limit: usize) -> io::Result<usize> {
    let count = usize::try_from(read_u32(reader)?)
        .map_err(|_| invalid_data("protocol count does not fit this platform"))?;
    if count > limit {
        return Err(invalid_data("protocol count exceeds the limit"));
    }
    Ok(count)
}

fn checked_u32(value: usize) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| invalid_input("protocol value is too large"))
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Entry point used by the `au-nyquist-worker` executable.
pub fn worker_main() -> i32 {
    let job = match read_job(io::stdin().lock()) {
        Ok(job) => job,
        Err(error) => {
            let _ = write_response(
                io::stdout().lock(),
                Err(format!("invalid Nyquist worker request: {error}")),
            );
            return 0;
        }
    };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_worker_job(&job)
    }))
    .unwrap_or_else(|_| Err("Nyquist interpreter panicked".into()));

    match write_response(io::stdout().lock(), result) {
        Ok(()) => 0,
        Err(_) => 2,
    }
}

fn run_worker_job(job: &NyquistJob) -> Result<NyquistOutput, String> {
    if let Some((_, hook)) = job.params.iter().find(|(key, _)| key == "__au_test") {
        match hook.as_str() {
            "crash" => {
                // Test-only hook: deliberately fault the worker process.
                unsafe {
                    std::ptr::write_volatile(16usize as *mut i32, 1);
                }
                return Err("crash test hook unexpectedly returned".into());
            }
            "hang" => loop {
                thread::sleep(Duration::from_millis(50));
            },
            "panic" => panic!("Nyquist worker panic test hook"),
            "echo" => {
                if let Some(audio) = &job.audio {
                    let gain = job
                        .params
                        .iter()
                        .find(|(key, _)| key == "__au_gain")
                        .and_then(|(_, value)| value.parse::<f32>().ok())
                        .unwrap_or(1.0);
                    let mut output = audio.clone();
                    for channel in &mut output.channels {
                        for sample in channel {
                            *sample *= gain;
                        }
                    }
                    return Ok(NyquistOutput::Audio(output));
                }
                return Ok(NyquistOutput::Text("generated".into()));
            }
            "labels" => {
                return Ok(NyquistOutput::Labels(vec![(
                    0.0,
                    1.0,
                    "test".into(),
                )]));
            }
            _ => {}
        }
    }

    match ENGINE.get() {
        Some(engine) => engine.run(job, &|| false),
        None => Err("Nyquist interpreter is not available in this build".into()),
    }
}

/// Offline-only Nyquist plug-in instance.
pub struct NyquistInstance {
    descriptor: PluginDescriptor,
    params: Vec<(String, String)>,
    timeout: Duration,
    last_output: Option<NyquistOutput>,
    last_error: Option<String>,
}

impl NyquistInstance {
    /// Returns the most recent non-audio or audio worker output.
    pub fn last_output(&self) -> Option<&NyquistOutput> {
        self.last_output.as_ref()
    }

    /// Returns the most recent sandbox error.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

impl PluginInstance for NyquistInstance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    /// Nyquist controls are string-valued and set through `load_state`.
    fn set_param(&mut self, _id: u32, _v: f32) -> Result<(), PluginError> {
        Ok(())
    }

    fn get_param(&self, _id: u32) -> Option<f32> {
        None
    }

    fn activate(&mut self, _sample_rate: u32, _max_block: usize) {}

    /// Nyquist is offline-only; realtime blocks are copied through unchanged.
    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [Vec<f32>]) {
        let frame_count = inputs.first().map_or(0, |input| input.len());
        for (index, output) in outputs.iter_mut().enumerate() {
            output.clear();
            if let Some(input) = inputs.get(index) {
                output.extend_from_slice(input);
            } else {
                output.resize(frame_count, 0.0);
            }
        }
    }

    fn process_buffer(&mut self, buffer: &mut au_core::AudioBuffer) {
        let frames = buffer.channels.first().map_or(0, Vec::len);
        let duration = if buffer.sample_rate == 0 {
            0.0
        } else {
            frames as f64 / f64::from(buffer.sample_rate)
        };
        let job = NyquistJob {
            script_path: self.descriptor.path.clone().unwrap_or_default(),
            params: self.params.clone(),
            audio: Some(buffer.clone()),
            selection_secs: (0.0, duration),
        };
        let cancel = CancelToken::new();

        match run_sandboxed(&job, self.timeout, &cancel) {
            Ok(output) => {
                self.last_error = None;
                if let NyquistOutput::Audio(audio) = &output {
                    *buffer = audio.clone();
                }
                self.last_output = Some(output);
            }
            Err(error) => {
                let message = format!("{error:?}");
                au_runtime::crash::log_action(format!(
                    "Nyquist plug-in {} failed: {message}",
                    self.descriptor.name
                ));
                self.last_error = Some(message);
            }
        }
    }

    fn save_state(&mut self) -> Result<Vec<(String, String)>, PluginError> {
        Ok(self.params.clone())
    }

    fn load_state(&mut self, state: &[(String, String)]) -> Result<(), PluginError> {
        self.params = state.to_vec();
        Ok(())
    }
}

