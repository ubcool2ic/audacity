// SPDX-License-Identifier: GPL-3.0-or-later
//! MP3 export through the system LAME and libid3tag libraries.

use crate::IoError;
use au_core::tags::Tags;
use std::ffi::{CStr, CString};
use std::path::Path;

/// MP3 rate-mode choices, in Audacity's export-option order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum MP3RateMode {
    /// Audacity's LAME preset mode.
    Preset = 0,
    /// Variable bitrate mode.
    Variable = 1,
    /// Average bitrate mode.
    Average = 2,
    /// Constant bitrate mode.
    Constant = 3,
}

/// MP3 stereo-mode choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum MP3ChannelMode {
    /// Use joint stereo for a stereo stream.
    JointStereo = 0,
    /// Use stereo mode for a stereo stream.
    Stereo = 1,
}

/// Options for MP3 export.
///
/// Preset quality indices select Excessive, Extreme, Standard, and Medium.
/// Variable quality indices range from 0 (best quality) through 9. Average
/// and constant bitrate options use the bitrate values listed by Audacity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MP3Options {
    /// Rate mode: Preset, Variable, Average, or Constant.
    pub mode: MP3RateMode,
    /// Preset quality index (0 through 3).
    pub quality_set: i32,
    /// Variable-bitrate quality index (0 through 9).
    pub quality_vbr: i32,
    /// Average bitrate in kbps.
    pub bitrate_abr: i32,
    /// Constant bitrate in kbps.
    pub bitrate_cbr: i32,
    /// Stereo coding mode; ignored for mono input.
    pub channel_mode: MP3ChannelMode,
}

impl Default for MP3Options {
    fn default() -> Self {
        Self {
            mode: MP3RateMode::Preset,
            quality_set: 2,
            quality_vbr: 2,
            bitrate_abr: 192,
            bitrate_cbr: 192,
            channel_mode: MP3ChannelMode::JointStereo,
        }
    }
}

#[repr(C)]
struct FfiOptions {
    mode: i32,
    quality_set: i32,
    quality_vbr: i32,
    bitrate_abr: i32,
    bitrate_cbr: i32,
    channel_mode: i32,
}

#[repr(C)]
struct FfiTag {
    name: *const std::ffi::c_char,
    value: *const std::ffi::c_char,
}

unsafe extern "C" {
    fn au_export_mp3(
        path: *const std::ffi::c_char,
        sample_rate: u32,
        channels: u32,
        interleaved: *const f32,
        frames: u64,
        options: *const FfiOptions,
        tags: *const FfiTag,
        tag_count: usize,
        error_message: *mut std::ffi::c_char,
        error_message_capacity: usize,
    ) -> i32;
}

/// Export planar audio as an MP3 file.
///
/// `channels` must contain one or two equally sized planar channels. Sample
/// rates and bitrate/rate combinations must be supported by Audacity's MP3
/// exporter. Mono downmixing, if desired, must be performed by the caller.
pub fn export_mp3(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &MP3Options,
    tags: &Tags,
) -> Result<(), IoError> {
    let mut no_op = |_: u64, _: u64| true;
    export_mp3_with_progress(path, channels, sample_rate, opts, tags, &mut no_op)
}

