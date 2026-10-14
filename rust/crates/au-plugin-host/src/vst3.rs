use au_plugin_api::{
    EditorParent, ParamInfo, PluginDescriptor, PluginError, PluginFormat, PluginHost,
    PluginInstance,
};
#[cfg(not(target_os = "linux"))]
use libloading::Library;
use std::collections::HashMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use vst3::Steinberg::Vst::{
    BusDirections_::*, BusTypes_::*, MediaTypes_::*, ProcessModes_::*,
    SpeakerArr::{kMono, kStereo}, SymbolicSampleSizes_::*,
};
use vst3::Steinberg::IBStream_::IStreamSeekMode_::*;
use vst3::{
    Class, ComPtr, ComWrapper, Interface, Steinberg::*, Steinberg::Linux::*, Steinberg::Vst::*,
};

const AUDIO_MODULE_CLASS: &str = "Audio Module Class";

fn load_error(message: impl Into<String>) -> PluginError {
    PluginError::LoadFailed(message.into())
}

fn result_ok(result: tresult) -> bool {
    result == kResultOk
}

fn char_array_string(bytes: &[std::ffi::c_char]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let bytes: Vec<u8> = bytes[..end].iter().map(|&b| b as u8).collect();
    String::from_utf8_lossy(&bytes).trim().to_owned()
}

fn string128_string(value: &String128) -> String {
    let end = value.iter().position(|&c| c == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}

fn path_uid(descriptor: &PluginDescriptor) -> Result<(PathBuf, String), PluginError> {
    match crate::state::parse_effect_id(&descriptor.id) {
        Some((PluginFormat::Vst3, path, uid)) => Ok((path, uid)),
        _ => Err(load_error(format!(
            "invalid VST3 effect id: {}",
            descriptor.id
        ))),
    }
}

fn is_vst3_path(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("vst3"))
}

fn walk_candidates(
    path: &Path,
    depth: usize,
    seen: &mut HashMap<PathBuf, ()>,
    out: &mut Vec<PathBuf>,
) {
    if is_vst3_path(path) {
        if seen.insert(path.to_path_buf(), ()).is_none() {
            out.push(path.to_path_buf());
        }
        return;
    }

    if depth >= 4 {
        return;
    }

    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };

    let mut entries: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for entry in entries {
        if is_vst3_path(&entry) {
            if seen.insert(entry.clone(), ()).is_none() {
                out.push(entry);
            }
        } else if entry.is_dir() {
            walk_candidates(&entry, depth + 1, seen, out);
        }
    }
}

pub fn candidates(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashMap::new();
    let mut result = Vec::new();
    for path in paths {
        walk_candidates(path, 0, &mut seen, &mut result);
    }
    result
}

pub fn module_binary(bundle: &Path) -> Option<PathBuf> {
    if bundle.is_file() {
        return Some(bundle.to_path_buf());
    }

    let stem = bundle.file_stem()?.to_string_lossy();
    #[cfg(target_os = "linux")]
    {
        #[cfg(target_arch = "aarch64")]
        let architecture = "aarch64-linux";
        #[cfg(not(target_arch = "aarch64"))]
        let architecture = "x86_64-linux";
        Some(
            bundle
                .join("Contents")
                .join(architecture)
                .join(format!("{stem}.so")),
        )
    }
    #[cfg(target_os = "windows")]
    {
        Some(
            bundle
                .join("Contents")
                .join("x86_64-win")
                .join(format!("{stem}.vst3")),
        )
    }
    #[cfg(target_os = "macos")]
    {
        Some(bundle.join("Contents").join("MacOS").join(stem.as_ref()))
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        None
    }
}

struct Vst3Module {
    factory: Option<ComPtr<IPluginFactory>>,
    #[cfg(target_os = "linux")]
    library: Option<libloading::os::unix::Library>,
    #[cfg(not(target_os = "linux"))]
    library: Option<Library>,
    entry_started: bool,
}

impl Vst3Module {
    fn open(bundle: &Path) -> Result<Self, PluginError> {
        let binary =
            module_binary(bundle).ok_or_else(|| load_error("unsupported VST3 bundle layout"))?;

        #[cfg(target_os = "linux")]
        {
            let library = unsafe { libloading::os::unix::Library::new(&binary) }
                .map_err(|e| load_error(format!("cannot load {}: {e}", binary.display())))?;
            let raw_handle = library.into_raw();
            // SAFETY: The raw handle was produced by `into_raw` and is transferred back to
            // an owning Library immediately; it remains alive until this module is dropped.
            let library = unsafe { libloading::os::unix::Library::from_raw(raw_handle) };
            let entry_started = if let Ok(symbol) =
                unsafe { library.get::<unsafe extern "C" fn(*mut c_void) -> bool>(b"ModuleEntry\0") }
            {
                let entry = *symbol;
                if !unsafe { entry(raw_handle) } {
                    return Err(load_error(format!(
                        "VST3 ModuleEntry failed for {}",
                        binary.display()
                    )));
                }
                true
            } else {
                return Err(load_error(format!(
                    "{} does not export ModuleEntry",
                    binary.display()
                )));
            };

            let factory_ptr = match unsafe {
                library.get::<unsafe extern "C" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0")
            } {
                Ok(symbol) => unsafe { (*symbol)() },
                Err(e) => {
                    if let Ok(exit) =
                        unsafe { library.get::<unsafe extern "C" fn() -> bool>(b"ModuleExit\0") }
                    {
                        let _ = unsafe { (*exit)() };
                    }
                    return Err(load_error(format!(
                        "{} has no GetPluginFactory: {e}",
                        binary.display()
                    )));
                }
            };

            if factory_ptr.is_null() {
                if let Ok(exit) =
                    unsafe { library.get::<unsafe extern "C" fn() -> bool>(b"ModuleExit\0") }
                {
                    let _ = unsafe { (*exit)() };
                }
                return Err(load_error("VST3 returned a null plugin factory"));
            }

            // SAFETY: GetPluginFactory returns an owned COM reference.
            let factory = unsafe { ComPtr::<IPluginFactory>::from_raw(factory_ptr) }
                .ok_or_else(|| load_error("VST3 returned an invalid plugin factory"))?;
            set_factory_host_context(&factory);
            return Ok(Self {
                factory: Some(factory),
                library: Some(library),
                entry_started,
            });
        }

        #[cfg(not(target_os = "linux"))]
        {
            let library = unsafe { Library::new(&binary) }
                .map_err(|e| load_error(format!("cannot load {}: {e}", binary.display())))?;

            #[cfg(target_os = "windows")]
            let entry_started = {
                if let Ok(symbol) =
                    unsafe { library.get::<unsafe extern "system" fn() -> bool>(b"InitDll\0") }
                {
                    if !unsafe { (*symbol)() } {
                        return Err(load_error(format!(
                            "VST3 InitDll failed for {}",
                            binary.display()
                        )));
                    }
                    true
                } else {
                    false
                }
            };
            #[cfg(target_os = "macos")]
            let entry_started = false;
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            let entry_started = false;

            let factory_ptr = unsafe {
                library
                    .get::<unsafe extern "C" fn() -> *mut IPluginFactory>(b"GetPluginFactory\0")
            }
            .map_err(|e| load_error(format!("{} has no GetPluginFactory: {e}", binary.display())))?;
            let factory_ptr = unsafe { (*factory_ptr)() };
            if factory_ptr.is_null() {
                return Err(load_error("VST3 returned a null plugin factory"));
            }

            // SAFETY: GetPluginFactory returns an owned COM reference.
            let factory = unsafe { ComPtr::<IPluginFactory>::from_raw(factory_ptr) }
                .ok_or_else(|| load_error("VST3 returned an invalid plugin factory"))?;
            set_factory_host_context(&factory);
            Ok(Self {
                factory: Some(factory),
                library: Some(library),
                entry_started,
            })
        }
    }

