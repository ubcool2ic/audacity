// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! Runtime-loaded LV2 plugin host backed by Lilv.

use au_plugin_api::{
    ParamInfo, PluginDescriptor, PluginError, PluginFormat, PluginHost, PluginInstance,
};
use libloading::Library;
use std::{
    collections::HashMap,
    env,
    ffi::{c_char, c_void, CStr, CString},
    path::PathBuf,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

const LILV_LIBRARY_NAMES: &[&str] = &[
    "liblilv-0.so.0",
    "liblilv-0.so",
    "liblilv-0.dylib",
    "lilv-0.dll",
];

const URI_AUDIO_PORT: &str = "http://lv2plug.in/ns/lv2core#AudioPort";
const URI_CONTROL_PORT: &str = "http://lv2plug.in/ns/lv2core#ControlPort";
const URI_INPUT_PORT: &str = "http://lv2plug.in/ns/lv2core#InputPort";
const URI_OUTPUT_PORT: &str = "http://lv2plug.in/ns/lv2core#OutputPort";
const URI_CONNECTION_OPTIONAL: &str = "http://lv2plug.in/ns/lv2core#connectionOptional";
const URI_INTEGER: &str = "http://lv2plug.in/ns/lv2core#integer";
const URI_TOGGLED: &str = "http://lv2plug.in/ns/lv2core#toggled";
const URI_LOGARITHMIC: &str = "http://lv2plug.in/ns/ext/port-props#logarithmic";
const URI_REPORTS_LATENCY: &str = "http://lv2plug.in/ns/lv2core#reportsLatency";

const URI_FEATURE_URID_MAP: &[u8] = b"http://lv2plug.in/ns/ext/urid#map\0";
const URI_FEATURE_URID_UNMAP: &[u8] = b"http://lv2plug.in/ns/ext/urid#unmap\0";
const URI_FEATURE_OPTIONS: &[u8] = b"http://lv2plug.in/ns/ext/options#options\0";
const URI_FEATURE_BOUNDED_BLOCK: &[u8] =
    b"http://lv2plug.in/ns/ext/buf-size#boundedBlockLength\0";

const URI_ATOM_INT: &str = "http://lv2plug.in/ns/ext/atom#Int";
const URI_ATOM_FLOAT: &str = "http://lv2plug.in/ns/ext/atom#Float";
const URI_OPTION_NOMINAL_BLOCK: &str =
    "http://lv2plug.in/ns/ext/buf-size#nominalBlockLength";
const URI_OPTION_MAX_BLOCK: &str = "http://lv2plug.in/ns/ext/buf-size#maxBlockLength";
const URI_OPTION_MIN_BLOCK: &str = "http://lv2plug.in/ns/ext/buf-size#minBlockLength";
const URI_OPTION_SAMPLE_RATE: &str = "http://lv2plug.in/ns/ext/parameters#sampleRate";

type LilvWorld = c_void;
type LilvPlugin = c_void;
type LilvPort = c_void;
type LilvNode = c_void;
type LilvPlugins = c_void;
type LilvIter = c_void;
type LilvInstancePtr = *mut LilvInstance;

type WorldNew = unsafe extern "C" fn() -> *mut LilvWorld;
type WorldLoadAll = unsafe extern "C" fn(*mut LilvWorld);
type WorldSetOption =
    unsafe extern "C" fn(*mut LilvWorld, *const c_char, *const LilvNode);
type WorldGetAllPlugins = unsafe extern "C" fn(*const LilvWorld) -> *const LilvPlugins;
type WorldFree = unsafe extern "C" fn(*mut LilvWorld);
type NewUri = unsafe extern "C" fn(*mut LilvWorld, *const c_char) -> *mut LilvNode;
type NewString = unsafe extern "C" fn(*mut LilvWorld, *const c_char) -> *mut LilvNode;
type NodeFree = unsafe extern "C" fn(*mut LilvNode);
type NodeAsString = unsafe extern "C" fn(*const LilvNode) -> *const c_char;
type NodeIsFloat = unsafe extern "C" fn(*const LilvNode) -> bool;
type NodeAsFloat = unsafe extern "C" fn(*const LilvNode) -> f32;
type NodeIsInt = unsafe extern "C" fn(*const LilvNode) -> bool;
type NodeAsInt = unsafe extern "C" fn(*const LilvNode) -> i32;
type PluginsBegin = unsafe extern "C" fn(*const LilvPlugins) -> *mut LilvIter;
type PluginsIsEnd = unsafe extern "C" fn(*const LilvPlugins, *const LilvIter) -> bool;
type PluginsGet =
    unsafe extern "C" fn(*const LilvPlugins, *const LilvIter) -> *const LilvPlugin;
type PluginsNext =
    unsafe extern "C" fn(*const LilvPlugins, *mut LilvIter) -> *mut LilvIter;
type PluginGetUri = unsafe extern "C" fn(*const LilvPlugin) -> *const LilvNode;
type PluginGetName = unsafe extern "C" fn(*const LilvPlugin) -> *mut LilvNode;
type PluginGetAuthorName = unsafe extern "C" fn(*const LilvPlugin) -> *mut LilvNode;
type PluginGetNumPorts = unsafe extern "C" fn(*const LilvPlugin) -> u32;
type PluginGetPortByIndex =
    unsafe extern "C" fn(*const LilvPlugin, u32) -> *const LilvPort;
type PortIsA =
    unsafe extern "C" fn(*const LilvPlugin, *const LilvPort, *const LilvNode) -> bool;
type PortGetSymbol =
    unsafe extern "C" fn(*const LilvPlugin, *const LilvPort) -> *const LilvNode;
type PortGetName =
    unsafe extern "C" fn(*const LilvPlugin, *const LilvPort) -> *mut LilvNode;
type PortGetRange = unsafe extern "C" fn(
    *const LilvPlugin,
    *const LilvPort,
    *mut *mut LilvNode,
    *mut *mut LilvNode,
    *mut *mut LilvNode,
);
type PortHasProperty =
    unsafe extern "C" fn(*const LilvPlugin, *const LilvPort, *const LilvNode) -> bool;
type PluginGetPortRangesFloat =
    unsafe extern "C" fn(*const LilvPlugin, *mut f32, *mut f32, *mut f32);
type PluginHasLatency = unsafe extern "C" fn(*const LilvPlugin) -> bool;
type PluginGetLatencyPortIndex = unsafe extern "C" fn(*const LilvPlugin) -> u32;
type PluginInstantiate = unsafe extern "C" fn(
    *const LilvPlugin,
    f64,
    *const *const Lv2Feature,
) -> LilvInstancePtr;
type InstanceFree = unsafe extern "C" fn(LilvInstancePtr);

struct LilvApi {
    _library: Library,
    world_new: WorldNew,
    world_load_all: WorldLoadAll,
    world_set_option: WorldSetOption,
    world_get_all_plugins: WorldGetAllPlugins,
    world_free: WorldFree,
    new_uri: NewUri,
    new_string: NewString,
    node_free: NodeFree,
    node_as_string: NodeAsString,
    node_is_float: NodeIsFloat,
    node_as_float: NodeAsFloat,
    node_is_int: NodeIsInt,
    node_as_int: NodeAsInt,
    plugins_begin: PluginsBegin,
    plugins_is_end: PluginsIsEnd,
    plugins_get: PluginsGet,
    plugins_next: PluginsNext,
    plugin_get_uri: PluginGetUri,
    plugin_get_name: PluginGetName,
    plugin_get_author_name: PluginGetAuthorName,
    plugin_get_num_ports: PluginGetNumPorts,
    plugin_get_port_by_index: PluginGetPortByIndex,
    port_is_a: PortIsA,
    port_get_symbol: PortGetSymbol,
    port_get_name: PortGetName,
    port_get_range: PortGetRange,
    port_has_property: PortHasProperty,
    plugin_get_port_ranges_float: PluginGetPortRangesFloat,
    plugin_has_latency: PluginHasLatency,
    plugin_get_latency_port_index: PluginGetLatencyPortIndex,
    plugin_instantiate: PluginInstantiate,
    instance_free: InstanceFree,
}

// Lilv's function pointers are immutable after loading, and the library is held
// for their entire lifetime. Worlds and plugin instances are owned separately.
unsafe impl Send for LilvApi {}
unsafe impl Sync for LilvApi {}

impl LilvApi {
    fn load() -> Result<Arc<Self>, PluginError> {
        let mut last_error = String::new();
        let mut loaded = None;

        for name in LILV_LIBRARY_NAMES {
            // SAFETY: The library handle is retained in LilvApi for the lifetime
            // of every resolved function pointer.
            match unsafe { Library::new(name) } {
                Ok(library) => {
                    loaded = Some(library);
                    break;
                }
                Err(error) => last_error = error.to_string(),
            }
        }

        let library = loaded.ok_or_else(|| {
            PluginError::LoadFailed(format!(
                "Lilv is missing: could not load liblilv-0.so.0 ({last_error})"
            ))
        })?;

        macro_rules! resolve {
            ($name:literal, $ty:ty) => {{
                // SAFETY: The requested symbol has the declared Lilv C ABI and
                // the Library is retained by the resulting LilvApi.
                let symbol: libloading::Symbol<'_, $ty> =
                    unsafe { library.get(concat!($name, "\0").as_bytes()) }.map_err(|e| {
                        PluginError::LoadFailed(format!(
                            "Lilv is missing symbol {}: {e}",
                            $name
                        ))
                    })?;
                *symbol
            }};
        }

        let api = Self {
            world_new: resolve!("lilv_world_new", WorldNew),
            world_load_all: resolve!("lilv_world_load_all", WorldLoadAll),
            world_set_option: resolve!("lilv_world_set_option", WorldSetOption),
            world_get_all_plugins: resolve!("lilv_world_get_all_plugins", WorldGetAllPlugins),
            world_free: resolve!("lilv_world_free", WorldFree),
            new_uri: resolve!("lilv_new_uri", NewUri),
            new_string: resolve!("lilv_new_string", NewString),
            node_free: resolve!("lilv_node_free", NodeFree),
            node_as_string: resolve!("lilv_node_as_string", NodeAsString),
            node_is_float: resolve!("lilv_node_is_float", NodeIsFloat),
            node_as_float: resolve!("lilv_node_as_float", NodeAsFloat),
            node_is_int: resolve!("lilv_node_is_int", NodeIsInt),
            node_as_int: resolve!("lilv_node_as_int", NodeAsInt),
            plugins_begin: resolve!("lilv_plugins_begin", PluginsBegin),
            plugins_is_end: resolve!("lilv_plugins_is_end", PluginsIsEnd),
            plugins_get: resolve!("lilv_plugins_get", PluginsGet),
            plugins_next: resolve!("lilv_plugins_next", PluginsNext),
            plugin_get_uri: resolve!("lilv_plugin_get_uri", PluginGetUri),
            plugin_get_name: resolve!("lilv_plugin_get_name", PluginGetName),
            plugin_get_author_name: resolve!("lilv_plugin_get_author_name", PluginGetAuthorName),
            plugin_get_num_ports: resolve!("lilv_plugin_get_num_ports", PluginGetNumPorts),
            plugin_get_port_by_index: resolve!(
                "lilv_plugin_get_port_by_index",
                PluginGetPortByIndex
            ),
            port_is_a: resolve!("lilv_port_is_a", PortIsA),
            port_get_symbol: resolve!("lilv_port_get_symbol", PortGetSymbol),
            port_get_name: resolve!("lilv_port_get_name", PortGetName),
            port_get_range: resolve!("lilv_port_get_range", PortGetRange),
            port_has_property: resolve!("lilv_port_has_property", PortHasProperty),
            plugin_get_port_ranges_float: resolve!(
                "lilv_plugin_get_port_ranges_float",
                PluginGetPortRangesFloat
            ),
            plugin_has_latency: resolve!("lilv_plugin_has_latency", PluginHasLatency),
            plugin_get_latency_port_index: resolve!(
                "lilv_plugin_get_latency_port_index",
                PluginGetLatencyPortIndex
            ),
            plugin_instantiate: resolve!("lilv_plugin_instantiate", PluginInstantiate),
            instance_free: resolve!("lilv_instance_free", InstanceFree),
            _library: library,
        };
        Ok(Arc::new(api))
    }
}

#[repr(C)]
struct Lv2Feature {
    uri: *const c_char,
    data: *mut c_void,
}

type UridMapCallback =
    unsafe extern "C" fn(handle: *mut c_void, uri: *const c_char) -> u32;
type UridUnmapCallback =
    unsafe extern "C" fn(handle: *mut c_void, urid: u32) -> *const c_char;

#[repr(C)]
struct Lv2UridMap {
    handle: *mut c_void,
    map: Option<UridMapCallback>,
}

#[repr(C)]
struct Lv2UridUnmap {
    handle: *mut c_void,
    unmap: Option<UridUnmapCallback>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Lv2Option {
    context: u32,
    subject: u32,
    key: u32,
    size: usize,
    type_: u32,
    value: *const c_void,
}

#[derive(Default)]
struct UridInner {
    next_id: u32,
    by_uri: HashMap<String, u32>,
    by_id: HashMap<u32, CString>,
}

struct UridTable {
    inner: Mutex<UridInner>,
    frozen: AtomicBool,
}

impl UridTable {
    fn new() -> Arc<Self> {
        let table = Arc::new(Self {
            inner: Mutex::new(UridInner {
                next_id: 1,
                ..UridInner::default()
            }),
            frozen: AtomicBool::new(false),
        });

        for uri in [
            URI_ATOM_INT,
            URI_ATOM_FLOAT,
            URI_OPTION_NOMINAL_BLOCK,
            URI_OPTION_MAX_BLOCK,
            URI_OPTION_MIN_BLOCK,
            URI_OPTION_SAMPLE_RATE,
            "http://lv2plug.in/ns/ext/urid#map",
            "http://lv2plug.in/ns/ext/urid#unmap",
        ] {
            table.map_uri(uri);
        }
        table
    }

    fn map_uri(&self, uri: &str) -> u32 {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(id) = inner.by_uri.get(uri) {
            return *id;
        }
        if self.frozen.load(Ordering::Acquire) {
            return 0;
        }

        let Ok(c_uri) = CString::new(uri) else {
            return 0;
        };
        let id = inner.next_id;
        if id == 0 {
            return 0;
        }
        inner.next_id = inner.next_id.wrapping_add(1);
        inner.by_uri.insert(uri.to_owned(), id);
        inner.by_id.insert(id, c_uri);
        id
    }

    fn unmap_uri(&self, id: u32) -> *const c_char {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner
            .by_id
            .get(&id)
            .map_or(ptr::null(), |uri| uri.as_ptr())
    }
}

// SAFETY: The callback handle is an Arc-owned UridTable retained by the host
// instance for as long as an LV2 instance can call these functions.
unsafe extern "C" fn urid_map_callback(handle: *mut c_void, uri: *const c_char) -> u32 {
    if handle.is_null() || uri.is_null() {
        return 0;
    }
    // SAFETY: The host places a live UridTable pointer in the feature handle.
    let table = unsafe { &*handle.cast::<UridTable>() };
    // SAFETY: LV2 passes a NUL-terminated URI string for the duration of the call.
    let uri = unsafe { CStr::from_ptr(uri) };
    uri.to_str().map_or(0, |uri| table.map_uri(uri))
}

// SAFETY: The callback handle and returned CString are owned by the live
// UridTable retained by the host instance.
unsafe extern "C" fn urid_unmap_callback(handle: *mut c_void, id: u32) -> *const c_char {
    if handle.is_null() {
        return ptr::null();
    }
    // SAFETY: The host places a live UridTable pointer in the feature handle.
    let table = unsafe { &*handle.cast::<UridTable>() };
    table.unmap_uri(id)
}

#[repr(C)]
struct Lv2Descriptor {
    uri: *const c_char,
    instantiate: Option<
        unsafe extern "C" fn(
            descriptor: *const Lv2Descriptor,
            sample_rate: f64,
            bundle_path: *const c_char,
            features: *const *const Lv2Feature,
        ) -> *mut c_void,
    >,
    connect_port:
        Option<unsafe extern "C" fn(instance: *mut c_void, port: u32, data: *mut c_void)>,
    activate: Option<unsafe extern "C" fn(instance: *mut c_void)>,
    run: Option<unsafe extern "C" fn(instance: *mut c_void, sample_count: u32)>,
    deactivate: Option<unsafe extern "C" fn(instance: *mut c_void)>,
    cleanup: Option<unsafe extern "C" fn(instance: *mut c_void)>,
    extension_data: Option<unsafe extern "C" fn(uri: *const c_char) -> *const c_void>,
}

#[repr(C)]
struct LilvInstance {
    lv2_descriptor: *const Lv2Descriptor,
    lv2_handle: *mut c_void,
    pimpl: *mut c_void,
}

struct HostFeatures {
    map: Lv2UridMap,
    unmap: Lv2UridUnmap,
    nominal_block: u32,
    max_block: u32,
    min_block: u32,
    sample_rate: f32,
    options: [Lv2Option; 5],
    map_feature: Lv2Feature,
    unmap_feature: Lv2Feature,
    options_feature: Lv2Feature,
    bounded_feature: Lv2Feature,
    feature_ptrs: [*const Lv2Feature; 5],
}

impl HostFeatures {
    fn new(table: &Arc<UridTable>, sample_rate: u32, max_block: usize) -> Box<Self> {
        let int_type = table.map_uri(URI_ATOM_INT);
        let float_type = table.map_uri(URI_ATOM_FLOAT);
        let nominal_key = table.map_uri(URI_OPTION_NOMINAL_BLOCK);
        let max_key = table.map_uri(URI_OPTION_MAX_BLOCK);
        let min_key = table.map_uri(URI_OPTION_MIN_BLOCK);
        let sample_rate_key = table.map_uri(URI_OPTION_SAMPLE_RATE);

        let placeholder = Lv2Option {
            context: 1,
            subject: 0,
            key: 0,
            size: 0,
            type_: 0,
            value: ptr::null(),
        };
        let mut features = Box::new(Self {
            map: Lv2UridMap {
                handle: ptr::null_mut(),
                map: Some(urid_map_callback),
            },
            unmap: Lv2UridUnmap {
                handle: ptr::null_mut(),
                unmap: Some(urid_unmap_callback),
            },
            nominal_block: max_block.max(1).min(u32::MAX as usize) as u32,
            max_block: max_block.max(1).min(u32::MAX as usize) as u32,
            min_block: 1,
            sample_rate: sample_rate as f32,
            options: [placeholder; 5],
            map_feature: Lv2Feature {
                uri: ptr::null(),
                data: ptr::null_mut(),
            },
            unmap_feature: Lv2Feature {
                uri: ptr::null(),
                data: ptr::null_mut(),
            },
            options_feature: Lv2Feature {
                uri: ptr::null(),
                data: ptr::null_mut(),
            },
            bounded_feature: Lv2Feature {
                uri: ptr::null(),
                data: ptr::null_mut(),
            },
            feature_ptrs: [ptr::null(); 5],
        });

        let raw = &mut *features as *mut HostFeatures;
        // SAFETY: The boxed object has a stable address, and these pointers refer
        // to fields and arrays owned by that box for its entire lifetime.
        unsafe {
            (*raw).map.handle = Arc::as_ptr(table).cast_mut().cast();
            (*raw).unmap.handle = Arc::as_ptr(table).cast_mut().cast();

            (*raw).options = [
                Lv2Option {
                    context: 1,
                    subject: 0,
                    key: nominal_key,
                    size: std::mem::size_of::<u32>(),
                    type_: int_type,
                    value: std::ptr::addr_of!((*raw).nominal_block).cast(),
                },
                Lv2Option {
                    context: 1,
                    subject: 0,
                    key: max_key,
                    size: std::mem::size_of::<u32>(),
                    type_: int_type,
                    value: std::ptr::addr_of!((*raw).max_block).cast(),
                },
                Lv2Option {
                    context: 1,
                    subject: 0,
                    key: min_key,
                    size: std::mem::size_of::<u32>(),
                    type_: int_type,
                    value: std::ptr::addr_of!((*raw).min_block).cast(),
                },
                Lv2Option {
                    context: 1,
                    subject: 0,
                    key: sample_rate_key,
                    size: std::mem::size_of::<f32>(),
                    type_: float_type,
                    value: std::ptr::addr_of!((*raw).sample_rate).cast(),
                },
                placeholder,
            ];

            (*raw).map_feature = Lv2Feature {
                uri: URI_FEATURE_URID_MAP.as_ptr().cast(),
                data: std::ptr::addr_of_mut!((*raw).map).cast(),
            };
            (*raw).unmap_feature = Lv2Feature {
                uri: URI_FEATURE_URID_UNMAP.as_ptr().cast(),
                data: std::ptr::addr_of_mut!((*raw).unmap).cast(),
            };
            (*raw).options_feature = Lv2Feature {
                uri: URI_FEATURE_OPTIONS.as_ptr().cast(),
                data: std::ptr::addr_of_mut!((*raw).options).cast(),
            };
            (*raw).bounded_feature = Lv2Feature {
                uri: URI_FEATURE_BOUNDED_BLOCK.as_ptr().cast(),
                data: ptr::null_mut(),
            };
            (*raw).feature_ptrs = [
                std::ptr::addr_of!((*raw).map_feature),
                std::ptr::addr_of!((*raw).unmap_feature),
                std::ptr::addr_of!((*raw).options_feature),
                std::ptr::addr_of!((*raw).bounded_feature),
                ptr::null(),
            ];
        }

        features
    }
}

struct UriNodes {
    api: Arc<LilvApi>,
    audio: *mut LilvNode,
    control: *mut LilvNode,
    input: *mut LilvNode,
    output: *mut LilvNode,
    optional: *mut LilvNode,
    integer: *mut LilvNode,
    toggled: *mut LilvNode,
    logarithmic: *mut LilvNode,
    reports_latency: *mut LilvNode,
}

impl UriNodes {
    fn new(api: &Arc<LilvApi>, world: *mut LilvWorld) -> Result<Self, String> {
        let make = |uri: &str| -> *mut LilvNode {
            let Ok(uri) = CString::new(uri) else {
                return ptr::null_mut();
            };
            // SAFETY: world is a live Lilv world and uri is NUL-terminated.
            unsafe { (api.new_uri)(world, uri.as_ptr()) }
        };

        let nodes = Self {
            api: Arc::clone(api),
            audio: make(URI_AUDIO_PORT),
            control: make(URI_CONTROL_PORT),
            input: make(URI_INPUT_PORT),
            output: make(URI_OUTPUT_PORT),
            optional: make(URI_CONNECTION_OPTIONAL),
            integer: make(URI_INTEGER),
            toggled: make(URI_TOGGLED),
            logarithmic: make(URI_LOGARITHMIC),
            reports_latency: make(URI_REPORTS_LATENCY),
        };

        if [
            nodes.audio,
            nodes.control,
            nodes.input,
            nodes.output,
            nodes.optional,
            nodes.integer,
            nodes.toggled,
            nodes.logarithmic,
            nodes.reports_latency,
        ]
        .iter()
        .any(|node| node.is_null())
        {
            return Err("Lilv could not allocate LV2 port metadata nodes".into());
        }
        Ok(nodes)
    }
}

impl Drop for UriNodes {
    fn drop(&mut self) {
        for node in [
            self.audio,
            self.control,
            self.input,
            self.output,
            self.optional,
            self.integer,
            self.toggled,
            self.logarithmic,
            self.reports_latency,
        ] {
            if !node.is_null() {
                // SAFETY: Every non-null node was allocated by lilv_new_uri and
                // is owned by this UriNodes value.
                unsafe { (self.api.node_free)(node) };
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PortKind {
    Audio,
    Control,
    Other,
}

struct PortMeta {
    index: u32,
    kind: PortKind,
    input: bool,
    audio_slot: Option<usize>,
    symbol: String,
    name: String,
    min: f32,
    max: f32,
    default: f32,
    integer: bool,
    toggled: bool,
    _logarithmic: bool,
    #[allow(dead_code)] // parsed port properties, kept for future UI
    optional: bool,
    #[allow(dead_code)]
    reports_latency: bool,
}

struct PluginMetadata {
    uri: String,
    name: String,
    vendor: String,
    ports: Vec<PortMeta>,
    n_inputs: u32,
    n_outputs: u32,
    latency_port: Option<u32>,
    unsupported_required: Option<String>,
}

fn node_text(api: &LilvApi, node: *const LilvNode) -> Option<String> {
    if node.is_null() {
        return None;
    }
    // SAFETY: node is a live Lilv node; the returned string is borrowed from it.
    let text = unsafe { (api.node_as_string)(node) };
    if text.is_null() {
        return None;
    }
    // SAFETY: Lilv returns a NUL-terminated string valid while node is alive.
    unsafe { CStr::from_ptr(text) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

fn node_number(api: &LilvApi, node: *const LilvNode) -> Option<f32> {
    if node.is_null() {
        return None;
    }
    // SAFETY: node is a live Lilv node obtained from a plugin port range.
    unsafe {
        if (api.node_is_float)(node) {
            Some((api.node_as_float)(node))
        } else if (api.node_is_int)(node) {
            Some((api.node_as_int)(node) as f32)
        } else {
            None
        }
    }
}

fn has_port_class(
    api: &LilvApi,
    plugin: *const LilvPlugin,
    port: *const LilvPort,
    class: *const LilvNode,
) -> bool {
    // SAFETY: all pointers are borrowed from the live world and plugin.
    unsafe { (api.port_is_a)(plugin, port, class) }
}

fn has_port_property(
    api: &LilvApi,
    plugin: *const LilvPlugin,
    port: *const LilvPort,
    property: *const LilvNode,
) -> bool {
    // SAFETY: all pointers are borrowed from the live world and plugin.
    unsafe { (api.port_has_property)(plugin, port, property) }
}

fn plugin_metadata(
    api: &Arc<LilvApi>,
    world: *mut LilvWorld,
    plugin: *const LilvPlugin,
) -> Result<PluginMetadata, String> {
    let nodes = UriNodes::new(api, world)?;
    // SAFETY: plugin is a live Lilv plugin in the supplied world.
    let uri_node = unsafe { (api.plugin_get_uri)(plugin) };
    let uri = node_text(api, uri_node).ok_or_else(|| "LV2 plugin has no URI".to_owned())?;

    // These name functions return owned nodes.
    // SAFETY: plugin is valid and the returned name nodes are owned by the caller.
    let name_node = unsafe { (api.plugin_get_name)(plugin) };
    // SAFETY: plugin is valid and the returned author node is owned by the caller.
    let author_node = unsafe { (api.plugin_get_author_name)(plugin) };
    let name = node_text(api, name_node).unwrap_or_else(|| uri.clone());
    let vendor = node_text(api, author_node).unwrap_or_default();
    if !name_node.is_null() {
        // SAFETY: these nodes were returned as owned values by Lilv.
        unsafe { (api.node_free)(name_node) };
    }
    if !author_node.is_null() {
        // SAFETY: these nodes were returned as owned values by Lilv.
        unsafe { (api.node_free)(author_node) };
    }

    let mut input_mins = Vec::new();
    let mut input_maxs = Vec::new();
    let mut input_defaults = Vec::new();
    // SAFETY: plugin is live and the arrays have one entry per plugin port.
    let port_count = unsafe { (api.plugin_get_num_ports)(plugin) } as usize;
    input_mins.resize(port_count, f32::NAN);
    input_maxs.resize(port_count, f32::NAN);
    input_defaults.resize(port_count, f32::NAN);
    // SAFETY: each range array is writable for port_count entries.
    unsafe {
        (api.plugin_get_port_ranges_float)(
            plugin,
            input_mins.as_mut_ptr(),
            input_maxs.as_mut_ptr(),
            input_defaults.as_mut_ptr(),
        )
    };

    let mut ports = Vec::with_capacity(port_count);
    let mut audio_inputs = 0usize;
    let mut audio_outputs = 0usize;
    let mut unsupported_required = None;
    let mut reports_latency_port = None;

    for index in 0..port_count {
        // SAFETY: index is within the plugin's reported port count.
        let port = unsafe { (api.plugin_get_port_by_index)(plugin, index as u32) };
        if port.is_null() {
            continue;
        }

        let is_audio = has_port_class(api, plugin, port, nodes.audio);
        let is_control = has_port_class(api, plugin, port, nodes.control);
        let input = has_port_class(api, plugin, port, nodes.input);
        let output = has_port_class(api, plugin, port, nodes.output);
        let optional = has_port_property(api, plugin, port, nodes.optional);
        let integer = has_port_property(api, plugin, port, nodes.integer);
        let toggled = has_port_property(api, plugin, port, nodes.toggled);
        let logarithmic = has_port_property(api, plugin, port, nodes.logarithmic);
        let reports_latency =
            has_port_property(api, plugin, port, nodes.reports_latency);

        let symbol_node = unsafe { (api.port_get_symbol)(plugin, port) };
        let symbol = node_text(api, symbol_node)
            .unwrap_or_else(|| format!("port_{index}"));
        // port_get_name returns an owned node.
        let name_node = unsafe { (api.port_get_name)(plugin, port) };
        let port_name = node_text(api, name_node).unwrap_or_else(|| symbol.clone());
        if !name_node.is_null() {
            // SAFETY: name_node is owned by this function.
            unsafe { (api.node_free)(name_node) };
        }

        let kind = if is_audio {
            PortKind::Audio
        } else if is_control {
            PortKind::Control
        } else {
            PortKind::Other
        };

        if kind == PortKind::Other && !optional && unsupported_required.is_none() {
            unsupported_required = Some(format!(
                "required non-audio/non-control LV2 port {port_name:?} is unsupported"
            ));
        }
        if kind != PortKind::Other && !input && !output && unsupported_required.is_none() {
            unsupported_required =
                Some(format!("LV2 port {port_name:?} has no input/output direction"));
        }

        let mut minimum = input_mins[index];
        let mut maximum = input_maxs[index];
        let mut default = input_defaults[index];

        if !minimum.is_finite() || !maximum.is_finite() || !default.is_finite() {
            let mut def_node = ptr::null_mut();
            let mut min_node = ptr::null_mut();
            let mut max_node = ptr::null_mut();
            // SAFETY: the output pointers are valid and port belongs to plugin.
            unsafe {
                (api.port_get_range)(
                    plugin,
                    port,
                    &mut def_node,
                    &mut min_node,
                    &mut max_node,
                )
            };
            if !minimum.is_finite() {
                minimum = node_number(api, min_node).unwrap_or(0.0);
            }
            if !maximum.is_finite() {
                maximum = node_number(api, max_node).unwrap_or(1.0);
            }
            if !default.is_finite() {
                default = node_number(api, def_node).unwrap_or(0.0);
            }
            for node in [def_node, min_node, max_node] {
                if !node.is_null() {
                    // SAFETY: lilv_port_get_range returned these owned nodes.
                    unsafe { (api.node_free)(node) };
                }
            }
        }

        if !minimum.is_finite() {
            minimum = 0.0;
        }
        if !maximum.is_finite() {
            maximum = 1.0;
        }
        if maximum < minimum {
            std::mem::swap(&mut minimum, &mut maximum);
        }
        if !default.is_finite() {
            default = minimum;
        }
        default = default.clamp(minimum, maximum);

        let audio_slot = if kind == PortKind::Audio {
            if input {
                let slot = audio_inputs;
                audio_inputs += 1;
                Some(slot)
            } else if output {
                let slot = audio_outputs;
                audio_outputs += 1;
                Some(slot)
            } else {
                None
            }
        } else {
            None
        };

        if reports_latency && kind == PortKind::Control && output {
            reports_latency_port = Some(index as u32);
        }

        ports.push(PortMeta {
            index: index as u32,
            kind,
            input,
            audio_slot,
            symbol,
            name: port_name,
            min: minimum,
            max: maximum,
            default,
            integer,
            toggled,
            _logarithmic: logarithmic,
            optional,
            reports_latency,
        });
    }

    let latency_port = reports_latency_port.or_else(|| {
        // SAFETY: plugin is a live Lilv plugin in the world.
        let has_latency = unsafe { (api.plugin_has_latency)(plugin) };
        if !has_latency {
            return None;
        }
        // SAFETY: valid when lilv_plugin_has_latency returned true.
        let index = unsafe { (api.plugin_get_latency_port_index)(plugin) };
        ports.iter().find_map(|port| {
            (port.index == index
                && port.kind == PortKind::Control
                && !port.input)
                .then_some(index)
        })
    });

    Ok(PluginMetadata {
        uri,
        name,
        vendor,
        ports,
        n_inputs: audio_inputs as u32,
        n_outputs: audio_outputs as u32,
        latency_port,
        unsupported_required,
    })
}

fn descriptor_from_metadata(metadata: &PluginMetadata) -> PluginDescriptor {
    PluginDescriptor {
        id: format!("lv2:{}", metadata.uri),
        name: metadata.name.clone(),
        vendor: metadata.vendor.clone(),
        format: PluginFormat::Lv2,
        path: None,
        params: metadata
            .ports
            .iter()
            .filter(|port| port.kind == PortKind::Control && port.input)
            .map(|port| ParamInfo {
                id: port.index,
                name: port.name.clone(),
                min: port.min,
                max: port.max,
                default: port.default,
            })
            .collect(),
        n_inputs: metadata.n_inputs,
        n_outputs: metadata.n_outputs,
    }
}

fn open_world(
    api: &Arc<LilvApi>,
    paths: &[PathBuf],
) -> Result<*mut LilvWorld, PluginError> {
    // SAFETY: Lilv creates an owned world or returns null.
    let world = unsafe { (api.world_new)() };
    if world.is_null() {
        return Err(PluginError::LoadFailed(
            "Lilv could not create a plugin world".into(),
        ));
    }

    if !paths.is_empty() {
        let joined = env::join_paths(paths).map_err(|error| {
            // SAFETY: world was created above and is not otherwise used.
            unsafe { (api.world_free)(world) };
            PluginError::LoadFailed(format!("invalid LV2 search path: {error}"))
        })?;
        let path = CString::new(joined.to_string_lossy().as_bytes()).map_err(|_| {
            // SAFETY: world was created above and is not otherwise used.
            unsafe { (api.world_free)(world) };
            PluginError::LoadFailed("LV2 search path contains a NUL byte".into())
        })?;
        let option_uri = b"http://drobilla.net/ns/lilv#lv2-path\0";
        // SAFETY: world is live and both arguments are NUL-terminated.
        let value = unsafe { (api.new_string)(world, path.as_ptr()) };
        if !value.is_null() {
            // SAFETY: Lilv copies the option value into the world.
            unsafe {
                (api.world_set_option)(world, option_uri.as_ptr().cast(), value);
                (api.node_free)(value);
            }
        }
    }

    // SAFETY: world is live and owned by the caller.
    unsafe { (api.world_load_all)(world) };
    Ok(world)
}

struct RuntimeInstance {
    instance: LilvInstancePtr,
    descriptor: *const Lv2Descriptor,
    input_buffers: Vec<Vec<f32>>,
    output_buffers: Vec<Vec<f32>>,
    control_values: Vec<f32>,
    #[allow(dead_code)] // owns the memory connected to atom/opaque ports; must outlive run()
    opaque_buffers: Vec<Vec<u8>>,
    active: bool,
}

// SAFETY: A runtime instance is exclusively owned and processed by its host
// instance. The plugin's LV2 handle is not shared with other threads.
unsafe impl Send for RuntimeInstance {}

fn destroy_runtime(api: &LilvApi, runtime: &mut RuntimeInstance) {
    if runtime.instance.is_null() {
        return;
    }
    if !runtime.descriptor.is_null() {
        // SAFETY: descriptor and handle belong to this live Lilv instance.
        unsafe {
            let handle = (*runtime.instance).lv2_handle;
            if runtime.active {
                if let Some(deactivate) = (*runtime.descriptor).deactivate {
                    deactivate(handle);
                }
            }
        }
    }
    // SAFETY: runtime.instance is a live Lilv instance and is freed exactly once.
    unsafe { (api.instance_free)(runtime.instance) };
    runtime.instance = ptr::null_mut();
    runtime.active = false;
}

fn make_runtime(
    api: &LilvApi,
    plugin: *const LilvPlugin,
    metadata: &PluginMetadata,
    features: &HostFeatures,
    max_block: usize,
) -> Result<RuntimeInstance, String> {
    // SAFETY: plugin is owned by the live Lilv world and feature pointers remain
    // valid in the host's boxed HostFeatures until this instance is destroyed.
    let instance = unsafe {
        (api.plugin_instantiate)(
            plugin,
            features.sample_rate as f64,
            features.feature_ptrs.as_ptr(),
        )
    };
    if instance.is_null() {
        return Err(format!(
            "Lilv failed to instantiate LV2 plugin {}",
            metadata.uri
        ));
    }

    // SAFETY: a successful Lilv instance contains the LV2 descriptor and handle.
    let descriptor = unsafe { (*instance).lv2_descriptor };
    if descriptor.is_null() {
        // SAFETY: instance was successfully created and can be freed by Lilv.
        unsafe { (api.instance_free)(instance) };
        return Err("LV2 plugin returned a null descriptor".into());
    }

    // SAFETY: descriptor is provided by the plugin and remains valid while its
    // Lilv instance and the Lilv world are alive.
    let connect_port = unsafe { (*descriptor).connect_port };
    let Some(connect_port) = connect_port else {
        // SAFETY: instance was successfully created and can be freed by Lilv.
        unsafe { (api.instance_free)(instance) };
        return Err("LV2 plugin has no connect_port callback".into());
    };

    let mut input_slot_count = 0usize;
    let mut output_slot_count = 0usize;
    for port in &metadata.ports {
        if let Some(slot) = port.audio_slot {
            if port.input {
                input_slot_count = input_slot_count.max(slot + 1);
            } else {
                output_slot_count = output_slot_count.max(slot + 1);
            }
        }
    }

    let mut input_buffers: Vec<Vec<f32>> = (0..input_slot_count)
        .map(|_| vec![0.0; max_block])
        .collect();
    let mut output_buffers: Vec<Vec<f32>> = (0..output_slot_count)
        .map(|_| vec![0.0; max_block])
        .collect();
    let mut control_values = vec![0.0; metadata.ports.len()];
    let mut opaque_buffers: Vec<Vec<u8>> = (0..metadata.ports.len())
        .map(|_| vec![0; max_block.saturating_mul(4).saturating_add(64)])
        .collect();

    for port in &metadata.ports {
        let data = match port.kind {
            PortKind::Audio => {
                let buffers = if port.input {
                    &mut input_buffers
                } else {
                    &mut output_buffers
                };
                port.audio_slot
                    .and_then(|slot| buffers.get_mut(slot))
                    .map_or(ptr::null_mut(), |buffer| buffer.as_mut_ptr().cast())
            }
            PortKind::Control => {
                control_values[port.index as usize] = if port.input {
                    port.default
                } else {
                    0.0
                };
                (&mut control_values[port.index as usize] as *mut f32).cast()
            }
            PortKind::Other => opaque_buffers[port.index as usize].as_mut_ptr().cast(),
        };

        // SAFETY: the port index belongs to the plugin, and the connected data
        // pointer addresses a buffer retained by RuntimeInstance.
        unsafe {
            connect_port((*instance).lv2_handle, port.index, data)
        };
    }

    // SAFETY: descriptor belongs to this live instance.
    if let Some(activate) = unsafe { (*descriptor).activate } {
        // SAFETY: the LV2 handle belongs to this instance.
        unsafe { activate((*instance).lv2_handle) };
    }

    Ok(RuntimeInstance {
        instance,
        descriptor,
        input_buffers,
        output_buffers,
        control_values,
        opaque_buffers,
        active: true,
    })
}

/// Runtime-loaded host for LV2 plugins using Lilv.
pub struct Lv2Host {
    api: Option<Arc<LilvApi>>,
    load_error: Option<String>,
    last_paths: Mutex<Vec<PathBuf>>,
}

impl Lv2Host {
    /// Creates an LV2 host. Lilv is loaded at runtime; its absence is reported
    /// by an empty scan and by an explanatory error from `instantiate`.
    pub fn new() -> Self {
        match LilvApi::load() {
            Ok(api) => Self {
                api: Some(api),
                load_error: None,
                last_paths: Mutex::new(Vec::new()),
            },
            Err(error) => Self {
                api: None,
                load_error: Some(error.to_string()),
                last_paths: Mutex::new(Vec::new()),
            },
        }
    }
}

impl Default for Lv2Host {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost for Lv2Host {
    fn format(&self) -> PluginFormat {
        PluginFormat::Lv2
    }

    fn default_search_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();

        if let Some(home) = env::var_os("HOME") {
            let home = PathBuf::from(home);
            paths.push(home.join(".lv2"));
            paths.push(home.join(".local/lib/lv2"));
        }

        paths.extend([
            PathBuf::from("/usr/lib/lv2"),
            PathBuf::from("/usr/local/lib/lv2"),
            PathBuf::from("/usr/lib64/lv2"),
        ]);

        if let Some(value) = env::var_os("LV2_PATH") {
            paths.extend(env::split_paths(&value));
        }

        paths
    }

    fn scan(&self, paths: &[PathBuf]) -> Vec<PluginDescriptor> {
        *self
            .last_paths
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = paths.to_vec();

        let Some(api) = &self.api else {
            return Vec::new();
        };
        let Ok(world) = open_world(api, paths) else {
            return Vec::new();
        };

        // SAFETY: world is live and owned until the end of this scan.
        let plugins = unsafe { (api.world_get_all_plugins)(world) };
        let mut descriptors = Vec::new();

        if !plugins.is_null() {
            // SAFETY: Lilv plugin iterator functions are valid for this collection.
            let mut iter = unsafe { (api.plugins_begin)(plugins) };
            while !unsafe { (api.plugins_is_end)(plugins, iter) } {
                // SAFETY: iter is a valid iterator for plugins.
                let plugin = unsafe { (api.plugins_get)(plugins, iter) };
                if !plugin.is_null() {
                    if let Ok(metadata) = plugin_metadata(api, world, plugin) {
                        descriptors.push(descriptor_from_metadata(&metadata));
                    }
                }
                // SAFETY: iter is advanced according to the Lilv iterator API.
                iter = unsafe { (api.plugins_next)(plugins, iter) };
            }
        }

        // SAFETY: world was created by open_world and is no longer needed.
        unsafe { (api.world_free)(world) };
        descriptors
    }

    fn instantiate(
        &self,
        descriptor: &PluginDescriptor,
    ) -> Result<Box<dyn PluginInstance>, PluginError> {
        let api = self.api.as_ref().ok_or_else(|| {
            PluginError::LoadFailed(self.load_error.clone().unwrap_or_else(|| {
                "Lilv is missing; install liblilv-0.so.0 to host LV2 plugins".into()
            }))
        })?;

        let uri = descriptor
            .id
            .strip_prefix("lv2:")
            .ok_or_else(|| PluginError::LoadFailed("invalid LV2 plugin id".into()))?;
        let paths = {
            let cached = self
                .last_paths
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let mut paths = if cached.is_empty() {
                self.default_search_paths()
            } else {
                cached
            };
            // Out-of-process scans report the LV2 directory the plugin was found in.
            if let Some(dir) = &descriptor.path {
                if !paths.contains(dir) {
                    paths.insert(0, dir.clone());
                }
            }
            paths
        };

        let world = open_world(api, &paths)?;
        let result = (|| {
            // SAFETY: world is live and the URI is passed as a NUL-terminated string.
            let uri_c = CString::new(uri)
                .map_err(|_| PluginError::LoadFailed("LV2 URI contains a NUL byte".into()))?;
            let target = unsafe { (api.new_uri)(world, uri_c.as_ptr()) };
            if target.is_null() {
                return Err(PluginError::LoadFailed(
                    "Lilv could not create the LV2 plugin URI node".into(),
                ));
            }

            // Find the plugin by its stable URI.
            // SAFETY: the collection belongs to the live world.
            let plugins = unsafe { (api.world_get_all_plugins)(world) };
            let mut found = None;
            if !plugins.is_null() {
                // SAFETY: Lilv iterator functions are valid for this collection.
                let mut iter = unsafe { (api.plugins_begin)(plugins) };
                while !unsafe { (api.plugins_is_end)(plugins, iter) } {
                    // SAFETY: iter is valid for the collection.
                    let plugin = unsafe { (api.plugins_get)(plugins, iter) };
                    if !plugin.is_null() {
                        // SAFETY: plugin URI is borrowed from the live world.
                        let plugin_uri = unsafe { (api.plugin_get_uri)(plugin) };
                        if node_text(api, plugin_uri).as_deref() == Some(uri) {
                            found = Some(plugin);
                            break;
                        }
                    }
                    // SAFETY: advance the live Lilv iterator.
                    iter = unsafe { (api.plugins_next)(plugins, iter) };
                }
            }
            // SAFETY: target was allocated by lilv_new_uri.
            unsafe { (api.node_free)(target) };

            let plugin = found.ok_or_else(|| {
                PluginError::LoadFailed(format!("LV2 plugin not found: {uri}"))
            })?;
            let metadata = plugin_metadata(api, world, plugin)
                .map_err(PluginError::LoadFailed)?;

            if let Some(message) = &metadata.unsupported_required {
                return Err(PluginError::LoadFailed(message.clone()));
            }

            let urids = UridTable::new();
            let features = HostFeatures::new(&urids, 44_100, 1024);
            let units = if metadata.n_inputs == 1 && metadata.n_outputs == 1 {
                2
            } else {
                1
            };

            // Probe instantiation now so callers receive a useful error instead
            // of a silent no-op later in activate().
            for _ in 0..units {
                // SAFETY: plugin, world, and feature pointers remain live here.
                let probe = unsafe {
                    (api.plugin_instantiate)(
                        plugin,
                        44_100.0,
                        features.feature_ptrs.as_ptr(),
                    )
                };
                if probe.is_null() {
                    return Err(PluginError::LoadFailed(format!(
                        "Lilv failed to instantiate LV2 plugin {uri}"
                    )));
                }
                // SAFETY: probe is a successful Lilv instance.
                unsafe { (api.instance_free)(probe) };
            }

            Ok(Box::new(Lv2Instance {
                api: Arc::clone(api),
                world,
                plugin,
                metadata,
                descriptor: descriptor.clone(),
                urids,
                features: None,
                control_values: Vec::new(),
                runtimes: Vec::new(),
                max_block: 0,
                latency_samples: 0,
                unit_count: units,
            }) as Box<dyn PluginInstance>)
        })();

        if result.is_err() {
            // SAFETY: world was created above and ownership was not transferred.
            unsafe { (api.world_free)(world) };
        }
        result
    }
}

struct Lv2Instance {
    api: Arc<LilvApi>,
    world: *mut LilvWorld,
    plugin: *const LilvPlugin,
    metadata: PluginMetadata,
    descriptor: PluginDescriptor,
    urids: Arc<UridTable>,
    features: Option<Box<HostFeatures>>,
    control_values: Vec<f32>,
    runtimes: Vec<RuntimeInstance>,
    max_block: usize,
    latency_samples: u32,
    unit_count: usize,
}

// SAFETY: The Lilv world and plugin instances are exclusively owned by this
// PluginInstance and are not concurrently accessed by the host.
unsafe impl Send for Lv2Instance {}

impl Drop for Lv2Instance {
    fn drop(&mut self) {
        for runtime in &mut self.runtimes {
            destroy_runtime(&self.api, runtime);
        }
        self.runtimes.clear();
        // Features and their URID table remain alive until all LV2 instances
        // have been destroyed.
        self.features.take();
        if !self.world.is_null() {
            // SAFETY: this instance owns the world and frees it exactly once.
            unsafe { (self.api.world_free)(self.world) };
            self.world = ptr::null_mut();
        }
    }
}

impl PluginInstance for Lv2Instance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn latency_samples(&self) -> u32 {
        self.latency_samples
    }

    fn save_state(&mut self) -> Result<Vec<(String, String)>, PluginError> {
        Ok(self
            .metadata
            .ports
            .iter()
            .filter(|port| port.kind == PortKind::Control && port.input)
            .filter_map(|port| {
                self.control_values
                    .get(port.index as usize)
                    .map(|value| (port.symbol.clone(), value.to_string()))
            })
            .collect())
    }

    fn load_state(&mut self, state: &[(String, String)]) -> Result<(), PluginError> {
        for (symbol, value) in state {
            let Ok(value) = value.parse::<f32>() else {
                continue;
            };
            if let Some(port) = self.metadata.ports.iter().find(|port| {
                port.kind == PortKind::Control && port.input && port.symbol == *symbol
            }) {
                self.set_param(port.index, value)?;
            }
        }
        Ok(())
    }

    fn set_param(&mut self, id: u32, value: f32) -> Result<(), PluginError> {
        let port = self
            .metadata
            .ports
            .iter()
            .find(|port| {
                port.index == id && port.kind == PortKind::Control && port.input
            })
            .ok_or_else(|| PluginError::Unknown(format!("unknown LV2 parameter {id}")))?;

        if !value.is_finite() {
            return Err(PluginError::Unknown(
                "LV2 parameter must be finite".into(),
            ));
        }

        let mut value = value.clamp(port.min, port.max);
        if port.integer || port.toggled {
            value = value.round();
        }
        if port.toggled {
            value = value.clamp(0.0, 1.0);
        }

        if self.control_values.len() != self.metadata.ports.len() {
            self.control_values = vec![0.0; self.metadata.ports.len()];
            for meta in &self.metadata.ports {
                if meta.kind == PortKind::Control && meta.input {
                    self.control_values[meta.index as usize] = meta.default;
                }
            }
        }
        self.control_values[id as usize] = value;
        Ok(())
    }

    fn get_param(&self, id: u32) -> Option<f32> {
        self.control_values
            .get(id as usize)
            .copied()
            .or_else(|| {
                self.metadata
                    .ports
                    .iter()
                    .find(|port| {
                        port.index == id
                            && port.kind == PortKind::Control
                            && port.input
                    })
                    .map(|port| port.default)
            })
    }

    fn activate(&mut self, sample_rate: u32, max_block: usize) {
        for runtime in &mut self.runtimes {
            destroy_runtime(&self.api, runtime);
        }
        self.runtimes.clear();
        self.features.take();

        self.max_block = max_block.max(1);
        self.latency_samples = 0;
        if self.control_values.len() != self.metadata.ports.len() {
            self.control_values = vec![0.0; self.metadata.ports.len()];
            for port in &self.metadata.ports {
                if port.kind == PortKind::Control && port.input {
                    self.control_values[port.index as usize] = port.default;
                }
            }
        }

        self.urids.frozen.store(false, Ordering::Release);
        let features = HostFeatures::new(&self.urids, sample_rate.max(1), self.max_block);

        for _ in 0..self.unit_count {
            match make_runtime(
                &self.api,
                self.plugin,
                &self.metadata,
                &features,
                self.max_block,
            ) {
                Ok(mut runtime) => {
                    for port in &self.metadata.ports {
                        if port.kind == PortKind::Control && port.input {
                            runtime.control_values[port.index as usize] =
                                self.control_values[port.index as usize];
                        }
                    }
                    self.runtimes.push(runtime);
                }
                Err(_) => {
                    for runtime in &mut self.runtimes {
                        destroy_runtime(&self.api, runtime);
                    }
                    self.runtimes.clear();
                    self.features = Some(features);
                    return;
                }
            }
        }

        self.features = Some(features);
        self.urids.frozen.store(true, Ordering::Release);
    }

    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [Vec<f32>]) {
        for output in outputs.iter_mut() {
            output.fill(0.0);
        }
        if self.runtimes.is_empty() || outputs.is_empty() {
            return;
        }

        let frames = outputs.iter().map(Vec::len).max().unwrap_or(0);
        let mono_instances = self.metadata.n_inputs == 1 && self.metadata.n_outputs == 1;
        let units = self.runtimes.len();

        for unit in 0..units {
            let runtime = &mut self.runtimes[unit];
            let Some(run) = (unsafe { (*runtime.descriptor).run }) else {
                continue;
            };

            for port in &self.metadata.ports {
                if port.kind == PortKind::Control && port.input {
                    runtime.control_values[port.index as usize] =
                        self.control_values[port.index as usize];
                }
            }

            let mut offset = 0usize;
            while offset < frames {
                let count = self.max_block.min(frames - offset);
                for port in &self.metadata.ports {
                    if port.kind != PortKind::Audio || !port.input {
                        continue;
                    }
                    let Some(slot) = port.audio_slot else {
                        continue;
                    };
                    let Some(buffer) = runtime.input_buffers.get_mut(slot) else {
                        continue;
                    };
                    buffer[..count].fill(0.0);
                    let channel = if mono_instances {
                        unit
                    } else {
                        slot
                    };
                    if let Some(input) = inputs.get(channel) {
                        if offset < input.len() {
                            let copied = count.min(input.len() - offset);
                            buffer[..copied]
                                .copy_from_slice(&input[offset..offset + copied]);
                        }
                    }
                }

                for port in &self.metadata.ports {
                    if port.kind == PortKind::Audio && !port.input {
                        if let Some(slot) = port.audio_slot {
                            if let Some(buffer) = runtime.output_buffers.get_mut(slot) {
                                buffer[..count].fill(0.0);
                            }
                        }
                    }
                }

                // SAFETY: the descriptor and LV2 handle belong to this runtime;
                // count is bounded by the preallocated buffers and u32 range.
                unsafe { run((*runtime.instance).lv2_handle, count as u32) };

                for port in &self.metadata.ports {
                    if port.kind != PortKind::Audio || port.input {
                        continue;
                    }
                    let Some(slot) = port.audio_slot else {
                        continue;
                    };
                    let channel = if mono_instances { unit } else { slot };
                    let Some(output) = outputs.get_mut(channel) else {
                        continue;
                    };
                    if offset >= output.len() {
                        continue;
                    }
                    let copied = count.min(output.len() - offset);
                    if let Some(buffer) = runtime.output_buffers.get(slot) {
                        output[offset..offset + copied]
                            .copy_from_slice(&buffer[..copied]);
                    }
                }

                if let Some(latency_port) = self.metadata.latency_port {
                    let latency = runtime
                        .control_values
                        .get(latency_port as usize)
                        .copied()
                        .unwrap_or(0.0);
                    self.latency_samples = if latency.is_finite() && latency > 0.0 {
                        latency.min(u32::MAX as f32) as u32
                    } else {
                        0
                    };
                }

                offset += count;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use au_core::AudioBuffer;

    fn amp_descriptor(host: &Lv2Host) -> Option<PluginDescriptor> {
        host.scan(&host.default_search_paths())
            .into_iter()
            .find(|descriptor| {
                descriptor.id == "lv2:http://plugin.org.uk/swh-plugins/amp"
            })
    }

    fn skip_without_system_amp() -> bool {
        if !PathBuf::from("/usr/lib/lv2/amp-swh.lv2").exists() {
            eprintln!("skipping LV2 amp test: /usr/lib/lv2/amp-swh.lv2 is not installed");
            return true;
        }
        false
    }

    #[test]
    fn amp_scan_processes_and_reports_latency() {
        if skip_without_system_amp() {
            return;
        }

        let host = Lv2Host::new();
        let Some(descriptor) = amp_descriptor(&host) else {
            eprintln!("skipping LV2 amp test: Lilv did not discover the system amp plugin");
            return;
        };
        let mut instance = host.instantiate(&descriptor).unwrap();
        let Some(parameter) = descriptor.params.first() else {
            panic!("SWH amp plugin has no input control parameter");
        };
        instance.set_param(parameter.id, 6.0206).unwrap();
        instance.activate(48_000, 128);

        let frames = 128;
        let input_a: Vec<f32> = (0..frames)
            .map(|i| (i as f32 * 0.11).sin() * 0.2)
            .collect();
        let input_b: Vec<f32> = (0..frames)
            .map(|i| (i as f32 * 0.07).cos() * 0.15)
            .collect();
        let mut output = vec![vec![0.0; frames], vec![0.0; frames]];
        instance.process(&[&input_a, &input_b], &mut output);

        for (input, output) in input_a.iter().zip(&output[0]) {
            assert!((output - input * 2.0).abs() < 1.0e-6);
        }
        for (input, output) in input_b.iter().zip(&output[1]) {
            assert!((output - input * 2.0).abs() < 1.0e-6);
        }
        assert_eq!(instance.latency_samples(), 0);
    }

    #[test]
    fn amp_state_round_trip_and_stereo_processing() {
        if skip_without_system_amp() {
            return;
        }

        let host = Lv2Host::new();
        let Some(descriptor) = amp_descriptor(&host) else {
            eprintln!("skipping LV2 amp test: Lilv did not discover the system amp plugin");
            return;
        };
        let mut instance = host.instantiate(&descriptor).unwrap();
        let Some(parameter) = descriptor.params.first() else {
            panic!("SWH amp plugin has no input control parameter");
        };

        instance.set_param(parameter.id, 3.0).unwrap();
        let state = instance.save_state().unwrap();
        instance.set_param(parameter.id, -3.0).unwrap();
        instance.load_state(&state).unwrap();
        assert_eq!(instance.get_param(parameter.id), Some(3.0));

        instance.set_param(parameter.id, 0.0).unwrap();
        instance.activate(48_000, 64);

        let input_a = vec![0.25; 64];
        let input_b = vec![-0.375; 64];
        let mut output = vec![vec![0.0; 64], vec![0.0; 64]];
        instance.process(&[&input_a, &input_b], &mut output);

        for sample in &output[0] {
            assert!((*sample - 0.25).abs() < 1.0e-6);
        }
        for sample in &output[1] {
            assert!((*sample + 0.375).abs() < 1.0e-6);
        }
    }

    #[test]
    fn missing_lilv_is_reported_without_linking() {
        let host = Lv2Host::new();
        if host.api.is_none() {
            assert!(host.scan(&[]).is_empty());
            let fake = PluginDescriptor {
                id: "lv2:http://example.invalid/plugin".into(),
                name: "missing".into(),
                vendor: String::new(),
                format: PluginFormat::Lv2,
                path: None,
                params: Vec::new(),
                n_inputs: 0,
                n_outputs: 0,
            };
            assert!(matches!(
                host.instantiate(&fake),
                Err(PluginError::LoadFailed(message)) if message.contains("Lilv")
            ));
        }
    }

    #[allow(dead_code)]
    fn _audio_buffer_type_check(_: AudioBuffer) {}
}
