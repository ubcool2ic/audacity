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
    /// Discovered VST3 and CLAP plugin descriptors.
    pub plugins: Vec<PluginDescriptor>,
    rt: au_runtime::Runtime,
    job: Option<au_runtime::JobHandle<Vec<PluginDescriptor>>>,
    /// The most recent scanning error, if any.
    pub last_error: Option<String>,
}

impl PluginCatalog {
    /// Creates an empty catalog.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            rt: au_runtime::Runtime::new(1, 2),
            job: None,
            last_error: None,
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
                        PluginFormat::Vst3 | PluginFormat::Clap
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

        let instance: Box<dyn PluginInstance> = match descriptor.format {
            PluginFormat::Vst3 => au_plugin_host::Vst3Host::new()
                .instantiate(descriptor)
                .map_err(|error| format!("{error:?}"))?,
            PluginFormat::Clap => au_plugin_host::ClapHost::new()
                .instantiate(descriptor)
                .map_err(|error| format!("{error:?}"))?,
            _ => return Err(format!("Unsupported plugin format for {id}")),
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