    fn factory(&self) -> &ComPtr<IPluginFactory> {
        self.factory.as_ref().expect("module factory is available")
    }
}

impl Drop for Vst3Module {
    fn drop(&mut self) {
        // Release the factory before invoking the module's exit function.
        self.factory.take();

        #[cfg(target_os = "linux")]
        if self.entry_started {
            if let Some(library) = self.library.as_ref() {
                if let Ok(exit) =
                    unsafe { library.get::<unsafe extern "C" fn() -> bool>(b"ModuleExit\0") }
                {
                    let _ = unsafe { (*exit)() };
                }
            }
        }

        #[cfg(target_os = "windows")]
        if self.entry_started {
            if let Some(library) = self.library.as_ref() {
                if let Ok(exit) =
                    unsafe { library.get::<unsafe extern "system" fn() -> bool>(b"ExitDll\0") }
                {
                    let _ = unsafe { (*exit)() };
                }
            }
        }

        let _ = self.library.take();
    }
}

fn module_cache() -> &'static Mutex<HashMap<PathBuf, Weak<Vst3Module>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Weak<Vst3Module>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn get_module(path: &Path) -> Result<Arc<Vst3Module>, PluginError> {
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let mut cache = module_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(module) = cache.get(&key).and_then(Weak::upgrade) {
        return Ok(module);
    }
    let module = Arc::new(Vst3Module::open(path)?);
    cache.insert(key, Arc::downgrade(&module));
    Ok(module)
}

fn class_uid(cid: &TUID) -> String {
    cid.iter()
        .map(|byte| format!("{:02X}", *byte as u8))
        .collect()
}

fn uid_bytes(uid: &str) -> Option<TUID> {
    if uid.len() != 32 {
        return None;
    }
    let mut bytes = [0 as std::ffi::c_char; 16];
    for (index, slot) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        let value = u8::from_str_radix(&uid[start..start + 2], 16).ok()?;
        *slot = value as std::ffi::c_char;
    }
    Some(bytes)
}

pub fn scan_file(path: &Path) -> Result<Vec<PluginDescriptor>, PluginError> {
    let module = get_module(path)?;
    let factory = module.factory();
    let mut factory_info: PFactoryInfo = unsafe { std::mem::zeroed() };
    let vendor = if result_ok(unsafe { factory.getFactoryInfo(&mut factory_info) }) {
        char_array_string(&factory_info.vendor)
    } else {
        String::new()
    };

    let count = unsafe { factory.countClasses() }.max(0);
    let mut descriptors = Vec::new();
    for index in 0..count {
        let mut info: PClassInfo = unsafe { std::mem::zeroed() };
        if !result_ok(unsafe { factory.getClassInfo(index, &mut info) }) {
            continue;
        }
        if char_array_string(&info.category) != AUDIO_MODULE_CLASS {
            continue;
        }

        let uid = class_uid(&info.cid);
        let name = char_array_string(&info.name);
        let n_inputs = class_channel_count(factory, &info.cid, kInput as i32);
        let n_outputs = class_channel_count(factory, &info.cid, kOutput as i32);
        let params = class_parameters(factory, &info.cid);
        descriptors.push(PluginDescriptor {
            id: crate::state::effect_id(PluginFormat::Vst3, path, &uid, &vendor, &name),
            name,
            vendor: vendor.clone(),
            format: PluginFormat::Vst3,
            path: Some(path.to_path_buf()),
            params,
            n_inputs,
            n_outputs,
        });
    }
    Ok(descriptors)
}

fn class_channel_count(
    factory: &ComPtr<IPluginFactory>,
    cid: &TUID,
    direction: BusDirection,
) -> u32 {
    let mut raw: *mut c_void = std::ptr::null_mut();
    let created = unsafe {
        factory.createInstance(
            cid.as_ptr(),
            IComponent::IID.as_ptr().cast(),
            &mut raw,
        )
    };
    if !result_ok(created) || raw.is_null() {
        return 0;
    }
    // SAFETY: createInstance returned an owned IComponent reference.
    let component: ComPtr<IComponent> = unsafe {
        ComPtr::<IComponent>::from_raw(raw.cast())
            .expect("createInstance returned a non-null IComponent")
    };
    if !initialize_component(&component) {
        return 0;
    }
    let mut bus: BusInfo = unsafe { std::mem::zeroed() };
    let channels = if result_ok(unsafe {
        component.getBusInfo(kAudio as i32, direction, 0, &mut bus)
    }) && bus.busType == kMain as i32
    {
        bus.channelCount.max(0) as u32
    } else {
        0
    };
    let _ = unsafe { component.terminate() };
    channels
}

