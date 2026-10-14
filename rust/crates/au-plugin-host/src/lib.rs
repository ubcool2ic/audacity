// SPDX-License-Identifier: GPL-3.0-or-later

pub mod state;
pub mod clap;
pub mod vst3;
pub mod sandbox;
pub mod ladspa;
pub mod lv2;
pub mod editor;

pub use clap::ClapHost;
pub use sandbox::{SandboxedHost, ScanOutcome};
pub use vst3::Vst3Host;

use au_plugin_api::{
    NativeHost, PluginDescriptor, PluginError, PluginFormat, PluginHost, PluginInstance,
    PluginRegistry,
};
use au_project::aup3::RealtimeEffect;
use std::path::PathBuf;

/// Creates the plugin registry used by the application.
pub fn registry() -> PluginRegistry {
    PluginRegistry::with_hosts(vec![
        Box::new(NativeHost::new()) as Box<dyn PluginHost>,
        Box::new(SandboxedHost::new(Vst3Host::new())),
        Box::new(SandboxedHost::new(ClapHost::new())),
        Box::new(lv2::Lv2Host::new()),
        Box::new(ladspa::LadspaHost::new()),
    ])
}

/// Saves a realtime effect's identity, active state, and plugin state.
pub fn save_effect(
    inst: &mut dyn PluginInstance,
    active: bool,
) -> Result<RealtimeEffect, PluginError> {
    Ok(RealtimeEffect {
        id: inst.descriptor().id.clone(),
        active,
        params: inst.save_state()?,
    })
}

/// Instantiates an effect and restores its saved plugin state.
pub fn restore_effect(
    reg: &PluginRegistry,
    effect: &RealtimeEffect,
) -> Result<Box<dyn PluginInstance>, PluginError> {
    match reg.instantiate(&effect.id) {
        Ok(mut instance) => {
            instance.load_state(&effect.params)?;
            Ok(instance)
        }
        Err(registry_error) => {
            let Some((format, path, uid)) = state::parse_effect_id(&effect.id) else {
                return Err(registry_error);
            };

            let descriptor = PluginDescriptor {
                id: effect.id.clone(),
                name: uid,
                vendor: String::new(),
                format,
                path: Some(PathBuf::from(&path)),
                params: Vec::new(),
                n_inputs: 0,
                n_outputs: 0,
            };

            let mut instance: Box<dyn PluginInstance> = match format {
                PluginFormat::Clap => ClapHost::new().instantiate(&descriptor)?,
                PluginFormat::Vst3 => Vst3Host::new().instantiate(&descriptor)?,
                PluginFormat::Ladspa => ladspa::LadspaHost::new().instantiate(&descriptor)?,
                PluginFormat::Lv2 => lv2::Lv2Host::new().instantiate(&descriptor)?,
                _ => return Err(registry_error),
            };
            instance.load_state(&effect.params)?;
            Ok(instance)
        }
    }
}

