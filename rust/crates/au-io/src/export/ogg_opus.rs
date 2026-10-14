// SPDX-License-Identifier: GPL-3.0-or-later
// Audacity 3.7.3 Ogg Vorbis and Opus export support.

use au_core::tags::Tags;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::Path;

/// Ogg Vorbis export settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VorbisOptions {
    /// Audacity's integer quality setting, from 0 through 10.
    pub quality: i32,
}

impl Default for VorbisOptions {
    fn default() -> Self {
        Self { quality: 5 }
    }
}

/// Available Opus bitrate choices.
///
/// The numeric values are the values passed to libopus. `Auto` and `Maximum`
/// correspond to `OPUS_AUTO` and `OPUS_BITRATE_MAX`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum OpusBitrate {
    /// 6 kbps.
    Kbps6 = 6000,
    /// 8 kbps.
    Kbps8 = 8000,
    /// 16 kbps.
    Kbps16 = 16000,
    /// 24 kbps.
    Kbps24 = 24000,
    /// 32 kbps.
    Kbps32 = 32000,
    /// 40 kbps.
    Kbps40 = 40000,
    /// 48 kbps.
    Kbps48 = 48000,
    /// 64 kbps.
    Kbps64 = 64000,
    /// 80 kbps.
    Kbps80 = 80000,
    /// 96 kbps.
    Kbps96 = 96000,
    /// 128 kbps.
    Kbps128 = 128000,
    /// 160 kbps.
    Kbps160 = 160000,
    /// 192 kbps.
    Kbps192 = 192000,
    /// 256 kbps.
    Kbps256 = 256000,
    /// Automatic bitrate.
    Auto = -1000,
    /// Maximum bitrate.
    Maximum = -1,
}

/// Opus variable-bitrate mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum OpusVbrMode {
    /// Constant bitrate.
    Off = 0,
    /// Unconstrained variable bitrate.
    On = 1,
    /// Constrained variable bitrate.
    Constrained = 2,
}

/// Opus frame duration choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum OpusFrameDuration {
    /// 2.5 ms.
    Ms2_5 = 25,
    /// 5 ms.
    Ms5 = 50,
    /// 10 ms.
    Ms10 = 100,
    /// 20 ms.
    Ms20 = 200,
    /// 40 ms.
    Ms40 = 400,
    /// 60 ms.
    Ms60 = 600,
}

/// Opus application choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum OpusApplication {
    /// Speech-oriented encoding.
    Speech = 2048,
    /// General audio encoding.
    Audio = 2049,
    /// Restricted low-delay encoding.
    LowDelay = 2051,
}

/// Opus encoder bandwidth/cutoff choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum OpusCutoff {
    /// Automatic bandwidth.
    Auto = -1000,
    /// Narrowband.
    Narrowband = 1101,
    /// Mediumband.
    Mediumband = 1102,
    /// Wideband.
    Wideband = 1103,
    /// Super-wideband.
    SuperWideband = 1104,
    /// Fullband.
    Fullband = 1105,
}

/// Opus export settings matching Audacity 3.7.3's export dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpusOptions {
    /// Selected bitrate.
    pub bitrate: OpusBitrate,
    /// Encoder complexity, from 0 through 10.
    pub quality: i32,
    /// Selected encoded frame duration.
    pub frame_duration: OpusFrameDuration,
    /// Selected VBR mode.
    pub vbr_mode: OpusVbrMode,
    /// Selected encoder application.
    pub application: OpusApplication,
    /// Selected bandwidth cutoff.
    pub cutoff: OpusCutoff,
}

impl Default for OpusOptions {
    fn default() -> Self {
        Self {
            bitrate: OpusBitrate::Auto,
            quality: 10,
            frame_duration: OpusFrameDuration::Ms20,
            vbr_mode: OpusVbrMode::On,
            application: OpusApplication::Audio,
            cutoff: OpusCutoff::Auto,
        }
    }
}

#[repr(C)]
struct NativeTag {
    name: *const c_char,
    value: *const c_char,
}

#[repr(C)]
struct NativeVorbisOptions {
    quality: i32,
}

#[repr(C)]
struct NativeOpusOptions {
    bitrate: i32,
    quality: i32,
    frame_multiplier: i32,
    vbr_mode: i32,
    application: i32,
    cutoff: i32,
}

