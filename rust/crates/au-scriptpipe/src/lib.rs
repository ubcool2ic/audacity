//! Transport for Audacity's mod-script-pipe protocol.
//!
//! This crate moves command lines between pipe clients and an application-provided
//! command interpreter. It does not interpret commands.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// The success line appended to successful command responses.
pub const RESPONSE_OK: &str = "BatchCommand finished: OK";

/// The failure line appended to failed command responses.
pub const RESPONSE_FAILED: &str = "BatchCommand finished: Failed!";

/// One command received from a client.
///
/// A request must be answered exactly once. Dropping it without calling
/// [`Request::respond`] sends a failure response to the waiting client.
pub struct Request {
    command: String,
    reply: mpsc::Sender<String>,
    answered: bool,
}

impl Request {
    /// Returns the command line received from the client.
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Sends response text for this request.
    ///
    /// The transport removes carriage returns and adds the protocol's newline
    /// terminator when it writes the response to the client.
    pub fn respond(mut self, text: &str) {
        self.answered = true;
        let _ = self.reply.send(text.to_owned());
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        if !self.answered {
            let response = format!("{} was not answered\n{}", self.command, RESPONSE_FAILED);
            let _ = self.reply.send(response);
            self.answered = true;
        }
    }
}

/// Builds a response body containing `output` followed by the finished line.
///
/// A newline is inserted between non-empty output and the finished line when
/// the output does not already end with one.
pub fn finished(output: &str, ok: bool) -> String {
    let mut response = String::with_capacity(output.len() + RESPONSE_OK.len() + 1);
    response.push_str(output);

    if !output.is_empty() && !output.ends_with('\n') {
        response.push('\n');
    }

    response.push_str(if ok { RESPONSE_OK } else { RESPONSE_FAILED });
    response
}

/// The paths or names used for the client-to-server and server-to-client pipes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PipeNames {
    /// The client-to-server pipe.
    pub to_server: PathBuf,
    /// The server-to-client pipe.
    pub from_server: PathBuf,
}

impl PipeNames {
    /// Returns Audacity's standard pipe names for the current platform.
    pub fn audacity_default() -> Self {
        #[cfg(unix)]
        {
            let uid = unsafe { libc::getuid() };
            Self {
                to_server: PathBuf::from(format!("/tmp/audacity_script_pipe.to.{uid}")),
                from_server: PathBuf::from(format!("/tmp/audacity_script_pipe.from.{uid}")),
            }
        }

        #[cfg(windows)]
        {
            Self {
                to_server: PathBuf::from(r"\\.\pipe\ToSrvPipe"),
                from_server: PathBuf::from(r"\\.\pipe\FromSrvPipe"),
            }
        }

        #[cfg(not(any(unix, windows)))]
        {
            Self {
                to_server: PathBuf::from("audacity_script_pipe.to"),
                from_server: PathBuf::from("audacity_script_pipe.from"),
            }
        }
    }

    /// Creates test or tool pipe names inside `dir`.
    ///
    /// On Unix, the names are `<tag>.to` and `<tag>.from`.
    pub fn in_dir(dir: &Path, tag: &str) -> Self {
        Self {
            to_server: dir.join(format!("{tag}.to")),
            from_server: dir.join(format!("{tag}.from")),
        }
    }
}

