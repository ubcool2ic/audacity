use au_plugin_api::{PluginDescriptor, PluginError};
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum EditorEvent {
    Ready {
        width: u32,
        height: u32,
        window: u64,
    },
    Param {
        id: u32,
        value: f32,
    },
    State(Vec<(String, String)>),
    /// Whether the child embedded its editor in the host window (`true`) or fell
    /// back to a top-level window (`false`). Sent before `Ready`.
    Embedded(bool),
    Error(String),
    Closed,
    Crashed {
        status: String,
    },
}

enum ReaderMessage {
    Event(EditorEvent),
    Eof,
}

pub struct EditorProcess {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    receiver: Receiver<ReaderMessage>,
    backlog: VecDeque<EditorEvent>,
    stdout_eof: bool,
    exit_event_sent: bool,
    closed_line_seen: bool,
}

fn plugin_error(message: impl Into<String>) -> PluginError {
    PluginError::Unknown(message.into())
}

pub(crate) fn escape_field(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '\t' | '\n' | '\r' => ' ',
            other => other,
        })
        .collect()
}

pub(crate) fn make_state_line(key: &str, value: &str) -> String {
    format!("STATE\t{}\t{}", escape_field(key), escape_field(value))
}

#[cfg_attr(not(test), allow(dead_code))]
fn parse_state_line(line: &str) -> Option<(String, String)> {
    let mut parts = line.splitn(3, '\t');
    if parts.next()? != "STATE" {
        return None;
    }
    Some((parts.next()?.to_owned(), parts.next()?.to_owned()))
}

fn parse_child_line(
    line: &str,
    state_accumulator: &mut Vec<(String, String)>,
) -> Option<EditorEvent> {
    let mut parts = line.splitn(4, '\t');
    match parts.next()? {
        "EMBED" => Some(EditorEvent::Embedded(parts.next()?.trim() == "1")),
        "READY" => {
            let width = parts.next()?.parse().ok()?;
            let height = parts.next()?.parse().ok()?;
            let window = parts.next()?.parse().ok()?;
            Some(EditorEvent::Ready {
                width,
                height,
                window,
            })
        }
        "PARAM" => {
            let id = parts.next()?.parse().ok()?;
            let value = parts.next()?.parse().ok()?;
            Some(EditorEvent::Param { id, value })
        }
        "STATE" => {
            state_accumulator.push((parts.next()?.to_owned(), parts.next()?.to_owned()));
            None
        }
        "STATEEND" => Some(EditorEvent::State(std::mem::take(state_accumulator))),
        "ERROR" => Some(EditorEvent::Error(parts.next().unwrap_or_default().to_owned())),
        "CLOSED" => Some(EditorEvent::Closed),
        _ => None,
    }
}

fn read_child_stdout(stdout: impl Read, sender: mpsc::Sender<ReaderMessage>) {
    let mut reader = BufReader::new(stdout);
    let mut state_accumulator = Vec::new();
    let mut line = String::new();

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let line = line.trim_end_matches(['\r', '\n']);
                if let Some(event) = parse_child_line(line, &mut state_accumulator) {
                    if sender.send(ReaderMessage::Event(event)).is_err() {
                        return;
                    }
                }
            }
        }
    }

    let _ = sender.send(ReaderMessage::Eof);
}

pub fn editor_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AU_PLUGIN_EDITOR").map(PathBuf::from) {
        if path.exists() {
            return Some(path);
        }
    }

    let executable_name = format!("au-plugin-editor{}", std::env::consts::EXE_SUFFIX);
    let current_exe = std::env::current_exe().ok()?;
    let executable_dir = current_exe.parent()?;

    let beside_current_exe = executable_dir.join(&executable_name);
    if beside_current_exe.exists() {
        return Some(beside_current_exe);
    }

    executable_dir
        .parent()
        .map(|parent| parent.join(executable_name))
        .filter(|path| path.exists())
}

impl EditorProcess {
    pub fn spawn(
        descriptor: &PluginDescriptor,
        state: &[(String, String)],
    ) -> Result<Self, PluginError> {
        Self::spawn_with_env(descriptor, state, &[])
    }

