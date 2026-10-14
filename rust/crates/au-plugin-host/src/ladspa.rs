// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! LADSPA 1.1 plugin discovery and hosting.

use au_plugin_api::{
    ParamInfo, PluginDescriptor, PluginError, PluginFormat, PluginHost, PluginInstance,
};
use libloading::{Library, Symbol};
use std::{
    collections::HashSet,
    env,
    ffi::{c_char, c_int, c_ulong, c_void, CStr},
    path::{Path, PathBuf},
};

const LADSPA_PORT_INPUT: c_int = 0x1;
const LADSPA_PORT_OUTPUT: c_int = 0x2;
const LADSPA_PORT_CONTROL: c_int = 0x4;
const LADSPA_PORT_AUDIO: c_int = 0x8;

const LADSPA_HINT_BOUNDED_BELOW: c_int = 0x1;
const LADSPA_HINT_BOUNDED_ABOVE: c_int = 0x2;
const LADSPA_HINT_TOGGLED: c_int = 0x4;
const LADSPA_HINT_SAMPLE_RATE: c_int = 0x8;
const LADSPA_HINT_LOGARITHMIC: c_int = 0x10;
const LADSPA_HINT_INTEGER: c_int = 0x20;
const LADSPA_HINT_DEFAULT_MASK: c_int = 0x3c0;
const LADSPA_HINT_DEFAULT_MINIMUM: c_int = 0x40;
const LADSPA_HINT_DEFAULT_LOW: c_int = 0x80;
const LADSPA_HINT_DEFAULT_MIDDLE: c_int = 0xc0;
const LADSPA_HINT_DEFAULT_HIGH: c_int = 0x100;
const LADSPA_HINT_DEFAULT_MAXIMUM: c_int = 0x140;
const LADSPA_HINT_DEFAULT_0: c_int = 0x200;
const LADSPA_HINT_DEFAULT_1: c_int = 0x240;
const LADSPA_HINT_DEFAULT_100: c_int = 0x280;
const LADSPA_HINT_DEFAULT_440: c_int = 0x2c0;

const DEFAULT_SCAN_SAMPLE_RATE: u32 = 44_100;
const MAX_DESCRIPTOR_PORTS: usize = 65_536;
const MAX_DESCRIPTORS_PER_LIBRARY: usize = 65_536;

/// LADSPA 1.1 port range hint.
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(non_snake_case)]
struct LADSPA_PortRangeHint {
    HintDescriptor: c_int,
    LowerBound: f32,
    UpperBound: f32,
}

type LadspaHandle = *mut c_void;
type InstantiateFn =
    unsafe extern "C" fn(*const LADSPA_Descriptor, c_ulong) -> LadspaHandle;
type ConnectPortFn = unsafe extern "C" fn(LadspaHandle, c_ulong, *mut f32);
type ActivateFn = unsafe extern "C" fn(LadspaHandle);
type RunFn = unsafe extern "C" fn(LadspaHandle, c_ulong);
type DeactivateFn = unsafe extern "C" fn(LadspaHandle);
type CleanupFn = unsafe extern "C" fn(LadspaHandle);
type DescriptorFn = unsafe extern "C" fn(c_ulong) -> *const LADSPA_Descriptor;

/// LADSPA 1.1 descriptor, with fields in the order specified by `ladspa.h`.
#[repr(C)]
#[allow(non_snake_case)]
struct LADSPA_Descriptor {
    UniqueID: c_ulong,
    Label: *const c_char,
    Properties: c_int,
    Name: *const c_char,
    Maker: *const c_char,
    Copyright: *const c_char,
    PortCount: c_ulong,
    PortDescriptors: *const c_int,
    PortNames: *const *const c_char,
    PortRangeHints: *const LADSPA_PortRangeHint,
    ImplementationData: *mut c_void,
    instantiate: Option<InstantiateFn>,
    connect_port: Option<ConnectPortFn>,
    activate: Option<ActivateFn>,
    run: Option<RunFn>,
    run_adding: Option<RunFn>,
    set_run_adding_gain: Option<unsafe extern "C" fn(LadspaHandle, f32)>,
    deactivate: Option<DeactivateFn>,
    cleanup: Option<CleanupFn>,
}

