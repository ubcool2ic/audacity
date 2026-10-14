use au_plugin_api::{
    EditorParent, ParamInfo, PluginDescriptor, PluginError, PluginFormat, PluginHost,
    PluginInstance,
};
use clap_sys::{
    audio_buffer::clap_audio_buffer,
    entry::clap_plugin_entry,
    events::{
        clap_event_header, clap_event_param_value, clap_input_events, clap_output_events,
        CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_PARAM_VALUE,
    },
    ext::{
        audio_ports::{
            clap_audio_port_info, clap_plugin_audio_ports, CLAP_AUDIO_PORT_IS_MAIN,
            CLAP_EXT_AUDIO_PORTS,
        },
        gui::{clap_host_gui, clap_plugin_gui, clap_window, clap_window_handle, CLAP_EXT_GUI},
        latency::{clap_host_latency, clap_plugin_latency, CLAP_EXT_LATENCY},
        log::{clap_host_log, CLAP_EXT_LOG},
        params::{clap_host_params, clap_param_info, clap_plugin_params, CLAP_EXT_PARAMS},
        state::{clap_host_state, clap_plugin_state, CLAP_EXT_STATE},
        thread_check::{clap_host_thread_check, CLAP_EXT_THREAD_CHECK},
    },
    factory::plugin_factory::{clap_plugin_factory, CLAP_PLUGIN_FACTORY_ID},
    host::clap_host,
    plugin::clap_plugin,
    process::clap_process,
    stream::{clap_istream, clap_ostream},
    version::{clap_version_is_compatible, CLAP_VERSION},
};
use libloading::{Library, Symbol};
use std::{
    collections::{HashMap, HashSet},
    env,
    ffi::{c_char, c_void, CStr, CString},
    fs,
    path::{Path, PathBuf},
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    thread::{self, ThreadId},
};

static MODULES: OnceLock<Mutex<HashMap<PathBuf, Weak<ClapModule>>>> = OnceLock::new();

struct ClapModule {
    entry: *const clap_plugin_entry,
    _library: Library,
}

unsafe impl Send for ClapModule {}
unsafe impl Sync for ClapModule {}

impl ClapModule {
    fn open(path: &Path) -> Result<Self, PluginError> {
        let library = unsafe { Library::new(path) }
            .map_err(|e| PluginError::LoadFailed(format!("{}: {e}", path.display())))?;
        let entry = {
            let symbol: Symbol<'_, *const clap_plugin_entry> =
                unsafe { library.get(b"clap_entry\0") }.map_err(|e| {
                    PluginError::LoadFailed(format!(
                        "{} has no usable clap_entry: {e}",
                        path.display()
                    ))
                })?;
            *symbol
        };

        let version = unsafe { (*entry).clap_version };
        if !clap_version_is_compatible(version) {
            return Err(PluginError::LoadFailed(format!(
                "{} uses an incompatible CLAP version",
                path.display()
            )));
        }

        let init = unsafe { (*entry).init }
            .ok_or_else(|| PluginError::LoadFailed("CLAP entry has no init callback".into()))?;
        let cpath = path_cstring(path)?;
        if !unsafe { init(cpath.as_ptr()) } {
            return Err(PluginError::LoadFailed(format!(
                "{} rejected initialization",
                path.display()
            )));
        }

        Ok(Self {
            entry,
            _library: library,
        })
    }
}

impl Drop for ClapModule {
    fn drop(&mut self) {
        if let Some(deinit) = unsafe { (*self.entry).deinit } {
            unsafe { deinit() };
        }
    }
}

fn load_module(path: &Path) -> Result<Arc<ClapModule>, PluginError> {
    let key = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let cache = MODULES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(module) = cache.get(&key).and_then(Weak::upgrade) {
        return Ok(module);
    }

    let module = Arc::new(ClapModule::open(&key)?);
    cache.insert(key, Arc::downgrade(&module));
    Ok(module)
}

fn path_cstring(path: &Path) -> Result<CString, PluginError> {
    CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| PluginError::LoadFailed("plugin path contains a NUL byte".into()))
}

fn plugin_id_cstring(id: &str) -> Result<CString, PluginError> {
    CString::new(id)
        .map_err(|_| PluginError::LoadFailed("plugin id contains a NUL byte".into()))
}

fn read_c_string(value: *const c_char) -> String {
    if value.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned()
    }
}

fn fixed_string<const N: usize>(value: &[c_char; N]) -> String {
    let bytes: Vec<u8> = value
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn get_plugin_extension<T>(plugin: *const clap_plugin, id: &CStr) -> Option<*const T> {
    let callback = unsafe { (*plugin).get_extension }?;
    let extension = unsafe { callback(plugin, id.as_ptr()) };
    if extension.is_null() {
        None
    } else {
        Some(extension.cast())
    }
}

fn read_audio_channel_count(
    plugin: *const clap_plugin,
    ports: Option<*const clap_plugin_audio_ports>,
    is_input: bool,
) -> u32 {
    let Some(ports) = ports else {
        return 0;
    };
    let count = unsafe { (*ports).count };
    let get = unsafe { (*ports).get };
    let (Some(count), Some(get)) = (count, get) else {
        return 0;
    };

    let port_count = unsafe { count(plugin, is_input) };
    let mut first = None;
    for index in 0..port_count {
        let mut info: clap_audio_port_info = unsafe { std::mem::zeroed() };
        if unsafe { get(plugin, index, is_input, &mut info) } {
            if first.is_none() {
                first = Some(info.channel_count);
            }
            if info.flags & CLAP_AUDIO_PORT_IS_MAIN != 0 {
                return info.channel_count;
            }
        }
    }
    first.unwrap_or(0)
}

fn read_params(plugin: *const clap_plugin) -> Vec<ParamInfo> {
    let Some(params) = get_plugin_extension::<clap_plugin_params>(plugin, CLAP_EXT_PARAMS) else {
        return Vec::new();
    };
    let count = unsafe { (*params).count };
    let get_info = unsafe { (*params).get_info };
    let (Some(count), Some(get_info)) = (count, get_info) else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for index in 0..unsafe { count(plugin) } {
        let mut info: clap_param_info = unsafe { std::mem::zeroed() };
        if unsafe { get_info(plugin, index, &mut info) } {
            result.push(ParamInfo {
                id: info.id,
                name: fixed_string(&info.name),
                min: info.min_value as f32,
                max: info.max_value as f32,
                default: info.default_value as f32,
            });
        }
    }
    result
}

fn bundle_binary(path: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        if path.is_dir() {
            let stem = path.file_stem()?.to_str()?;
            let binary = path.join("Contents").join("MacOS").join(stem);
            return binary.is_file().then_some(binary);
        }
    }
    path.is_file().then(|| path.to_path_buf())
}