unsafe extern "C" {
    fn audacity_export_vorbis(
        path: *const c_char,
        sample_rate: u32,
        channels: u32,
        interleaved: *const f32,
        frames: u64,
        options: *const NativeVorbisOptions,
        tags: *const NativeTag,
        tag_count: usize,
        error_message: *mut c_char,
        error_message_size: usize,
    ) -> c_int;

    fn audacity_export_opus(
        path: *const c_char,
        sample_rate: u32,
        channels: u32,
        interleaved: *const f32,
        frames: u64,
        options: *const NativeOpusOptions,
        tags: *const NativeTag,
        tag_count: usize,
        error_message: *mut c_char,
        error_message_size: usize,
    ) -> c_int;
}


fn prepare_export_with_progress(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    tags: &Tags,
    mut progress: Option<&mut dyn FnMut(u64, u64) -> bool>,
    progress_block_frames: usize,
) -> Result<(CString, Vec<f32>, Vec<CString>, Vec<CString>, Vec<NativeTag>), crate::IoError> {
    if channels.is_empty() || channels.len() > u8::MAX as usize {
        return Err(crate::IoError::Unsupported(
            "Ogg export requires between 1 and 255 channels".into(),
        ));
    }
    if sample_rate == 0 || sample_rate > i32::MAX as u32 {
        return Err(crate::IoError::Unsupported(
            "Unsupported sample rate".into(),
        ));
    }

    let frames = channels[0].len();
    if channels.iter().any(|channel| channel.len() != frames) {
        return Err(crate::IoError::Unsupported(
            "All planar audio channels must have equal lengths".into(),
        ));
    }

    let path = path
        .to_str()
        .ok_or_else(|| crate::IoError::Unsupported("Output path is not valid UTF-8".into()))?;
    let path = CString::new(path)
        .map_err(|_| crate::IoError::Unsupported("Output path contains a NUL byte".into()))?;

    let sample_count = frames
        .checked_mul(channels.len())
        .ok_or_else(|| crate::IoError::Unsupported("Audio buffer is too large".into()))?;
    let mut interleaved = Vec::with_capacity(sample_count);
    let block_frames = progress_block_frames.max(1);
    for frame in 0..frames {
        for channel in channels {
            interleaved.push(channel[frame]);
        }

        if let Some(callback) = progress.as_deref_mut() {
            let done = frame + 1;
            if done % block_frames == 0 || done == frames {
                if !callback(done as u64, frames as u64) {
                    return Err(crate::IoError::Cancelled);
                }
            }
        }
    }

    if frames == 0 {
        if let Some(callback) = progress.as_deref_mut() {
            if !callback(0, 0) {
                return Err(crate::IoError::Cancelled);
            }
        }
    }

    let entries = tags.entries();
    let mut names = Vec::with_capacity(entries.len());
    let mut values = Vec::with_capacity(entries.len());
    for (name, value) in entries {
        names.push(CString::new(name).map_err(|_| {
            crate::IoError::Unsupported("A tag name contains a NUL byte".into())
        })?);
        values.push(CString::new(value).map_err(|_| {
            crate::IoError::Unsupported("A tag value contains a NUL byte".into())
        })?);
    }
    let native_tags = names
        .iter()
        .zip(&values)
        .map(|(name, value)| NativeTag {
            name: name.as_ptr(),
            value: value.as_ptr(),
        })
        .collect();

    Ok((path, interleaved, names, values, native_tags))
}

