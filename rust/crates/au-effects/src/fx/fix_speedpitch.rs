// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.
//! Audacity-compatible speed, pitch, and tempo effects.

use crate::core::*;
use au_core::AudioBuffer;
use std::ffi::{c_double, c_int, c_uint, c_void};
use std::ptr;

const SOXR_BLOCK_SIZE: usize = 65_536;
const SOUND_TOUCH_MONO_BLOCK_SIZE: usize = 8_192;
const SOUND_TOUCH_STEREO_BLOCK_SIZE: usize = 65_536;

#[repr(C)]
struct SoxrQualitySpec {
    precision: c_double,
    phase_response: c_double,
    passband_end: c_double,
    stopband_begin: c_double,
    e: *mut c_void,
    flags: std::ffi::c_ulong,
}

#[link(name = "soxr")]
extern "C" {
    fn soxr_quality_spec(recipe: std::ffi::c_ulong, flags: std::ffi::c_ulong)
        -> SoxrQualitySpec;
    fn soxr_create(
        input_rate: c_double,
        output_rate: c_double,
        num_channels: c_uint,
        error: *mut *const std::ffi::c_char,
        io_spec: *const c_void,
        quality_spec: *const SoxrQualitySpec,
        runtime_spec: *const c_void,
    ) -> *mut c_void;
    fn soxr_process(
        resampler: *mut c_void,
        input: *const c_void,
        input_len: usize,
        input_done: *mut usize,
        output: *mut c_void,
        output_len: usize,
        output_done: *mut usize,
    ) -> *const std::ffi::c_char;
    fn soxr_delete(resampler: *mut c_void);
}

extern "C" {
    fn aust_new() -> *mut c_void;
    fn aust_free(instance: *mut c_void);
    fn aust_set_sample_rate(instance: *mut c_void, sample_rate: c_uint);
    fn aust_set_channels(instance: *mut c_void, channels: c_uint);
    fn aust_set_tempo_change(instance: *mut c_void, percent: c_double);
    fn aust_set_pitch_semitones(instance: *mut c_void, semitones: c_double);
    #[allow(dead_code)]
    fn aust_set_rate_change(instance: *mut c_void, percent: c_double);
    #[allow(dead_code)]
    fn aust_set_setting(instance: *mut c_void, setting: c_int, value: c_int) -> c_int;
    fn aust_put_samples(instance: *mut c_void, samples: *const f32, frames: c_uint);
    fn aust_receive_samples(instance: *mut c_void, samples: *mut f32, frames: c_uint)
        -> c_uint;
    fn aust_flush(instance: *mut c_void);
    fn aust_num_samples(instance: *mut c_void) -> c_uint;
}

struct SoxrHandle(*mut c_void);

impl Drop for SoxrHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the handle was returned by soxr_create and is deleted once.
            unsafe { soxr_delete(self.0) };
        }
    }
}

struct SoundTouchHandle(*mut c_void);

impl SoundTouchHandle {
    fn new() -> Result<Self, EffectError> {
        // SAFETY: aust_new creates and returns an owned SoundTouch instance.
        let instance = unsafe { aust_new() };
        if instance.is_null() {
            Err(EffectError::Invalid(
                "could not create SoundTouch instance".into(),
            ))
        } else {
            Ok(Self(instance))
        }
    }
}

impl Drop for SoundTouchHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the handle was returned by aust_new and is deleted once.
            unsafe { aust_free(self.0) };
        }
    }
}

fn clamped(value: f64, min: f64, max: f64, default: f64) -> f64 {
    if value.is_nan() {
        default
    } else {
        value.clamp(min, max)
    }
}