pub fn scan_file(path: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
    let binary = bundle_binary(path).ok_or_else(|| {
        PluginError::LoadFailed(format!("{} is not a CLAP module", path.display()))
    })?;
    let module = load_module(&binary)?;
    let get_factory = unsafe { (*module.entry).get_factory }
        .ok_or_else(|| PluginError::LoadFailed("CLAP entry has no factory callback".into()))?;
    let factory_ptr =
        unsafe { get_factory(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }.cast::<clap_plugin_factory>();
    if factory_ptr.is_null() {
        return Err(PluginError::LoadFailed(format!(
            "{} has no CLAP plugin factory",
            path.display()
        )));
    }
    let factory = unsafe { &*factory_ptr };
    let get_count = factory
        .get_plugin_count
        .ok_or_else(|| PluginError::LoadFailed("CLAP factory has no count callback".into()))?;
    let get_descriptor = factory
        .get_plugin_descriptor
        .ok_or_else(|| PluginError::LoadFailed("CLAP factory has no descriptor callback".into()))?;
    let create_plugin = factory
        .create_plugin
        .ok_or_else(|| PluginError::LoadFailed("CLAP factory has no create callback".into()))?;

    let host = HostData::new();
    let mut descriptors = Vec::new();
    for index in 0..unsafe { get_count(factory_ptr) } {
        let descriptor_ptr = unsafe { get_descriptor(factory_ptr, index) };
        if descriptor_ptr.is_null() {
            continue;
        }
        let factory_descriptor = unsafe { &*descriptor_ptr };
        let id = read_c_string(factory_descriptor.id);
        if id.is_empty() {
            continue;
        }

        let plugin = unsafe { create_plugin(factory_ptr, &host.host, factory_descriptor.id) };
        if plugin.is_null() {
            continue;
        }
        let initialized = unsafe { (*plugin).init.map(|init| init(plugin)).unwrap_or(false) };
        if !initialized {
            if let Some(destroy) = unsafe { (*plugin).destroy } {
                unsafe { destroy(plugin) };
            }
            continue;
        }

        let ports = get_plugin_extension::<clap_plugin_audio_ports>(plugin, CLAP_EXT_AUDIO_PORTS);
        descriptors.push(PluginDescriptor {
            id: crate::state::effect_id(PluginFormat::Clap, path, &id),
            name: read_c_string(factory_descriptor.name),
            vendor: read_c_string(factory_descriptor.vendor),
            format: PluginFormat::Clap,
            path: Some(path.to_path_buf()),
            params: read_params(plugin),
            n_inputs: read_audio_channel_count(plugin, ports, true),
            n_outputs: read_audio_channel_count(plugin, ports, false),
        });

        if let Some(destroy) = unsafe { (*plugin).destroy } {
            unsafe { destroy(plugin) };
        }
    }

    Ok(descriptors)
}

pub fn candidates(paths: &[PathBuf]) -> Vec<PathBuf> {
    fn is_clap_path(path: &Path) -> bool {
        path.extension().and_then(|ext| ext.to_str()) == Some("clap")
    }

    fn canonical_key(path: &Path) -> PathBuf {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    }

    fn walk(
        path: &Path,
        depth: usize,
        visited_dirs: &mut HashSet<PathBuf>,
        found: &mut HashSet<PathBuf>,
    ) {
        if depth > 4 {
            return;
        }
        let Ok(metadata) = fs::metadata(path) else {
            return;
        };

        if metadata.is_file() {
            if is_clap_path(path) {
                found.insert(canonical_key(path));
            }
            return;
        }
        if !metadata.is_dir() {
            return;
        }

        #[cfg(target_os = "macos")]
        if is_clap_path(path) {
            found.insert(canonical_key(path));
            return;
        }

        let key = canonical_key(path);
        if !visited_dirs.insert(key) {
            return;
        }

        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            walk(&entry.path(), depth + 1, visited_dirs, found);
        }
    }

    let mut visited_dirs = HashSet::new();
    let mut found = HashSet::new();
    for path in paths {
        walk(path, 0, &mut visited_dirs, &mut found);
    }

    let mut result: Vec<_> = found.into_iter().collect();
    result.sort();
    result
}

struct HostData {
    host: clap_host,
    main_thread: ThreadId,
    in_audio: AtomicBool,
    request_restart: AtomicBool,
    request_process: AtomicBool,
    request_callback: AtomicBool,
    params_flush_requested: AtomicBool,
    latency_changed: AtomicBool,
    state_dirty: AtomicBool,
    editor_open: AtomicBool,
    resize_width: AtomicU32,
    resize_height: AtomicU32,
}