fn class_parameters(factory: &ComPtr<IPluginFactory>, cid: &TUID) -> Vec<ParamInfo> {
    let mut raw: *mut c_void = std::ptr::null_mut();
    let created = unsafe {
        factory.createInstance(
            cid.as_ptr(),
            IComponent::IID.as_ptr().cast(),
            &mut raw,
        )
    };
    if !result_ok(created) || raw.is_null() {
        return Vec::new();
    }
    // SAFETY: createInstance returned an owned IComponent reference.
    let component: ComPtr<IComponent> = unsafe {
        ComPtr::<IComponent>::from_raw(raw.cast())
            .expect("createInstance returned a non-null IComponent")
    };
    if !initialize_component(&component) {
        return Vec::new();
    }
    let mut controller: Option<ComPtr<IEditController>> = component.cast();
    let mut separate = false;
    if controller.is_none() {
        let mut controller_cid = [0 as std::ffi::c_char; 16];
        if result_ok(unsafe { component.getControllerClassId(&mut controller_cid) }) {
            let mut controller_raw: *mut c_void = std::ptr::null_mut();
            if result_ok(unsafe {
                factory.createInstance(
                    controller_cid.as_ptr(),
                    IEditController::IID.as_ptr().cast(),
                    &mut controller_raw,
                )
            }) && !controller_raw.is_null()
            {
                // SAFETY: createInstance returned an owned IEditController reference.
                let created: ComPtr<IEditController> = unsafe {
                    ComPtr::<IEditController>::from_raw(controller_raw.cast())
                        .expect("createInstance returned a non-null IEditController")
                };
                let host_ptr = host_application_ptr();
                if result_ok(unsafe { created.initialize(host_ptr.as_ptr().cast()) }) {
                    controller = Some(created);
                    separate = true;
                }
            }
        }
    }

    let params = controller
        .as_ref()
        .map(|controller| {
            let count = unsafe { controller.getParameterCount() }.max(0);
            (0..count)
                .filter_map(|index| {
                    let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
                    if !result_ok(unsafe { controller.getParameterInfo(index, &mut info) }) {
                        return None;
                    }
                    Some(ParamInfo {
                        id: info.id,
                        name: string128_string(&info.title),
                        min: 0.0,
                        max: 1.0,
                        default: info.defaultNormalizedValue as f32,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    if separate {
        if let Some(controller) = controller {
            let _ = unsafe { controller.terminate() };
        }
    }
    let _ = unsafe { component.terminate() };
    params
}

pub struct Vst3Host;

impl Vst3Host {
    pub fn new() -> Self {
        Self
    }
}

impl Default for Vst3Host {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost for Vst3Host {
    fn format(&self) -> PluginFormat {
        PluginFormat::Vst3
    }

    fn default_search_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(value) = std::env::var_os("VST3_PATH") {
            paths.extend(std::env::split_paths(&value));
        }

        #[cfg(target_os = "linux")]
        {
            if let Some(home) = std::env::var_os("HOME") {
                paths.push(PathBuf::from(home).join(".vst3"));
            }
            paths.extend([
                PathBuf::from("/usr/lib/vst3"),
                PathBuf::from("/usr/local/lib/vst3"),
                PathBuf::from("/usr/lib/x86_64-linux-gnu/vst3"),
                PathBuf::from("/usr/lib/aarch64-linux-gnu/vst3"),
            ]);
        }
        #[cfg(target_os = "windows")]
        {
            if let Some(program_files) = std::env::var_os("ProgramFiles") {
                paths.push(
                    PathBuf::from(program_files)
                        .join("Common Files")
                        .join("VST3"),
                );
            }
            if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
                paths.push(
                    PathBuf::from(local_app_data)
                        .join("Programs")
                        .join("Common")
                        .join("VST3"),
                );
            }
        }
        #[cfg(target_os = "macos")]
        {
            paths.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
            if let Some(home) = std::env::var_os("HOME") {
                paths.push(
                    PathBuf::from(home)
                        .join("Library")
                        .join("Audio")
                        .join("Plug-Ins")
                        .join("VST3"),
                );
            }
        }
        paths
    }

    fn scan(&self, paths: &[PathBuf]) -> Vec<PluginDescriptor> {
        candidates(paths)
            .into_iter()
            .filter_map(|path| scan_file(&path).ok())
            .flatten()
            .collect()
    }

    fn instantiate(
        &self,
        descriptor: &PluginDescriptor,
    ) -> Result<Box<dyn PluginInstance>, PluginError> {
        let (path, uid) = path_uid(descriptor)?;
        let module = get_module(&path)?;
        let cid = uid_bytes(&uid).ok_or_else(|| load_error("invalid VST3 class UID"))?;
        let mut raw: *mut c_void = std::ptr::null_mut();
        let result = unsafe {
            module.factory().createInstance(
                cid.as_ptr(),
                IComponent::IID.as_ptr().cast(),
                &mut raw,
            )
        };
        if !result_ok(result) || raw.is_null() {
            return Err(load_error(format!(
                "cannot create VST3 component {}",
                descriptor.id
            )));
        }
        // SAFETY: createInstance returned an owned IComponent reference.
        let component: ComPtr<IComponent> = unsafe {
            ComPtr::<IComponent>::from_raw(raw.cast())
                .expect("createInstance returned a non-null IComponent")
        };
        let host_ptr = host_application_ptr();
        if !result_ok(unsafe { component.initialize(host_ptr.as_ptr().cast()) }) {
            return Err(load_error("VST3 component initialization failed"));
        }

        let processor = match component.cast::<IAudioProcessor>() {
            Some(processor) => processor,
            None => {
                let _ = unsafe { component.terminate() };
                return Err(load_error("VST3 component has no audio processor"));
            }
        };

        let (controller, separate_controller) = match component.cast::<IEditController>() {
            Some(controller) => (controller, false),
            None => {
                let mut controller_cid = [0 as std::ffi::c_char; 16];
                if !result_ok(unsafe { component.getControllerClassId(&mut controller_cid) }) {
                    let _ = unsafe { component.terminate() };
                    return Err(load_error("VST3 component has no edit controller"));
                }
                let mut controller_raw: *mut c_void = std::ptr::null_mut();
                if !result_ok(unsafe {
                    module.factory().createInstance(
                        controller_cid.as_ptr(),
                        IEditController::IID.as_ptr().cast(),
                        &mut controller_raw,
                    )
                }) || controller_raw.is_null()
                {
                    let _ = unsafe { component.terminate() };
                    return Err(load_error("cannot create VST3 edit controller"));
                }
                // SAFETY: createInstance returned an owned IEditController reference.
                let controller: ComPtr<IEditController> = unsafe {
                    ComPtr::<IEditController>::from_raw(controller_raw.cast())
                        .expect("createInstance returned a non-null IEditController")
                };
                if !result_ok(unsafe {
                    controller.initialize(host_ptr.as_ptr().cast())
                }) {
                    let _ = unsafe { component.terminate() };
                    return Err(load_error("VST3 controller initialization failed"));
                }
                (controller, true)
            }
        };

        let handler_state = Arc::new(Mutex::new(Vec::new()));
        let handler = ComWrapper::new(ComponentHandler {
            edits: handler_state.clone(),
        });
        let handler_ptr = handler
            .to_com_ptr::<IComponentHandler>()
            .expect("ComponentHandler exposes IComponentHandler");
        let _ = unsafe { controller.setComponentHandler(handler_ptr.as_ptr()) };

        let mut component_connection: Option<ComPtr<IConnectionPoint>> =
            component.cast();
        let mut controller_connection: Option<ComPtr<IConnectionPoint>> =
            controller.cast();
        if separate_controller {
            if let (Some(component_point), Some(controller_point)) =
                (&component_connection, &controller_connection)
            {
                let _ = unsafe { component_point.connect(controller_point.as_ptr()) };
                let _ = unsafe { controller_point.connect(component_point.as_ptr()) };
            }
        }

        let stream = MemoryStream::new(Vec::new());
        let stream_ptr = stream
            .wrapper
            .to_com_ptr::<IBStream>()
            .expect("MemoryStream exposes IBStream");
        if result_ok(unsafe { component.getState(stream_ptr.as_ptr()) }) {
            let _ = unsafe {
                stream_ptr.seek(0, kIBSeekSet as i32, std::ptr::null_mut())
            };
            let _ = unsafe { controller.setComponentState(stream_ptr.as_ptr()) };
        }

        let n_inputs = component_channel_count(&component, kInput as i32);
        let n_outputs = component_channel_count(&component, kOutput as i32);
        let params = controller_parameters(&controller);
        let mut instance = Vst3Instance {
            descriptor: PluginDescriptor {
                id: descriptor.id.clone(),
                name: descriptor.name.clone(),
                vendor: descriptor.vendor.clone(),
                format: PluginFormat::Vst3,
                path: Some(path),
                params,
                n_inputs,
                n_outputs,
            },
            module: Some(module),
            component: Some(component),
            processor: Some(processor),
            controller: Some(controller),
            handler: Some(handler),
            handler_state,
            separate_controller,
            component_connection: component_connection.take(),
            controller_connection: controller_connection.take(),
            parameter_changes_state: Arc::new(Mutex::new(Vec::new())),
            parameter_changes: None,
            active: false,
            sample_rate: 48_000,
            max_block: 0,
            input_scratch: Vec::new(),
            output_scratch: Vec::new(),
            input_ptrs: Vec::new(),
            output_ptrs: Vec::new(),
            input_bus_count: usize::from(n_inputs > 0),
            output_bus_count: usize::from(n_outputs > 0),
            editor: None,
            frame: None,
        };
        instance.parameter_changes = Some(ComWrapper::new(ParamChanges {
            queues: instance.parameter_changes_state.clone(),
        }));

        Ok(Box::new(instance))
    }
}

fn component_channel_count(component: &ComPtr<IComponent>, direction: BusDirection) -> u32 {
    let mut info: BusInfo = unsafe { std::mem::zeroed() };
    if result_ok(unsafe {
        component.getBusInfo(kAudio as i32, direction, 0, &mut info)
    }) && info.busType == kMain as i32
    {
        info.channelCount.max(0) as u32
    } else {
        0
    }
}

fn controller_parameters(controller: &ComPtr<IEditController>) -> Vec<ParamInfo> {
    let count = unsafe { controller.getParameterCount() }.max(0);
    (0..count)
        .filter_map(|index| {
            let mut info: ParameterInfo = unsafe { std::mem::zeroed() };
            if !result_ok(unsafe { controller.getParameterInfo(index, &mut info) }) {
                return None;
            }
            Some(ParamInfo {
                id: info.id,
                name: string128_string(&info.title),
                min: 0.0,
                max: 1.0,
                default: info.defaultNormalizedValue as f32,
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
struct LoopState {
    fds: Vec<(usize, Linux::FileDescriptor)>,
    timers: Vec<(usize, Linux::TimerInterval, std::time::Instant)>,
}

struct HostContext {
    #[cfg(target_os = "linux")]
    state: Mutex<LoopState>,
}

impl Class for HostContext {
    #[cfg(target_os = "linux")]
    type Interfaces = (IHostApplication, Linux::IRunLoop);

    #[cfg(not(target_os = "linux"))]
    type Interfaces = (IHostApplication,);
}

impl IHostApplicationTrait for HostContext {
    unsafe fn getName(&self, name: *mut String128) -> tresult {
        if name.is_null() {
            return kInvalidArgument;
        }
        let mut result = [0u16; 128];
        for (slot, unit) in result.iter_mut().zip("Audacity-RS".encode_utf16()) {
            *slot = unit;
        }
        // SAFETY: The caller supplies a writable String128 pointer.
        unsafe { *name = result };
        kResultOk
    }

    unsafe fn createInstance(
        &self,
        _cid: *mut TUID,
        _iid: *mut TUID,
        obj: *mut *mut c_void,
    ) -> tresult {
        if obj.is_null() {
            return kInvalidArgument;
        }
        // SAFETY: The caller supplies a writable object pointer.
        unsafe { *obj = std::ptr::null_mut() };
        kResultFalse
    }
}

fn host_context() -> &'static ComWrapper<HostContext> {
    static HOST_CONTEXT: OnceLock<ComWrapper<HostContext>> = OnceLock::new();
    HOST_CONTEXT.get_or_init(|| {
        #[cfg(target_os = "linux")]
        {
            ComWrapper::new(HostContext {
                state: Mutex::new(LoopState {
                    fds: Vec::new(),
                    timers: Vec::new(),
                }),
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            ComWrapper::new(HostContext {})
        }
    })
}

fn host_application_ptr() -> ComPtr<IHostApplication> {
    host_context()
        .to_com_ptr::<IHostApplication>()
        .expect("HostContext exposes IHostApplication")
}

fn initialize_component(component: &ComPtr<IComponent>) -> bool {
    let host_ptr = host_application_ptr();
    result_ok(unsafe { component.initialize(host_ptr.as_ptr().cast()) })
}

fn set_factory_host_context(factory: &ComPtr<IPluginFactory>) {
    if let Some(factory3) = factory.cast::<IPluginFactory3>() {
        let host_ptr = host_application_ptr();
        let _ = unsafe { factory3.setHostContext(host_ptr.as_ptr().cast::<FUnknown>()) };
    }
}

#[cfg(target_os = "linux")]
impl Linux::IRunLoopTrait for HostContext {
    unsafe fn registerEventHandler(
        &self,
        handler: *mut Linux::IEventHandler,
        fd: Linux::FileDescriptor,
    ) -> tresult {
        register_event_handler(handler, fd)
    }

    unsafe fn unregisterEventHandler(&self, handler: *mut Linux::IEventHandler) -> tresult {
        unregister_event_handler(handler)
    }

    unsafe fn registerTimer(
        &self,
        handler: *mut Linux::ITimerHandler,
        milliseconds: Linux::TimerInterval,
    ) -> tresult {
        register_timer(handler, milliseconds)
    }

    unsafe fn unregisterTimer(&self, handler: *mut Linux::ITimerHandler) -> tresult {
        unregister_timer(handler)
    }
}

struct ComponentHandler {
    edits: Arc<Mutex<Vec<(ParamID, ParamValue)>>>,
}

impl Class for ComponentHandler {
    type Interfaces = (IComponentHandler,);
}

impl IComponentHandlerTrait for ComponentHandler {
    unsafe fn beginEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn performEdit(&self, id: ParamID, value_normalized: ParamValue) -> tresult {
        self.edits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((id, value_normalized));
        kResultOk
    }

    unsafe fn endEdit(&self, _id: ParamID) -> tresult {
        kResultOk
    }

    unsafe fn restartComponent(&self, _flags: int32) -> tresult {
        kResultOk
    }
}

struct MemoryStream {
    data: Arc<Mutex<(Vec<u8>, usize)>>,
    wrapper: ComWrapper<MemoryStreamCom>,
}

impl MemoryStream {
    fn new(bytes: Vec<u8>) -> Self {
        let data = Arc::new(Mutex::new((bytes, 0)));
        let wrapper = ComWrapper::new(MemoryStreamCom {
            data: data.clone(),
        });
        Self { data, wrapper }
    }

    fn bytes(&self) -> Vec<u8> {
        self.data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .0
            .clone()
    }
}

struct MemoryStreamCom {
    data: Arc<Mutex<(Vec<u8>, usize)>>,
}

impl Class for MemoryStreamCom {
    type Interfaces = (IBStream,);
}

impl IBStreamTrait for MemoryStreamCom {
    unsafe fn read(
        &self,
        buffer: *mut c_void,
        num_bytes: int32,
        num_bytes_read: *mut int32,
    ) -> tresult {
        if !num_bytes_read.is_null() {
            // SAFETY: The caller supplies a writable byte count pointer.
            unsafe { *num_bytes_read = 0 };
        }
        if num_bytes < 0 || (num_bytes > 0 && buffer.is_null()) {
            return kInvalidArgument;
        }

        let mut data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let available = data.0.len().saturating_sub(data.1);
        let count = available.min(num_bytes as usize);
        if count > 0 {
            // SAFETY: The source range is within the vector and the destination is checked non-null.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    data.0.as_ptr().add(data.1),
                    buffer.cast::<u8>(),
                    count,
                );
            }
            data.1 += count;
        }
        if !num_bytes_read.is_null() {
            // SAFETY: The caller supplies a writable byte count pointer.
            unsafe { *num_bytes_read = count as int32 };
        }
        kResultOk
    }

    unsafe fn write(
        &self,
        buffer: *mut c_void,
        num_bytes: int32,
        num_bytes_written: *mut int32,
    ) -> tresult {
        if !num_bytes_written.is_null() {
            // SAFETY: The caller supplies a writable byte count pointer.
            unsafe { *num_bytes_written = 0 };
        }
        if num_bytes < 0 || (num_bytes > 0 && buffer.is_null()) {
            return kInvalidArgument;
        }

        let count = num_bytes as usize;
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let end = match data.1.checked_add(count) {
            Some(end) => end,
            None => return kInvalidArgument,
        };
        if end > data.0.len() {
            data.0.resize(end, 0);
        }
        if count > 0 {
            // SAFETY: The destination range is resized and the input pointer is checked non-null.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    buffer.cast::<u8>(),
                    data.0.as_mut_ptr().add(data.1),
                    count,
                );
            }
        }
        data.1 = end;
        if !num_bytes_written.is_null() {
            // SAFETY: The caller supplies a writable byte count pointer.
            unsafe { *num_bytes_written = count as int32 };
        }
        kResultOk
    }

    unsafe fn seek(&self, pos: int64, mode: int32, result: *mut int64) -> tresult {
        let mut data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let base = if mode == kIBSeekSet as i32 {
            0i64
        } else if mode == kIBSeekCur as i32 {
            data.1 as i64
        } else if mode == kIBSeekEnd as i32 {
            data.0.len() as i64
        } else {
            return kInvalidArgument;
        };
        let Some(position) = base.checked_add(pos) else {
            return kInvalidArgument;
        };
        if position < 0 || position as u128 > usize::MAX as u128 {
            return kInvalidArgument;
        }
        data.1 = position as usize;
        if !result.is_null() {
            // SAFETY: The caller supplies a writable seek-result pointer.
            unsafe { *result = position };
        }
        kResultOk
    }

    unsafe fn tell(&self, pos: *mut int64) -> tresult {
        if pos.is_null() {
            return kInvalidArgument;
        }
        let data = self
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: The caller supplies a writable stream-position pointer.
        unsafe { *pos = data.1 as int64 };
        kResultOk
    }
}

struct ParamQueue {
    id: ParamID,
    points: Mutex<Vec<(int32, ParamValue)>>,
}

impl Class for ParamQueue {
    type Interfaces = (IParamValueQueue,);
}

impl IParamValueQueueTrait for ParamQueue {
    unsafe fn getParameterId(&self) -> ParamID {
        self.id
    }

    unsafe fn getPointCount(&self) -> int32 {
        self.points
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len() as int32
    }

    unsafe fn getPoint(
        &self,
        index: int32,
        sample_offset: *mut int32,
        value: *mut ParamValue,
    ) -> tresult {
        if index < 0 || sample_offset.is_null() || value.is_null() {
            return kInvalidArgument;
        }
        let points = self
            .points
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(&(offset, point_value)) = points.get(index as usize) else {
            return kResultFalse;
        };
        // SAFETY: Both output pointers were checked non-null.
        unsafe {
            *sample_offset = offset;
            *value = point_value;
        }
        kResultOk
    }

    unsafe fn addPoint(
        &self,
        sample_offset: int32,
        value: ParamValue,
        index: *mut int32,
    ) -> tresult {
        let mut points = self
            .points
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !index.is_null() {
            // SAFETY: The caller supplies a writable point-index pointer.
            unsafe { *index = points.len() as int32 };
        }
        points.push((sample_offset, value));
        kResultOk
    }
}

struct ParamChanges {
    queues: Arc<Mutex<Vec<ComWrapper<ParamQueue>>>>,
}

impl Class for ParamChanges {
    type Interfaces = (IParameterChanges,);
}

impl IParameterChangesTrait for ParamChanges {
    unsafe fn getParameterCount(&self) -> int32 {
        self.queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len() as int32
    }

    unsafe fn getParameterData(&self, index: int32) -> *mut IParamValueQueue {
        if index < 0 {
            return std::ptr::null_mut();
        }
        let queues = self
            .queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queues
            .get(index as usize)
            .and_then(|queue| {
                let pointer = queue.to_com_ptr::<IParamValueQueue>()?;
                let raw = pointer.as_ptr();
                std::mem::forget(pointer);
                Some(raw)
            })
            .unwrap_or(std::ptr::null_mut())
    }

    unsafe fn addParameterData(
        &self,
        _id: *const ParamID,
        _index: *mut int32,
    ) -> *mut IParamValueQueue {
        std::ptr::null_mut()
    }
}

fn make_param_queues(events: &[(ParamID, ParamValue)]) -> Vec<ComWrapper<ParamQueue>> {
    events
        .iter()
        .map(|&(id, value)| {
            ComWrapper::new(ParamQueue {
                id,
                points: Mutex::new(vec![(0, value)]),
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn register_event_handler(handler: *mut Linux::IEventHandler, fd: Linux::FileDescriptor) -> tresult {
    if handler.is_null() {
        return kInvalidArgument;
    }
    let address = handler as usize;
    let mut state = host_context()
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.fds.retain(|(registered, _)| *registered != address);
    state.fds.push((address, fd));
    kResultOk
}

#[cfg(target_os = "linux")]
fn unregister_event_handler(handler: *mut Linux::IEventHandler) -> tresult {
    let address = handler as usize;
    host_context()
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .fds
        .retain(|(registered, _)| *registered != address);
    kResultOk
}

#[cfg(target_os = "linux")]
fn register_timer(
    handler: *mut Linux::ITimerHandler,
    milliseconds: Linux::TimerInterval,
) -> tresult {
    if handler.is_null() {
        return kInvalidArgument;
    }
    let address = handler as usize;
    let interval = std::time::Duration::from_millis(milliseconds.max(1) as u64);
    let mut state = host_context()
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state
        .timers
        .retain(|(registered, _, _)| *registered != address);
    state.timers.push((
        address,
        milliseconds.max(1),
        std::time::Instant::now() + interval,
    ));
    kResultOk
}

#[cfg(target_os = "linux")]
fn unregister_timer(handler: *mut Linux::ITimerHandler) -> tresult {
    let address = handler as usize;
    host_context()
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .timers
        .retain(|(registered, _, _)| *registered != address);
    kResultOk
}

#[cfg(target_os = "linux")]
pub fn run_loop_idle() {
    let now = std::time::Instant::now();
    let (fds, due_timers) = {
        let mut state = host_context()
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let fds = state.fds.clone();
        let mut due = Vec::new();
        for timer in &mut state.timers {
            if timer.2 <= now {
                due.push(timer.0);
                timer.2 = now + std::time::Duration::from_millis(timer.1.max(1) as u64);
            }
        }
        (fds, due)
    };

    for (address, fd) in fds {
        // SAFETY: The plugin's registered handler remains owned by the editor until it unregisters.
        let Some(handler) = (unsafe {
            ComPtr::<Linux::IEventHandler>::from_raw(address as *mut Linux::IEventHandler)
        }) else {
            continue;
        };
        unsafe { handler.onFDIsSet(fd) };
        std::mem::forget(handler);
    }
    for address in due_timers {
        // SAFETY: The plugin's registered handler remains owned by the editor until it unregisters.
        let Some(handler) = (unsafe {
            ComPtr::<Linux::ITimerHandler>::from_raw(address as *mut Linux::ITimerHandler)
        }) else {
            continue;
        };
        unsafe { handler.onTimer() };
        std::mem::forget(handler);
    }
}

#[cfg(not(target_os = "linux"))]
pub fn run_loop_idle() {}

struct PlugFrame;

impl Class for PlugFrame {
    #[cfg(target_os = "linux")]
    type Interfaces = (IPlugFrame, Linux::IRunLoop);

    #[cfg(not(target_os = "linux"))]
    type Interfaces = (IPlugFrame,);
}

impl IPlugFrameTrait for PlugFrame {
    unsafe fn resizeView(&self, _view: *mut IPlugView, _new_size: *mut ViewRect) -> tresult {
        kResultOk
    }
}

#[cfg(target_os = "linux")]
impl Linux::IRunLoopTrait for PlugFrame {
    unsafe fn registerEventHandler(
        &self,
        handler: *mut Linux::IEventHandler,
        fd: Linux::FileDescriptor,
    ) -> tresult {
        register_event_handler(handler, fd)
    }

    unsafe fn unregisterEventHandler(&self, handler: *mut Linux::IEventHandler) -> tresult {
        unregister_event_handler(handler)
    }

    unsafe fn registerTimer(
        &self,
        handler: *mut Linux::ITimerHandler,
        milliseconds: Linux::TimerInterval,
    ) -> tresult {
        register_timer(handler, milliseconds)
    }

    unsafe fn unregisterTimer(&self, handler: *mut Linux::ITimerHandler) -> tresult {
        unregister_timer(handler)
    }
}

type EditorFrame = ComWrapper<PlugFrame>;

pub struct Vst3Instance {
    descriptor: PluginDescriptor,
    module: Option<Arc<Vst3Module>>,
    component: Option<ComPtr<IComponent>>,
    processor: Option<ComPtr<IAudioProcessor>>,
    controller: Option<ComPtr<IEditController>>,
    handler: Option<ComWrapper<ComponentHandler>>,
    handler_state: Arc<Mutex<Vec<(ParamID, ParamValue)>>>,
    separate_controller: bool,
    component_connection: Option<ComPtr<IConnectionPoint>>,
    controller_connection: Option<ComPtr<IConnectionPoint>>,
    parameter_changes_state: Arc<Mutex<Vec<ComWrapper<ParamQueue>>>>,
    parameter_changes: Option<ComWrapper<ParamChanges>>,
    active: bool,
    sample_rate: u32,
    max_block: usize,
    input_scratch: Vec<Vec<f32>>,
    output_scratch: Vec<Vec<f32>>,
    input_ptrs: Vec<*mut Sample32>,
    output_ptrs: Vec<*mut Sample32>,
    input_bus_count: usize,
    output_bus_count: usize,
    editor: Option<ComPtr<IPlugView>>,
    frame: Option<EditorFrame>,
}

// All COM objects are used serially by Audacity's main/processing thread.
unsafe impl Send for Vst3Instance {}

impl Vst3Instance {
    fn processor(&self) -> &ComPtr<IAudioProcessor> {
        self.processor.as_ref().expect("VST3 processor is available")
    }

    fn controller(&self) -> &ComPtr<IEditController> {
        self.controller.as_ref().expect("VST3 controller is available")
    }

    fn component(&self) -> &ComPtr<IComponent> {
        self.component.as_ref().expect("VST3 component is available")
    }

    fn queue_param(&mut self, id: ParamID, value: ParamValue) {
        self.parameter_changes_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(ComWrapper::new(ParamQueue {
                id,
                points: Mutex::new(vec![(0, value)]),
            }));
    }

    fn gather_handler_edits(&mut self) {
        let edits = {
            let mut state = self
                .handler_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *state)
        };
        if !edits.is_empty() {
            let mut queues = self
                .parameter_changes_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            queues.extend(make_param_queues(&edits));
        }
    }

    fn save_stream(&self, component: bool) -> Result<Vec<u8>, PluginError> {
        let stream = MemoryStream::new(Vec::new());
        let pointer = stream
            .wrapper
            .to_com_ptr::<IBStream>()
            .expect("MemoryStream exposes IBStream");
        let status = unsafe {
            if component {
                self.component().getState(pointer.as_ptr())
            } else {
                self.controller().getState(pointer.as_ptr())
            }
        };
        if !result_ok(status) {
            return Err(load_error("VST3 state serialization failed"));
        }
        Ok(stream.bytes())
    }

    fn load_component_state(&mut self, bytes: &[u8]) -> Result<(), PluginError> {
        let stream = MemoryStream::new(bytes.to_vec());
        let pointer = stream
            .wrapper
            .to_com_ptr::<IBStream>()
            .expect("MemoryStream exposes IBStream");
        if !result_ok(unsafe { self.component().setState(pointer.as_ptr()) }) {
            return Err(load_error("VST3 processor state restore failed"));
        }
        let _ = unsafe {
            pointer.seek(0, kIBSeekSet as i32, std::ptr::null_mut())
        };
        if !result_ok(unsafe { self.controller().setComponentState(pointer.as_ptr()) }) {
            return Err(load_error("VST3 controller component-state restore failed"));
        }
        Ok(())
    }

    fn load_controller_state(&mut self, bytes: &[u8]) -> Result<(), PluginError> {
        let stream = MemoryStream::new(bytes.to_vec());
        let pointer = stream
            .wrapper
            .to_com_ptr::<IBStream>()
            .expect("MemoryStream exposes IBStream");
        if !result_ok(unsafe { self.controller().setState(pointer.as_ptr()) }) {
            return Err(load_error("VST3 controller state restore failed"));
        }
        Ok(())
    }

    fn restore_params(&mut self, text: &str) {
        for (id, value) in crate::state::params_from_string(text) {
            let normalized = value.clamp(0.0, 1.0);
            let _ = unsafe { self.controller().setParamNormalized(id, normalized) };
            self.queue_param(id, normalized);
        }
    }

    fn process_chunk(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [Vec<f32>],
        start: usize,
        count: usize,
    ) {
        let input_channels = self.descriptor.n_inputs as usize;
        let output_channels = self.descriptor.n_outputs as usize;
        let max_block = self.max_block.max(1);

        for channel in 0..input_channels {
            let scratch = &mut self.input_scratch[channel];
            scratch[..count].fill(0.0);
            let source = if inputs.len() == 1 && input_channels >= 2 && channel < 2 {
                inputs.first().copied()
            } else {
                inputs.get(channel).copied()
            };
            if let Some(source) = source {
                let available = source.len().saturating_sub(start).min(count);
                scratch[..available].copy_from_slice(&source[start..start + available]);
            }
            self.input_ptrs[channel] = scratch.as_mut_ptr();
        }

        for channel in 0..output_channels {
            let scratch = &mut self.output_scratch[channel];
            scratch[..count].fill(0.0);
            self.output_ptrs[channel] = scratch.as_mut_ptr();
        }

        let mut input_bus = AudioBusBuffers {
            numChannels: input_channels as int32,
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 {
                channelBuffers32: self.input_ptrs.as_mut_ptr(),
            },
        };
        let mut output_bus = AudioBusBuffers {
            numChannels: output_channels as int32,
            silenceFlags: 0,
            __field0: AudioBusBuffers__type0 {
                channelBuffers32: self.output_ptrs.as_mut_ptr(),
            },
        };

        self.gather_handler_edits();
        let pending = {
            let mut state = self
                .parameter_changes_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *state)
        };
        let mut events = Vec::with_capacity(pending.len());
        for queue in &pending {
            let queue_ptr = queue
                .to_com_ptr::<IParamValueQueue>()
                .expect("ParamQueue exposes IParamValueQueue");
            let mut offset = 0;
            let mut value = 0.0;
            if result_ok(unsafe { queue_ptr.getPoint(0, &mut offset, &mut value) }) {
                events.push((unsafe { queue_ptr.getParameterId() }, value));
            }
        }
        let new_queues = make_param_queues(&events);
        {
            let mut state = self
                .parameter_changes_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *state = new_queues;
        }

        let change_ptr = self
            .parameter_changes
            .as_ref()
            .and_then(|changes| changes.to_com_ptr::<IParameterChanges>());
        let mut data = ProcessData {
            processMode: kRealtime as i32,
            symbolicSampleSize: kSample32 as i32,
            numSamples: count.min(max_block) as int32,
            numInputs: self.input_bus_count as int32,
            numOutputs: self.output_bus_count as int32,
            inputs: if self.input_bus_count > 0 {
                &mut input_bus
            } else {
                std::ptr::null_mut()
            },
            outputs: if self.output_bus_count > 0 {
                &mut output_bus
            } else {
                std::ptr::null_mut()
            },
            inputParameterChanges: if events.is_empty() {
                std::ptr::null_mut()
            } else {
                change_ptr
                    .as_ref()
                    .map(|pointer| pointer.as_ptr())
                    .unwrap_or(std::ptr::null_mut())
            },
            outputParameterChanges: std::ptr::null_mut(),
            inputEvents: std::ptr::null_mut(),
            outputEvents: std::ptr::null_mut(),
            processContext: std::ptr::null_mut(),
        };

        let status = unsafe { self.processor().process(&mut data) };
        if !result_ok(status) {
            for output in outputs.iter_mut() {
                let available = output.len().saturating_sub(start).min(count);
                output[start..start + available].fill(0.0);
            }
            return;
        }

        for channel in 0..outputs.len() {
            let source_channel = if channel < output_channels { channel } else { 0 };
            let available = outputs[channel].len().saturating_sub(start).min(count);
            if output_channels > 0 && available > 0 {
                outputs[channel][start..start + available]
                    .copy_from_slice(&self.output_scratch[source_channel][..available]);
            }
        }
    }

    pub fn idle_editor(&mut self) {
        run_loop_idle();
    }
}

impl PluginInstance for Vst3Instance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn latency_samples(&self) -> u32 {
        unsafe { self.processor().getLatencySamples() }
    }

    fn save_state(&mut self) -> Result<Vec<(String, String)>, PluginError> {
        let mut state = Vec::new();
        let processor_state = self.save_stream(true)?;
        state.push((
            crate::state::PROCESSOR_STATE.to_owned(),
            crate::state::b64_encode(&processor_state),
        ));

        if let Ok(controller_state) = self.save_stream(false) {
            if !controller_state.is_empty() {
                state.push((
                    crate::state::CONTROLLER_STATE.to_owned(),
                    crate::state::b64_encode(&controller_state),
                ));
            }
        }

        let params: Vec<(u32, f64)> = self
            .descriptor
            .params
            .iter()
            .map(|parameter| {
                (
                    parameter.id,
                    unsafe { self.controller().getParamNormalized(parameter.id) },
                )
            })
            .collect();
        state.push((
            crate::state::PARAMETERS.to_owned(),
            crate::state::params_to_string(&params),
        ));
        Ok(state)
    }

    fn load_state(&mut self, state: &[(String, String)]) -> Result<(), PluginError> {
        if let Some(encoded) = crate::state::get(state, crate::state::PROCESSOR_STATE) {
            let bytes = crate::state::b64_decode(encoded)?;
            self.load_component_state(&bytes)?;
        }
        if let Some(encoded) = crate::state::get(state, crate::state::CONTROLLER_STATE) {
            let bytes = crate::state::b64_decode(encoded)?;
            self.load_controller_state(&bytes)?;
        }
        if let Some(params) = crate::state::get(state, crate::state::PARAMETERS) {
            self.restore_params(params);
        }
        Ok(())
    }

    fn has_editor(&self) -> bool {
        !self.descriptor.params.is_empty()
            || self.controller.as_ref().is_some_and(|controller| unsafe {
                let raw = controller.createView(b"editor\0".as_ptr().cast());
                if raw.is_null() {
                    false
                } else {
                    ComPtr::<IPlugView>::from_raw(raw).is_some()
                }
            })
    }

    fn open_editor(&mut self, parent: EditorParent) -> Result<(u32, u32), PluginError> {
        self.close_editor();
        let platform_type: &'static [u8] = match parent {
            EditorParent::X11(_) => {
                #[cfg(target_os = "linux")]
                {
                    b"X11EmbedWindowID\0"
                }
                #[cfg(not(target_os = "linux"))]
                {
                    return Err(load_error("X11 editor embedding is unavailable on this platform"));
                }
            }
            EditorParent::Win32(_) => {
                #[cfg(target_os = "windows")]
                {
                    b"HWND\0"
                }
                #[cfg(not(target_os = "windows"))]
                {
                    return Err(load_error("Win32 editor embedding is unavailable on this platform"));
                }
            }
            EditorParent::Cocoa(_) => {
                #[cfg(target_os = "macos")]
                {
                    b"NSView\0"
                }
                #[cfg(not(target_os = "macos"))]
                {
                    return Err(load_error("Cocoa editor embedding is unavailable on this platform"));
                }
            }
            EditorParent::Floating => {
                return Err(load_error("VST3 editors require a parent window"));
            }
        };

        let raw = unsafe { self.controller().createView(b"editor\0".as_ptr().cast()) };
        if raw.is_null() {
            return Err(load_error("VST3 plugin has no editor"));
        }
        // SAFETY: createView returns an owned IPlugView reference.
        let view = unsafe {
            ComPtr::<IPlugView>::from_raw(raw).expect("createView returned a non-null IPlugView")
        };
        if !result_ok(unsafe {
            view.isPlatformTypeSupported(platform_type.as_ptr().cast())
        }) {
            return Err(load_error("VST3 editor does not support this window system"));
        }

        let frame = ComWrapper::new(PlugFrame);
        let frame_ptr = frame
            .to_com_ptr::<IPlugFrame>()
            .expect("PlugFrame exposes IPlugFrame");
        let _ = unsafe { view.setFrame(frame_ptr.as_ptr()) };

        let parent_ptr = match parent {
            EditorParent::X11(window) => window as usize as *mut c_void,
            EditorParent::Win32(window) | EditorParent::Cocoa(window) => window as *mut c_void,
            EditorParent::Floating => std::ptr::null_mut(),
        };
        if !result_ok(unsafe {
            view.attached(parent_ptr, platform_type.as_ptr().cast())
        }) {
            let _ = unsafe { view.setFrame(std::ptr::null_mut()) };
            return Err(load_error("VST3 editor could not attach to its parent"));
        }

        let mut rect: ViewRect = unsafe { std::mem::zeroed() };
        if !result_ok(unsafe { view.getSize(&mut rect) }) {
            let _ = unsafe { view.removed() };
            let _ = unsafe { view.setFrame(std::ptr::null_mut()) };
            return Err(load_error("VST3 editor did not provide its size"));
        }
        let width = (rect.right - rect.left).max(0) as u32;
        let height = (rect.bottom - rect.top).max(0) as u32;
        self.editor = Some(view);
        self.frame = Some(frame);
        Ok((width, height))
    }

    fn close_editor(&mut self) {
        if let Some(view) = self.editor.take() {
            let _ = unsafe { view.removed() };
            let _ = unsafe { view.setFrame(std::ptr::null_mut()) };
        }
        self.frame.take();
    }

    fn set_param(&mut self, id: u32, value: f32) -> Result<(), PluginError> {
        if !self.descriptor.params.iter().any(|parameter| parameter.id == id) {
            return Err(load_error(format!("unknown VST3 parameter {id}")));
        }
        let value = (value as f64).clamp(0.0, 1.0);
        if !result_ok(unsafe { self.controller().setParamNormalized(id, value) }) {
            return Err(load_error(format!("VST3 rejected parameter {id}")));
        }
        self.queue_param(id, value);
        Ok(())
    }

    fn get_param(&self, id: u32) -> Option<f32> {
        self.descriptor
            .params
            .iter()
            .any(|parameter| parameter.id == id)
            .then(|| unsafe { self.controller().getParamNormalized(id) as f32 })
    }

    fn activate(&mut self, sample_rate: u32, max_block: usize) {
        if self.active {
            let _ = unsafe { self.processor().setProcessing(0) };
            let _ = unsafe { self.component().setActive(0) };
            self.active = false;
        }

        self.sample_rate = sample_rate.max(1);
        self.max_block = max_block.max(1);
        let mut setup = ProcessSetup {
            processMode: kRealtime as i32,
            symbolicSampleSize: kSample32 as i32,
            maxSamplesPerBlock: self.max_block.min(int32::MAX as usize) as int32,
            sampleRate: self.sample_rate as f64,
        };
        let _ = unsafe { self.processor().setupProcessing(&mut setup) };

        let mut input_arrangement: SpeakerArrangement = match self.descriptor.n_inputs {
            1 => kMono,
            2 => kStereo,
            _ => 0,
        };
        let mut output_arrangement: SpeakerArrangement = match self.descriptor.n_outputs {
            1 => kMono,
            2 => kStereo,
            _ => 0,
        };
        let input_pointer = &mut input_arrangement as *mut SpeakerArrangement;
        let output_pointer = &mut output_arrangement as *mut SpeakerArrangement;
        let _ = unsafe {
            self.processor().setBusArrangements(
                if self.input_bus_count > 0 {
                    input_pointer
                } else {
                    std::ptr::null_mut()
                },
                self.input_bus_count as int32,
                if self.output_bus_count > 0 {
                    output_pointer
                } else {
                    std::ptr::null_mut()
                },
                self.output_bus_count as int32,
            )
        };

        if self.input_bus_count > 0 {
            let _ = unsafe { self.component().activateBus(kAudio as i32, kInput as i32, 0, 1) };
        }
        if self.output_bus_count > 0 {
            let _ = unsafe { self.component().activateBus(kAudio as i32, kOutput as i32, 0, 1) };
        }
        if result_ok(unsafe { self.component().setActive(1) }) {
            if result_ok(unsafe { self.processor().setProcessing(1) }) {
                self.active = true;
            } else {
                let _ = unsafe { self.component().setActive(0) };
            }
        }

        self.input_scratch = (0..self.descriptor.n_inputs)
            .map(|_| vec![0.0; self.max_block])
            .collect();
        self.output_scratch = (0..self.descriptor.n_outputs)
            .map(|_| vec![0.0; self.max_block])
            .collect();
        self.input_ptrs = vec![std::ptr::null_mut(); self.descriptor.n_inputs as usize];
        self.output_ptrs = vec![std::ptr::null_mut(); self.descriptor.n_outputs as usize];
    }

    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [Vec<f32>]) {
        let frames = outputs
            .iter()
            .map(Vec::len)
            .max()
            .unwrap_or_else(|| inputs.iter().map(|input| input.len()).max().unwrap_or(0));
        if !self.active {
            self.activate(48_000, frames.max(1));
        }
        let mut start = 0;
        while start < frames {
            let count = (frames - start).min(self.max_block.max(1));
            self.process_chunk(inputs, outputs, start, count);
            start += count;
        }
    }
}

impl Drop for Vst3Instance {
    fn drop(&mut self) {
        self.close_editor();
        if self.active {
            if let Some(processor) = self.processor.as_ref() {
                let _ = unsafe { processor.setProcessing(0) };
            }
            if let Some(component) = self.component.as_ref() {
                let _ = unsafe { component.setActive(0) };
            }
            self.active = false;
        }

        if self.separate_controller {
            if let (Some(component_point), Some(controller_point)) =
                (&self.component_connection, &self.controller_connection)
            {
                let _ = unsafe { component_point.disconnect(controller_point.as_ptr()) };
                let _ = unsafe { controller_point.disconnect(component_point.as_ptr()) };
            }
        }

        if let Some(controller) = self.controller.as_ref() {
            let _ = unsafe { controller.setComponentHandler(std::ptr::null_mut()) };
            if self.separate_controller {
                let _ = unsafe { controller.terminate() };
            }
        }
        if let Some(component) = self.component.as_ref() {
            let _ = unsafe { component.terminate() };
        }

        self.component_connection.take();
        self.controller_connection.take();
        self.processor.take();
        self.controller.take();
        self.component.take();
        self.handler.take();
        self.parameter_changes.take();
        self.module.take();
    }
}
