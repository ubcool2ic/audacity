//! Offline effect rendering on the shared runtime pool.

use crate::core::{BuiltinEffect, EffectContext, OutLabel};
use au_core::AudioBuffer;
use au_runtime::compute::{map_items, Exec};
use au_runtime::pool::{isolate, CancelToken, JobError, Progress, Runtime};
use std::sync::OnceLock;

/// Returns the global shared runtime for offline work.
pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1);
        Runtime::new(threads, 64)
    })
}

/// Errors encountered while rendering an offline effect.
#[derive(Debug)]
pub enum OfflineError {
    /// The effect returned an error.
    Effect(String),
    /// The operation was cancelled.
    Cancelled,
    /// The effect panicked while processing.
    Panicked(String),
    /// Another runtime or setup error occurred.
    Other(String),
}

impl std::fmt::Display for OfflineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Effect(message) => write!(f, "effect failed: {message}"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::Panicked(message) => write!(f, "effect panicked: {message}"),
            Self::Other(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for OfflineError {}

impl From<JobError> for OfflineError {
    fn from(error: JobError) -> Self {
        match error {
            JobError::Cancelled => Self::Cancelled,
            JobError::Panicked(message) => Self::Panicked(message),
            JobError::Busy => Self::Other("offline runtime queue is busy".into()),
            JobError::Failed(message) => Self::Other(message),
        }
    }
}

type RenderedBuffer = (AudioBuffer, String, Vec<OutLabel>);

const EFFECT_ERROR_PREFIX: &str = "\u{1f}offline-effect-error:";

fn render_one(
    input: &AudioBuffer,
    aux: Option<&AudioBuffer>,
    make_effect: impl FnOnce() -> Result<Box<dyn BuiltinEffect>, JobError>,
) -> Result<RenderedBuffer, JobError> {
    isolate(|| {
        let mut effect = make_effect()?;
        let mut buffer = AudioBuffer::from_channels(input.sample_rate, input.channels.clone());
        let mut context = EffectContext {
            aux,
            ..EffectContext::default()
        };

        effect
            .process(&mut buffer, &mut context)
            .map_err(|error| JobError::Failed(format!("{EFFECT_ERROR_PREFIX}{error}")))?;

        Ok((buffer, context.report, context.labels))
    })
}

fn map_render_error(error: JobError) -> OfflineError {
    match error {
        JobError::Failed(message) => match message.strip_prefix(EFFECT_ERROR_PREFIX) {
            Some(effect_message) => OfflineError::Effect(effect_message.to_owned()),
            None => OfflineError::Other(message),
        },
        other => OfflineError::from(other),
    }
}

/// Renders an effect over independent buffers.
///
/// Each buffer receives a fresh effect instance with the template's numeric and
/// text settings copied from a preset. Effects whose state cannot be captured
/// that way are rendered serially using instances supplied by `make`.
pub fn render_buffers(
    exec: Exec,
    template: &mut dyn BuiltinEffect,
    make: Option<&(dyn Fn() -> Box<dyn BuiltinEffect> + Sync)>,
    buffers: Vec<AudioBuffer>,
    aux: Option<&AudioBuffer>,
    cancel: &CancelToken,
    progress: &Progress,
) -> Result<Vec<RenderedBuffer>, OfflineError> {
    cancel.check()?;

    let id = template.id();
    let can_copy_settings =
        id != "NoiseReduction" && crate::registry::create(&id).is_some();

    if can_copy_settings {
        let preset = crate::stack::Preset::capture("", template);
        let result = runtime().pool().install(|| {
            map_items(exec, &buffers, cancel, progress, |input| {
                render_one(input, aux, || {
                    let mut effect = crate::registry::create(&id).ok_or_else(|| {
                        JobError::Failed(format!("effect {id} is not registered"))
                    })?;
                    preset.apply(effect.as_mut());
                    Ok(effect)
                })
            })
        });
        result.map_err(map_render_error)
    } else {
        let factory = make.ok_or_else(|| {
            OfflineError::Other(format!(
                "effect {id} requires a caller-supplied instance factory"
            ))
        })?;

        let result = runtime().pool().install(|| {
            map_items(Exec::Serial, &buffers, cancel, progress, |input| {
                render_one(input, aux, || Ok(factory()))
            })
        });
        result.map_err(map_render_error)
    }
}

/// Processes one buffer on the shared pool, containing panics from the effect.
pub fn process_isolated(
    effect: &mut dyn BuiltinEffect,
    buf: &mut AudioBuffer,
    ctx: &mut EffectContext,
) -> Result<(), OfflineError> {
    let result = runtime().pool().install(|| {
        isolate(|| {
            effect
                .process(buf, ctx)
                .map_err(|error| JobError::Failed(error.to_string()))
        })
    });

    match result {
        Ok(()) => Ok(()),
        Err(JobError::Cancelled) => Err(OfflineError::Cancelled),
        Err(JobError::Panicked(message)) => Err(OfflineError::Panicked(message)),
        Err(JobError::Busy) => Err(OfflineError::Other(
            "offline runtime queue is busy".into(),
        )),
        Err(JobError::Failed(message)) => Err(OfflineError::Effect(message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{set_param, EffectError, ParamType};
    use crate::stack::Preset;

    fn make_buffer(seed: usize, frames: usize) -> AudioBuffer {
        let make_channel = |channel: usize| {
            (0..frames)
                .map(|frame| {
                    let value = (frame * 37 + seed * 101 + channel * 211) % 2003;
                    value as f32 / 1001.5 - 1.0
                })
                .collect::<Vec<_>>()
        };
        AudioBuffer::from_channels(44_100, vec![make_channel(0), make_channel(1)])
    }

    fn make_buffers(count: usize) -> Vec<AudioBuffer> {
        (0..count)
            .map(|index| make_buffer(index, 44_100 + index * 3_777))
            .collect()
    }

    fn set_non_default_params(effect: &mut dyn BuiltinEffect) {
        for parameter in effect.params() {
            let candidate = match parameter.ty {
                ParamType::Bool => {
                    if parameter.default == 0.0 {
                        1.0
                    } else {
                        0.0
                    }
                }
                ParamType::Choice(_) => {
                    if parameter.default == 0.0 && parameter.max >= 1.0 {
                        1.0
                    } else {
                        0.0
                    }
                }
                ParamType::Int => {
                    if parameter.default + 1.0 <= parameter.max {
                        parameter.default + 1.0
                    } else if parameter.default - 1.0 >= parameter.min {
                        parameter.default - 1.0
                    } else {
                        parameter.default
                    }
                }
                ParamType::Float => {
                    let delta = ((parameter.max - parameter.min).abs() * 0.005).clamp(0.001, 0.5);
                    if parameter.default + delta <= parameter.max {
                        parameter.default + delta
                    } else if parameter.default - delta >= parameter.min {
                        parameter.default - delta
                    } else {
                        parameter.default
                    }
                }
            };
            assert!(set_param(effect, parameter.key, candidate));
        }
    }

    fn direct_results(id: &str, preset: &Preset, inputs: Vec<AudioBuffer>) -> Vec<AudioBuffer> {
        inputs
            .into_iter()
            .map(|mut buffer| {
                let mut effect =
                    crate::registry::create(id).unwrap_or_else(|| panic!("missing effect {id}"));
                preset.apply(effect.as_mut());
                let mut context = EffectContext::default();
                effect.process(&mut buffer, &mut context).unwrap();
                buffer
            })
            .collect()
    }

    fn assert_same_audio(left: &[AudioBuffer], right: &[AudioBuffer]) {
        assert_eq!(left.len(), right.len());
        for (left, right) in left.iter().zip(right) {
            assert_eq!(left.sample_rate, right.sample_rate);
            assert_eq!(left.channels.len(), right.channels.len());
            for (left_channel, right_channel) in left.channels.iter().zip(&right.channels) {
                assert_eq!(left_channel.len(), right_channel.len());
                for (left_sample, right_sample) in left_channel.iter().zip(right_channel) {
                    assert_eq!(left_sample.to_bits(), right_sample.to_bits());
                }
            }
        }
    }

    #[test]
    fn parallel_serial_and_direct_render_are_bit_identical() {
        for id in [
            "Amplify",
            "Echo",
            "Reverb",
            "Phaser",
            "Compressor",
            "Paulstretch",
            "BassAndTreble",
        ] {
            let mut template =
                crate::registry::create(id).unwrap_or_else(|| panic!("missing effect {id}"));
            set_non_default_params(template.as_mut());
            let preset = Preset::capture("", template.as_mut());

            let parallel = render_buffers(
                Exec::Parallel,
                template.as_mut(),
                None,
                make_buffers(8),
                None,
                &CancelToken::new(),
                &Progress::new(8),
            )
            .unwrap()
            .into_iter()
            .map(|item| item.0)
            .collect::<Vec<_>>();

            let serial = render_buffers(
                Exec::Serial,
                template.as_mut(),
                None,
                make_buffers(8),
                None,
                &CancelToken::new(),
                &Progress::new(8),
            )
            .unwrap()
            .into_iter()
            .map(|item| item.0)
            .collect::<Vec<_>>();

            let direct = direct_results(id, &preset, make_buffers(8));
            assert_same_audio(&parallel, &serial);
            assert_same_audio(&parallel, &direct);
        }
    }

    #[test]
    fn parallel_reverb_stress_is_repeatable() {
        let mut template = crate::registry::create("Reverb").expect("Reverb should be registered");

        let first = render_buffers(
            Exec::Parallel,
            template.as_mut(),
            None,
            make_buffers(64),
            None,
            &CancelToken::new(),
            &Progress::new(64),
        )
        .unwrap()
        .into_iter()
        .map(|item| item.0)
        .collect::<Vec<_>>();

        let second = render_buffers(
            Exec::Parallel,
            template.as_mut(),
            None,
            make_buffers(64),
            None,
            &CancelToken::new(),
            &Progress::new(64),
        )
        .unwrap()
        .into_iter()
        .map(|item| item.0)
        .collect::<Vec<_>>();

        assert_same_audio(&first, &second);
    }

    struct PanickingEffect;

    impl BuiltinEffect for PanickingEffect {
        fn id(&self) -> std::borrow::Cow<'static, str> { std::borrow::Cow::Borrowed({
            "NoiseReduction"
        }) }

        fn params(&self) -> Vec<crate::core::ParamDef> {
            Vec::new()
        }

        fn get(&self, _key: &str) -> Option<f64> {
            None
        }

        fn set(&mut self, _key: &str, _value: f64) -> bool {
            false
        }

        fn process(
            &mut self,
            _buf: &mut AudioBuffer,
            _ctx: &mut EffectContext,
        ) -> Result<(), EffectError> {
            panic!("injected effect panic");
        }
    }

    #[test]
    fn panics_are_isolated_and_do_not_poison_later_work() {
        let mut template = PanickingEffect;
        let factory = || Box::new(PanickingEffect) as Box<dyn BuiltinEffect>;
        let result = render_buffers(
            Exec::Parallel,
            &mut template,
            Some(&factory),
            vec![make_buffer(0, 256)],
            None,
            &CancelToken::new(),
            &Progress::new(1),
        );
        assert!(matches!(result, Err(OfflineError::Panicked(_))));

        let mut effect = PanickingEffect;
        let mut buffer = make_buffer(1, 256);
        let mut context = EffectContext::default();
        assert!(matches!(
            process_isolated(&mut effect, &mut buffer, &mut context),
            Err(OfflineError::Panicked(_))
        ));

        let mut normal = crate::registry::create("Echo").expect("Echo should be registered");
        assert!(render_buffers(
            Exec::Parallel,
            normal.as_mut(),
            None,
            vec![make_buffer(2, 256)],
            None,
            &CancelToken::new(),
            &Progress::new(1),
        )
        .is_ok());
    }

    #[test]
    fn cancelled_render_returns_cancelled() {
        let mut template = crate::registry::create("Echo").expect("Echo should be registered");
        let cancel = CancelToken::new();
        cancel.cancel();

        let result = render_buffers(
            Exec::Parallel,
            template.as_mut(),
            None,
            vec![make_buffer(0, 128)],
            None,
            &cancel,
            &Progress::new(1),
        );

        assert!(matches!(result, Err(OfflineError::Cancelled)));
    }
}