fn soxr_resample(input: &[f32], factor: f64) -> Result<Vec<f32>, EffectError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    // Audacity's Resample::SetMethod selects Best Quality (index 3), whose
    // recipe byte in "\0\1\4\6" is 6.
    // SAFETY: soxr_quality_spec returns the C ABI quality specification value.
    let quality = unsafe { soxr_quality_spec(6, 0) };
    let mut error = ptr::null();
    // Audacity creates a mono constant-rate stream with input rate 1 and
    // output rate equal to the speed-change factor.
    // SAFETY: arguments and structure layout match libsoxr's C API.
    let handle = unsafe {
        soxr_create(
            1.0,
            factor,
            1,
            &mut error,
            ptr::null(),
            &quality,
            ptr::null(),
        )
    };
    if handle.is_null() {
        return Err(EffectError::Invalid(
            "could not create libsoxr resampler".into(),
        ));
    }
    let handle = SoxrHandle(handle);
    let output_capacity = ((factor * SOXR_BLOCK_SIZE as f64) as usize).saturating_add(10);
    let output_capacity = output_capacity.max(10);
    let mut output = Vec::new();

    let mut sample_pos = 0usize;
    while sample_pos < input.len() {
        let block_size = SOXR_BLOCK_SIZE.min(input.len() - sample_pos);
        let mut in_block_offset = 0usize;

        while in_block_offset < block_size {
            let current_len = block_size - in_block_offset;
            let last = sample_pos + block_size >= input.len();
            let mut block_output = vec![0.0f32; output_capacity];
            let mut input_done = 0usize;
            let mut output_done = 0usize;
            let input_len = if last { !current_len } else { current_len };

            // SAFETY: the input slice remains valid for current_len samples,
            // output has output_capacity writable samples, and the handle is live.
            let result = unsafe {
                soxr_process(
                    handle.0,
                    input[sample_pos + in_block_offset..].as_ptr().cast(),
                    input_len,
                    &mut input_done,
                    block_output.as_mut_ptr().cast(),
                    block_output.len(),
                    &mut output_done,
                )
            };
            if !result.is_null() {
                return Err(EffectError::Invalid(
                    "libsoxr failed while resampling".into(),
                ));
            }

            output.extend_from_slice(&block_output[..output_done]);
            in_block_offset = in_block_offset.saturating_add(input_done);
            if input_done == 0 {
                break;
            }
        }

        sample_pos = sample_pos.saturating_add(block_size);
    }

    Ok(output)
}

fn soundtouch_drain(instance: *mut c_void, channels: usize, output: &mut [Vec<f32>]) {
    // SAFETY: the SoundTouch instance is live.
    let available = unsafe { aust_num_samples(instance) } as usize;
    if available == 0 {
        return;
    }

    let mut interleaved = vec![0.0f32; available.saturating_mul(channels)];
    // SAFETY: interleaved has room for the requested number of frames and all
    // channel data is stored interleaved as SoundTouch expects.
    let received = unsafe {
        aust_receive_samples(
            instance,
            interleaved.as_mut_ptr(),
            available.min(c_uint::MAX as usize) as c_uint,
        )
    } as usize;
    let received = received.min(available);

    for frame in 0..received {
        for channel in 0..channels {
            output[channel].push(interleaved[frame * channels + channel]);
        }
    }
}

fn soundtouch_process_group(
    input: &[&[f32]],
    sample_rate: u32,
    pitch_semitones: Option<f64>,
    tempo_change: Option<f64>,
    output: &mut [Vec<f32>],
) -> Result<(), EffectError> {
    let channels = input.len();
    if channels == 0 {
        return Ok(());
    }

    let instance = SoundTouchHandle::new()?;

    if let Some(semitones) = pitch_semitones {
        // Audacity casts the calculated semitone value to float before calling
        // SoundTouch's setPitchSemiTones overload.
        let semitones = (semitones as f32) as f64;
        // SAFETY: instance is a live SoundTouch object.
        unsafe { aust_set_pitch_semitones(instance.0, semitones) };
    }
    if let Some(tempo) = tempo_change {
        // SAFETY: instance is a live SoundTouch object.
        unsafe { aust_set_tempo_change(instance.0, tempo) };
    }

    // Match SoundTouchBase: set the channel count before setting the track rate.
    // SAFETY: instance is a live SoundTouch object.
    unsafe {
        aust_set_channels(instance.0, channels as c_uint);
        aust_set_sample_rate(instance.0, sample_rate);
    }

    let _ = SOUND_TOUCH_SETTING_USE_AA_FILTER;
    let _ = SOUND_TOUCH_SETTING_AA_FILTER_LENGTH;
    let _ = SOUND_TOUCH_SETTING_USE_QUICKSEEK;
    let _ = SOUND_TOUCH_SETTING_SEQUENCE_MS;
    let _ = SOUND_TOUCH_SETTING_SEEKWINDOW_MS;
    let _ = SOUND_TOUCH_SETTING_OVERLAP_MS;

    let frames = input.iter().map(|channel| channel.len()).min().unwrap_or(0);
    let block_limit = if channels == 1 {
        SOUND_TOUCH_MONO_BLOCK_SIZE
    } else {
        SOUND_TOUCH_STEREO_BLOCK_SIZE
    };
    let mut frame_pos = 0usize;
    let mut interleaved = Vec::new();

    while frame_pos < frames {
        let block_size = block_limit.min(frames - frame_pos);
        interleaved.clear();
        interleaved.reserve(block_size.saturating_mul(channels));
        for frame in 0..block_size {
            for channel in input {
                interleaved.push(channel[frame_pos + frame]);
            }
        }

        // SAFETY: interleaved contains block_size complete frames and the
        // SoundTouch instance is configured for this channel count.
        unsafe {
            aust_put_samples(
                instance.0,
                interleaved.as_ptr(),
                block_size.min(c_uint::MAX as usize) as c_uint,
            )
        };
        soundtouch_drain(instance.0, channels, output);
        frame_pos += block_size;
    }

    // Match SoundTouchBase::ProcessOne/ProcessStereo: flush, then drain all
    // samples made available by the flush.
    // SAFETY: instance is live and may be flushed once processing is complete.
    unsafe { aust_flush(instance.0) };
    soundtouch_drain(instance.0, channels, output);
    Ok(())
}