    /// Spawn the editor embedded in the host's X11 window `parent_xid` at physical
    /// position (`x`, `y`). Falls back to a top-level window inside the child if
    /// the parent is not a valid X11 window (see [`EditorEvent::Embedded`]).
    pub fn spawn_embedded(
        descriptor: &PluginDescriptor,
        state: &[(String, String)],
        parent_xid: u64,
        x: i32,
        y: i32,
    ) -> Result<Self, PluginError> {
        Self::spawn_inner(descriptor, state, &[], Some((parent_xid, x, y)))
    }

    /// Move an embedded editor within its parent window (physical pixels).
    pub fn move_to(&mut self, x: i32, y: i32) -> Result<(), PluginError> {
        self.write_command(&format!("MOVE\t{x}\t{y}"))
            .map_err(|error| plugin_error(format!("failed to move editor: {error}")))
    }

    pub fn spawn_with_env(
        descriptor: &PluginDescriptor,
        state: &[(String, String)],
        env: &[(&str, &str)],
    ) -> Result<Self, PluginError> {
        Self::spawn_inner(descriptor, state, env, None)
    }

    fn spawn_inner(
        descriptor: &PluginDescriptor,
        state: &[(String, String)],
        env: &[(&str, &str)],
        embed: Option<(u64, i32, i32)>,
    ) -> Result<Self, PluginError> {
        let executable = editor_path()
            .ok_or_else(|| plugin_error("could not find au-plugin-editor"))?;
        let module_path = descriptor
            .path
            .as_ref()
            .ok_or_else(|| plugin_error("plugin descriptor has no module path"))?;
        let format = format!("{:?}", descriptor.format).to_ascii_lowercase();
        let format = if format.contains("clap") {
            "clap"
        } else if format.contains("vst3") {
            "vst3"
        } else {
            return Err(plugin_error(format!(
                "unsupported plugin format: {:?}",
                descriptor.format
            )));
        };

        let mut command = Command::new(executable);
        command
            .arg("--format")
            .arg(format)
            .arg("--path")
            .arg(module_path)
            .arg("--id")
            .arg(&descriptor.id);
        if let Some((parent_xid, x, y)) = embed {
            command
                .arg("--parent-xid")
                .arg(parent_xid.to_string())
                .arg("--x")
                .arg(x.to_string())
                .arg("--y")
                .arg(y.to_string());
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (key, value) in env {
            command.env(key, value);
        }

        let mut child = command
            .spawn()
            .map_err(|error| plugin_error(format!("failed to start editor process: {error}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| plugin_error("editor process stdin was unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| plugin_error("editor process stdout was unavailable"))?;

        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || read_child_stdout(stdout, sender));

        let mut process = Self {
            child,
            stdin: Some(stdin),
            receiver,
            backlog: VecDeque::new(),
            stdout_eof: false,
            exit_event_sent: false,
            closed_line_seen: false,
        };

        for (key, value) in state {
            if let Err(error) = process.write_command(&make_state_line(key, value)) {
                process.terminate_and_reap();
                return Err(plugin_error(format!(
                    "failed to send plugin state to editor: {error}"
                )));
            }
        }
        if let Err(error) = process.write_command("LOAD") {
            process.terminate_and_reap();
            return Err(plugin_error(format!(
                "failed to initialize editor state: {error}"
            )));
        }

        Ok(process)
    }

    fn write_command(&mut self, command: &str) -> io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "editor stdin is closed"))?;
        writeln!(stdin, "{command}")?;
        stdin.flush()
    }

    pub fn poll(&mut self) -> Vec<EditorEvent> {
        let mut events: Vec<EditorEvent> = self.backlog.drain(..).collect();

        loop {
            match self.receiver.try_recv() {
                Ok(ReaderMessage::Event(event)) => {
                    if matches!(event, EditorEvent::Closed) {
                        self.closed_line_seen = true;
                    }
                    events.push(event);
                }
                Ok(ReaderMessage::Eof) => self.stdout_eof = true,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.stdout_eof = true;
                    break;
                }
            }
        }

        if self.stdout_eof && !self.exit_event_sent {
            if let Ok(Some(status)) = self.child.try_wait() {
                self.exit_event_sent = true;
                if status.success() {
                    if !self.closed_line_seen {
                        events.push(EditorEvent::Closed);
                    }
                } else {
                    let description = exit_status_description(status);
                    au_runtime::crash::log_action(format!(
                        "Plugin editor process crashed: {description}"
                    ));
                    events.push(EditorEvent::Crashed {
                        status: description,
                    });
                }
            }
        }

        events
    }

    pub fn wait_event(
        &mut self,
        timeout: Duration,
        pred: impl Fn(&EditorEvent) -> bool,
    ) -> Option<EditorEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let events = self.poll();
            let mut retained = VecDeque::new();
            let mut matched = None;

            for event in events {
                if matched.is_none() && pred(&event) {
                    matched = Some(event);
                } else {
                    retained.push_back(event);
                }
            }

            while let Some(event) = retained.pop_back() {
                self.backlog.push_front(event);
            }

            if matched.is_some() || Instant::now() >= deadline {
                return matched;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    pub fn request_state(&mut self) -> Result<(), PluginError> {
        self.write_command("GETSTATE")
            .map_err(|error| plugin_error(format!("failed to request plugin state: {error}")))
    }

    pub fn set_param(&mut self, id: u32, value: f32) -> Result<(), PluginError> {
        self.write_command(&format!("SETPARAM\t{id}\t{value}"))
            .map_err(|error| plugin_error(format!("failed to set plugin parameter: {error}")))
    }

    pub fn is_running(&mut self) -> bool {
        let _ = self.poll();
        matches!(self.child.try_wait(), Ok(None))
    }

    fn terminate_and_reap(&mut self) {
        self.stdin.take();
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }

    pub fn close(mut self) {
        let _ = self.write_command("CLOSE");
        self.stdin.take();

        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            let _ = self.poll();
            if !matches!(self.child.try_wait(), Ok(None)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

impl Drop for EditorProcess {
    fn drop(&mut self) {
        self.terminate_and_reap();
    }
}

#[cfg(unix)]
fn exit_status_description(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;

    if let Some(signal) = status.signal() {
        format!("signal {signal}")
    } else {
        format!("exit code {}", status.code().unwrap_or(-1))
    }
}

#[cfg(not(unix))]
fn exit_status_description(status: std::process::ExitStatus) -> String {
    format!("exit code {}", status.code().unwrap_or(-1))
}

pub fn editor_main(args: &[String]) -> i32 {
    #[cfg(target_os = "linux")]
    {
        match editor_main_linux(args) {
            Ok(()) => 0,
            Err(message) => {
                let mut stdout = io::stdout().lock();
                let _ = writeln!(stdout, "ERROR\t{}", escape_field(&message));
                let _ = stdout.flush();
                2
            }
        }
    }

    #[cfg(windows)]
    {
        match crate::editor_win::editor_main_windows(args) {
            Ok(()) => 0,
            Err(message) => {
                let mut stdout = io::stdout().lock();
                let _ = writeln!(stdout, "ERROR\t{}", escape_field(&message));
                let _ = stdout.flush();
                2
            }
        }
    }

    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = args;
        let mut stdout = io::stdout().lock();
        let _ = writeln!(stdout, "ERROR\tunsupported operating system");
        let _ = stdout.flush();
        2
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct EditorArgs {
    pub format: String,
    pub path: PathBuf,
    pub id: String,
    pub parent_xid: Option<u64>,
    pub x: i32,
    pub y: i32,
}

pub fn parse_editor_args(args: &[String]) -> Result<EditorArgs, String> {
    let mut format_arg: Option<String> = None;
    let mut path_arg: Option<PathBuf> = None;
    let mut id_arg: Option<String> = None;
    let mut parent_xid: Option<u64> = None;
    let mut x: i32 = 0;
    let mut y: i32 = 0;

    let mut index = 0;
    while index < args.len() {
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {}", args[index]))?;
        match args[index].as_str() {
            "--format" => format_arg = Some(value.clone()),
            "--path" => path_arg = Some(PathBuf::from(value)),
            "--id" => id_arg = Some(value.clone()),
            "--parent-xid" => {
                parent_xid = Some(
                    value
                        .parse::<u64>()
                        .map_err(|e| format!("invalid --parent-xid: {e}"))?,
                )
            }
            "--x" => {
                x = value
                    .parse::<i32>()
                    .map_err(|e| format!("invalid --x: {e}"))?
            }
            "--y" => {
                y = value
                    .parse::<i32>()
                    .map_err(|e| format!("invalid --y: {e}"))?
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
        index += 2;
    }

    Ok(EditorArgs {
        format: format_arg.ok_or_else(|| "missing --format".to_owned())?,
        path: path_arg.ok_or_else(|| "missing --path".to_owned())?,
        id: id_arg.ok_or_else(|| "missing --id".to_owned())?,
        parent_xid,
        x,
        y,
    })
}

#[cfg(test)]
mod editor_args_tests {
    use super::*;

    #[test]
    fn test_parse_editor_args() {
        let args = vec![
            "--format".to_string(),
            "clap".to_string(),
            "--path".to_string(),
            "/path/to/plugin.clap".to_string(),
            "--id".to_string(),
            "plugin-id".to_string(),
            "--parent-xid".to_string(),
            "123456".to_string(),
            "--x".to_string(),
            "10".to_string(),
            "--y".to_string(),
            "20".to_string(),
        ];
        let parsed = parse_editor_args(&args).unwrap();
        assert_eq!(
            parsed,
            EditorArgs {
                format: "clap".to_string(),
                path: PathBuf::from("/path/to/plugin.clap"),
                id: "plugin-id".to_string(),
                parent_xid: Some(123456),
                x: 10,
                y: 20,
            }
        );
    }
}

#[cfg(target_os = "linux")]
fn editor_main_linux(args: &[String]) -> Result<(), String> {
    use au_plugin_api::{EditorParent, PluginHost, PluginInstance};
    use std::collections::HashMap;
    use std::ffi::{c_char, c_int, c_long, c_ulong, c_void, CString};
    use std::ptr;
    use std::time::Instant;

    let parsed_args = parse_editor_args(args)?;
    let format = parsed_args.format.clone();
    let path = parsed_args.path.clone();
    let requested_id = parsed_args.id.clone();

    let (descriptor, is_vst3) = match format.to_ascii_lowercase().as_str() {
        "clap" => {
            let descriptors =
                crate::clap::scan_file(&path).map_err(|error| format!("{error:?}"))?;
            let descriptor = descriptors
                .iter()
                .find(|descriptor| descriptor.id == requested_id)
                .or_else(|| descriptors.first())
                .cloned()
                .ok_or_else(|| "no plugin descriptors found in module".to_owned())?;
            (descriptor, false)
        }
        "vst3" => {
            let descriptors =
                crate::vst3::scan_file(&path).map_err(|error| format!("{error:?}"))?;
            let descriptor = descriptors
                .iter()
                .find(|descriptor| descriptor.id == requested_id)
                .or_else(|| descriptors.first())
                .cloned()
                .ok_or_else(|| "no plugin descriptors found in module".to_owned())?;
            (descriptor, true)
        }
        other => return Err(format!("unsupported plugin format: {other}")),
    };

    let mut plugin: Box<dyn PluginInstance> = if is_vst3 {
        crate::vst3::Vst3Host::new()
            .instantiate(&descriptor)
            .map_err(|error| format!("{error:?}"))?
    } else {
        crate::clap::ClapHost::new()
            .instantiate(&descriptor)
            .map_err(|error| format!("{error:?}"))?
    };

    if !plugin.has_editor() {
        return Err("plugin has no editor".to_owned());
    }

    type Display = c_void;
    type Window = c_ulong;
    type Atom = c_ulong;
    type XEvent = [c_long; 24];

    type XOpenDisplay = unsafe extern "C" fn(*const c_char) -> *mut Display;
    type XDefaultScreen = unsafe extern "C" fn(*mut Display) -> c_int;
    type XRootWindow = unsafe extern "C" fn(*mut Display, c_int) -> Window;
    type XBlackPixel = unsafe extern "C" fn(*mut Display, c_int) -> c_ulong;
    type XWhitePixel = unsafe extern "C" fn(*mut Display, c_int) -> c_ulong;
    type XCreateSimpleWindow = unsafe extern "C" fn(
        *mut Display,
        Window,
        c_int,
        c_int,
        c_uint,
        c_uint,
        c_uint,
        c_ulong,
        c_ulong,
    ) -> Window;
    type XStoreName = unsafe extern "C" fn(*mut Display, Window, *const c_char) -> c_int;
    type XInternAtom =
        unsafe extern "C" fn(*mut Display, *const c_char, c_int) -> Atom;
    type XSetWMProtocols =
        unsafe extern "C" fn(*mut Display, Window, *mut Atom, c_int) -> c_int;
    type XMapWindow = unsafe extern "C" fn(*mut Display, Window) -> c_int;
    type XResizeWindow = unsafe extern "C" fn(*mut Display, Window, c_uint, c_uint) -> c_int;
    type XFlush = unsafe extern "C" fn(*mut Display) -> c_int;
    type XPending = unsafe extern "C" fn(*mut Display) -> c_int;
    type XNextEvent = unsafe extern "C" fn(*mut Display, *mut XEvent) -> c_int;
    type XDestroyWindow = unsafe extern "C" fn(*mut Display, Window) -> c_int;
    type XCloseDisplay = unsafe extern "C" fn(*mut Display) -> c_int;
    type XErrorHandler = unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int;
    type XSetErrorHandler = unsafe extern "C" fn(Option<XErrorHandler>) -> Option<XErrorHandler>;
    type XGetWindowAttributes = unsafe extern "C" fn(*mut Display, Window, *mut u8) -> c_int;
    type XMoveWindow = unsafe extern "C" fn(*mut Display, Window, c_int, c_int) -> c_int;
    type XSync = unsafe extern "C" fn(*mut Display, c_int) -> c_int;

    static X11_HAD_ERROR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    unsafe extern "C" fn x11_error_handler(_display: *mut c_void, _event: *mut c_void) -> c_int {
        X11_HAD_ERROR.store(true, std::sync::atomic::Ordering::SeqCst);
        0
    }

    struct X11 {
        _library: libloading::Library,
        open_display: XOpenDisplay,
        default_screen: XDefaultScreen,
        root_window: XRootWindow,
        black_pixel: XBlackPixel,
        white_pixel: XWhitePixel,
        create_simple_window: XCreateSimpleWindow,
        store_name: XStoreName,
        intern_atom: XInternAtom,
        set_wm_protocols: XSetWMProtocols,
        map_window: XMapWindow,
        resize_window: XResizeWindow,
        flush: XFlush,
        pending: XPending,
        next_event: XNextEvent,
        destroy_window: XDestroyWindow,
        close_display: XCloseDisplay,
        set_error_handler: XSetErrorHandler,
        get_window_attributes: XGetWindowAttributes,
        move_window: XMoveWindow,
        sync: XSync,
    }

    use std::ffi::c_uint;

    let library = unsafe { libloading::Library::new("libX11.so.6") }
        .map_err(|error| format!("could not load libX11.so.6: {error}"))?;

    macro_rules! symbol {
        ($name:literal, $ty:ty) => {
            unsafe {
                *library
                    .get::<$ty>(concat!($name, "\0").as_bytes())
                    .map_err(|error| format!("could not resolve {}: {error}", $name))?
            }
        };
    }

    let x11 = X11 {
        open_display: symbol!("XOpenDisplay", XOpenDisplay),
        default_screen: symbol!("XDefaultScreen", XDefaultScreen),
        root_window: symbol!("XRootWindow", XRootWindow),
        black_pixel: symbol!("XBlackPixel", XBlackPixel),
        white_pixel: symbol!("XWhitePixel", XWhitePixel),
        create_simple_window: symbol!("XCreateSimpleWindow", XCreateSimpleWindow),
        store_name: symbol!("XStoreName", XStoreName),
        intern_atom: symbol!("XInternAtom", XInternAtom),
        set_wm_protocols: symbol!("XSetWMProtocols", XSetWMProtocols),
        map_window: symbol!("XMapWindow", XMapWindow),
        resize_window: symbol!("XResizeWindow", XResizeWindow),
        flush: symbol!("XFlush", XFlush),
        pending: symbol!("XPending", XPending),
        next_event: symbol!("XNextEvent", XNextEvent),
        destroy_window: symbol!("XDestroyWindow", XDestroyWindow),
        close_display: symbol!("XCloseDisplay", XCloseDisplay),
        set_error_handler: symbol!("XSetErrorHandler", XSetErrorHandler),
        get_window_attributes: symbol!("XGetWindowAttributes", XGetWindowAttributes),
        move_window: symbol!("XMoveWindow", XMoveWindow),
        sync: symbol!("XSync", XSync),
        _library: library,
    };

    let display = unsafe { (x11.open_display)(ptr::null()) };
    if display.is_null() {
        return Err("could not open X11 display (is DISPLAY set?)".to_owned());
    }

    let screen = unsafe { (x11.default_screen)(display) };
    let root = unsafe { (x11.root_window)(display, screen) };
    let black = unsafe { (x11.black_pixel)(display, screen) };
    let white = unsafe { (x11.white_pixel)(display, screen) };
    unsafe { (x11.set_error_handler)(Some(x11_error_handler)) };

    // Embed into the host's effect dialog when the parent X11 window is valid
    // (X11 reparenting); otherwise fall back to a top-level window.
    let mut embedded = false;
    let mut parent_window = root;
    let (mut win_x, mut win_y) = (0, 0);
    if let Some(parent_xid) = parsed_args.parent_xid {
        X11_HAD_ERROR.store(false, std::sync::atomic::Ordering::SeqCst);
        let mut attrs = [0u8; 256];
        let status = unsafe {
            (x11.sync)(display, 0);
            let status = (x11.get_window_attributes)(display, parent_xid as Window, attrs.as_mut_ptr());
            (x11.sync)(display, 0);
            status
        };
        if status != 0 && !X11_HAD_ERROR.load(std::sync::atomic::Ordering::SeqCst) {
            parent_window = parent_xid as Window;
            win_x = parsed_args.x;
            win_y = parsed_args.y;
            embedded = true;
        }
    }

    let window = unsafe {
        (x11.create_simple_window)(display, parent_window, win_x, win_y, 640, 480, 0, black, white)
    };
    if window == 0 {
        unsafe {
            (x11.close_display)(display);
        }
        return Err("could not create X11 editor window".to_owned());
    }

    let delete_name = CString::new("WM_DELETE_WINDOW").expect("static atom name");
    let wm_delete = unsafe { (x11.intern_atom)(display, delete_name.as_ptr(), 0) };
    if !embedded {
        let title = CString::new(format!("{} - Audacity-RS", descriptor.name).replace('\0', " "))
            .map_err(|error| error.to_string())?;
        let mut wm_delete_mut = wm_delete;
        unsafe {
            (x11.store_name)(display, window, title.as_ptr());
            (x11.set_wm_protocols)(display, window, &mut wm_delete_mut, 1);
        }
    }
    unsafe {
        (x11.map_window)(display, window);
        (x11.flush)(display);
    }

    let (width, height) = match plugin.open_editor(EditorParent::X11(window as u64)) {
        Ok(size) => size,
        Err(error) => {
            unsafe {
                (x11.destroy_window)(display, window);
                (x11.close_display)(display);
            }
            return Err(format!("failed to open plugin editor: {error:?}"));
        }
    };

    unsafe {
        (x11.resize_window)(display, window, width, height);
        (x11.flush)(display);
    }

    let (stdin_sender, stdin_receiver) = mpsc::channel::<Option<String>>();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            match line {
                Ok(line) => {
                    if stdin_sender.send(Some(line)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = stdin_sender.send(None);
    });

    let stdout = io::stdout();
    let mut output = stdout.lock();
    writeln!(output, "EMBED\t{}", u8::from(embedded))
        .map_err(|error| format!("failed to report embedding: {error}"))?;
    writeln!(output, "READY\t{width}\t{height}\t{window}")
        .and_then(|()| output.flush())
        .map_err(|error| format!("failed to report editor readiness: {error}"))?;

    let crash_after = std::env::var("AU_EDITOR_TEST_CRASH_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis);
    let ready_at = Instant::now();
    let mut state_pairs = Vec::<(String, String)>::new();
    let mut last_sent = HashMap::<u32, f32>::new();
    for parameter in &descriptor.params {
        if let Some(value) = plugin.get_param(parameter.id) {
            last_sent.insert(parameter.id, value);
        }
    }

    let mut quitting = false;
    let mut last_param_check = Instant::now();

    while !quitting {
        if is_vst3 {
            crate::vst3::run_loop_idle();
        } else {
            crate::clap::run_loop_idle();
        }

        while unsafe { (x11.pending)(display) } > 0 {
            let mut event = [0 as c_long; 24];
            unsafe {
                (x11.next_event)(display, &mut event);
            }
            let event_type = event[0] as c_int;
            if event_type == 33 && event[7] as c_ulong == wm_delete {
                quitting = true;
                break;
            }
        }

        loop {
            match stdin_receiver.try_recv() {
                Ok(Some(line)) => {
                    let mut parts = line.splitn(3, '\t');
                    match parts.next().unwrap_or_default() {
                        "STATE" => {
                            if let (Some(key), Some(value)) = (parts.next(), parts.next()) {
                                state_pairs.push((key.to_owned(), value.to_owned()));
                            }
                        }
                        "LOAD" => {
                            if let Err(error) = plugin.load_state(&state_pairs) {
                                writeln!(output, "ERROR\t{}", escape_field(&format!("{error:?}")))
                                    .and_then(|()| output.flush())
                                    .ok();
                            }
                            state_pairs.clear();
                        }
                        "GETSTATE" => match plugin.save_state() {
                            Ok(pairs) => {
                                for (key, value) in pairs {
                                    writeln!(output, "{}", make_state_line(&key, &value)).ok();
                                }
                                writeln!(output, "STATEEND").ok();
                                output.flush().ok();
                            }
                            Err(error) => {
                                writeln!(output, "ERROR\t{}", escape_field(&format!("{error:?}")))
                                    .and_then(|()| output.flush())
                                    .ok();
                            }
                        },
                        "SETPARAM" => {
                            if let (Some(id), Some(value)) = (parts.next(), parts.next()) {
                                if let (Ok(id), Ok(value)) = (id.parse::<u32>(), value.parse::<f32>())
                                {
                                    if let Err(error) = plugin.set_param(id, value) {
                                        writeln!(
                                            output,
                                            "ERROR\t{}",
                                            escape_field(&format!("{error:?}"))
                                        )
                                        .and_then(|()| output.flush())
                                        .ok();
                                    }
                                }
                            }
                        }
                        "MOVE" => {
                            if let (Some(x), Some(y)) = (parts.next(), parts.next()) {
                                if let (Ok(x), Ok(y)) = (x.parse::<i32>(), y.trim().parse::<i32>()) {
                                    unsafe {
                                        (x11.move_window)(display, window, x, y);
                                        (x11.flush)(display);
                                    }
                                }
                            }
                        }
                        "CLOSE" => {
                            quitting = true;
                            break;
                        }
                        _ => {}
                    }
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    quitting = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }

        if let Some(delay) = crash_after {
            if ready_at.elapsed() >= delay {
                unsafe {
                    (16usize as *mut i32).write_volatile(1) /* invalid address, real SIGSEGV (null would trip a debug precondition abort) */;
                }
            }
        }

        if last_param_check.elapsed() >= Duration::from_millis(100) {
            for parameter in &descriptor.params {
                if let Some(value) = plugin.get_param(parameter.id) {
                    let changed = last_sent
                        .get(&parameter.id)
                        .map(|last| last.to_bits() != value.to_bits())
                        .unwrap_or(true);
                    if changed {
                        writeln!(output, "PARAM\t{}\t{}", parameter.id, value).ok();
                        output.flush().ok();
                        last_sent.insert(parameter.id, value);
                    }
                }
            }
            last_param_check = Instant::now();
        }

        if !quitting {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    plugin.close_editor();
    unsafe {
        (x11.destroy_window)(display, window);
        (x11.flush)(display);
        (x11.close_display)(display);
    }
    writeln!(output, "CLOSED")
        .and_then(|()| output.flush())
        .map_err(|error| format!("failed to report editor closure: {error}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{escape_field, make_state_line, parse_state_line};

    #[test]
    fn protocol_state_helpers_round_trip_sanitized_fields() {
        let key = "key\tname";
        let value = "line one\nline two\rline three";
        let line = make_state_line(key, value);
        assert_eq!(
            parse_state_line(&line),
            Some((
                escape_field(key),
                escape_field(value),
            ))
        );
    }

    #[test]
    fn protocol_state_parser_rejects_other_commands() {
        assert_eq!(parse_state_line("GETSTATE"), None);
        assert_eq!(parse_state_line("STATE\tkey"), None);
    }
}