/// A running mod-script-pipe server.
pub struct PipeServer {
    names: PipeNames,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl PipeServer {
    /// Starts the server thread named `au-script-pipe`.
    ///
    /// Requests are sent to the returned receiver for the application to drain
    /// and answer. `wake`, when supplied, is called after each request is queued.
    pub fn start(
        names: PipeNames,
        wake: Option<Box<dyn Fn() + Send + 'static>>,
    ) -> io::Result<(PipeServer, mpsc::Receiver<Request>)> {
        #[cfg(unix)]
        {
            prepare_unix_fifos(&names)?;
            ignore_sigpipe();
        }

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_names = names.clone();
        let (request_tx, request_rx) = mpsc::channel();

        let thread = match thread::Builder::new()
            .name("au-script-pipe".to_owned())
            .spawn(move || {
                au_runtime::crash::log_action("mod-script-pipe server started".to_owned());

                let result = au_runtime::pool::isolate(
                    || -> Result<(), au_runtime::JobError> {
                        #[cfg(unix)]
                        run_unix_server(
                            &thread_names,
                            &thread_stop,
                            &request_tx,
                            wake,
                        );

                        #[cfg(windows)]
                        run_windows_server(
                            &thread_names,
                            &thread_stop,
                            &request_tx,
                            wake,
                        );

                        #[cfg(not(any(unix, windows)))]
                        {
                            let _ = (&thread_names, &thread_stop, &request_tx, wake);
                        }

                        Ok(())
                    },
                );

                if let Err(error) = result {
                    au_runtime::crash::log_action(format!(
                        "mod-script-pipe server stopped after an error: {error:?}"
                    ));
                } else {
                    au_runtime::crash::log_action(
                        "mod-script-pipe server stopped".to_owned(),
                    );
                }
            }) {
            Ok(thread) => thread,
            Err(error) => {
                #[cfg(unix)]
                cleanup_unix_fifos(&names);
                return Err(error);
            }
        };

        Ok((
            PipeServer {
                names,
                stop,
                thread: Some(thread),
            },
            request_rx,
        ))
    }

    /// Returns the names used by this server.
    pub fn names(&self) -> &PipeNames {
        &self.names
    }

    /// Stops the server, joins its thread, and removes its Unix FIFOs.
    pub fn stop(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.stop.store(true, Ordering::SeqCst);

        #[cfg(windows)]
        wake_windows_server(&self.names);

        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }

        #[cfg(unix)]
        cleanup_unix_fifos(&self.names);
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

/// Starts a server that handles requests on its own thread.
///
/// The handler's returned string is sent as the response body for each command.
pub fn serve_with(
    names: PipeNames,
    mut handler: impl FnMut(&str) -> String + Send + 'static,
) -> io::Result<PipeServer> {
    let (server, requests) = PipeServer::start(names, None)?;

    if let Err(error) = thread::Builder::new()
        .name("au-script-pipe-handler".to_owned())
        .spawn(move || {
            while let Ok(request) = requests.recv() {
                let response = handler(request.command());
                request.respond(&response);
            }
        })
    {
        server.stop();
        return Err(error);
    }

    Ok(server)
}

fn dispatch_request(
    command: String,
    requests: &mpsc::Sender<Request>,
    wake: &Option<Box<dyn Fn() + Send + 'static>>,
) -> Option<String> {
    let (reply_tx, reply_rx) = mpsc::channel();
    let request = Request {
        command: command.clone(),
        reply: reply_tx,
        answered: false,
    };

    if requests.send(request).is_err() {
        return None;
    }

    if let Some(wake) = wake {
        wake();
    }

    Some(reply_rx.recv().unwrap_or_else(|_| {
        format!("{command} was not answered\n{RESPONSE_FAILED}")
    }))
}

fn normalize_command(line: &[u8]) -> String {
    String::from_utf8_lossy(line)
        .trim_end_matches(|character| character == '\r' || character == '\n' || character == '\0')
        .to_owned()
}

fn frame_response(text: &str) -> Vec<u8> {
    let mut response = text.replace('\r', "");
    if !response.ends_with('\n') {
        response.push('\n');
    }
    response.push('\n');
    response.into_bytes()
}

#[cfg(unix)]
use std::ffi::CString;
#[cfg(unix)]
use std::fs::{self, File};
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;

#[cfg(unix)]
fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "pipe path contains NUL"))
}