const SOUND_TOUCH_SETTING_USE_AA_FILTER: c_int = 0;
const SOUND_TOUCH_SETTING_AA_FILTER_LENGTH: c_int = 1;
const SOUND_TOUCH_SETTING_USE_QUICKSEEK: c_int = 2;
const SOUND_TOUCH_SETTING_SEQUENCE_MS: c_int = 3;
const SOUND_TOUCH_SETTING_SEEKWINDOW_MS: c_int = 4;
const SOUND_TOUCH_SETTING_OVERLAP_MS: c_int = 5;

fn process_soundtouch(
    buf: &mut AudioBuffer,
    pitch_semitones: Option<f64>,
    tempo_change: Option<f64>,
    preserve_length: bool,
) -> Result<(), EffectError> {
    let original_len = buf.len();
    if original_len == 0 || buf.channels.is_empty() {
        return Ok(());
    }

    let mut result = vec![Vec::new(); buf.channels.len()];

    if buf.channels.len() == 2 {
        let input = [&buf.channels[0][..], &buf.channels[1][..]];
        soundtouch_process_group(
            &input,
            buf.sample_rate,
            pitch_semitones,
            tempo_change,
            &mut result,
        )?;
    } else {
        // Audacity wave tracks are mono or stereo. Processing any additional
        // channels independently keeps the buffer well-defined.
        for (channel, samples) in buf.channels.iter().enumerate() {
            let input = [&samples[..]];
            soundtouch_process_group(
                &input,
                buf.sample_rate,
                pitch_semitones,
                tempo_change,
                &mut result[channel..=channel],
            )?;
        }
    }

    if preserve_length {
        for channel in &mut result {
            channel.truncate(original_len);
            channel.resize(original_len, 0.0);
        }
    }

    buf.channels = result;
    Ok(())
}

/// Change the speed and pitch together using Audacity's libsoxr resampling path.
#[derive(Clone, Debug)]
pub struct ChangeSpeed {
    /// Percentage speed change.
    pub percentage: f64,
}

impl Default for ChangeSpeed {
    fn default() -> Self {
        Self { percentage: 0.0 }
    }
}

impl BuiltinEffect for ChangeSpeed {
    fn id(&self) -> &'static str {
        "ChangeSpeed"
    }

    fn name(&self) -> &'static str {
        "Change Speed"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![ParamDef::float(
            "Percentage",
            "Percent Change",
            -99.0,
            4900.0,
            0.0,
        )]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Percentage" => Some(self.percentage),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        if key != "Percentage" {
            return false;
        }
        self.percentage = clamped(value, -99.0, 4900.0, 0.0);
        true
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if self.percentage == 0.0 || buf.channels.is_empty() || buf.len() == 0 {
            return Ok(());
        }

        let factor = 100.0 / (100.0 + self.percentage);
        let mut output_channels = Vec::with_capacity(buf.channels.len());
        for channel in &buf.channels {
            output_channels.push(soxr_resample(channel, factor)?);
        }
        buf.channels = output_channels;
        Ok(())
    }
}

/// Change pitch without changing tempo, using Audacity's SoundTouch path.
#[derive(Clone, Debug)]
pub struct ChangePitch {
    /// Percentage pitch change.
    pub percentage: f64,
    /// Whether the SBSMS option is selected (SoundTouch is used when unavailable).
    pub use_sbsms: bool,
}

impl Default for ChangePitch {
    fn default() -> Self {
        Self {
            percentage: 0.0,
            use_sbsms: false,
        }
    }
}

impl BuiltinEffect for ChangePitch {
    fn id(&self) -> &'static str {
        "ChangePitch"
    }

    fn name(&self) -> &'static str {
        "Change Pitch"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Percentage", "Percent Change", -99.0, 3000.0, 0.0),
            ParamDef::boolean("SBSMS", "Use high quality algorithm", false),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Percentage" => Some(self.percentage),
            "SBSMS" => Some(if self.use_sbsms { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Percentage" => {
                self.percentage = clamped(value, -99.0, 3000.0, 0.0);
                true
            }
            "SBSMS" | "UseSBSMS" => {
                self.use_sbsms = clamped(value, 0.0, 1.0, 0.0).round() != 0.0;
                true
            }
            _ => false,
        }
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if self.percentage == 0.0 || buf.channels.is_empty() || buf.len() == 0 {
            return Ok(());
        }

        let semitones =
            (12.0 * ((100.0 + self.percentage) / 100.0).ln()) / 2.0f64.ln();
        process_soundtouch(buf, Some(semitones), None, true)
    }
}

