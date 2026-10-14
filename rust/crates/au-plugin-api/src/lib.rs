// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

use au_core::AudioBuffer;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PluginFormat {
    Native,
    Vst3,
    Clap,
    Lv2,
    Ladspa,
    Nyquist,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PluginDescriptor {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub format: PluginFormat,
    pub path: Option<PathBuf>,
    pub params: Vec<ParamInfo>,
    pub n_inputs: u32,
    pub n_outputs: u32,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PluginError {
    #[error("plugin format {0:?} is not implemented")]
    NotImplemented(PluginFormat),
    #[error("plugin load failed: {0}")]
    LoadFailed(String),
    #[error("{0}")]
    Unknown(String),
}

pub trait PluginInstance: Send {
    fn descriptor(&self) -> &PluginDescriptor;

    fn set_param(&mut self, id: u32, v: f32) -> Result<(), PluginError>;

    fn get_param(&self, id: u32) -> Option<f32>;

    fn activate(&mut self, sample_rate: u32, max_block: usize);

    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [Vec<f32>]);

    fn process_buffer(&mut self, buf: &mut AudioBuffer) {
        const BLOCK_SIZE: usize = 1024;

        let frames = buf.len();
        // Stage 1: the host maps the plugin onto the buffer's own channel layout.
        let n_inputs = buf.n_channels();
        let n_outputs = buf.n_channels();
        let mut result = vec![vec![0.0; frames]; n_outputs];

        for start in (0..frames).step_by(BLOCK_SIZE) {
            let end = (start + BLOCK_SIZE).min(frames);
            let block_len = end - start;
            let mut input_blocks = Vec::with_capacity(n_inputs);

            for channel in 0..n_inputs {
                if let Some(samples) = buf.channels.get(channel) {
                    input_blocks.push(&samples[start..end]);
                } else {
                    input_blocks.push(&[][..]);
                }
            }

            let input_refs: Vec<&[f32]> = input_blocks;
            let mut output_blocks = vec![vec![0.0; block_len]; n_outputs];
            self.process(&input_refs, &mut output_blocks);

            for (channel, output) in output_blocks.iter().enumerate() {
                let copied = output.len().min(block_len);
                result[channel][start..start + copied].copy_from_slice(&output[..copied]);
            }
        }

        buf.channels = result;
    }
}

pub trait PluginHost: Send {
    fn format(&self) -> PluginFormat;

    fn default_search_paths(&self) -> Vec<PathBuf>;

    fn scan(&self, paths: &[PathBuf]) -> Vec<PluginDescriptor>;

    fn instantiate(&self, d: &PluginDescriptor) -> Result<Box<dyn PluginInstance>, PluginError>;
}

pub trait NativePlugin: PluginInstance {}

pub struct NativeHost {
    factories: Vec<(PluginDescriptor, fn() -> Box<dyn PluginInstance>)>,
}

impl NativeHost {
    pub fn new() -> Self {
        let mut host = Self {
            factories: Vec::new(),
        };
        host.register(gain_descriptor(), create_gain_plugin);
        host
    }

    pub fn register(
        &mut self,
        descriptor: PluginDescriptor,
        factory: fn() -> Box<dyn PluginInstance>,
    ) {
        self.factories.push((descriptor, factory));
    }
}

impl Default for NativeHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost for NativeHost {
    fn format(&self) -> PluginFormat {
        PluginFormat::Native
    }

    fn default_search_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn scan(&self, _paths: &[PathBuf]) -> Vec<PluginDescriptor> {
        self.factories
            .iter()
            .map(|(descriptor, _)| descriptor.clone())
            .collect()
    }

    fn instantiate(&self, d: &PluginDescriptor) -> Result<Box<dyn PluginInstance>, PluginError> {
        self.factories
            .iter()
            .find(|(descriptor, _)| descriptor.id == d.id)
            .map(|(_, factory)| factory())
            .ok_or_else(|| PluginError::LoadFailed(d.id.clone()))
    }
}

#[derive(Clone, Debug)]
struct GainPlugin {
    descriptor: PluginDescriptor,
    gain: f32,
}

impl GainPlugin {
    fn new() -> Self {
        Self {
            descriptor: gain_descriptor(),
            gain: 1.0,
        }
    }
}

impl PluginInstance for GainPlugin {
    fn descriptor(&self) -> &PluginDescriptor {
        &self.descriptor
    }

    fn set_param(&mut self, id: u32, v: f32) -> Result<(), PluginError> {
        if id != 0 {
            return Err(PluginError::Unknown(format!("unknown parameter {id}")));
        }
        if !v.is_finite() {
            return Err(PluginError::Unknown("parameter must be finite".into()));
        }
        self.gain = v.clamp(0.0, 4.0);
        Ok(())
    }

    fn get_param(&self, id: u32) -> Option<f32> {
        (id == 0).then_some(self.gain)
    }

    fn activate(&mut self, _sample_rate: u32, _max_block: usize) {}

    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [Vec<f32>]) {
        for (channel, output) in outputs.iter_mut().enumerate() {
            let input = inputs.get(channel).copied().unwrap_or(&[]);
            for (index, sample) in output.iter_mut().enumerate() {
                *sample = input.get(index).copied().unwrap_or(0.0) * self.gain;
            }
        }
    }
}