impl HostData {
    fn new() -> Box<Self> {
        let mut data = Box::new(Self {
            host: clap_host {
                clap_version: CLAP_VERSION,
                host_data: ptr::null_mut(),
                name: b"Audacity-RS\0".as_ptr().cast(),
                vendor: b"Audacity\0".as_ptr().cast(),
                url: b"https://www.audacityteam.org/\0".as_ptr().cast(),
                version: b"0.1.0\0".as_ptr().cast(),
                get_extension: Some(host_get_extension),
                request_restart: Some(host_request_restart),
                request_process: Some(host_request_process),
                request_callback: Some(host_request_callback),
            },
            main_thread: thread::current().id(),
            in_audio: AtomicBool::new(false),
            request_restart: AtomicBool::new(false),
            request_process: AtomicBool::new(false),
            request_callback: AtomicBool::new(false),
            params_flush_requested: AtomicBool::new(false),
            latency_changed: AtomicBool::new(false),
            state_dirty: AtomicBool::new(false),
            editor_open: AtomicBool::new(false),
            resize_width: AtomicU32::new(0),
            resize_height: AtomicU32::new(0),
        });
        let self_ptr = (&mut *data) as *mut HostData;
        data.host.host_data = self_ptr.cast();
        data
    }
}

unsafe fn host_data<'a>(host: *const clap_host) -> Option<&'a HostData> {
    if host.is_null() {
        return None;
    }
    let data = unsafe { (*host).host_data.cast::<HostData>() };
    unsafe { data.as_ref() }
}

unsafe extern "C" fn host_get_extension(
    _host: *const clap_host,
    extension_id: *const c_char,
) -> *const c_void {
    if extension_id.is_null() {
        return ptr::null();
    }
    let requested = unsafe { CStr::from_ptr(extension_id) };
    if requested == CLAP_EXT_LOG {
        (&HOST_LOG as *const clap_host_log).cast()
    } else if requested == CLAP_EXT_THREAD_CHECK {
        (&HOST_THREAD_CHECK as *const clap_host_thread_check).cast()
    } else if requested == CLAP_EXT_PARAMS {
        (&HOST_PARAMS as *const clap_host_params).cast()
    } else if requested == CLAP_EXT_LATENCY {
        (&HOST_LATENCY as *const clap_host_latency).cast()
    } else if requested == CLAP_EXT_STATE {
        (&HOST_STATE as *const clap_host_state).cast()
    } else if requested == CLAP_EXT_GUI {
        (&HOST_GUI as *const clap_host_gui).cast()
    } else {
        ptr::null()
    }
}