#[cfg(unix)]
fn unlink_path(path: &Path) -> io::Result<()> {
    let path = c_path(path)?;
    let result = unsafe { libc::unlink(path.as_ptr()) };
    if result == 0 {
        Ok(())
    } else {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::NotFound {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(unix)]
fn prepare_unix_fifos(names: &PipeNames) -> io::Result<()> {
    unlink_path(&names.to_server)?;
    unlink_path(&names.from_server)?;

    let to_path = c_path(&names.to_server)?;
    let from_path = c_path(&names.from_server)?;

    if unsafe { libc::mkfifo(to_path.as_ptr(), 0o700) } != 0 {
        return Err(io::Error::last_os_error());
    }

    if unsafe { libc::mkfifo(from_path.as_ptr(), 0o700) } != 0 {
        let error = io::Error::last_os_error();
        let _ = unlink_path(&names.to_server);
        return Err(error);
    }

    Ok(())
}

#[cfg(unix)]
fn cleanup_unix_fifos(names: &PipeNames) {
    let _ = fs::remove_file(&names.to_server);
    let _ = fs::remove_file(&names.from_server);
}

#[cfg(unix)]
fn ignore_sigpipe() {
    use std::sync::Once;

    static IGNORE_SIGPIPE: Once = Once::new();
    IGNORE_SIGPIPE.call_once(|| unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    });
}

#[cfg(unix)]
fn open_unix_input(path: &Path) -> io::Result<File> {
    let path = c_path(path)?;
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

#[cfg(unix)]
fn open_unix_output(names: &PipeNames, stop: &AtomicBool, retry: bool) -> io::Result<Option<File>> {
    let path = c_path(&names.from_server)?;

    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(None);
        }

        let fd = unsafe { libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_NONBLOCK) };
        if fd >= 0 {
            let output = unsafe { File::from_raw_fd(fd) };
            let flags = unsafe { libc::fcntl(output.as_raw_fd(), libc::F_GETFL) };
            if flags < 0 {
                return Err(io::Error::last_os_error());
            }
            if unsafe {
                libc::fcntl(
                    output.as_raw_fd(),
                    libc::F_SETFL,
                    flags & !libc::O_NONBLOCK,
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
            return Ok(Some(output));
        }

        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ENXIO) {
            return Err(error);
        }
        if !retry {
            return Ok(None);
        }

        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn send_unix_command(
    command: String,
    names: &PipeNames,
    output: &mut Option<File>,
    stop: &AtomicBool,
    requests: &mpsc::Sender<Request>,
    wake: &Option<Box<dyn Fn() + Send + 'static>>,
) -> io::Result<bool> {
    if command.is_empty() {
        return Ok(true);
    }

    if output.is_none() {
        *output = open_unix_output(names, stop, true)?;
    }
    let Some(output) = output.as_mut() else {
        return Ok(false);
    };

    let Some(response) = dispatch_request(command, requests, wake) else {
        return Ok(false);
    };

    // A client may close while its response is being written. Treat any
    // write error as a disconnected client and continue accepting clients.
    if output.write_all(&frame_response(&response)).is_err() {
        return Ok(false);
    }

    Ok(true)
}

#[cfg(unix)]
fn run_unix_server(
    names: &PipeNames,
    stop: &AtomicBool,
    requests: &mpsc::Sender<Request>,
    wake: Option<Box<dyn Fn() + Send + 'static>>,
) {
    while !stop.load(Ordering::SeqCst) {
        let to_server = match open_unix_input(&names.to_server) {
            Ok(file) => file,
            Err(error) => {
                if !stop.load(Ordering::SeqCst) {
                    au_runtime::crash::log_action(format!(
                        "mod-script-pipe could not open input FIFO: {error}"
                    ));
                    thread::sleep(Duration::from_millis(50));
                }
                continue;
            }
        };

        if stop.load(Ordering::SeqCst) {
            drop(to_server);
            break;
        }

        if let Err(error) = run_unix_session(&to_server, names, stop, requests, &wake) {
            if !stop.load(Ordering::SeqCst) {
                au_runtime::crash::log_action(format!(
                    "mod-script-pipe client session ended: {error}"
                ));
            }
        }

        drop(to_server);
    }
}

#[cfg(unix)]
fn run_unix_session(
    input: &File,
    names: &PipeNames,
    stop: &AtomicBool,
    requests: &mpsc::Sender<Request>,
    wake: &Option<Box<dyn Fn() + Send + 'static>>,
) -> io::Result<()> {
    let mut pending = Vec::new();
    let mut output = None;
    let mut client_connected = false;
    let mut chunk = [0u8; 4096];

    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        // Clients (pipe_test.py, PipeClient) open the from-FIFO for reading
        // before sending anything, so connect our writer as soon as a reader waits.
        if output.is_none() {
            output = open_unix_output(names, stop, false)?;
        }

        let mut poll_fd = libc::pollfd {
            fd: input.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        };
        let poll_result = unsafe { libc::poll(&mut poll_fd, 1, 100) };

        if poll_result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }

        if poll_result == 0 {
            continue;
        }

        let events = poll_fd.revents;
        if events & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "poll reported an error on the input FIFO",
            ));
        }

        let readable = events & libc::POLLIN != 0;
        let hung_up = events & libc::POLLHUP != 0;

        // On Unix, a read-only FIFO commonly reports POLLHUP before its first
        // writer connects. Ignore that idle state without spinning.
        if hung_up && !readable && !client_connected {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        if !readable && !hung_up {
            continue;
        }

        let count = match (&*input).read(&mut chunk) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
            Err(error) => return Err(error),
        };

        if count == 0 {
            if client_connected {
                return Ok(());
            }

            if hung_up {
                thread::sleep(Duration::from_millis(100));
            }
            continue;
        }

        client_connected = true;
        pending.extend_from_slice(&chunk[..count]);

        while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=newline).collect();
            let command = normalize_command(&line);

            if !send_unix_command(command, names, &mut output, stop, requests, wake)? {
                return Ok(());
            }
        }
    }
}