#[derive(Clone)]
struct PortMeta {
    descriptor: c_int,
    name: String,
    hint: LADSPA_PortRangeHint,
}

#[derive(Clone, Copy)]
struct LadspaFns {
    instantiate: InstantiateFn,
    connect_port: ConnectPortFn,
    activate: Option<ActivateFn>,
    run: RunFn,
    deactivate: Option<DeactivateFn>,
    cleanup: Option<CleanupFn>,
}

struct PluginMeta {
    descriptor: PluginDescriptor,
    descriptor_ptr: *const LADSPA_Descriptor,
    ports: Vec<PortMeta>,
    audio_input_ports: Vec<usize>,
    audio_output_ports: Vec<usize>,
    parameter_ports: Vec<usize>,
    latency_port: Option<usize>,
    fns: LadspaFns,
}

fn c_string(pointer: *const c_char) -> Option<String> {
    if pointer.is_null() {
        return None;
    }

    // SAFETY: LADSPA specifies these descriptor fields as NUL-terminated strings.
    Some(unsafe { CStr::from_ptr(pointer) }.to_string_lossy().into_owned())
}

fn effective_range(hint: LADSPA_PortRangeHint, sample_rate: u32) -> (f32, f32, f32, bool, bool) {
    let flags = hint.HintDescriptor;
    let sample_scaled = flags & LADSPA_HINT_SAMPLE_RATE != 0;
    let scale = if sample_scaled {
        sample_rate as f64
    } else {
        1.0
    };

    let mut lower = if flags & LADSPA_HINT_BOUNDED_BELOW != 0 {
        hint.LowerBound as f64 * scale
    } else {
        0.0
    };
    let mut upper = if flags & LADSPA_HINT_BOUNDED_ABOVE != 0 {
        hint.UpperBound as f64 * scale
    } else {
        1.0
    };

    if flags & LADSPA_HINT_TOGGLED != 0 {
        lower = 0.0;
        upper = 1.0;
    }

    let logarithmic = flags & LADSPA_HINT_LOGARITHMIC != 0;
    let default_kind = flags & LADSPA_HINT_DEFAULT_MASK;
    let mut default = match default_kind {
        LADSPA_HINT_DEFAULT_MINIMUM => lower,
        LADSPA_HINT_DEFAULT_LOW if logarithmic && lower > 0.0 && upper > 0.0 => {
            (lower.ln() * 0.75 + upper.ln() * 0.25).exp()
        }
        LADSPA_HINT_DEFAULT_LOW => lower * 0.75 + upper * 0.25,
        LADSPA_HINT_DEFAULT_MIDDLE if logarithmic && lower > 0.0 && upper > 0.0 => {
            (lower.ln() * 0.5 + upper.ln() * 0.5).exp()
        }
        LADSPA_HINT_DEFAULT_MIDDLE => lower * 0.5 + upper * 0.5,
        LADSPA_HINT_DEFAULT_HIGH if logarithmic && lower > 0.0 && upper > 0.0 => {
            (lower.ln() * 0.25 + upper.ln() * 0.75).exp()
        }
        LADSPA_HINT_DEFAULT_HIGH => lower * 0.25 + upper * 0.75,
        LADSPA_HINT_DEFAULT_MAXIMUM => upper,
        LADSPA_HINT_DEFAULT_0 => 0.0,
        LADSPA_HINT_DEFAULT_1 => 1.0 * scale,
        LADSPA_HINT_DEFAULT_100 => 100.0 * scale,
        LADSPA_HINT_DEFAULT_440 => 440.0 * scale,
        _ => 0.0,
    };

    if flags & LADSPA_HINT_TOGGLED != 0 {
        default = if default >= 0.5 { 1.0 } else { 0.0 };
    }

    let integer = flags & LADSPA_HINT_INTEGER != 0;
    if integer {
        lower = lower.ceil();
        upper = upper.floor();
        if lower > upper {
            lower = if flags & LADSPA_HINT_BOUNDED_BELOW != 0 {
                (hint.LowerBound as f64 * scale).round()
            } else {
                0.0
            };
            upper = if flags & LADSPA_HINT_BOUNDED_ABOVE != 0 {
                (hint.UpperBound as f64 * scale).round()
            } else {
                1.0
            };
        }
        default = default.round();
    }

    if !lower.is_finite() {
        lower = 0.0;
    }
    if !upper.is_finite() {
        upper = 1.0;
    }
    if lower > upper {
        std::mem::swap(&mut lower, &mut upper);
    }
    if !default.is_finite() {
        default = lower;
    }
    default = default.clamp(lower, upper);

    (
        lower as f32,
        upper as f32,
        default as f32,
        integer,
        flags & LADSPA_HINT_TOGGLED != 0,
    )
}