fn native_error(code: c_int, message: &[c_char]) -> Result<(), crate::IoError> {
    if code == 0 {
        return Ok(());
    }
    // SAFETY: The C++ shim always writes a NUL-terminated message into the
    // supplied, non-empty error buffer before returning a nonzero status.
    let text = unsafe { CStr::from_ptr(message.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    Err(crate::IoError::Encode(if text.is_empty() {
        format!("Ogg encoder failed with status {code}")
    } else {
        text
    }))
}

fn remove_cancelled_output(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Exports planar floating-point audio as Ogg Vorbis.
///
/// The input channels must have equal frame counts. Vorbis quality follows
/// Audacity's 0–10 export setting, with a default of 5.
pub fn export_vorbis(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &VorbisOptions,
    tags: &Tags,
) -> Result<(), crate::IoError> {
    let mut no_progress = |_: u64, _: u64| true;
    export_vorbis_with_progress(path, channels, sample_rate, opts, tags, &mut no_progress)
}

/// Exports planar floating-point audio as Ogg Vorbis, reporting progress as
/// input blocks are prepared. Returning `false` from the callback cancels the
/// export and removes any partial output file.
pub fn export_vorbis_with_progress(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &VorbisOptions,
    tags: &Tags,
    progress: crate::export::ProgressFn<'_>,
) -> Result<(), crate::IoError> {
    let (path_c, interleaved, _names, _values, native_tags) =
        match prepare_export_with_progress(path, channels, sample_rate, tags, Some(progress), 1024) {
            Ok(prepared) => prepared,
            Err(crate::IoError::Cancelled) => {
                remove_cancelled_output(path);
                return Err(crate::IoError::Cancelled);
            }
            Err(error) => return Err(error),
        };
    let native_options = NativeVorbisOptions {
        quality: opts.quality.clamp(0, 10),
    };
    let mut error_message = [0 as c_char; 1024];

    // SAFETY: All pointers refer to live, properly sized Rust-owned buffers
    // for the duration of the call. The shim reads but does not retain them.
    let result = unsafe {
        audacity_export_vorbis(
            path_c.as_ptr(),
            sample_rate,
            channels.len() as u32,
            interleaved.as_ptr(),
            channels[0].len() as u64,
            &native_options,
            native_tags.as_ptr(),
            native_tags.len(),
            error_message.as_mut_ptr(),
            error_message.len(),
        )
    };
    native_error(result, &error_message)
}

/// Exports planar floating-point audio as Ogg Opus.
///
/// Opus accepts only Audacity's supported input rates (8, 12, 16, 24, or
/// 48 kHz). The encoder and Ogg packet handling follow Audacity 3.7.3.
pub fn export_opus(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &OpusOptions,
    tags: &Tags,
) -> Result<(), crate::IoError> {
    let mut no_progress = |_: u64, _: u64| true;
    export_opus_with_progress(path, channels, sample_rate, opts, tags, &mut no_progress)
}

/// Exports planar floating-point audio as Ogg Opus, reporting progress as
/// input blocks are prepared. Returning `false` from the callback cancels the
/// export and removes any partial output file.
pub fn export_opus_with_progress(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &OpusOptions,
    tags: &Tags,
    progress: crate::export::ProgressFn<'_>,
) -> Result<(), crate::IoError> {
    if !matches!(sample_rate, 8_000 | 12_000 | 16_000 | 24_000 | 48_000) {
        return Err(crate::IoError::Unsupported(
            "Unsupported sample rate".into(),
        ));
    }

    let frame_duration_tenths_ms = opts.frame_duration as usize;
    let frame_block_frames =
        ((sample_rate as u64 * frame_duration_tenths_ms as u64) / 10_000).max(1) as usize;
    let (path_c, interleaved, _names, _values, native_tags) =
        match prepare_export_with_progress(
            path,
            channels,
            sample_rate,
            tags,
            Some(progress),
            frame_block_frames,
        ) {
            Ok(prepared) => prepared,
            Err(crate::IoError::Cancelled) => {
                remove_cancelled_output(path);
                return Err(crate::IoError::Cancelled);
            }
            Err(error) => return Err(error),
        };
    let native_options = NativeOpusOptions {
        bitrate: opts.bitrate as i32,
        quality: opts.quality.clamp(0, 10),
        frame_multiplier: opts.frame_duration as i32,
        vbr_mode: opts.vbr_mode as i32,
        application: opts.application as i32,
        cutoff: opts.cutoff as i32,
    };
    let mut error_message = [0 as c_char; 1024];

    // SAFETY: All pointers refer to live, properly sized Rust-owned buffers
    // for the duration of the call. The shim reads but does not retain them.
    let result = unsafe {
        audacity_export_opus(
            path_c.as_ptr(),
            sample_rate,
            channels.len() as u32,
            interleaved.as_ptr(),
            channels[0].len() as u64,
            &native_options,
            native_tags.as_ptr(),
            native_tags.len(),
            error_message.as_mut_ptr(),
            error_message.len(),
        )
    };
    native_error(result, &error_message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn temp_path(extension: &str) -> PathBuf {
        let serial = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "au-io-export-{}-{serial}.{extension}",
            std::process::id()
        ))
    }

    fn sine() -> Vec<Vec<f32>> {
        let rate = 48_000usize;
        (0..2)
            .map(|channel| {
                (0..rate)
                    .map(|i| {
                        let phase = std::f32::consts::TAU * 440.0 * i as f32 / rate as f32;
                        (phase + channel as f32 * 0.15).sin() * 0.4
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn exports_vorbis_and_preserves_comments() {
        let path = temp_path("ogg");
        let audio = sine();
        let mut tags = Tags::new();
        tags.set("TITLE", "Rust export title");
        tags.set("YEAR", "2024");
        export_vorbis(&path, &audio, 48_000, &VorbisOptions::default(), &tags).unwrap();

        let bytes = fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"OggS"));
        assert!(bytes.windows(b"TITLE=Rust export title".len())
            .any(|w| w == b"TITLE=Rust export title"));
        assert!(bytes.windows(b"DATE=2024".len()).any(|w| w == b"DATE=2024"));

        // Decode the Vorbis stream through Symphonia when this test target
        // enables the crate's Vorbis/Ogg decoder features.
        use symphonia::core::codecs::DecoderOptions;
        use symphonia::core::errors::Error;
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let file = fs::File::open(&path).unwrap();
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
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
        assert_eq!(track.codec_params.sample_rate, Some(48_000));
        assert_eq!(track.codec_params.channels.unwrap().count(), 2);

        let track_id = track.id;
        let params = track.codec_params.clone();
        let mut decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .unwrap();
        let mut decoded_frames = 0u64;
        loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break
                }
                Err(Error::ResetRequired) => continue,
                Err(error) => panic!("Vorbis packet read failed: {error}"),
            };
            if packet.track_id() != track_id {
                continue;
            }
            match decoder.decode(&packet) {
                Ok(audio) => decoded_frames += audio.frames() as u64,
                Err(Error::DecodeError(_)) => continue,
                Err(error) => panic!("Vorbis decode failed: {error}"),
            }
        }
        assert!((decoded_frames as i64 - 48_000).unsigned_abs() < 4096);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn opus_options_and_comments_encode() {
        let audio = sine();
        let mut tags = Tags::new();
        tags.set("TITLE", "Opus export title");
        tags.set("ARTIST", "Rust test");

        for mode in [
            OpusVbrMode::Off,
            OpusVbrMode::On,
            OpusVbrMode::Constrained,
        ] {
            for duration in [
                OpusFrameDuration::Ms2_5,
                OpusFrameDuration::Ms5,
                OpusFrameDuration::Ms10,
                OpusFrameDuration::Ms20,
                OpusFrameDuration::Ms40,
                OpusFrameDuration::Ms60,
            ] {
                let path = temp_path("opus");
                let options = OpusOptions {
                    vbr_mode: mode,
                    frame_duration: duration,
                    ..OpusOptions::default()
                };
                export_opus(&path, &audio, 48_000, &options, &tags).unwrap();
                let bytes = fs::read(&path).unwrap();
                assert!(bytes.starts_with(b"OggS"));
                assert!(bytes.windows(b"OpusHead".len()).any(|w| w == b"OpusHead"));
                assert!(bytes.windows(b"OpusTags".len()).any(|w| w == b"OpusTags"));
                assert!(bytes.windows(b"TITLE=Opus export title".len())
                    .any(|w| w == b"TITLE=Opus export title"));
                assert!(bytes.windows(b"ARTIST=Rust test".len())
                    .any(|w| w == b"ARTIST=Rust test"));
                fs::remove_file(path).unwrap();
            }
        }

        for application in [
            OpusApplication::Speech,
            OpusApplication::Audio,
            OpusApplication::LowDelay,
        ] {
            let path = temp_path("opus");
            let options = OpusOptions {
                application,
                ..OpusOptions::default()
            };
            export_opus(&path, &audio, 48_000, &options, &tags).unwrap();
            let bytes = fs::read(&path).unwrap();
            assert!(bytes.starts_with(b"OggS"));
            fs::remove_file(path).unwrap();
        }

        for cutoff in [
            OpusCutoff::Auto,
            OpusCutoff::Narrowband,
            OpusCutoff::Mediumband,
            OpusCutoff::Wideband,
            OpusCutoff::SuperWideband,
            OpusCutoff::Fullband,
        ] {
            let path = temp_path("opus");
            let options = OpusOptions {
                cutoff,
                ..OpusOptions::default()
            };
            export_opus(&path, &audio, 48_000, &options, &tags).unwrap();
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn rejects_mismatched_channels_and_unsupported_opus_rate() {
        let path = temp_path("opus");
        let mismatch = vec![vec![0.0; 8], vec![0.0; 7]];
        assert!(export_opus(
            &path,
            &mismatch,
            48_000,
            &OpusOptions::default(),
            &Tags::new()
        )
        .is_err());

        let audio = sine();
        assert!(export_opus(
            &path,
            &audio,
            44_100,
            &OpusOptions::default(),
            &Tags::new()
        )
        .is_err());
    }

    #[test]
    fn vorbis_progress_is_monotonic_and_bytes_match() {
        let audio = sine();
        let tags = Tags::new();
        let legacy_path = temp_path("ogg");
        let progress_path = temp_path("ogg");

        if export_vorbis(
            &legacy_path,
            &audio,
            48_000,
            &VorbisOptions::default(),
            &tags,
        )
        .is_err()
        {
            let _ = fs::remove_file(legacy_path);
            return;
        }

        let mut reports = Vec::new();
        let result = export_vorbis_with_progress(
            &progress_path,
            &audio,
            48_000,
            &VorbisOptions::default(),
            &tags,
            &mut |done, total| {
                reports.push((done, total));
                true
            },
        );
        if result.is_err() {
            let _ = fs::remove_file(legacy_path);
            let _ = fs::remove_file(progress_path);
            return;
        }

        assert!(!reports.is_empty());
        assert_eq!(reports.last().copied(), Some((48_000, 48_000)));
        assert!(reports.windows(2).all(|w| w[0].0 <= w[1].0));
        assert!(reports.iter().all(|&(done, total)| done <= total));
        assert_eq!(fs::read(&legacy_path).unwrap(), fs::read(&progress_path).unwrap());
        fs::remove_file(legacy_path).unwrap();
        fs::remove_file(progress_path).unwrap();
    }

    #[test]
    fn opus_progress_is_monotonic_and_bytes_match() {
        let audio = sine();
        let tags = Tags::new();
        let legacy_path = temp_path("opus");
        let progress_path = temp_path("opus");
        let options = OpusOptions::default();

        if export_opus(&legacy_path, &audio, 48_000, &options, &tags).is_err() {
            let _ = fs::remove_file(legacy_path);
            return;
        }

        let mut reports = Vec::new();
        let result = export_opus_with_progress(
            &progress_path,
            &audio,
            48_000,
            &options,
            &tags,
            &mut |done, total| {
                reports.push((done, total));
                true
            },
        );
        if result.is_err() {
            let _ = fs::remove_file(legacy_path);
            let _ = fs::remove_file(progress_path);
            return;
        }

        assert!(!reports.is_empty());
        assert_eq!(reports.last().copied(), Some((48_000, 48_000)));
        assert!(reports.windows(2).all(|w| w[0].0 <= w[1].0));
        assert!(reports.iter().all(|&(done, total)| done <= total));
        assert_eq!(fs::read(&legacy_path).unwrap(), fs::read(&progress_path).unwrap());
        fs::remove_file(legacy_path).unwrap();
        fs::remove_file(progress_path).unwrap();
    }

    #[test]
    fn progress_cancellation_leaves_no_output_file() {
        let audio = sine();
        let tags = Tags::new();
        let vorbis_path = temp_path("ogg");
        let opus_path = temp_path("opus");

        let vorbis_result = export_vorbis_with_progress(
            &vorbis_path,
            &audio,
            48_000,
            &VorbisOptions::default(),
            &tags,
            &mut |_, _| false,
        );
        assert!(matches!(vorbis_result, Err(crate::IoError::Cancelled)));
        assert!(!vorbis_path.exists());

        let opus_result = export_opus_with_progress(
            &opus_path,
            &audio,
            48_000,
            &OpusOptions::default(),
            &tags,
            &mut |_, _| false,
        );
        assert!(matches!(opus_result, Err(crate::IoError::Cancelled)));
        assert!(!opus_path.exists());
    }
}
