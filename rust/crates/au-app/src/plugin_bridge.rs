// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

use au_core::AudioBuffer;
use au_effects::core::{BuiltinEffect, EffectContext, EffectError, ParamDef};
use au_effects::stack::StackSlot;
use au_plugin_api::{PluginDescriptor, PluginFormat, PluginHost, PluginInstance};
use au_project::aup3::RealtimeEffect;

/// Interns strings required by `ParamDef`, whose keys and labels are `'static`.
///
/// Plugin ids and names are stored as owned strings; they do not use this interner.
fn intern(s: &str) -> &'static str {
    static INTERNED: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();

    let table = INTERNED.get_or_init(|| Mutex::new(HashSet::new()));
    let mut table = table
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    if let Some(value) = table.get(s) {
        return value;
    }

    let value: &'static str = Box::leak(s.to_owned().into_boxed_str());
    table.insert(value);
    value
}

/// Returns whether `id` identifies a supported hosted plugin.
pub fn is_plugin_id(id: &str) -> bool {
    matches!(
        au_plugin_host::state::parse_effect_id(id),
        Some((PluginFormat::Vst3 | PluginFormat::Clap, _, _))
    )
}

/// A hosted VST3 or CLAP plugin exposed as a built-in effect.
pub struct PluginEffect {
    id: String,
    name: String,
    params: Vec<ParamDef>,
    ids: Vec<u32>,
    inner: Mutex<Box<dyn PluginInstance>>,
    rate: u32,
    channels: usize,
    scratch: Vec<Vec<f32>>,
}

impl PluginEffect {
    /// Creates a bridge around an instantiated plugin.
    pub fn new(inst: Box<dyn PluginInstance>) -> Self {
        let descriptor = inst.descriptor().clone();
        let mut params = Vec::with_capacity(descriptor.params.len());
        let mut ids = Vec::with_capacity(descriptor.params.len());

        // ParamDef keys and labels must be &'static str, so only these strings
        // are interned; the plugin id and display name remain owned strings.
        for param in &descriptor.params {
            let key = intern(&format!("p{}", param.id));
            let label = intern(&param.name);
            params.push(ParamDef::float(
                key,
                label,
                param.min as f64,
                param.max as f64,
                param.default as f64,
            ));
            ids.push(param.id);
        }

        Self {
            id: descriptor.id,
            name: descriptor.name,
            params,
            ids,
            inner: Mutex::new(inst),
            rate: 44_100,
            channels: 0,
            scratch: Vec::new(),
        }
    }

    /// Saves the plugin's serialized state.
    pub fn state(&self) -> Vec<(String, String)> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .save_state()
            .unwrap_or_default()
    }

    /// Loads serialized plugin state, returning whether loading succeeded.
    pub fn load(&self, state: &[(String, String)]) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .load_state(state)
            .is_ok()
    }

    /// Returns the plugin descriptor.
    pub fn descriptor(&self) -> PluginDescriptor {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .descriptor()
            .clone()
    }
}

impl BuiltinEffect for PluginEffect {
    fn id(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Owned(self.id.clone())
    }