fn plugin_meta(
    descriptor_ptr: *const LADSPA_Descriptor,
    path: &Path,
) -> Result<PluginMeta, PluginError> {
    if descriptor_ptr.is_null() {
        return Err(PluginError::LoadFailed(
            "LADSPA returned a null descriptor".into(),
        ));
    }

    // SAFETY: the descriptor is owned by the loaded LADSPA library, which remains
    // alive throughout this call and, for instantiated plugins, for the instance lifetime.
    let descriptor = unsafe { &*descriptor_ptr };
    let port_count = usize::try_from(descriptor.PortCount)
        .map_err(|_| PluginError::LoadFailed("invalid LADSPA port count".into()))?;
    if port_count > MAX_DESCRIPTOR_PORTS {
        return Err(PluginError::LoadFailed(
            "LADSPA descriptor has too many ports".into(),
        ));
    }
    if port_count != 0
        && (descriptor.PortDescriptors.is_null()
            || descriptor.PortNames.is_null()
            || descriptor.PortRangeHints.is_null())
    {
        return Err(PluginError::LoadFailed(
            "LADSPA descriptor has missing port metadata".into(),
        ));
    }

    let instantiate = descriptor
        .instantiate
        .ok_or_else(|| PluginError::LoadFailed("LADSPA plugin has no instantiate callback".into()))?;
    let connect_port = descriptor
        .connect_port
        .ok_or_else(|| PluginError::LoadFailed("LADSPA plugin has no connect_port callback".into()))?;
    let run = descriptor
        .run
        .ok_or_else(|| PluginError::LoadFailed("LADSPA plugin has no run callback".into()))?;

    let mut ports = Vec::with_capacity(port_count);
    let mut audio_input_ports = Vec::new();
    let mut audio_output_ports = Vec::new();
    let mut parameter_ports = Vec::new();
    let mut params = Vec::new();
    let mut n_inputs = 0u32;
    let mut n_outputs = 0u32;
    let mut latency_port = None;

    for index in 0..port_count {
        // SAFETY: non-null port arrays of PortCount elements were checked above.
        let (port_descriptor, name_ptr, hint) = unsafe {
            (
                *descriptor.PortDescriptors.add(index),
                *descriptor.PortNames.add(index),
                *descriptor.PortRangeHints.add(index),
            )
        };
        let name = c_string(name_ptr).unwrap_or_else(|| format!("Port {index}"));

        if port_descriptor & LADSPA_PORT_AUDIO != 0 {
            if port_descriptor & LADSPA_PORT_INPUT != 0 {
                audio_input_ports.push(index);
                n_inputs = n_inputs.saturating_add(1);
            }
            if port_descriptor & LADSPA_PORT_OUTPUT != 0 {
                audio_output_ports.push(index);
                n_outputs = n_outputs.saturating_add(1);
            }
        }

        if port_descriptor & LADSPA_PORT_CONTROL != 0
            && port_descriptor & LADSPA_PORT_INPUT != 0
        {
            let (min, max, default, _, _) = effective_range(hint, DEFAULT_SCAN_SAMPLE_RATE);
            let id = u32::try_from(index)
                .map_err(|_| PluginError::LoadFailed("LADSPA port index is too large".into()))?;
            params.push(ParamInfo {
                id,
                name: name.clone(),
                min,
                max,
                default,
            });
            parameter_ports.push(index);
        }

        if port_descriptor & LADSPA_PORT_CONTROL != 0
            && port_descriptor & LADSPA_PORT_OUTPUT != 0
            && name.eq_ignore_ascii_case("latency")
        {
            latency_port = Some(index);
        }

        ports.push(PortMeta {
            descriptor: port_descriptor,
            name,
            hint,
        });
    }

    let unique_id = descriptor.UniqueID.to_string();
    let path = path.to_path_buf();
    let plugin_name = c_string(descriptor.Name).unwrap_or_else(|| "Unknown LADSPA plugin".into());
    let plugin_vendor = c_string(descriptor.Maker).unwrap_or_default();
    let id = crate::state::effect_id(PluginFormat::Ladspa, &path, &unique_id, &plugin_vendor, &plugin_name);
    let plugin_descriptor = PluginDescriptor {
        id,
        name: plugin_name,
        vendor: plugin_vendor,
        format: PluginFormat::Ladspa,
        path: Some(path),
        params,
        n_inputs,
        n_outputs,
    };

    Ok(PluginMeta {
        descriptor: plugin_descriptor,
        descriptor_ptr,
        ports,
        audio_input_ports,
        audio_output_ports,
        parameter_ports,
        latency_port,
        fns: LadspaFns {
            instantiate,
            connect_port,
            activate: descriptor.activate,
            run,
            deactivate: descriptor.deactivate,
            cleanup: descriptor.cleanup,
        },
    })
}