#[cfg(windows)]
fn run_windows_server(
    names: &PipeNames,
    stop: &AtomicBool,
    requests: &mpsc::Sender<Request>,
    wake: Option<Box<dyn Fn() + Send + 'static>>,
) {
    windows_transport::run(names, stop, requests, wake);
}

#[cfg(windows)]
fn wake_windows_server(names: &PipeNames) {
    windows_transport::wake(names);
}

#[cfg(unix)]
mod client {
    use super::PipeNames;
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    /// A minimal Unix client for the mod-script-pipe protocol.
    pub struct PipeClient {
        writer: File,
        reader: File,
        timeout: Duration,
        buffered: Vec<u8>,
    }

    impl PipeClient {
        /// Connects to the server, waiting up to `timeout` for the input FIFO.
        pub fn connect(names: &PipeNames, timeout: Duration) -> io::Result<Self> {
            let deadline = Instant::now() + timeout;

            let writer = loop {
                match OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&names.to_server)
                {
                    Ok(file) => break file,
                    Err(error)
                        if error.raw_os_error() == Some(libc::ENXIO)
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "timed out connecting to input FIFO",
                        ));
                    }
                    Err(error) => return Err(error),
                }
            };

            let flags = unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_GETFL) };
            if flags >= 0 {
                unsafe {
                    libc::fcntl(
                        writer.as_raw_fd(),
                        libc::F_SETFL,
                        flags & !libc::O_NONBLOCK,
                    );
                }
            }

            let reader = OpenOptions::new().read(true).open(&names.from_server)?;
            let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
            if flags >= 0 {
                unsafe {
                    libc::fcntl(
                        reader.as_raw_fd(),
                        libc::F_SETFL,
                        flags | libc::O_NONBLOCK,
                    );
                }
            }

            Ok(Self {
                writer,
                reader,
                timeout,
                buffered: Vec::new(),
            })
        }

        /// Sends one command and reads its response, excluding the empty-line
        /// protocol terminator.
        pub fn command(&mut self, cmd: &str) -> io::Result<String> {
            self.writer.write_all(cmd.as_bytes())?;
            self.writer.write_all(b"\n")?;
            self.writer.flush()?;

            let deadline = Instant::now() + self.timeout;
            let mut response = Vec::new();

            loop {
                if let Some(newline) = self.buffered.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = self.buffered.drain(..=newline).collect();
                    if line == b"\n" || line == b"\r\n" {
                        if response.last() == Some(&b'\n') {
                            response.pop();
                        }
                        return Ok(String::from_utf8_lossy(&response).into_owned());
                    }
                    response.extend_from_slice(&line);
                    continue;
                }

                let now = Instant::now();
                if now >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "timed out waiting for pipe response",
                    ));
                }

                let remaining = deadline.saturating_duration_since(now);
                let timeout_ms = remaining.as_millis().clamp(1, i32::MAX as u128) as i32;
                let mut poll_fd = libc::pollfd {
                    fd: self.reader.as_raw_fd(),
                    events: libc::POLLIN | libc::POLLHUP,
                    revents: 0,
                };
                let poll_result = unsafe { libc::poll(&mut poll_fd, 1, timeout_ms) };

                if poll_result < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if poll_result == 0 {
                    continue;
                }

                let mut chunk = [0u8; 4096];
                match self.reader.read(&mut chunk) {
                    Ok(0) => {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "server closed the output FIFO",
                        ));
                    }
                    Ok(count) => self.buffered.extend_from_slice(&chunk[..count]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
        }
    }

    use std::os::unix::fs::OpenOptionsExt;
}

#[cfg(unix)]
pub use client::PipeClient;