unsafe extern "C" fn host_request_restart(host: *const clap_host) {
    if let Some(data) = unsafe { host_data(host) } {
        data.request_restart.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_request_process(host: *const clap_host) {
    if let Some(data) = unsafe { host_data(host) } {
        data.request_process.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_request_callback(host: *const clap_host) {
    if let Some(data) = unsafe { host_data(host) } {
        data.request_callback.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_log(
    _host: *const clap_host,
    severity: i32,
    message: *const c_char,
) {
    if env::var_os("AU_PLUGIN_LOG").is_none() || message.is_null() {
        return;
    }
    let message = unsafe { CStr::from_ptr(message) }.to_string_lossy();
    eprintln!("[CLAP {severity}] {message}");
}

unsafe extern "C" fn host_is_main_thread(host: *const clap_host) -> bool {
    unsafe { host_data(host) }
        .map(|data| data.main_thread == thread::current().id())
        .unwrap_or(false)
}

unsafe extern "C" fn host_is_audio_thread(host: *const clap_host) -> bool {
    unsafe { host_data(host) }
        .map(|data| data.in_audio.load(Ordering::Relaxed))
        .unwrap_or(false)
}

unsafe extern "C" fn host_params_rescan(_host: *const clap_host, _flags: u32) {}

unsafe extern "C" fn host_params_clear(_host: *const clap_host, _id: u32, _flags: u32) {}

unsafe extern "C" fn host_params_request_flush(host: *const clap_host) {
    if let Some(data) = unsafe { host_data(host) } {
        data.params_flush_requested.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_latency_changed(host: *const clap_host) {
    if let Some(data) = unsafe { host_data(host) } {
        data.latency_changed.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_state_mark_dirty(host: *const clap_host) {
    if let Some(data) = unsafe { host_data(host) } {
        data.state_dirty.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn host_gui_resize_hints_changed(_host: *const clap_host) {}

unsafe extern "C" fn host_gui_request_resize(
    host: *const clap_host,
    width: u32,
    height: u32,
) -> bool {
    if let Some(data) = unsafe { host_data(host) } {
        data.resize_width.store(width, Ordering::Relaxed);
        data.resize_height.store(height, Ordering::Relaxed);
        true
    } else {
        false
    }
}

unsafe extern "C" fn host_gui_request_show(_host: *const clap_host) -> bool {
    true
}

unsafe extern "C" fn host_gui_request_hide(_host: *const clap_host) -> bool {
    true
}

unsafe extern "C" fn host_gui_closed(host: *const clap_host, _was_destroyed: bool) {
    if let Some(data) = unsafe { host_data(host) } {
        data.editor_open.store(false, Ordering::Relaxed);
    }
}

static HOST_LOG: clap_host_log = clap_host_log { log: Some(host_log) };
static HOST_THREAD_CHECK: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(host_is_main_thread),
    is_audio_thread: Some(host_is_audio_thread),
};
static HOST_PARAMS: clap_host_params = clap_host_params {
    rescan: Some(host_params_rescan),
    clear: Some(host_params_clear),
    request_flush: Some(host_params_request_flush),
};
static HOST_LATENCY: clap_host_latency = clap_host_latency {
    changed: Some(host_latency_changed),
};
static HOST_STATE: clap_host_state = clap_host_state {
    mark_dirty: Some(host_state_mark_dirty),
};
static HOST_GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(host_gui_resize_hints_changed),
    request_resize: Some(host_gui_request_resize),
    request_show: Some(host_gui_request_show),
    request_hide: Some(host_gui_request_hide),
    closed: Some(host_gui_closed),
};

unsafe extern "C" fn input_events_size(list: *const clap_input_events) -> u32 {
    if list.is_null() {
        return 0;
    }
    let events = unsafe { (*list).ctx.cast::<Vec<clap_event_param_value>>() };
    if events.is_null() {
        0
    } else {
        unsafe { (*events).len().min(u32::MAX as usize) as u32 }
    }
}

unsafe extern "C" fn input_events_get(
    list: *const clap_input_events,
    index: u32,
) -> *const clap_event_header {
    if list.is_null() {
        return ptr::null();
    }
    let events = unsafe { (*list).ctx.cast::<Vec<clap_event_param_value>>() };
    if events.is_null() {
        return ptr::null();
    }
    unsafe { (*events).get(index as usize) }
        .map(|event| &event.header as *const clap_event_header)
        .unwrap_or(ptr::null())
}

unsafe extern "C" fn output_events_try_push(
    _list: *const clap_output_events,
    _event: *const clap_event_header,
) -> bool {
    true
}

fn make_input_events(events: &Vec<clap_event_param_value>) -> clap_input_events {
    clap_input_events {
        ctx: (events as *const Vec<clap_event_param_value>)
            .cast_mut()
            .cast(),
        size: Some(input_events_size),
        get: Some(input_events_get),
    }
}

fn make_output_events() -> clap_output_events {
    clap_output_events {
        ctx: ptr::null_mut(),
        try_push: Some(output_events_try_push),
    }
}

struct WriteContext {
    bytes: *mut Vec<u8>,
}

unsafe extern "C" fn stream_write(
    stream: *const clap_ostream,
    buffer: *const c_void,
    size: u64,
) -> i64 {
    if stream.is_null() {
        return -1;
    }
    if size == 0 {
        return 0;
    }
    if buffer.is_null() {
        return -1;
    }
    let context = unsafe { (*stream).ctx.cast::<WriteContext>() };
    if context.is_null() {
        return -1;
    }
    let Ok(length) = usize::try_from(size) else {
        return -1;
    };
    let bytes = unsafe { std::slice::from_raw_parts(buffer.cast::<u8>(), length) };
    let output = unsafe { &mut *(*context).bytes };
    if output.try_reserve(length).is_err() {
        return -1;
    }
    output.extend_from_slice(bytes);
    i64::try_from(length).unwrap_or(-1)
}

struct ReadContext {
    bytes: Vec<u8>,
    position: usize,
}

unsafe extern "C" fn stream_read(
    stream: *const clap_istream,
    buffer: *mut c_void,
    size: u64,
) -> i64 {
    if stream.is_null() {
        return -1;
    }
    if size == 0 {
        return 0;
    }
    if buffer.is_null() {
        return -1;
    }
    let context = unsafe { (*stream).ctx.cast::<ReadContext>() };
    if context.is_null() {
        return -1;
    }
    let Ok(requested) = usize::try_from(size) else {
        return -1;
    };
    let context = unsafe { &mut *context };
    let available = context.bytes.len().saturating_sub(context.position);
    let count = requested.min(available);
    if count == 0 {
        return 0;
    }
    unsafe {
        ptr::copy_nonoverlapping(
            context.bytes.as_ptr().add(context.position),
            buffer.cast::<u8>(),
            count,
        );
    }
    context.position += count;
    i64::try_from(count).unwrap_or(-1)
}

pub struct ClapHost;

impl ClapHost {
    pub fn new() -> Self {
        Self
    }

    fn search_paths() -> Vec<PathBuf> {
        let mut paths = Vec::new();

        #[cfg(target_os = "linux")]
        {
            if let Some(home) = env::var_os("HOME") {
                let home = PathBuf::from(home);
                paths.push(home.join(".clap"));
                paths.push(home.join(".local/lib/clap"));
            }
            paths.push(PathBuf::from("/usr/lib/clap"));
            paths.push(PathBuf::from("/usr/local/lib/clap"));
            paths.push(PathBuf::from("/usr/lib/x86_64-linux-gnu/clap"));
        }

        #[cfg(target_os = "windows")]
        {
            if let Some(common) = env::var_os("CommonProgramFiles") {
                paths.push(PathBuf::from(common).join("CLAP"));
            }
            if let Some(local) = env::var_os("LOCALAPPDATA") {
                paths.push(PathBuf::from(local).join("Programs/Common/CLAP"));
            }
            if let Some(appdata) = env::var_os("APPDATA") {
                paths.push(PathBuf::from(appdata).join("CLAP"));
            }
        }

        #[cfg(target_os = "macos")]
        {
            if let Some(home) = env::var_os("HOME") {
                paths.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/CLAP"));
            }
            paths.push(PathBuf::from("/Library/Audio/Plug-Ins/CLAP"));
        }

        if let Some(extra) = env::var_os("CLAP_PATH") {
            paths.extend(env::split_paths(&extra));
        }
        paths
    }
}

impl Default for ClapHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost for ClapHost {
    fn format(&self) -> PluginFormat {
        PluginFormat::Clap
    }

    fn default_search_paths(&self) -> Vec<PathBuf> {
        Self::search_paths()
    }

    fn scan(&self, paths: &[PathBuf]) -> Vec<PluginDescriptor> {
        candidates(paths)
            .iter()
            .filter_map(|path| scan_file(path).ok())
            .flatten()
            .collect()
    }

    fn instantiate(
        &self,
        descriptor: &PluginDescriptor,
    ) -> Result<Box<dyn PluginInstance>, PluginError> {
        if descriptor.format != PluginFormat::Clap {
            return Err(PluginError::LoadFailed(
                "descriptor is not a CLAP plugin".into(),
            ));
        }
        let (format, id_path, uid) = crate::state::parse_effect_id(&descriptor.id)
            .ok_or_else(|| PluginError::LoadFailed("invalid CLAP effect id".into()))?;
        if format != PluginFormat::Clap {
            return Err(PluginError::LoadFailed(
                "effect id is not a CLAP plugin".into(),
            ));
        }
        let path = descriptor.path.as_deref().unwrap_or(id_path.as_path());
        let binary = bundle_binary(path).ok_or_else(|| {
            PluginError::LoadFailed(format!("{} is not a CLAP module", path.display()))
        })?;
        let module = load_module(&binary)?;
        let factory_fn = unsafe { (*module.entry).get_factory }
            .ok_or_else(|| PluginError::LoadFailed("CLAP entry has no factory callback".into()))?;
        let factory_ptr =
            unsafe { factory_fn(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }.cast::<clap_plugin_factory>();
        if factory_ptr.is_null() {
            return Err(PluginError::LoadFailed(
                "CLAP plugin factory is unavailable".into(),
            ));
        }
        let factory = unsafe { &*factory_ptr };
        let create = factory
            .create_plugin
            .ok_or_else(|| PluginError::LoadFailed("CLAP factory has no create callback".into()))?;
        let host = HostData::new();
        let uid = plugin_id_cstring(&uid)?;
        let plugin = unsafe { create(factory_ptr, &host.host, uid.as_ptr()) };
        if plugin.is_null() {
            return Err(PluginError::LoadFailed(format!(
                "CLAP factory could not create {}",
                descriptor.id
            )));
        }
        let initialized = unsafe { (*plugin).init.map(|init| init(plugin)).unwrap_or(false) };
        if !initialized {
            if let Some(destroy) = unsafe { (*plugin).destroy } {
                unsafe { destroy(plugin) };
            }
            return Err(PluginError::LoadFailed(format!(
                "CLAP plugin initialization failed: {}",
                descriptor.id
            )));
        }

        Ok(Box::new(ClapInstance::new(
            module,
            host,
            plugin,
            descriptor.clone(),
        )))
    }
}

pub struct ClapInstance {
    module: Arc<ClapModule>,
    host: Box<HostData>,
    plugin: *const clap_plugin,
    params: Option<*const clap_plugin_params>,
    state: Option<*const clap_plugin_state>,
    latency: Option<*const clap_plugin_latency>,
    gui: Option<*const clap_plugin_gui>,
    descriptor: PluginDescriptor,
    active: bool,
    max_block: usize,
    steady_time: i64,
    pending_events: Vec<clap_event_param_value>,
    cached_params: HashMap<u32, f32>,
    input_channels: u32,
    output_channels: u32,
    scratch_inputs: Vec<Vec<f32>>,
    scratch_outputs: Vec<Vec<f32>>,
    input_ptrs: Vec<*mut f32>,
    output_ptrs: Vec<*mut f32>,
    editor_open: bool,
}

unsafe impl Send for ClapInstance {}

impl ClapInstance {
    fn new(
        module: Arc<ClapModule>,
        host: Box<HostData>,
        plugin: *const clap_plugin,
        descriptor: PluginDescriptor,
    ) -> Self {
        let params = get_plugin_extension::<clap_plugin_params>(plugin, CLAP_EXT_PARAMS);
        let state = get_plugin_extension::<clap_plugin_state>(plugin, CLAP_EXT_STATE);
        let latency = get_plugin_extension::<clap_plugin_latency>(plugin, CLAP_EXT_LATENCY);
        let audio_ports =
            get_plugin_extension::<clap_plugin_audio_ports>(plugin, CLAP_EXT_AUDIO_PORTS);
        let gui = get_plugin_extension::<clap_plugin_gui>(plugin, CLAP_EXT_GUI);

        let (input_channels, output_channels) = if audio_ports.is_some() {
            (
                read_audio_channel_count(plugin, audio_ports, true),
                read_audio_channel_count(plugin, audio_ports, false),
            )
        } else {
            (2, 2)
        };

        let mut instance = Self {
            module,
            host,
            plugin,
            params,
            state,
            latency,
            gui,
            pending_events: Vec::with_capacity(descriptor.params.len().max(8)),
            cached_params: HashMap::with_capacity(descriptor.params.len()),
            descriptor,
            active: false,
            max_block: 0,
            steady_time: 0,
            input_channels,
            output_channels,
            scratch_inputs: Vec::new(),
            scratch_outputs: Vec::new(),
            input_ptrs: Vec::new(),
            output_ptrs: Vec::new(),
            editor_open: false,
        };
        instance.refresh_cached_params();
        instance
    }

    fn refresh_cached_params(&mut self) {
        self.cached_params.clear();
        for param in &self.descriptor.params {
            let value = self.plugin_param_value(param.id).unwrap_or(param.default);
            self.cached_params.insert(param.id, value);
        }
    }

    fn plugin_param_value(&self, id: u32) -> Option<f32> {
        let params = self.params?;
        let get_value = unsafe { (*params).get_value }?;
        let mut value = 0.0_f64;
        if unsafe { get_value(self.plugin, id, &mut value) } {
            Some(value as f32)
        } else {
            None
        }
    }

    fn make_param_event(id: u32, value: f32) -> clap_event_param_value {
        clap_event_param_value {
            header: clap_event_header {
                size: std::mem::size_of::<clap_event_param_value>() as u32,
                time: 0,
                space_id: CLAP_CORE_EVENT_SPACE_ID,
                type_: CLAP_EVENT_PARAM_VALUE,
                flags: 0,
            },
            param_id: id,
            cookie: ptr::null_mut(),
            note_id: -1,
            port_index: -1,
            channel: -1,
            key: -1,
            value: value as f64,
        }
    }

    fn flush_pending_events(&mut self) {
        let Some(params) = self.params else {
            self.pending_events.clear();
            return;
        };
        if let Some(flush) = unsafe { (*params).flush } {
            let input = make_input_events(&self.pending_events);
            let output = make_output_events();
            unsafe { flush(self.plugin, &input, &output) };
        }
        self.pending_events.clear();
    }

    fn run_main_thread_callback_if_requested(&mut self) {
        if self.host.request_callback.swap(false, Ordering::Relaxed) {
            if let Some(callback) = unsafe { (*self.plugin).on_main_thread } {
                unsafe { callback(self.plugin) };
            }
        }
        self.host.request_restart.swap(false, Ordering::Relaxed);
        self.host.request_process.swap(false, Ordering::Relaxed);
        self.host
            .params_flush_requested
            .swap(false, Ordering::Relaxed);
        self.host.latency_changed.swap(false, Ordering::Relaxed);
    }

    fn deactivate(&mut self) {
        if !self.active {
            return;
        }
        if let Some(stop) = unsafe { (*self.plugin).stop_processing } {
            unsafe { stop(self.plugin) };
        }
        if let Some(deactivate) = unsafe { (*self.plugin).deactivate } {
            unsafe { deactivate(self.plugin) };
        }
        self.active = false;
    }

    fn process_chunk(&mut self, inputs: &[&[f32]], outputs: &mut [Vec<f32>], offset: usize) {
        let frames = outputs.first().map_or(0, Vec::len);
        if frames == 0 {
            return;
        }

        for channel in &mut self.scratch_inputs {
            channel[..frames].fill(0.0);
        }
        for plugin_channel in 0..self.input_channels as usize {
            let source_channel = if plugin_channel >= inputs.len() && inputs.len() == 1 {
                Some(0)
            } else {
                (plugin_channel < inputs.len()).then_some(plugin_channel)
            };
            if let Some(source_channel) = source_channel {
                let source = inputs[source_channel];
                if offset < source.len() {
                    let count = frames.min(source.len() - offset);
                    self.scratch_inputs[plugin_channel][..count]
                        .copy_from_slice(&source[offset..offset + count]);
                }
            }
        }

        for channel in &mut self.scratch_outputs {
            channel[..frames].fill(0.0);
        }
        self.input_ptrs.clear();
        self.input_ptrs.extend(
            self.scratch_inputs
                .iter_mut()
                .map(|channel| channel.as_mut_ptr()),
        );
        self.output_ptrs.clear();
        self.output_ptrs.extend(
            self.scratch_outputs
                .iter_mut()
                .map(|channel| channel.as_mut_ptr()),
        );

        let mut input_buffer = clap_audio_buffer {
            data32: if self.input_ptrs.is_empty() {
                ptr::null_mut()
            } else {
                self.input_ptrs.as_mut_ptr()
            },
            data64: ptr::null_mut(),
            channel_count: self.input_channels,
            latency: 0,
            constant_mask: 0,
        };
        let mut output_buffer = clap_audio_buffer {
            data32: if self.output_ptrs.is_empty() {
                ptr::null_mut()
            } else {
                self.output_ptrs.as_mut_ptr()
            },
            data64: ptr::null_mut(),
            channel_count: self.output_channels,
            latency: 0,
            constant_mask: 0,
        };
        let input_events = make_input_events(&self.pending_events);
        let output_events = make_output_events();
        let process = clap_process {
            steady_time: self.steady_time,
            frames_count: frames.min(u32::MAX as usize) as u32,
            transport: ptr::null(),
            audio_inputs: &mut input_buffer,
            audio_outputs: &mut output_buffer,
            audio_inputs_count: 1,
            audio_outputs_count: 1,
            in_events: &input_events,
            out_events: &output_events,
        };

        self.host.in_audio.store(true, Ordering::Relaxed);
        if let Some(process_callback) = unsafe { (*self.plugin).process } {
            unsafe { process_callback(self.plugin, &process) };
        }
        self.host.in_audio.store(false, Ordering::Relaxed);
        self.steady_time = self.steady_time.saturating_add(frames as i64);
        self.pending_events.clear();

        if !self.scratch_outputs.is_empty() {
            for (channel_index, output) in outputs.iter_mut().enumerate() {
                let plugin_channel = if channel_index < self.scratch_outputs.len() {
                    channel_index
                } else {
                    0
                };
                let count = output.len().min(frames);
                output[..count]
                    .copy_from_slice(&self.scratch_outputs[plugin_channel][..count]);
            }
        }
    }

    fn gui_api_supported(&self, api: &CStr, floating: bool) -> bool {
        let Some(gui) = self.gui else {
            return false;
        };
        let Some(supported) = (unsafe { (*gui).is_api_supported }) else {
            return false;
        };
        unsafe { supported(self.plugin, api.as_ptr(), floating) }
    }

    fn platform_gui_api() -> &'static CStr {
        #[cfg(target_os = "windows")]
        {
            clap_sys::ext::gui::CLAP_WINDOW_API_WIN32
        }
        #[cfg(target_os = "macos")]
        {
            clap_sys::ext::gui::CLAP_WINDOW_API_COCOA
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            clap_sys::ext::gui::CLAP_WINDOW_API_X11
        }
    }

    fn close_gui(&mut self) {
        if !self.editor_open {
            return;
        }
        if let Some(gui) = self.gui {
            if let Some(hide) = unsafe { (*gui).hide } {
                unsafe { hide(self.plugin) };
            }
            if let Some(destroy) = unsafe { (*gui).destroy } {
                unsafe { destroy(self.plugin) };
            }
        }
        self.editor_open = false;
        self.host.editor_open.store(false, Ordering::Relaxed);
    }

    fn open_gui(&mut self, parent: EditorParent) -> Result<(u32, u32), PluginError> {
        let gui = self
            .gui
            .ok_or_else(|| PluginError::Unknown("plugin has no CLAP GUI".into()))?;
        let create = unsafe { (*gui).create }
            .ok_or_else(|| PluginError::Unknown("CLAP GUI cannot be created".into()))?;

        let (api, floating) = match parent {
            EditorParent::Floating => {
                let preferred_api = unsafe { (*gui).get_preferred_api };
                let mut api: *const c_char = ptr::null();
                let mut preferred_floating = false;
                let preferred = preferred_api
                    .map(|callback| unsafe {
                        callback(self.plugin, &mut api, &mut preferred_floating)
                    })
                    .unwrap_or(false);
                if !preferred || api.is_null() || !preferred_floating {
                    (Self::platform_gui_api(), true)
                } else {
                    let api = unsafe { CStr::from_ptr(api) };
                    if !self.gui_api_supported(api, true) {
                        return Err(PluginError::Unknown(
                            "CLAP plugin does not support a floating GUI".into(),
                        ));
                    }
                    // The API pointer belongs to the plugin and remains valid for the call.
                    let api = unsafe { CStr::from_ptr(api.as_ptr()) };
                    return self.create_and_show_gui(gui, create, api, true, None);
                }
            }
            _ => (Self::platform_gui_api(), false),
        };

        if !self.gui_api_supported(api, floating) {
            return Err(PluginError::Unknown(
                "CLAP plugin does not support this GUI platform".into(),
            ));
        }

        let window = match parent {
            EditorParent::X11(id) => Some(clap_window {
                api: api.as_ptr(),
                specific: clap_window_handle { x11: id as _ },
            }),
            EditorParent::Win32(handle) => Some(clap_window {
                api: api.as_ptr(),
                specific: clap_window_handle {
                    win32: handle as *mut c_void,
                },
            }),
            EditorParent::Cocoa(handle) => Some(clap_window {
                api: api.as_ptr(),
                specific: clap_window_handle {
                    cocoa: handle as *mut c_void,
                },
            }),
            EditorParent::Floating => None,
        };
        self.create_and_show_gui(gui, create, api, floating, window)
    }

    fn create_and_show_gui(
        &mut self,
        gui: *const clap_plugin_gui,
        create: unsafe extern "C" fn(*const clap_plugin, *const c_char, bool) -> bool,
        api: &CStr,
        floating: bool,
        window: Option<clap_window>,
    ) -> Result<(u32, u32), PluginError> {
        if !unsafe { create(self.plugin, api.as_ptr(), floating) } {
            return Err(PluginError::Unknown(
                "CLAP plugin failed to create its GUI".into(),
            ));
        }

        if let Some(set_scale) = unsafe { (*gui).set_scale } {
            unsafe { set_scale(self.plugin, 1.0) };
        }

        let mut width = 0;
        let mut height = 0;
        if let Some(get_size) = unsafe { (*gui).get_size } {
            unsafe { get_size(self.plugin, &mut width, &mut height) };
        }

        if let Some(window) = window {
            let Some(set_parent) = (unsafe { (*gui).set_parent }) else {
                if let Some(destroy) = unsafe { (*gui).destroy } {
                    unsafe { destroy(self.plugin) };
                }
                return Err(PluginError::Unknown(
                    "CLAP GUI does not support parent windows".into(),
                ));
            };
            if !unsafe { set_parent(self.plugin, &window) } {
                if let Some(destroy) = unsafe { (*gui).destroy } {
                    unsafe { destroy(self.plugin) };
                }
                return Err(PluginError::Unknown(
                    "CLAP plugin rejected the editor parent window".into(),
                ));
            }
        }

        let Some(show) = (unsafe { (*gui).show }) else {
            if let Some(destroy) = unsafe { (*gui).destroy } {
                unsafe { destroy(self.plugin) };
            }
            return Err(PluginError::Unknown("CLAP GUI cannot be shown".into()));
        };
        if !unsafe { show(self.plugin) } {
            if let Some(destroy) = unsafe { (*gui).destroy } {
                unsafe { destroy(self.plugin) };
            }
            return Err(PluginError::Unknown(
                "CLAP plugin failed to show its GUI".into(),
            ));
        }

        self.editor_open = true;
        self.host.editor_open.store(true, Ordering::Relaxed);
        let _requested_size = (
            self.host.resize_width.load(Ordering::Relaxed),
            self.host.resize_height.load(Ordering::Relaxed),
        );
        Ok((width, height))
    }
}

impl PluginInstance for ClapInstance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn latency_samples(&self) -> u32 {
        let Some(latency) = self.latency else {
            return 0;
        };
        unsafe { (*latency).get }
            .map(|get| unsafe { get(self.plugin) })
            .unwrap_or(0)
    }

    fn save_state(&mut self) -> Result<Vec<(String, String)>, PluginError> {
        let mut state = Vec::new();
        if let Some(extension) = self.state {
            if let Some(save) = unsafe { (*extension).save } {
                let mut bytes = Vec::new();
                let mut context = WriteContext { bytes: &mut bytes };
                let stream = clap_ostream {
                    ctx: (&mut context as *mut WriteContext).cast(),
                    write: Some(stream_write),
                };
                if !unsafe { save(self.plugin, &stream) } {
                    return Err(PluginError::Unknown(
                        "CLAP plugin failed to save state".into(),
                    ));
                }
                state.push((crate::state::CLAP_STATE.into(), crate::state::b64_encode(&bytes)));
            }
        }

        let values: Vec<(u32, f64)> = self
            .descriptor
            .params
            .iter()
            .filter_map(|param| {
                self.plugin_param_value(param.id)
                    .or_else(|| self.cached_params.get(&param.id).copied())
                    .map(|value| (param.id, value as f64))
            })
            .collect();
        state.push((
            crate::state::PARAMETERS.into(),
            crate::state::params_to_string(&values),
        ));
        self.host.state_dirty.store(false, Ordering::Relaxed);
        Ok(state)
    }

    fn load_state(&mut self, state: &[(String, String)]) -> Result<(), PluginError> {
        if let Some(encoded) = crate::state::get(state, crate::state::CLAP_STATE) {
            let bytes = crate::state::b64_decode(encoded)?;
            let extension = self
                .state
                .ok_or_else(|| PluginError::Unknown("CLAP plugin has no state extension".into()))?;
            let load = unsafe { (*extension).load }
                .ok_or_else(|| PluginError::Unknown("CLAP plugin cannot load state".into()))?;
            let mut context = ReadContext { bytes, position: 0 };
            let stream = clap_istream {
                ctx: (&mut context as *mut ReadContext).cast(),
                read: Some(stream_read),
            };
            if !unsafe { load(self.plugin, &stream) } {
                return Err(PluginError::Unknown(
                    "CLAP plugin failed to load state".into(),
                ));
            }
        }

        if let Some(encoded) = crate::state::get(state, crate::state::PARAMETERS) {
            for (id, value) in crate::state::params_from_string(encoded) {
                self.set_param(id, value as f32)?;
            }
        }

        self.refresh_cached_params();
        Ok(())
    }

    fn has_editor(&self) -> bool {
        self.gui_api_supported(Self::platform_gui_api(), false)
            || self.gui_api_supported(Self::platform_gui_api(), true)
    }

    fn open_editor(&mut self, parent: EditorParent) -> Result<(u32, u32), PluginError> {
        if self.editor_open {
            self.close_gui();
        }
        self.open_gui(parent)
    }

    fn close_editor(&mut self) {
        self.close_gui();
    }

    fn set_param(&mut self, id: u32, value: f32) -> Result<(), PluginError> {
        let param = self
            .descriptor
            .params
            .iter()
            .find(|param| param.id == id)
            .ok_or_else(|| PluginError::Unknown(format!("unknown CLAP parameter {id}")))?;
        let value = value.clamp(param.min, param.max);
        self.cached_params.insert(id, value);
        self.pending_events.push(Self::make_param_event(id, value));

        if !self.active {
            self.flush_pending_events();
        }
        Ok(())
    }

    fn get_param(&self, id: u32) -> Option<f32> {
        self.plugin_param_value(id)
            .or_else(|| self.cached_params.get(&id).copied())
    }

    fn activate(&mut self, sample_rate: u32, max_block: usize) {
        self.deactivate();
        self.max_block = max_block.max(1);
        let block_u32 = self.max_block.min(u32::MAX as usize) as u32;

        self.scratch_inputs = (0..self.input_channels)
            .map(|_| vec![0.0; self.max_block])
            .collect();
        self.scratch_outputs = (0..self.output_channels)
            .map(|_| vec![0.0; self.max_block])
            .collect();
        self.input_ptrs = Vec::with_capacity(self.input_channels as usize);
        self.output_ptrs = Vec::with_capacity(self.output_channels as usize);

        let Some(activate) = (unsafe { (*self.plugin).activate }) else {
            return;
        };
        if !unsafe { activate(self.plugin, sample_rate as f64, 1, block_u32) } {
            return;
        }
        if let Some(start) = unsafe { (*self.plugin).start_processing } {
            if !unsafe { start(self.plugin) } {
                if let Some(deactivate) = unsafe { (*self.plugin).deactivate } {
                    unsafe { deactivate(self.plugin) };
                }
                return;
            }
        }
        self.active = true;
    }

    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [Vec<f32>]) {
        for output in outputs.iter_mut() {
            output.fill(0.0);
        }
        let frames = outputs.first().map_or(0, Vec::len);
        if frames == 0 {
            return;
        }
        if !self.active {
            self.activate(48_000, frames);
        }
        if !self.active {
            return;
        }

        self.run_main_thread_callback_if_requested();
        let mut offset = 0;
        while offset < frames {
            let block = (frames - offset).min(self.max_block);
            let mut block_outputs: Vec<Vec<f32>> =
                outputs.iter().map(|_| vec![0.0; block]).collect();
            self.process_chunk(inputs, &mut block_outputs, offset);
            for (destination, source) in outputs.iter_mut().zip(block_outputs.iter()) {
                let end = (offset + block).min(destination.len());
                let count = end.saturating_sub(offset).min(source.len());
                destination[offset..offset + count].copy_from_slice(&source[..count]);
            }
            offset += block;
        }
    }
}

impl Drop for ClapInstance {
    fn drop(&mut self) {
        self.close_gui();
        self.deactivate();
        if let Some(destroy) = unsafe { (*self.plugin).destroy } {
            unsafe { destroy(self.plugin) };
        }
        let _ = &self.module;
    }
}