fn is_shared_library(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    name.ends_with(".so") || name.contains(".so.")
}

fn scan_library(path: &Path) -> Vec<PluginDescriptor> {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // SAFETY: the library is kept open while its LADSPA entry point and descriptors are read.
    let Ok(library) = (unsafe { Library::new(&path) }) else {
        return Vec::new();
    };

    let descriptor_fn = {
        // SAFETY: LADSPA 1.1 exports this symbol with the declared signature.
        let Ok(symbol) = (unsafe { library.get::<DescriptorFn>(b"ladspa_descriptor\0") }) else {
            return Vec::new();
        };
        *symbol
    };

    let mut descriptors = Vec::new();
    for index in 0..MAX_DESCRIPTORS_PER_LIBRARY {
        // SAFETY: this is the LADSPA descriptor enumeration function.
        let pointer = unsafe { descriptor_fn(index as c_ulong) };
        if pointer.is_null() {
            break;
        }
        if let Ok(meta) = plugin_meta(pointer, &path) {
            descriptors.push(meta.descriptor);
        }
    }
    descriptors
}

fn cleanup_unit(fns: LadspaFns, handle: LadspaHandle, active: bool) {
    if handle.is_null() {
        return;
    }
    if active {
        if let Some(deactivate) = fns.deactivate {
            // SAFETY: this handle was created by this plugin and has not been cleaned up.
            unsafe { deactivate(handle) };
        }
    }
    if let Some(cleanup) = fns.cleanup {
        // SAFETY: this handle was created by this plugin and has not been cleaned up.
        unsafe { cleanup(handle) };
    }
}

struct LadspaUnit {
    handle: LadspaHandle,
    controls: Vec<f32>,
    audio_inputs: Vec<Vec<f32>>,
    audio_outputs: Vec<Vec<f32>>,
    active: bool,
}