#[cfg(windows)]
mod windows_transport {
    use super::{dispatch_request, frame_response, normalize_command, PipeNames};
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_MORE_DATA, ERROR_PIPE_CONNECTED, GENERIC_READ,
        GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, OPEN_EXISTING};
    use windows_sys::Win32::System::IO::{ReadFile, WriteFile};
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_ACCESS_DUPLEX,
        PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    fn wide(path: &Path) -> Vec<u16> {
        OsStr::new(path).encode_wide().chain(Some(0)).collect()
    }

    fn create_pipe(path: &Path) -> io::Result<HANDLE> {
        let name = wide(path);
        let handle = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                1024,
                1024,
                0,
                std::ptr::null(),
            )
        };

        if handle == INVALID_HANDLE_VALUE {
            Err(io::Error::last_os_error())
        } else {
            Ok(handle)
        }
    }

    fn connect_pipe(handle: HANDLE) -> io::Result<()> {
        let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
        if connected != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn disconnect_and_close(handle: HANDLE) {
        unsafe {
            DisconnectNamedPipe(handle);
            CloseHandle(handle);
        }
    }

    fn send_command(
        command: String,
        output: HANDLE,
        requests: &mpsc::Sender<super::Request>,
        wake: &Option<Box<dyn Fn() + Send + 'static>>,
    ) -> bool {
        let Some(response) = dispatch_request(command, requests, wake) else {
            return false;
        };
        let response = frame_response(&response);
        let mut written = 0u32;
        (unsafe {
            WriteFile(
                output,
                response.as_ptr(),
                response.len() as u32,
                &mut written,
                std::ptr::null_mut(),
            )
        }) != 0
    }

    fn serve_message(
        message: &[u8],
        output: HANDLE,
        requests: &mpsc::Sender<super::Request>,
        wake: &Option<Box<dyn Fn() + Send + 'static>>,
    ) -> bool {
        let mut pending = Vec::new();
        pending.extend_from_slice(message);

        while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = pending.drain(..=newline).collect();
            let command = normalize_command(&line);
            if !command.is_empty()
                && !send_command(command, output, requests, wake)
            {
                return false;
            }
        }

        if !pending.is_empty() {
            let command = normalize_command(&pending);
            if !command.is_empty() && !send_command(command, output, requests, wake) {
                return false;
            }
        }

        true
    }

    fn serve_session(
        input: HANDLE,
        output: HANDLE,
        stop: &AtomicBool,
        requests: &mpsc::Sender<super::Request>,
        wake: &Option<Box<dyn Fn() + Send + 'static>>,
    ) -> io::Result<()> {
        let mut message = Vec::new();
        let mut chunk = [0u8; 1024];

        while !stop.load(Ordering::SeqCst) {
            let mut read = 0u32;
            let succeeded = unsafe {
                ReadFile(
                    input,
                    chunk.as_mut_ptr(),
                    chunk.len() as u32,
                    &mut read,
                    std::ptr::null_mut(),
                )
            };

            if read != 0 {
                message.extend_from_slice(&chunk[..read as usize]);
            }

            if succeeded != 0 {
                if !serve_message(&message, output, requests, wake) {
                    return Ok(());
                }
                message.clear();
                continue;
            }

            let error = unsafe { GetLastError() };
            if error == ERROR_MORE_DATA {
                continue;
            }
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    pub(super) fn run(
        names: &PipeNames,
        stop: &AtomicBool,
        requests: &mpsc::Sender<super::Request>,
        wake: Option<Box<dyn Fn() + Send + 'static>>,
    ) {
        while !stop.load(Ordering::SeqCst) {
            let input = match create_pipe(&names.to_server) {
                Ok(handle) => handle,
                Err(error) => {
                    super::log_pipe_error("could not create input named pipe", error);
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };
            let output = match create_pipe(&names.from_server) {
                Ok(handle) => handle,
                Err(error) => {
                    unsafe { CloseHandle(input) };
                    super::log_pipe_error("could not create output named pipe", error);
                    thread::sleep(Duration::from_millis(50));
                    continue;
                }
            };

            let connected = connect_pipe(input).and_then(|()| connect_pipe(output));
            if connected.is_ok() && !stop.load(Ordering::SeqCst) {
                if let Err(error) = serve_session(input, output, stop, requests, &wake) {
                    super::log_pipe_error("client session ended", error);
                }
            }

            disconnect_and_close(input);
            disconnect_and_close(output);
        }
    }

    pub(super) fn wake(names: &PipeNames) {
        let input_name = wide(&names.to_server);
        let output_name = wide(&names.from_server);

        for _ in 0..500 {
            let input = unsafe {
                CreateFileW(
                    input_name.as_ptr(),
                    GENERIC_READ | GENERIC_WRITE,
                    0,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    0,
                )
            };

            if input != INVALID_HANDLE_VALUE {
                let output = unsafe {
                    CreateFileW(
                        output_name.as_ptr(),
                        GENERIC_READ | GENERIC_WRITE,
                        0,
                        std::ptr::null(),
                    )
                };

                thread::sleep(Duration::from_millis(20));

                if output != INVALID_HANDLE_VALUE {
                    unsafe { CloseHandle(output) };
                }
                unsafe { CloseHandle(input) };
                return;
            }

            thread::sleep(Duration::from_millis(2));
        }
    }
}

#[cfg(windows)]
fn log_pipe_error(context: &str, error: io::Error) {
    au_runtime::crash::log_action(format!("mod-script-pipe {context}: {error}"));
}