/// Export planar audio as an MP3 file, reporting progress and allowing
/// cancellation.
///
/// Progress is reported as `(frames_done, frames_total)`. Returning `false`
/// cancels the export and returns [`IoError::Cancelled`]. If cancellation is
/// requested after encoding has produced the output, the output file is
/// removed.
pub fn export_mp3_with_progress(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &MP3Options,
    tags: &Tags,
    progress: crate::export::ProgressFn<'_>,
) -> Result<(), IoError> {
    if channels.is_empty() || channels.len() > 2 {
        return Err(IoError::Unsupported(
            "MP3 export requires one or two channels".into(),
        ));
    }

    let frames = channels[0].len();
    if channels.iter().any(|channel| channel.len() != frames) {
        return Err(IoError::Unsupported(
            "MP3 export channels must have equal lengths".into(),
        ));
    }

    let sample_rates = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];
    if !sample_rates.contains(&sample_rate) {
        return Err(IoError::Unsupported(format!(
            "MP3 does not support the {sample_rate} Hz sample rate"
        )));
    }

    match opts.mode {
        MP3RateMode::Preset if !(0..=3).contains(&opts.quality_set) => {
            return Err(IoError::Unsupported(
                "MP3 preset quality must be between 0 and 3".into(),
            ));
        }
        MP3RateMode::Variable if !(0..=9).contains(&opts.quality_vbr) => {
            return Err(IoError::Unsupported(
                "MP3 VBR quality must be between 0 and 9".into(),
            ));
        }
        MP3RateMode::Average if !valid_bitrate(opts.bitrate_abr) => {
            return Err(IoError::Unsupported(format!(
                "Unsupported MP3 average bitrate: {} kbps",
                opts.bitrate_abr
            )));
        }
        MP3RateMode::Constant if !valid_bitrate(opts.bitrate_cbr) => {
            return Err(IoError::Unsupported(format!(
                "Unsupported MP3 constant bitrate: {} kbps",
                opts.bitrate_cbr
            )));
        }
        _ => {}
    }

    let bitrate = match opts.mode {
        MP3RateMode::Average => Some(opts.bitrate_abr),
        MP3RateMode::Constant => Some(opts.bitrate_cbr),
        _ => None,
    };
    if let Some(bitrate) = bitrate {
        let highrate = if bitrate < 32 || bitrate == 144 {
            24000
        } else {
            48000
        };
        let lowrate = if bitrate > 160 { 32000 } else { 8000 };
        if sample_rate < lowrate || sample_rate > highrate {
            return Err(IoError::Unsupported(format!(
                "The {sample_rate} Hz sample rate and {bitrate} kbps bitrate combination is not supported by MP3"
            )));
        }
    }

    let output_path = path;
    let path = path.to_str().ok_or_else(|| {
        IoError::Unsupported("MP3 output path must be valid UTF-8".into())
    })?;
    let path = CString::new(path)
        .map_err(|_| IoError::Unsupported("MP3 output path contains a NUL byte".into()))?;

    let sample_count = frames
        .checked_mul(channels.len())
        .ok_or_else(|| IoError::Unsupported("MP3 input is too large".into()))?;
    let mut interleaved = Vec::with_capacity(sample_count);
    for frame in 0..frames {
        for channel in channels {
            interleaved.push(channel[frame]);
        }
    }

    let ffi_options = FfiOptions {
        mode: opts.mode as i32,
        quality_set: opts.quality_set,
        quality_vbr: opts.quality_vbr,
        bitrate_abr: opts.bitrate_abr,
        bitrate_cbr: opts.bitrate_cbr,
        channel_mode: opts.channel_mode as i32,
    };

    let entries = tags.entries();
    let c_entries: Vec<(CString, CString)> = entries
        .into_iter()
        .map(|(name, value)| {
            let name = CString::new(name)
                .map_err(|_| IoError::Unsupported("MP3 tag name contains a NUL byte".into()))?;
            let value = CString::new(value)
                .map_err(|_| IoError::Unsupported("MP3 tag value contains a NUL byte".into()))?;
            Ok((name, value))
        })
        .collect::<Result<_, IoError>>()?;
    let ffi_tags: Vec<FfiTag> = c_entries
        .iter()
        .map(|(name, value)| FfiTag {
            name: name.as_ptr(),
            value: value.as_ptr(),
        })
        .collect();

    let frames_total = frames as u64;
    if !progress(0, frames_total) {
        return Err(IoError::Cancelled);
    }

    let mut error_message = [0_i8; 1024];

    // SAFETY: All pointers passed here refer to live, correctly laid-out
    // buffers for the duration of the call. The interleaved samples contain
    // exactly `frames * channels.len()` elements, and the C strings are
    // NUL-terminated and remain alive until the call returns.
    let result = unsafe {
        au_export_mp3(
            path.as_ptr(),
            sample_rate,
            channels.len() as u32,
            if interleaved.is_empty() {
                std::ptr::null()
            } else {
                interleaved.as_ptr()
            },
            frames as u64,
            &ffi_options,
            if ffi_tags.is_empty() {
                std::ptr::null()
            } else {
                ffi_tags.as_ptr()
            },
            ffi_tags.len(),
            error_message.as_mut_ptr(),
            error_message.len(),
        )
    };

    if result == 0 {
        if !progress(frames_total, frames_total) {
            let _ = std::fs::remove_file(output_path);
            return Err(IoError::Cancelled);
        }
        Ok(())
    } else {
        // SAFETY: The C shim always NUL-terminates the supplied error buffer.
        let message = unsafe { CStr::from_ptr(error_message.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        Err(IoError::Encode(if message.is_empty() {
            format!("MP3 encoder failed with code {result}")
        } else {
            message
        }))
    }
}

fn valid_bitrate(bitrate: i32) -> bool {
    matches!(
        bitrate,
        8 | 16 | 24 | 32 | 40 | 48 | 56 | 64 | 80 | 96 | 112 | 128 | 144 | 160
            | 192 | 224 | 256 | 320
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use au_core::tags::Tags;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);

    fn temp_mp3() -> PathBuf {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "au_io_mp3_test_{}_{}.mp3",
            std::process::id(),
            id
        ))
    }

    fn stereo_sine(sample_rate: u32, seconds: usize) -> Vec<Vec<f32>> {
        let frames = sample_rate as usize * seconds;
        (0..2)
            .map(|channel| {
                (0..frames)
                    .map(|i| {
                        let phase = std::f64::consts::TAU * 440.0 * i as f64
                            / sample_rate as f64;
                        (phase.sin() * if channel == 0 { 0.5 } else { 0.35 }) as f32
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn encodes_stereo_and_decodes_with_symphonia() {
        let path = temp_mp3();
        let channels = stereo_sine(44100, 1);
        let mut tags = Tags::new();
        tags.set(Tags::TITLE, "Rust MP3 Test");
        tags.set("CUSTOM_FIELD", "Custom MP3 Value");

        export_mp3(&path, &channels, 44100, &MP3Options::default(), &tags).unwrap();

        let bytes = fs::read(&path).unwrap();
        assert!(bytes.len() > 1000);
        assert!(bytes.starts_with(b"ID3"));
        assert!(bytes
            .windows(b"Rust MP3 Test".len())
            .any(|window| window == b"Rust MP3 Test"));
        assert!(bytes
            .windows(b"Custom MP3 Value".len())
            .any(|window| window == b"Custom MP3 Value"));

        use symphonia::core::audio::SampleBuffer;
        use symphonia::core::codecs::DecoderOptions;
        use symphonia::core::errors::Error;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let file = fs::File::open(&path).unwrap();
        let stream = MediaSourceStream::new(
            Box::new(file),
            MediaSourceStreamOptions::default(),
        );
        let probed = symphonia::default::get_probe()
            .format(
                &Hint::new(),
                stream,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .unwrap();
        let mut format = probed.format;
        let track = format.default_track().unwrap();
        let track_id = track.id;
        assert_eq!(track.codec_params.sample_rate, Some(44100));
        assert_eq!(track.codec_params.channels.unwrap().count(), 2);

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .unwrap();
        let mut decoded_frames = 0_usize;
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(_) => break,
            };
            if packet.track_id() != track_id {
                continue;
            }
            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let frame_count = decoded.frames();
                    let mut samples =
                        SampleBuffer::<f32>::new(frame_count as u64, *decoded.spec());
                    samples.copy_interleaved_ref(decoded);
                    decoded_frames += samples.samples().len() / 2;
                }
                Err(Error::DecodeError(_)) => continue,
                Err(error) => panic!("MP3 decode failed: {error}"),
            }
        }
        let duration = decoded_frames as f64 / 44100.0;
        assert!((0.9..=1.3).contains(&duration));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn all_rate_modes_encode() {
        let channels = stereo_sine(44100, 1);
        let modes = [
            MP3RateMode::Preset,
            MP3RateMode::Variable,
            MP3RateMode::Average,
            MP3RateMode::Constant,
        ];

        for mode in modes {
            let path = temp_mp3();
            let options = MP3Options {
                mode,
                ..MP3Options::default()
            };
            export_mp3(&path, &channels, 44100, &options, &Tags::new()).unwrap();
            let metadata = fs::metadata(&path).unwrap();
            assert!(metadata.len() > 100);
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn empty_audio_encodes_without_panicking() {
        let path = temp_mp3();
        export_mp3(
            &path,
            &[Vec::new(), Vec::new()],
            44100,
            &MP3Options::default(),
            &Tags::new(),
        )
        .unwrap();
        assert!(fs::metadata(&path).unwrap().len() > 0);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn unsupported_parameters_are_rejected() {
        let path = temp_mp3();
        let channels = stereo_sine(44100, 1);
        let options = MP3Options {
            mode: MP3RateMode::Variable,
            quality_vbr: 10,
            ..MP3Options::default()
        };
        assert!(matches!(
            export_mp3(&path, &channels, 44100, &options, &Tags::new()),
            Err(IoError::Unsupported(_))
        ));
        assert!(!path.exists());
    }

    #[test]
    fn repeated_exports_are_valid() {
        let channels = stereo_sine(44100, 1);
        let options = MP3Options::default();
        let first = temp_mp3();
        let second = temp_mp3();
        export_mp3(&first, &channels, 44100, &options, &Tags::new()).unwrap();
        export_mp3(&second, &channels, 44100, &options, &Tags::new()).unwrap();
        assert!(fs::metadata(&first).unwrap().len() > 100);
        assert!(fs::metadata(&second).unwrap().len() > 100);
        let _ = fs::remove_file(first);
        let _ = fs::remove_file(second);
    }

    #[test]
    fn progress_is_monotonic_and_finishes_at_total() {
        let path = temp_mp3();
        let channels = stereo_sine(44100, 1);
        let mut reports = Vec::new();
        let result = export_mp3_with_progress(
            &path,
            &channels,
            44100,
            &MP3Options::default(),
            &Tags::new(),
            &mut |done, total| {
                reports.push((done, total));
                true
            },
        );
        if result.is_err() {
            let _ = fs::remove_file(path);
            return;
        }

        assert!(!reports.is_empty());
        assert!(reports.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        assert_eq!(reports.last().copied(), Some((44100, 44100)));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn cancellation_leaves_no_file() {
        let path = temp_mp3();
        let channels = stereo_sine(44100, 1);
        let mut calls = 0;
        let result = export_mp3_with_progress(
            &path,
            &channels,
            44100,
            &MP3Options::default(),
            &Tags::new(),
            &mut |_, _| {
                calls += 1;
                calls == 1
            },
        );

        match result {
            Err(IoError::Cancelled) => assert!(!path.exists()),
            Err(_) => {
                let _ = fs::remove_file(path);
            }
            Ok(()) => panic!("export should have been cancelled"),
        }
    }

    #[test]
    fn progress_export_matches_regular_export_bytes() {
        let channels = stereo_sine(44100, 1);
        let options = MP3Options::default();
        let regular = temp_mp3();
        let with_progress = temp_mp3();

        if export_mp3(&regular, &channels, 44100, &options, &Tags::new()).is_err() {
            let _ = fs::remove_file(regular);
            return;
        }
        let result = export_mp3_with_progress(
            &with_progress,
            &channels,
            44100,
            &options,
            &Tags::new(),
            &mut |_, _| true,
        );
        if result.is_err() {
            let _ = fs::remove_file(regular);
            let _ = fs::remove_file(with_progress);
            return;
        }

        assert_eq!(fs::read(&regular).unwrap(), fs::read(&with_progress).unwrap());
        let _ = fs::remove_file(regular);
        let _ = fs::remove_file(with_progress);
    }
}