fn create_unit(
    meta: &PluginMeta,
    sample_rate: u32,
    max_block: usize,
    parameter_values: &[f32],
) -> Result<LadspaUnit, PluginError> {
    // SAFETY: the descriptor pointer belongs to the still-loaded plugin library.
    let handle = unsafe {
        (meta.fns.instantiate)(meta.descriptor_ptr, sample_rate.max(1) as c_ulong)
    };
    if handle.is_null() {
        return Err(PluginError::LoadFailed(
            "LADSPA plugin failed to instantiate".into(),
        ));
    }

    let mut controls = vec![0.0; meta.ports.len()];
    for (parameter_index, port_index) in meta.parameter_ports.iter().copied().enumerate() {
        if let Some(value) = parameter_values.get(parameter_index) {
            controls[port_index] = *value;
        }
    }

    let mut audio_inputs: Vec<Vec<f32>> = meta
        .audio_input_ports
        .iter()
        .map(|_| vec![0.0; max_block])
        .collect();
    let mut audio_outputs: Vec<Vec<f32>> = meta
        .audio_output_ports
        .iter()
        .map(|_| vec![0.0; max_block])
        .collect();

    let mut input_ordinal = 0;
    let mut output_ordinal = 0;
    for (port_index, port) in meta.ports.iter().enumerate() {
        let pointer = if port.descriptor & LADSPA_PORT_CONTROL != 0 {
            // SAFETY: controls has one stable allocation covering every port.
            unsafe { controls.as_mut_ptr().add(port_index) }
        } else if port.descriptor & LADSPA_PORT_AUDIO != 0
            && port.descriptor & LADSPA_PORT_INPUT != 0
        {
            // SAFETY: the audio buffer is allocated and remains at a stable address.
            let pointer = unsafe { audio_inputs[input_ordinal].as_mut_ptr() };
            input_ordinal += 1;
            pointer
        } else if port.descriptor & LADSPA_PORT_AUDIO != 0
            && port.descriptor & LADSPA_PORT_OUTPUT != 0
        {
            // SAFETY: the audio buffer is allocated and remains at a stable address.
            let pointer = unsafe { audio_outputs[output_ordinal].as_mut_ptr() };
            output_ordinal += 1;
            pointer
        } else {
            continue;
        };

        // SAFETY: this is a valid port index and pointer to storage of the required type.
        unsafe { (meta.fns.connect_port)(handle, port_index as c_ulong, pointer) };
    }

    Ok(LadspaUnit {
        handle,
        controls,
        audio_inputs,
        audio_outputs,
        active: false,
    })
}

/// Host for LADSPA 1.1 plugins.
pub struct LadspaHost;

impl LadspaHost {
    /// Creates a LADSPA host.
    pub fn new() -> Self {
        Self
    }
}

impl Default for LadspaHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost for LadspaHost {
    fn format(&self) -> PluginFormat {
        PluginFormat::Ladspa
    }

    fn default_search_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();

        if let Some(value) = env::var_os("LADSPA_PATH") {
            paths.extend(env::split_paths(&value));
        }
        if let Some(home) = env::var_os("HOME") {
            paths.push(PathBuf::from(home).join(".ladspa"));
        }
        paths.push(PathBuf::from("/usr/local/lib/ladspa"));
        paths.push(PathBuf::from("/usr/lib/ladspa"));
        paths.push(PathBuf::from("/usr/lib/x86_64-linux-gnu/ladspa"));