/// Change tempo without changing pitch, using Audacity's SoundTouch path.
#[derive(Clone, Debug)]
pub struct ChangeTempo {
    /// Percentage tempo change.
    pub percentage: f64,
    /// Whether the SBSMS option is selected (SoundTouch is used when unavailable).
    pub use_sbsms: bool,
}

impl Default for ChangeTempo {
    fn default() -> Self {
        Self {
            percentage: 0.0,
            use_sbsms: false,
        }
    }
}

impl BuiltinEffect for ChangeTempo {
    fn id(&self) -> &'static str {
        "ChangeTempo"
    }

    fn name(&self) -> &'static str {
        "Change Tempo"
    }

    fn params(&self) -> Vec<ParamDef> {
        vec![
            ParamDef::float("Percentage", "Percent Change", -95.0, 3000.0, 0.0),
            ParamDef::boolean("SBSMS", "Use high quality algorithm", false),
        ]
    }

    fn get(&self, key: &str) -> Option<f64> {
        match key {
            "Percentage" => Some(self.percentage),
            "SBSMS" => Some(if self.use_sbsms { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    fn set(&mut self, key: &str, value: f64) -> bool {
        match key {
            "Percentage" => {
                self.percentage = clamped(value, -95.0, 3000.0, 0.0);
                true
            }
            "SBSMS" | "UseSBSMS" => {
                self.use_sbsms = clamped(value, 0.0, 1.0, 0.0).round() != 0.0;
                true
            }
            _ => false,
        }
    }

    fn process(
        &mut self,
        buf: &mut AudioBuffer,
        _ctx: &mut EffectContext,
    ) -> Result<(), EffectError> {
        if self.percentage == 0.0 || buf.channels.is_empty() || buf.len() == 0 {
            return Ok(());
        }

        process_soundtouch(buf, None, Some(self.percentage), false)
    }
}

/// Returns one default instance of each effect in this module.
pub fn all() -> Vec<Box<dyn BuiltinEffect>> {
    vec![
        Box::new(ChangeSpeed::default()),
        Box::new(ChangePitch::default()),
        Box::new(ChangeTempo::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn short_stereo() -> AudioBuffer {
        let sample_rate = 44_100;
        let frames = 4_096;
        let left = (0..frames)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / sample_rate as f32).sin())
            .collect();
        let right = (0..frames)
            .map(|i| (std::f32::consts::TAU * 660.0 * i as f32 / sample_rate as f32).sin())
            .collect();
        AudioBuffer::from_channels(sample_rate, vec![left, right])
    }

    #[test]
    fn defaults_process_stereo_and_empty_buffers() {
        for mut effect in all() {
            let mut audio = short_stereo();
            effect
                .process(&mut audio, &mut EffectContext::default())
                .unwrap();

            let mut empty = AudioBuffer::from_channels(44_100, vec![Vec::new(), Vec::new()]);
            effect
                .process(&mut empty, &mut EffectContext::default())
                .unwrap();
        }
    }

    #[test]
    fn processing_is_repeatable() {
        for mut first in all() {
            let mut second: Box<dyn BuiltinEffect> = match first.id() {
                "ChangeSpeed" => Box::new(ChangeSpeed::default()),
                "ChangePitch" => Box::new(ChangePitch::default()),
                _ => Box::new(ChangeTempo::default()),
            };
            set_param(first.as_mut(), "Percentage", 20.0);
            set_param(second.as_mut(), "Percentage", 20.0);

            let mut a = short_stereo();
            let mut b = short_stereo();
            first
                .process(&mut a, &mut EffectContext::default())
                .unwrap();
            second
                .process(&mut b, &mut EffectContext::default())
                .unwrap();
            assert_eq!(a.channels, b.channels);
        }
    }

    #[test]
    fn set_param_clamps_values() {
        let mut speed = ChangeSpeed::default();
        assert!(set_param(&mut speed, "Percentage", 9_000.0));
        assert_eq!(speed.percentage, 4_900.0);

        let mut pitch = ChangePitch::default();
        assert!(set_param(&mut pitch, "Percentage", -500.0));
        assert_eq!(pitch.percentage, -99.0);
        assert!(set_param(&mut pitch, "SBSMS", 4.0));
        assert!(pitch.use_sbsms);

        let mut tempo = ChangeTempo::default();
        assert!(set_param(&mut tempo, "Percentage", -500.0));
        assert_eq!(tempo.percentage, -95.0);
    }
}