impl NativePlugin for GainPlugin {}

fn gain_descriptor() -> PluginDescriptor {
    PluginDescriptor {
        id: "au-core.gain".into(),
        name: "Gain".into(),
        vendor: "Audacity".into(),
        format: PluginFormat::Native,
        path: None,
        params: vec![ParamInfo {
            id: 0,
            name: "Gain".into(),
            min: 0.0,
            max: 4.0,
            default: 1.0,
        }],
        n_inputs: 2,
        n_outputs: 2,
    }
}

fn create_gain_plugin() -> Box<dyn PluginInstance> {
    Box::new(GainPlugin::new())
}

macro_rules! stub_host {
    ($name:ident, $format:ident, $extension:expr, $env:expr, $dirs:expr) => {
        pub struct $name;

        impl $name {
            pub fn new() -> Self {
                Self
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl PluginHost for $name {
            fn format(&self) -> PluginFormat {
                PluginFormat::$format
            }

            fn default_search_paths(&self) -> Vec<PathBuf> {
                let mut paths = Vec::new();

                if let Some(home) = std::env::var_os("HOME") {
                    let home = PathBuf::from(home);
                    paths.extend(($dirs).iter().map(|dir| home.join(dir)));
                }

                paths.extend(($dirs).iter().map(PathBuf::from));

                if let Some(value) = std::env::var_os($env) {
                    paths.extend(std::env::split_paths(&value));
                }

                paths
            }

            fn scan(&self, paths: &[PathBuf]) -> Vec<PluginDescriptor> {
                let mut descriptors = Vec::new();

                for path in paths {
                    let Ok(entries) = std::fs::read_dir(path) else {
                        continue;
                    };

                    for entry in entries.flatten() {
                        let candidate = entry.path();
                        let is_directory = candidate.is_dir();
                        let extension_matches = candidate
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| ext.eq_ignore_ascii_case($extension));

                        let matches = if PluginFormat::$format == PluginFormat::Lv2 {
                            is_directory
                                && candidate
                                    .extension()
                                    .and_then(|ext| ext.to_str())
                                    .is_some_and(|ext| ext.eq_ignore_ascii_case("lv2"))
                        } else if PluginFormat::$format == PluginFormat::Vst3
                            || PluginFormat::$format == PluginFormat::Clap
                        {
                            // .clap is a shared library file on Linux/Windows and a bundle on macOS;
                            // .vst3 is a bundle directory (or a single file on older Windows installs).
                            extension_matches
                        } else {
                            !is_directory && extension_matches
                        };

                        if !matches {
                            continue;
                        }

                        let name = candidate
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .unwrap_or("Unknown Plugin")
                            .to_owned();

                        descriptors.push(PluginDescriptor {
                            id: format!("{:?}:{}", PluginFormat::$format, candidate.display()),
                            name,
                            vendor: String::new(),
                            format: PluginFormat::$format,
                            path: Some(candidate),
                            params: Vec::new(),
                            n_inputs: 0,
                            n_outputs: 0,
                        });
                    }
                }

                descriptors
            }

            fn instantiate(
                &self,
                _d: &PluginDescriptor,
            ) -> Result<Box<dyn PluginInstance>, PluginError> {
                Err(PluginError::NotImplemented(PluginFormat::$format))
            }
        }
    };
}

stub_host!(
    Vst3Host,
    Vst3,
    "vst3",
    "VST3_PATH",
    &[".vst3", "/usr/lib/vst3", "/usr/local/lib/vst3"]
);
stub_host!(
    ClapHost,
    Clap,
    "clap",
    "CLAP_PATH",
    &[".clap", "/usr/lib/clap", "/usr/local/lib/clap"]
);
stub_host!(
    Lv2Host,
    Lv2,
    "lv2",
    "LV2_PATH",
    &[".lv2", "/usr/lib/lv2", "/usr/local/lib/lv2"]
);
stub_host!(
    LadspaHost,
    Ladspa,
    "so",
    "LADSPA_PATH",
    &[".ladspa", "/usr/lib/ladspa", "/usr/local/lib/ladspa"]
);

pub struct PluginRegistry {
    hosts: Vec<Box<dyn PluginHost>>,
    pub plugins: Vec<PluginDescriptor>,
}

impl PluginRegistry {
    pub fn with_default_hosts() -> Self {
        Self {
            hosts: vec![
                Box::new(NativeHost::new()),
                Box::new(Vst3Host::new()),
                Box::new(ClapHost::new()),
                Box::new(Lv2Host::new()),
                Box::new(LadspaHost::new()),
            ],
            plugins: Vec::new(),
        }
    }

    pub fn add_host(&mut self, host: Box<dyn PluginHost>) {
        self.hosts.push(host);
    }

    pub fn rescan(&mut self) {
        self.plugins.clear();
        for host in &self.hosts {
            let paths = host.default_search_paths();
            self.plugins.extend(host.scan(&paths));
        }
    }

    pub fn find(&self, id: &str) -> Option<&PluginDescriptor> {
        self.plugins.iter().find(|plugin| plugin.id == id)
    }

    pub fn instantiate(&self, id: &str) -> Result<Box<dyn PluginInstance>, PluginError> {
        let descriptor = self
            .find(id)
            .ok_or_else(|| PluginError::Unknown(format!("plugin not found: {id}")))?;

        self.hosts
            .iter()
            .find(|host| host.format() == descriptor.format)
            .ok_or_else(|| PluginError::Unknown(format!("no host for plugin: {id}")))?
            .instantiate(descriptor)
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::with_default_hosts()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_gain_processes_audio_buffer() {
        let mut registry = PluginRegistry::with_default_hosts();
        registry.rescan();

        let mut plugin = registry.instantiate("au-core.gain").unwrap();
        plugin.set_param(0, 2.0).unwrap();

        let mut buffer = AudioBuffer::from_channels(48_000, vec![vec![0.25, -0.5]]);
        plugin.process_buffer(&mut buffer);

        assert_eq!(buffer.channels, vec![vec![0.5, -1.0]]);
    }

    #[test]
    fn clap_host_scans_and_reports_not_implemented() {
        let root = std::env::temp_dir().join(format!(
            "au-plugin-api-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let fake_plugin = root.join("fake.clap");
        std::fs::write(&fake_plugin, b"stub").unwrap();

        let host = ClapHost::new();
        let descriptors = host.scan(std::slice::from_ref(&root));
        let descriptor = descriptors
            .iter()
            .find(|descriptor| descriptor.path.as_ref() == Some(&fake_plugin))
            .unwrap();

        assert_eq!(descriptor.name, "fake");
        assert!(matches!(
            host.instantiate(descriptor),
            Err(PluginError::NotImplemented(PluginFormat::Clap))
        ));

        let _ = std::fs::remove_dir_all(root);
    }
}