        paths
    }

    fn scan(&self, paths: &[PathBuf]) -> Vec<PluginDescriptor> {
        let mut descriptors = Vec::new();
        let mut seen = HashSet::new();

        for path in paths {
            let candidates = if path.is_file() {
                if is_shared_library(path) {
                    vec![path.clone()]
                } else {
                    Vec::new()
                }
            } else {
                let Ok(entries) = std::fs::read_dir(path) else {
                    continue;
                };
                let mut candidates: Vec<PathBuf> = entries
                    .flatten()
                    .map(|entry| entry.path())
                    .filter(|candidate| candidate.is_file() && is_shared_library(candidate))
                    .collect();
                candidates.sort();
                candidates
            };

            for candidate in candidates {
                for descriptor in scan_library(&candidate) {
                    if seen.insert(descriptor.id.clone()) {
                        descriptors.push(descriptor);
                    }
                }
            }
        }

        descriptors
    }

    fn instantiate(&self, descriptor: &PluginDescriptor) -> Result<Box<dyn PluginInstance>, PluginError> {
        if descriptor.format != PluginFormat::Ladspa {
            return Err(PluginError::NotImplemented(descriptor.format));
        }
        let path = descriptor
            .path
            .as_ref()
            .ok_or_else(|| PluginError::LoadFailed("LADSPA descriptor has no path".into()))?;
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.clone());

        // SAFETY: the library is retained in LadspaInstance for the lifetime of its callbacks.
        let library = unsafe { Library::new(&path) }
            .map_err(|error| PluginError::LoadFailed(format!("{}: {error}", path.display())))?;
        let descriptor_fn = {
            // SAFETY: LADSPA 1.1 exports this symbol with the declared signature.
            let symbol: Symbol<'_, DescriptorFn> =
                unsafe { library.get(b"ladspa_descriptor\0") }.map_err(|error| {
                    PluginError::LoadFailed(format!(
                        "{} has no usable ladspa_descriptor: {error}",
                        path.display()
                    ))
                })?;
            *symbol
        };

        let mut found = None;
        for index in 0..MAX_DESCRIPTORS_PER_LIBRARY {
            // SAFETY: this is the LADSPA descriptor enumeration function.
            let pointer = unsafe { descriptor_fn(index as c_ulong) };
            if pointer.is_null() {
                break;
            }
            let meta = plugin_meta(pointer, &path)?;
            if meta.descriptor.id == descriptor.id {
                found = Some(meta);
                break;
            }
        }
        let meta = found.ok_or_else(|| PluginError::LoadFailed(descriptor.id.clone()))?;

        let parameter_values: Vec<f32> = meta
            .descriptor
            .params
            .iter()
            .map(|parameter| parameter.default)
            .collect();
        let runtime_ranges = meta
            .parameter_ports
            .iter()
            .map(|port| {
                let (min, max, _, integer, toggled) =
                    effective_range(meta.ports[*port].hint, DEFAULT_SCAN_SAMPLE_RATE);
                (min, max, integer, toggled)
            })
            .collect();

        Ok(Box::new(LadspaInstance {
            descriptor: meta.descriptor.clone(),
            _library: library,
            meta,
            units: Vec::new(),
            sample_rate: DEFAULT_SCAN_SAMPLE_RATE,
            max_block: 1,
            parameter_values,
            parameter_explicit: vec![false; descriptor.params.len()],
            runtime_ranges,
        }))
    }
}

// LADSPA handles and descriptor pointers are library-owned raw pointers. The library
// is retained by the instance, and the host serializes calls to each instance.
unsafe impl Send for LadspaInstance {}

struct LadspaInstance {
    descriptor: PluginDescriptor,
    _library: Library,
    meta: PluginMeta,
    units: Vec<LadspaUnit>,
    sample_rate: u32,
    max_block: usize,
    parameter_values: Vec<f32>,
    parameter_explicit: Vec<bool>,
    runtime_ranges: Vec<(f32, f32, bool, bool)>,
}

impl LadspaInstance {
    fn clear_units(&mut self) {
        for unit in self.units.drain(..) {
            cleanup_unit(self.meta.fns, unit.handle, unit.active);
        }
    }

    fn groups_for_channels(channels: usize, plugin_channels: usize) -> usize {
        if plugin_channels == 0 {
            1
        } else {
            channels.div_ceil(plugin_channels).max(1)
        }
    }

    fn active_range(&self, index: usize) -> (f32, f32, bool, bool) {
        self.runtime_ranges
            .get(index)
            .copied()
            .unwrap_or_else(|| {
                let parameter = &self.descriptor.params[index];
                (parameter.min, parameter.max, false, false)
            })
    }
}

impl Drop for LadspaInstance {
    fn drop(&mut self) {
        self.clear_units();
    }
}

impl PluginInstance for LadspaInstance {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn latency_samples(&self) -> u32 {
        let Some(port) = self.meta.latency_port else {
            return 0;
        };
        self.units
            .iter()
            .filter_map(|unit| unit.controls.get(port).copied())
            .filter(|value| value.is_finite() && *value > 0.0)
            .map(|value| value.round() as u32)
            .max()
            .unwrap_or(0)
    }

    fn set_param(&mut self, id: u32, value: f32) -> Result<(), PluginError> {
        let index = self
            .descriptor
            .params
            .iter()
            .position(|parameter| parameter.id == id)
            .ok_or_else(|| PluginError::Unknown(format!("unknown LADSPA parameter {id}")))?;
        if !value.is_finite() {
            return Err(PluginError::Unknown(
                "LADSPA parameter must be finite".into(),
            ));
        }

        let (min, max, integer, toggled) = self.active_range(index);
        let mut value = value.clamp(min, max);
        if toggled {
            value = if value >= 0.5 { 1.0 } else { 0.0 };
        } else if integer {
            value = value.round();
        }

        self.parameter_values[index] = value;
        self.parameter_explicit[index] = true;
        let port = self.meta.parameter_ports[index];
        for unit in &mut self.units {
            unit.controls[port] = value;
        }
        Ok(())
    }

