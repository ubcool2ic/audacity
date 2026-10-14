use au_plugin_api::PluginHost;
use au_project::aup3::{Aup3Extras, Aup3Project, RealtimeEffect, RealtimeEffectList};
use std::error::Error;
use std::io;
use std::path::Path;

fn invalid_input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        return Err(invalid_input(
            "usage: aup3_vst3_state <again.vst3 bundle path> <audacity plugin id> <gain normalized f32> <out.aup3>",
        )
        .into());
    }

    let bundle_path = Path::new(&args[1]);
    let plugin_id = &args[2];
    let gain: f32 = args[3].parse()?;
    let out_path = Path::new(&args[4]);

    let descriptors = au_plugin_host::vst3::scan_file(bundle_path)?;
    let descriptor = descriptors
        .iter()
        .find(|descriptor| descriptor.name == "AGain VST3")
        .or_else(|| {
            descriptors.iter().find(|descriptor| {
                descriptor.name.contains("Gain") && !descriptor.name.contains("SideChain")
            })
        })
        .ok_or_else(|| invalid_input("no suitable Gain VST3 descriptor found"))?;

    let mut instance = au_plugin_host::Vst3Host::new().instantiate(descriptor)?;
    let gain_param = instance
        .descriptor()
        .params
        .iter()
        .find(|param| {
            param.name.to_lowercase().contains("gain")
                && !param.name.to_lowercase().contains("bypass")
        })
        .ok_or_else(|| invalid_input("no Gain parameter found"))?;
    let gain_param_id = gain_param.id;

    instance.set_param(gain_param_id, gain)?;
    instance.activate(44_100, 512);

    let silence = vec![0.0_f32; 512];
    let inputs: [&[f32]; 2] = [&silence, &silence];
    let mut outputs = vec![vec![0.0_f32; 512], vec![0.0_f32; 512]];
    instance.process(&inputs, &mut outputs);

    let state = instance.save_state()?;
    for (key, value) in &state {
        println!("{key}: {} bytes", value.len());
    }
    let readback = instance
        .get_param(gain_param_id)
        .ok_or_else(|| invalid_input("Gain parameter value is unavailable"))?;
    println!("Gain readback: {readback}");

    let samples: Vec<f32> = (0..88_200)
        .map(|sample| {
            (std::f32::consts::TAU * 440.0 * sample as f32 / 44_100.0).sin() * 0.8
        })
        .collect();
    let track = au_core::Track::new(
        "Sine",
        au_core::AudioBuffer::from_channels(44_100, vec![samples]),
    );
    let state_project = au_core::ProjectState {
        tracks: vec![track],
        sample_rate: 44_100,
        ..Default::default()
    };

    let extras = Aup3Extras {
        track_formats: vec![au_project::blocks::SampleFormat::Float],
        track_selected: vec![true],
        track_effects: vec![RealtimeEffectList {
            active: true,
            effects: vec![RealtimeEffect {
                id: plugin_id.clone(),
                active: true,
                params: state,
            }],
        }],
        ..Default::default()
    };

    au_project::aup3::save(
        out_path,
        &Aup3Project {
            state: state_project,
            extras,
        },
    )?;

    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