    fn name(&self) -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Owned(self.name.clone())
    }

    fn params(&self) -> Vec<ParamDef> {
        self.params.clone()
    }

    fn get(&self, key: &str) -> Option<f64> {
        let index = self.params.iter().position(|param| param.key == key)?;
        let id = *self.ids.get(index)?;
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_param(id)
            .map(f64::from)
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        let Some(index) = self.params.iter().position(|param| param.key == key) else {
            return false;
        };
        let Some(&id) = self.ids.get(index) else {
            return false;
        };
        let param = &self.params[index];
        let value = value.clamp(param.min, param.max) as f32;

        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .set_param(id, value)
            .is_ok()
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext<'_>,
    ) -> Result<(), EffectError> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.activate(buf.sample_rate, 4096);
        inner.process_buffer(buf);
        Ok(())
    }

    fn supports_realtime(&self) -> bool {
        true
    }

    fn rt_reset(&mut self, sample_rate: u32, channels: usize) {
        self.rate = sample_rate;
        self.channels = channels;

        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.activate(sample_rate, 4096);
        drop(inner);

        self.scratch = vec![vec![0.0; 4096]; channels];
    }

    fn rt_process(&mut self, block: &mut [Vec<f32>]) {
        let channels = block.len();
        if self.channels != channels {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            inner.activate(self.rate, 4096);
            drop(inner);

            self.channels = channels;
            self.scratch = vec![vec![0.0; 4096]; channels];
        }

        let block_len = block.iter().map(Vec::len).min().unwrap_or(0);
        let mut offset = 0;

        while offset < block_len {
            let frames = (block_len - offset).min(4096);
            let inputs: Vec<&[f32]> = block
                .iter()
                .map(|channel| &channel[offset..offset + frames])
                .collect();

            for output in &mut self.scratch {
                output.resize(frames, 0.0);
                output.fill(0.0);
            }

            {
                let mut inner = self
                    .inner
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                inner.process(&inputs, &mut self.scratch);
            }

            for (channel_index, channel) in block.iter_mut().enumerate() {
                if let Some(output) = self.scratch.get(channel_index) {
                    let copy_len = frames.min(output.len());
                    channel[offset..offset + copy_len].copy_from_slice(&output[..copy_len]);
                }
            }

            offset += frames;
        }
    }

    fn rt_latency(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .latency_samples() as usize
    }

    fn text_keys(&self) -> Vec<&'static str> {
        match self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .descriptor()
            .format
        {
            PluginFormat::Vst3 => vec![
                au_plugin_host::state::PROCESSOR_STATE,
                au_plugin_host::state::CONTROLLER_STATE,
                au_plugin_host::state::PARAMETERS,
            ],
            PluginFormat::Clap => vec![
                au_plugin_host::state::CLAP_STATE,
                au_plugin_host::state::PARAMETERS,
            ],
            _ => Vec::new(),
        }
    }

    fn get_text(&mut self, key: &str) -> Option<String> {
        let inner = self
            .inner
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner
            .save_state()
            .ok()?
            .into_iter()
            .find_map(|(state_key, value)| (state_key == key).then_some(value))
    }

    fn set_text(&mut self, key: &str, value: &str) -> bool {
        let inner = self
            .inner
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.load_state(&[(key.to_owned(), value.to_owned())]).is_ok()
    }
}

/// Restores a saved realtime plugin as a stack slot.
pub fn slot_from_saved(e: &RealtimeEffect) -> Option<StackSlot> {
    if !is_plugin_id(&e.id) {
        return None;
    }

    let registry = au_plugin_host::registry();
    let instance = au_plugin_host::restore_effect(&registry, e).ok()?;

    Some(StackSlot {
        active: e.active,
        effect: Box::new(PluginEffect::new(instance)),
    })
}

/// Serializes a hosted plugin effect into a realtime-effect record.
pub fn saved_from_effect(
    effect: &mut dyn BuiltinEffect,
    active: bool,
) -> Option<RealtimeEffect> {
    let id = effect.plugin_id();
    if !is_plugin_id(&id) {
        return None;
    }

    let params = effect
        .text_keys()
        .into_iter()
        .filter_map(|key| {
            effect
                .get_text(key)
                .map(|value| (key.to_owned(), value))
        })
        .collect();

    Some(RealtimeEffect { id, active, params })
}

/// Asynchronous catalog of available hosted plugins.
pub struct PluginCatalog {
    /// Discovered VST3, CLAP, LV2, LADSPA, and Nyquist plugin descriptors.
    pub plugins: Vec<PluginDescriptor>,
    rt: au_runtime::Runtime,
    job: Option<au_runtime::JobHandle<Vec<PluginDescriptor>>>,
    /// Plugin ids disabled in the persisted plugin-manager settings.
    pub disabled: std::collections::BTreeSet<String>,
    /// Path used to persist disabled plugin ids, if configuration is available.
    pub config_path: Option<std::path::PathBuf>,
    /// The most recent scanning or configuration error, if any.
    pub last_error: Option<String>,
}

impl PluginCatalog {
    /// Creates a catalog and loads disabled plugin ids from the default config path.
    pub fn new() -> Self {
        Self::with_config_path(Self::default_config_path())
    }

    /// Creates a catalog and loads disabled plugin ids from `path`, if present.
    pub fn with_config_path(path: Option<std::path::PathBuf>) -> Self {
        let mut catalog = Self {
            plugins: Vec::new(),
            rt: au_runtime::Runtime::new(1, 2),
            job: None,
            disabled: std::collections::BTreeSet::new(),
            config_path: path,
            last_error: None,
        };

        if let Some(path) = catalog.config_path.as_ref() {
            match std::fs::read_to_string(path) {
                Ok(contents) => {
                    catalog.disabled = contents
                        .lines()
                        .map(str::trim)
                        .filter(|id| !id.is_empty())
                        .map(str::to_owned)
                        .collect();
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    catalog.last_error = Some(format!(
                        "Could not read disabled plugin list {}: {error}",
                        path.display()
                    ));
                }
            }
        }

        catalog
    }