    fn get_param(&self, id: u32) -> Option<f32> {
        self.descriptor
            .params
            .iter()
            .position(|parameter| parameter.id == id)
            .and_then(|index| self.parameter_values.get(index).copied())
    }

    fn activate(&mut self, sample_rate: u32, max_block: usize) {
        self.clear_units();
        self.sample_rate = sample_rate.max(1);
        self.max_block = max_block.max(1);

        for (index, port) in self.meta.parameter_ports.iter().copied().enumerate() {
            let (min, max, default, integer, toggled) =
                effective_range(self.meta.ports[port].hint, self.sample_rate);
            self.runtime_ranges[index] = (min, max, integer, toggled);
            let value = if self.parameter_explicit[index] {
                self.parameter_values[index].clamp(min, max)
            } else {
                default
            };
            self.parameter_values[index] = if toggled {
                if value >= 0.5 { 1.0 } else { 0.0 }
            } else if integer {
                value.round()
            } else {
                value
            };
        }

        let groups = if self.descriptor.n_inputs > 0 {
            Self::groups_for_channels(2, self.descriptor.n_inputs as usize)
        } else {
            Self::groups_for_channels(2, self.descriptor.n_outputs as usize)
        };

        let mut units = Vec::with_capacity(groups);
        for _ in 0..groups {
            match create_unit(
                &self.meta,
                self.sample_rate,
                self.max_block,
                &self.parameter_values,
            ) {
                Ok(mut unit) => {
                    if let Some(activate) = self.meta.fns.activate {
                        // SAFETY: the handle is live and its ports have been connected.
                        unsafe { activate(unit.handle) };
                    }
                    unit.active = true;
                    units.push(unit);
                }
                Err(_) => {
                    for unit in units.drain(..) {
                        cleanup_unit(self.meta.fns, unit.handle, unit.active);
                    }
                    return;
                }
            }
        }
        self.units = units;
    }

    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [Vec<f32>]) {
        for output in outputs.iter_mut() {
            output.fill(0.0);
        }

        if self.units.is_empty() || outputs.is_empty() {
            return;
        }
        let frames = outputs.iter().map(Vec::len).max().unwrap_or(0);
        if frames == 0 {
            return;
        }

        let input_channels = self.meta.audio_input_ports.len();
        let output_channels = self.meta.audio_output_ports.len();
        let input_group_size = input_channels.max(1);
        let output_group_size = output_channels.max(1);

        let mut start = 0;
        while start < frames {
            let count = (frames - start).min(self.max_block);

            for (unit_index, unit) in self.units.iter_mut().enumerate() {
                for (local_channel, buffer) in unit.audio_inputs.iter_mut().enumerate() {
                    let source_channel = unit_index * input_group_size + local_channel;
                    let source = inputs.get(source_channel).copied().unwrap_or(&[]);
                    for offset in 0..count {
                        buffer[offset] = source.get(start + offset).copied().unwrap_or(0.0);
                    }
                }

                // SAFETY: the instance is active and all connected buffers hold at least
                // max_block samples; count is bounded by max_block.
                unsafe { (self.meta.fns.run)(unit.handle, count as c_ulong) };

                for (local_channel, buffer) in unit.audio_outputs.iter().enumerate() {
                    let destination_channel = unit_index * output_group_size + local_channel;
                    let Some(destination) = outputs.get_mut(destination_channel) else {
                        continue;
                    };
                    let available = destination.len().saturating_sub(start).min(count);
                    if available != 0 {
                        destination[start..start + available]
                            .copy_from_slice(&buffer[..available]);
                    }
                }
            }

            start += count;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed_amp() -> Option<PathBuf> {
        [
            "/usr/lib/ladspa/amp_1181.so",
            "/usr/lib/x86_64-linux-gnu/ladspa/amp_1181.so",
            "/usr/local/lib/ladspa/amp_1181.so",
        ]
        .iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
    }

    fn amp_descriptor(host: &LadspaHost, path: &Path) -> Option<PluginDescriptor> {
        let descriptors = host.scan(&[path.parent()?.to_path_buf()]);
        descriptors.into_iter().find(|descriptor| {
            descriptor.path.as_deref() == std::fs::canonicalize(path).ok().as_deref()
                && descriptor.name.to_ascii_lowercase().contains("amplifier")
        })
    }

    #[test]
    fn ladspa_amp_scans_processes_and_round_trips_state() {
        let Some(path) = installed_amp() else {
            eprintln!("skipping LADSPA amp test: system LADSPA plugins are not installed");
            return;
        };

        let host = LadspaHost::new();
        let Some(descriptor) = amp_descriptor(&host, &path) else {
            eprintln!("skipping LADSPA amp test: amp_1181.so could not be scanned");
            return;
        };
        let Some(gain) = descriptor
            .params
            .iter()
            .find(|parameter| parameter.name.eq_ignore_ascii_case("Amps gain (dB)"))
        else {
            eprintln!("skipping LADSPA amp test: expected gain control was not found");
            return;
        };

        let mut plugin = host.instantiate(&descriptor).unwrap();
        plugin.set_param(gain.id, 6.0206).unwrap();
        let saved = plugin.save_state().unwrap();
        plugin.set_param(gain.id, 0.0).unwrap();
        plugin.load_state(&saved).unwrap();
        assert!((plugin.get_param(gain.id).unwrap() - 6.0206).abs() < 1.0e-5);

        plugin.activate(48_000, 257);
        let input: Vec<f32> = (0..1024)
            .map(|index| (index as f32 * 0.031).sin() * 0.5)
            .collect();
        let mut output = vec![vec![0.0; input.len()]];
        plugin.process(&[&input], &mut output);

        for (expected, actual) in input.iter().zip(&output[0]) {
            assert!((actual - expected * 2.0).abs() < 1.0e-6);
        }
    }

    #[test]
    fn ladspa_amp_creates_separate_instances_for_stereo() {
        let Some(path) = installed_amp() else {
            eprintln!("skipping LADSPA stereo test: system LADSPA plugins are not installed");
            return;
        };

        let host = LadspaHost::new();
        let Some(descriptor) = amp_descriptor(&host, &path) else {
            eprintln!("skipping LADSPA stereo test: amp_1181.so could not be scanned");
            return;
        };
        let Some(gain) = descriptor
            .params
            .iter()
            .find(|parameter| parameter.name.eq_ignore_ascii_case("Amps gain (dB)"))
        else {
            eprintln!("skipping LADSPA stereo test: expected gain control was not found");
            return;
        };

        let mut plugin = host.instantiate(&descriptor).unwrap();
        plugin.set_param(gain.id, 6.0206).unwrap();
        plugin.activate(48_000, 128);
        // amp_1181 is mono: both channels below are only processed if one instance per channel exists.

        let left = vec![0.25; 256];
        let right = vec![-0.375; 256];
        let mut outputs = vec![vec![0.0; 256], vec![0.0; 256]];
        plugin.process(&[&left, &right], &mut outputs);

        for (input, output) in left.iter().zip(&outputs[0]) {
            assert!((output - input * 2.0).abs() < 1.0e-6);
        }
        for (input, output) in right.iter().zip(&outputs[1]) {
            assert!((output - input * 2.0).abs() < 1.0e-6);
        }
    }

    #[test]
    fn ladspa_scan_skips_non_plugin_shared_library() {
        let Some(_amp) = installed_amp() else {
            eprintln!("skipping LADSPA scan test: system LADSPA plugins are not installed");
            return;
        };

        let directory = std::env::temp_dir().join(format!(
            "au-ladspa-non-plugin-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let fake_library = directory.join("not-a-ladspa-plugin.so");
        std::fs::write(&fake_library, b"not a shared library").unwrap();

        let descriptors = LadspaHost::new().scan(std::slice::from_ref(&directory));
        assert!(descriptors.is_empty());

        let _ = std::fs::remove_dir_all(directory);
    }
}