    fn default_config_path() -> Option<std::path::PathBuf> {
        let config_dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .map(|home| home.join(".config"))
            })?;

        Some(
            config_dir
                .join("audacity-rs")
                .join("plugins-disabled.txt"),
        )
    }

    fn save_disabled(&mut self) {
        let Some(path) = self.config_path.as_ref() else {
            return;
        };

        let result = (|| {
            if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }

            let contents = self
                .disabled
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(path, contents)
        })();

        match result {
            Ok(()) => self.last_error = None,
            Err(error) => {
                self.last_error = Some(format!(
                    "Could not save disabled plugin list {}: {error}",
                    path.display()
                ));
            }
        }
    }

    /// Returns whether the plugin with `id` is enabled.
    pub fn is_enabled(&self, id: &str) -> bool {
        !self.disabled.contains(id)
    }

    /// Enables or disables a plugin and immediately persists the change.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        if enabled {
            self.disabled.remove(id);
        } else {
            self.disabled.insert(id.to_owned());
        }
        self.save_disabled();
    }

    /// Iterates over enabled plugin descriptors.
    pub fn enabled(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.plugins
            .iter()
            .filter(|descriptor| self.is_enabled(&descriptor.id))
    }

    /// Returns the display label for a plugin format.
    pub fn format_label(format: PluginFormat) -> &'static str {
        match format {
            PluginFormat::Vst3 => "VST3",
            PluginFormat::Clap => "CLAP",
            PluginFormat::Lv2 => "LV2",
            PluginFormat::Ladspa => "LADSPA",
            PluginFormat::Nyquist => "Nyquist",
            PluginFormat::Native => "Built-in",
        }
    }

    /// Starts scanning for plugins if a scan is not already running.
    pub fn start_scan(&mut self) {
        let _ = self.poll();
        if self.job.is_some() {
            return;
        }

        self.job = Some(self.rt.submit(1, |_| {
            let mut registry = au_plugin_host::registry();
            registry.rescan();
            let plugins = registry
                .plugins
                .into_iter()
                .filter(|descriptor| {
                    matches!(
                        descriptor.format,
                        PluginFormat::Vst3
                            | PluginFormat::Clap
                            | PluginFormat::Lv2
                            | PluginFormat::Ladspa
                            | PluginFormat::Nyquist
                    )
                })
                .collect();
            Ok(plugins)
        }));
    }

    /// Collects the scan result if it is ready, returning whether one was collected.
    pub fn poll(&mut self) -> bool {
        let result = match self.job.as_mut() {
            Some(job) => job.try_result(),
            None => return false,
        };

        let Some(result) = result else {
            return false;
        };
        self.job = None;

        match result {
            Ok(mut plugins) => {
                plugins.sort_by(|a, b| a.name.cmp(&b.name));
                self.plugins = plugins;
                self.last_error = None;
            }
            Err(error) => {
                self.last_error = Some(format!("{error:?}"));
            }
        }

        true
    }

    /// Returns whether a scan is currently running.
    pub fn is_scanning(&self) -> bool {
        self.job.is_some()
    }

    /// Finds a plugin by its descriptor id.
    pub fn find(&self, id: &str) -> Option<&PluginDescriptor> {
        self.plugins.iter().find(|descriptor| descriptor.id == id)
    }

    /// Instantiates a catalog plugin by descriptor id.
    pub fn instantiate(&self, id: &str) -> Result<PluginEffect, String> {
        let descriptor = self
            .find(id)
            .ok_or_else(|| format!("Plugin not found: {id}"))?;

        if !self.is_enabled(id) {
            return Err(format!("Plugin is disabled: {}", descriptor.name));
        }

        let instance: Box<dyn PluginInstance> = match descriptor.format {
            PluginFormat::Vst3 => au_plugin_host::Vst3Host::new()
                .instantiate(descriptor)
                .map_err(|error| format!("{error:?}"))?,
            PluginFormat::Clap => au_plugin_host::ClapHost::new()
                .instantiate(descriptor)
                .map_err(|error| format!("{error:?}"))?,
            PluginFormat::Lv2 => au_plugin_host::lv2::Lv2Host::new()
                .instantiate(descriptor)
                .map_err(|error| format!("{error:?}"))?,
            PluginFormat::Ladspa => au_plugin_host::ladspa::LadspaHost::new()
                .instantiate(descriptor)
                .map_err(|error| format!("{error:?}"))?,
            PluginFormat::Nyquist => au_plugin_host::nyquist::NyquistHost::new()
                .instantiate(descriptor)
                .map_err(|error| format!("{error:?}"))?,
            PluginFormat::Native => {
                return Err(format!("Unsupported plugin format for {id}"));
            }
        };

        Ok(PluginEffect::new(instance))
    }
}

impl Default for PluginCatalog {
    fn default() -> Self {
        Self::new()
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn intern_deduplicates_strings() {
        let first = intern("plugin-bridge-test-string");
        let second = intern("plugin-bridge-test-string");
        assert!(std::ptr::eq(first, second));
    }

    #[test]
    fn recognizes_plugin_ids() {
        assert!(is_plugin_id("Effect_VST3_V_N_/a/b.vst3;ABC"));
        assert!(!is_plugin_id("builtin.gain"));
    }

    #[test]
    fn again_vst3_round_trip_and_realtime_processing() {
        let path = Path::new(
            "/workspace/plugins/vst3sdk/build-gui/VST3/Release/again.vst3",
        );
        if !path.exists() {
            return;
        }

        let descriptors = au_plugin_host::vst3::scan_file(path).unwrap_or_default();
        let Some(descriptor) = descriptors
            .into_iter()
            .find(|descriptor| descriptor.name == "AGain VST3")
        else {
            return;
        };

        let Ok(instance) = au_plugin_host::Vst3Host::new().instantiate(&descriptor) else {
            return;
        };
        let mut effect = PluginEffect::new(instance);
        let Some(gain_key) = effect
            .params()
            .into_iter()
            .find(|param| param.label.to_ascii_lowercase().contains("gain"))
            .map(|param| param.key)
        else {
            return;
        };
        assert!(effect.set(gain_key, 0.5));

        let mut buffer = AudioBuffer {
            sample_rate: 44_100,
            channels: vec![(
                0..44_100)
                    .map(|sample| {
                        0.8 * (std::f32::consts::TAU * 440.0 * sample as f32 / 44_100.0).sin()
                    })
                    .collect(),
            ],
        };

        let mut stack = au_effects::stack::EffectStack::new();
        assert!(stack.push_effect(Box::new(effect)));
        let _ = stack.render(&mut buffer);

        let peak = buffer.channels[0]
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max);
        assert!((peak - 0.4).abs() < 0.01);

        let saved = saved_from_effect(stack.slots[0].effect.as_mut(), true).unwrap();
        let restored = slot_from_saved(&saved).unwrap();
        let restored_gain = restored.effect.get(gain_key).unwrap();
        assert!((restored_gain - 0.5).abs() < 0.01);
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEST_ID: AtomicUsize = AtomicUsize::new(0);

    fn test_path() -> PathBuf {
        let id = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "audacity-rs-plugin-catalog-{}-{id}-disabled.txt",
            std::process::id()
        ))
    }

    fn fake_descriptor(id: &str) -> PluginDescriptor {
        PluginDescriptor {
            id: id.to_owned(),
            name: "Test plugin".to_owned(),
            vendor: "Test vendor".to_owned(),
            path: PathBuf::from("/tmp/test-plugin.vst3"),
            format: PluginFormat::Vst3,
        }
    }

    #[test]
    fn disabled_plugins_persist_and_cannot_be_instantiated() {
        let path = test_path();
        let id = "test.plugin.disabled";

        let mut catalog = PluginCatalog::with_config_path(Some(path.clone()));
        catalog.plugins.push(fake_descriptor(id));
        catalog.set_enabled(id, false);

        assert!(!catalog.is_enabled(id));
        let persisted = PluginCatalog::with_config_path(Some(path.clone()));
        assert!(!persisted.is_enabled(id));

        assert_eq!(
            catalog.instantiate(id).unwrap_err(),
            "Plugin is disabled: Test plugin"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn format_labels_match_plugin_manager_labels() {
        assert_eq!(PluginCatalog::format_label(PluginFormat::Vst3), "VST3");
        assert_eq!(PluginCatalog::format_label(PluginFormat::Clap), "CLAP");
        assert_eq!(PluginCatalog::format_label(PluginFormat::Lv2), "LV2");
        assert_eq!(PluginCatalog::format_label(PluginFormat::Ladspa), "LADSPA");
        assert_eq!(PluginCatalog::format_label(PluginFormat::Nyquist), "Nyquist");
        assert_eq!(PluginCatalog::format_label(PluginFormat::Native), "Built-in");
    }
}
