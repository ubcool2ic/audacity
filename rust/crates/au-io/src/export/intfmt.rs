//! Audacity-compatible PCM, FLAC, and WavPack export.
//!
//! The encoder backends are provided by the C++ shim in `export_intfmt.cpp`.
//! Integer sample conversion and dithering are performed here before samples
//! are passed to those backends.

use au_core::tags::Tags;
use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::path::Path;

/// Audacity's dither algorithms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum AudDither {
    /// Do not add dither noise.
    None = 0,
    /// Add rectangular noise.
    Rectangle = 1,
    /// Add triangular noise.
    Triangle = 2,
    /// Apply Audacity's shaped triangular dither.
    #[default]
    Shaped = 3,
}

/// Options for libsndfile-based PCM export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmOptions {
    /// libsndfile header type, such as WAV or AIFF.
    pub header: i32,
    /// libsndfile encoding subtype, such as PCM_16 or PCM_24.
    pub encoding: i32,
    /// Dither algorithm used when converting float samples to integer samples.
    pub dither: AudDither,
    /// Whether the incoming float samples require dithering.
    pub needs_dither: bool,
}

impl Default for PcmOptions {
    fn default() -> Self {
        Self {
            header: SF_FORMAT_WAV,
            encoding: SF_FORMAT_PCM_16,
            dither: AudDither::Shaped,
            needs_dither: true,
        }
    }
}

/// Options for FLAC export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlacOptions {
    /// Bits per sample; Audacity offers 16 or 24.
    pub bit_depth: u32,
    /// Compression level, from 0 (fastest) through 8 (best).
    pub level: u32,
    /// Dither algorithm used when converting float samples to integer samples.
    pub dither: AudDither,
    /// Whether the incoming float samples require dithering.
    pub needs_dither: bool,
}

impl Default for FlacOptions {
    fn default() -> Self {
        Self {
            bit_depth: 16,
            level: 5,
            dither: AudDither::Shaped,
            needs_dither: true,
        }
    }
}

/// Options for WavPack export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavPackOptions {
    /// Quality: 0 = Low, 1 = Normal, 2 = High, 3 = Very High.
    pub quality: u32,
    /// Bits per sample: 16, 24, or 32 (float).
    pub bit_depth: u32,
    /// Enable hybrid mode.
    pub hybrid_mode: bool,
    /// Create a correction (`.wvc`) file in hybrid mode.
    pub create_correction: bool,
    /// Hybrid bitrate in tenths of a bit per sample.
    pub bit_rate: u32,
    /// Dither algorithm used when converting float samples to integer samples.
    pub dither: AudDither,
    /// Whether the incoming float samples require dithering.
    pub needs_dither: bool,
}

impl Default for WavPackOptions {
    fn default() -> Self {
        Self {
            quality: 1,
            bit_depth: 16,
            hybrid_mode: false,
            create_correction: false,
            bit_rate: 40,
            dither: AudDither::Shaped,
            needs_dither: true,
        }
    }
}

/// libsndfile header constant: WAV.
pub const SF_FORMAT_WAV: i32 = 0x010000;
/// libsndfile header constant: AIFF.
pub const SF_FORMAT_AIFF: i32 = 0x020000;
/// libsndfile header constant: AU.
pub const SF_FORMAT_AU: i32 = 0x030000;
/// libsndfile header constant: RAW.
pub const SF_FORMAT_RAW: i32 = 0x040000;
/// libsndfile header constant: PAF.
pub const SF_FORMAT_PAF: i32 = 0x050000;
/// libsndfile header constant: SVX.
pub const SF_FORMAT_SVX: i32 = 0x060000;
/// libsndfile header constant: NIST WAV.
pub const SF_FORMAT_NIST: i32 = 0x070000;
/// libsndfile header constant: VOC.
pub const SF_FORMAT_VOC: i32 = 0x080000;
/// libsndfile header constant: IRCAM.
pub const SF_FORMAT_IRCAM: i32 = 0x0A0000;
/// libsndfile header constant: W64.
pub const SF_FORMAT_W64: i32 = 0x0B0000;
/// libsndfile header constant: MATLAB 4.
pub const SF_FORMAT_MAT4: i32 = 0x0C0000;
/// libsndfile header constant: MATLAB 5.
pub const SF_FORMAT_MAT5: i32 = 0x0D0000;
/// libsndfile header constant: PVF.
pub const SF_FORMAT_PVF: i32 = 0x0E0000;
/// libsndfile header constant: XI.
pub const SF_FORMAT_XI: i32 = 0x0F0000;
/// libsndfile header constant: HTK.
pub const SF_FORMAT_HTK: i32 = 0x100000;
/// libsndfile header constant: SDS.
pub const SF_FORMAT_SDS: i32 = 0x110000;
/// libsndfile header constant: AVR.
pub const SF_FORMAT_AVR: i32 = 0x120000;
/// libsndfile header constant: WAVEX.
pub const SF_FORMAT_WAVEX: i32 = 0x130000;
/// libsndfile header constant: SD2.
pub const SF_FORMAT_SD2: i32 = 0x160000;
/// libsndfile header constant: FLAC.
pub const SF_FORMAT_FLAC: i32 = 0x170000;
/// libsndfile header constant: CAF.
pub const SF_FORMAT_CAF: i32 = 0x180000;
/// libsndfile header constant: WVE.
pub const SF_FORMAT_WVE: i32 = 0x190000;
/// libsndfile header constant: OGG.
pub const SF_FORMAT_OGG: i32 = 0x200000;
/// libsndfile header constant: RF64.
pub const SF_FORMAT_RF64: i32 = 0x220000;

/// libsndfile subtype constant: signed 8-bit PCM.
pub const SF_FORMAT_PCM_S8: i32 = 0x0001;
/// libsndfile subtype constant: signed 16-bit PCM.
pub const SF_FORMAT_PCM_16: i32 = 0x0002;
/// libsndfile subtype constant: signed 24-bit PCM.
pub const SF_FORMAT_PCM_24: i32 = 0x0003;
/// libsndfile subtype constant: signed 32-bit PCM.
pub const SF_FORMAT_PCM_32: i32 = 0x0004;
/// libsndfile subtype constant: unsigned 8-bit PCM.
pub const SF_FORMAT_PCM_U8: i32 = 0x0005;
/// libsndfile subtype constant: 32-bit float.
pub const SF_FORMAT_FLOAT: i32 = 0x0006;
/// libsndfile subtype constant: 64-bit float.
pub const SF_FORMAT_DOUBLE: i32 = 0x0007;
/// libsndfile subtype constant: u-law.
pub const SF_FORMAT_ULAW: i32 = 0x0010;
/// libsndfile subtype constant: A-law.
pub const SF_FORMAT_ALAW: i32 = 0x0011;
/// libsndfile subtype constant: Microsoft ADPCM.
pub const SF_FORMAT_MS_ADPCM: i32 = 0x0012;
/// libsndfile subtype constant: IMA ADPCM.
pub const SF_FORMAT_IMA_ADPCM: i32 = 0x0013;
/// libsndfile subtype constant: GSM 6.10.
pub const SF_FORMAT_GSM610: i32 = 0x0020;
/// libsndfile subtype constant: VOX ADPCM.
pub const SF_FORMAT_VOX_ADPCM: i32 = 0x0030;
/// libsndfile subtype constant: G.721 32-kbit ADPCM.
pub const SF_FORMAT_G721_32: i32 = 0x0040;
/// libsndfile subtype constant: G.723 24-kbit ADPCM.
pub const SF_FORMAT_G723_24: i32 = 0x0041;
/// libsndfile subtype constant: G.723 40-kbit ADPCM.
pub const SF_FORMAT_G723_40: i32 = 0x0042;
/// libsndfile subtype constant: DWVW 12-bit.
pub const SF_FORMAT_DWVW_12: i32 = 0x0050;
/// libsndfile subtype constant: DWVW 16-bit.
pub const SF_FORMAT_DWVW_16: i32 = 0x0051;
/// libsndfile subtype constant: DWVW 24-bit.
pub const SF_FORMAT_DWVW_24: i32 = 0x0052;
/// libsndfile subtype constant: DWVW N-bit.
pub const SF_FORMAT_DWVW_N: i32 = 0x0053;
/// libsndfile subtype constant: DPCM 8-bit.
pub const SF_FORMAT_DPCM_8: i32 = 0x0060;
/// libsndfile subtype constant: DPCM 16-bit.
pub const SF_FORMAT_DPCM_16: i32 = 0x0061;
/// libsndfile subtype constant: Vorbis.
pub const SF_FORMAT_VORBIS: i32 = 0x0060;

/// C-compatible flat PCM options.
#[repr(C)]
#[derive(Clone, Copy)]
struct PcmOptionsC {
    header: c_int,
    encoding: c_int,
}

/// C-compatible flat FLAC options.
#[repr(C)]
#[derive(Clone, Copy)]
struct FlacOptionsC {
    bit_depth: c_int,
    level: c_int,
}

/// C-compatible flat WavPack options.
#[repr(C)]
#[derive(Clone, Copy)]
struct WavPackOptionsC {
    quality: c_int,
    bit_depth: c_int,
    hybrid_mode: c_int,
    create_correction: c_int,
    bit_rate: c_int,
}

extern "C" {
    fn aud_export_pcm(
        path: *const c_char,
        sample_rate: u32,
        channels: u32,
        samples: *const std::ffi::c_void,
        sample_kind: c_int,
        frames: u64,
        options: *const PcmOptionsC,
        tag_names: *const *const c_char,
        tag_values: *const *const c_char,
        tag_count: usize,
        error: *mut c_char,
        error_len: usize,
    ) -> c_int;

    fn aud_export_flac(
        path: *const c_char,
        sample_rate: u32,
        channels: u32,
        samples: *const i32,
        frames: u64,
        options: *const FlacOptionsC,
        tag_names: *const *const c_char,
        tag_values: *const *const c_char,
        tag_count: usize,
        error: *mut c_char,
        error_len: usize,
    ) -> c_int;

    fn aud_export_wavpack(
        path: *const c_char,
        sample_rate: u32,
        channels: u32,
        samples: *const i32,
        frames: u64,
        options: *const WavPackOptionsC,
        tag_names: *const *const c_char,
        tag_values: *const *const c_char,
        tag_count: usize,
        error: *mut c_char,
        error_len: usize,
    ) -> c_int;
}

/// Export samples to a libsndfile PCM container.
///
/// `channels` is planar and all channel vectors must have equal length.
pub fn export_pcm(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &PcmOptions,
    tags: &Tags,
) -> Result<(), crate::IoError> {
    let mut no_progress = |_: u64, _: u64| true;
    export_pcm_with_progress(path, channels, sample_rate, opts, tags, &mut no_progress)
}

/// Export samples to a libsndfile PCM container, reporting progress by frame.
///
/// Progress is reported after each converted/interleaved block and once more
/// after a successful export. Returning `false` cancels the export.
pub fn export_pcm_with_progress(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &PcmOptions,
    tags: &Tags,
    progress: crate::export::ProgressFn<'_>,
) -> Result<(), crate::IoError> {
    let frames = validate_dimensions(channels, sample_rate)?;
    let channel_count = channels.len();
    let subtype = opts.encoding;
    let mut data_i32 = Vec::new();
    let mut data_f32 = Vec::new();
    let (data_ptr, sample_kind) = if subtype == SF_FORMAT_PCM_24 {
        data_f32.resize(frames.saturating_mul(channel_count), 0.0);
        convert_pcm24(
            channels,
            frames,
            opts.dither,
            opts.needs_dither,
            &mut data_f32,
            progress,
        )?;
        (data_f32.as_ptr().cast(), 0)
    } else if is_pcm_low_precision(subtype) {
        data_i32.resize(frames.saturating_mul(channel_count), 0);
        convert_to_integer(
            channels,
            frames,
            16,
            opts.dither,
            opts.needs_dither,
            &mut data_i32,
            progress,
        )?;
        (data_i32.as_ptr().cast(), 1)
    } else {
        data_f32 = interleave_with_progress(channels, frames, progress)?;
        (data_f32.as_ptr().cast(), 0)
    };

    let options = PcmOptionsC {
        header: opts.header,
        encoding: opts.encoding,
    };
    let path_c = path_cstring(path)?;
    let tag_set = FfiTags::new(tags)?;
    let mut error = [0 as c_char; 1024];
    // SAFETY: All pointers refer to live, correctly sized buffers for the duration
    // of the synchronous C++ call; the shim does not retain any pointer.
    let result = unsafe {
        aud_export_pcm(
            path_c.as_ptr(),
            sample_rate,
            channel_count as u32,
            data_ptr,
            sample_kind,
            frames as u64,
            &options,
            tag_set.names_ptr(),
            tag_set.values_ptr(),
            tag_set.len(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    check_result(result, &error)?;
    report_final_progress(progress, frames, path)
}

/// Export samples to FLAC.
pub fn export_flac(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &FlacOptions,
    tags: &Tags,
) -> Result<(), crate::IoError> {
    let mut no_progress = |_: u64, _: u64| true;
    export_flac_with_progress(path, channels, sample_rate, opts, tags, &mut no_progress)
}

/// Export samples to FLAC, reporting progress by frame.
///
/// Progress is reported after each converted block and once more after a
/// successful export. Returning `false` cancels the export.
pub fn export_flac_with_progress(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &FlacOptions,
    tags: &Tags,
    progress: crate::export::ProgressFn<'_>,
) -> Result<(), crate::IoError> {
    let frames = validate_dimensions(channels, sample_rate)?;
    if opts.bit_depth != 16 && opts.bit_depth != 24 {
        return Err(crate::IoError::Unsupported(
            "FLAC bit depth must be 16 or 24".into(),
        ));
    }
    let samples = convert_planar_integer(
        channels,
        frames,
        opts.bit_depth,
        opts.dither,
        opts.needs_dither,
        progress,
    )?;
    let options = FlacOptionsC {
        bit_depth: opts.bit_depth as c_int,
        level: opts.level.min(8) as c_int,
    };
    let path_c = path_cstring(path)?;
    let tag_set = FfiTags::new(tags)?;
    let mut error = [0 as c_char; 1024];
    // SAFETY: Buffers and option structs remain alive and immutable until the
    // synchronous C++ encoder call returns.
    let result = unsafe {
        aud_export_flac(
            path_c.as_ptr(),
            sample_rate,
            channels.len() as u32,
            samples.as_ptr(),
            frames as u64,
            &options,
            tag_set.names_ptr(),
            tag_set.values_ptr(),
            tag_set.len(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    check_result(result, &error)?;
    report_final_progress(progress, frames, path)
}

/// Export samples to WavPack.
pub fn export_wavpack(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &WavPackOptions,
    tags: &Tags,
) -> Result<(), crate::IoError> {
    let mut no_progress = |_: u64, _: u64| true;
    export_wavpack_with_progress(path, channels, sample_rate, opts, tags, &mut no_progress)
}

/// Export samples to WavPack, reporting progress by frame.
///
/// Progress is reported after each converted block and once more after a
/// successful export. Returning `false` cancels the export.
pub fn export_wavpack_with_progress(
    path: &Path,
    channels: &[Vec<f32>],
    sample_rate: u32,
    opts: &WavPackOptions,
    tags: &Tags,
    progress: crate::export::ProgressFn<'_>,
) -> Result<(), crate::IoError> {
    let frames = validate_dimensions(channels, sample_rate)?;
    let depth = match opts.bit_depth {
        16 | 24 | 32 => opts.bit_depth,
        _ => {
            return Err(crate::IoError::Unsupported(
                "WavPack bit depth must be 16, 24, or 32".into(),
            ))
        }
    };

    // WavPackPackSamples expects interleaved int32 samples. Float samples are
    // passed as their IEEE-754 bit patterns, also in frame/channel order.
    let samples = if depth == 32 {
        let channel_count = channels.len();
        let mut samples = Vec::with_capacity(frames.saturating_mul(channel_count));
        for block_start in (0..frames).step_by(FRAME_PROGRESS_BLOCK_SIZE) {
            let block_end = (block_start + FRAME_PROGRESS_BLOCK_SIZE).min(frames);
            for frame in block_start..block_end {
                for channel in channels {
                    samples.push(channel[frame].to_bits() as i32);
                }
            }
            if !progress(block_end as u64, frames as u64) {
                cancel_export_files(path);
                return Err(crate::IoError::Cancelled);
            }
        }
        samples
    } else {
        convert_planar_integer(
            channels,
            frames,
            depth,
            opts.dither,
            opts.needs_dither,
            progress,
        )?
    };

    let options = WavPackOptionsC {
        quality: opts.quality.min(3) as c_int,
        bit_depth: depth as c_int,
        hybrid_mode: i32::from(opts.hybrid_mode),
        create_correction: i32::from(opts.create_correction),
        // Audacity stores the option in tenths of a bit per sample; the C++
        // backend converts it to bits/sample for WavPack.
        bit_rate: opts.bit_rate as c_int,
    };
    let path_c = path_cstring(path)?;
    let tag_set = FfiTags::new(tags)?;
    let mut error = [0 as c_char; 1024];
    // SAFETY: Buffers and option structs remain alive and immutable until the
    // synchronous C++ encoder call returns.
    let result = unsafe {
        aud_export_wavpack(
            path_c.as_ptr(),
            sample_rate,
            channels.len() as u32,
            samples.as_ptr(),
            frames as u64,
            &options,
            tag_set.names_ptr(),
            tag_set.values_ptr(),
            tag_set.len(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    check_result(result, &error)?;
    report_final_progress(progress, frames, path)
}

fn report_final_progress(
    progress: crate::export::ProgressFn<'_>,
    frames: usize,
    path: &Path,
) -> Result<(), crate::IoError> {
    if !progress(frames as u64, frames as u64) {
        cancel_export_files(path);
        return Err(crate::IoError::Cancelled);
    }
    Ok(())
}

fn cancel_export_files(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("wvc"));
}

fn validate_dimensions(
    channels: &[Vec<f32>],
    sample_rate: u32,
) -> Result<usize, crate::IoError> {
    if sample_rate == 0 {
        return Err(crate::IoError::Unsupported(
            "sample rate must be greater than zero".into(),
        ));
    }
    if channels.is_empty() || channels.len() > 255 {
        return Err(crate::IoError::Unsupported(
            "channel count must be between 1 and 255".into(),
        ));
    }
    let frames = channels[0].len();
    if channels.iter().any(|channel| channel.len() != frames) {
        return Err(crate::IoError::Unsupported(
            "all planar channels must have equal lengths".into(),
        ));
    }
    Ok(frames)
}


fn interleave_with_progress(
    channels: &[Vec<f32>],
    frames: usize,
    progress: crate::export::ProgressFn<'_>,
) -> Result<Vec<f32>, crate::IoError> {
    let mut interleaved = Vec::with_capacity(frames.saturating_mul(channels.len()));
    for block_start in (0..frames).step_by(FRAME_PROGRESS_BLOCK_SIZE) {
        let block_end = (block_start + FRAME_PROGRESS_BLOCK_SIZE).min(frames);
        for frame in block_start..block_end {
            for channel in channels {
                interleaved.push(channel[frame]);
            }
        }
        if !progress(block_end as u64, frames as u64) {
            cancel_export_files_from_progress(progress);
            return Err(crate::IoError::Cancelled);
        }
    }
    Ok(interleaved)
}

// The callback itself does not expose the output path. Export callers remove the
// destination when conversion returns Cancelled.
fn cancel_export_files_from_progress(_progress: crate::export::ProgressFn<'_>) {}

fn is_pcm_low_precision(subtype: i32) -> bool {
    matches!(
        subtype,
        SF_FORMAT_PCM_S8
            | SF_FORMAT_PCM_16
            | SF_FORMAT_PCM_U8
            | SF_FORMAT_ULAW
            | SF_FORMAT_ALAW
            | SF_FORMAT_MS_ADPCM
            | SF_FORMAT_IMA_ADPCM
            | SF_FORMAT_GSM610
            | SF_FORMAT_VOX_ADPCM
            | SF_FORMAT_G721_32
            | SF_FORMAT_G723_24
            | SF_FORMAT_G723_40
            | SF_FORMAT_DWVW_12
            | SF_FORMAT_DWVW_16
            | SF_FORMAT_DPCM_8
            | SF_FORMAT_DPCM_16
    )
}

fn convert_pcm24(
    channels: &[Vec<f32>],
    frames: usize,
    dither: AudDither,
    needs_dither: bool,
    out: &mut [f32],
    progress: crate::export::ProgressFn<'_>,
) -> Result<(), crate::IoError> {
    const SCALE: f32 = 8_388_608.0;
    let channel_count = channels.len();
    let mut dither_state = DitherState::new();
    let mut converted = vec![0i32; frames];

    for block_start in (0..frames).step_by(PCM_BLOCK_SIZE) {
        let block_len = (frames - block_start).min(PCM_BLOCK_SIZE);
        for (channel_index, channel) in channels.iter().enumerate() {
            if needs_dither {
                dither_state.convert(
                    &channel[block_start..block_start + block_len],
                    24,
                    dither,
                    &mut converted[..block_len],
                );
                for i in 0..block_len {
                    out[(block_start + i) * channel_count + channel_index] =
                        converted[i] as f32 / SCALE;
                }
            } else {
                for i in 0..block_len {
                    out[(block_start + i) * channel_count + channel_index] =
                        channel[block_start + i];
                }
            }
        }
        if !progress((block_start + block_len) as u64, frames as u64) {
            return Err(crate::IoError::Cancelled);
        }
    }
    Ok(())
}

const PCM_BLOCK_SIZE: usize = 44_100 * 5;
const FLAC_BLOCK_SIZE: usize = 8_192;
const WAVPACK_BLOCK_SIZE: usize = 8_192;
const FRAME_PROGRESS_BLOCK_SIZE: usize = 65_536;

fn convert_to_integer(
    channels: &[Vec<f32>],
    frames: usize,
    bits: u32,
    dither: AudDither,
    needs_dither: bool,
    out: &mut [i32],
    progress: crate::export::ProgressFn<'_>,
) -> Result<(), crate::IoError> {
    let channel_count = channels.len();
    let mut dither_state = DitherState::new();
    let mut converted = vec![0i32; frames];

    for block_start in (0..frames).step_by(PCM_BLOCK_SIZE) {
        let block_len = (frames - block_start).min(PCM_BLOCK_SIZE);
        for (channel_index, channel) in channels.iter().enumerate() {
            dither_state.convert(
                &channel[block_start..block_start + block_len],
                bits,
                if needs_dither {
                    dither
                } else {
                    AudDither::None
                },
                &mut converted[..block_len],
            );
            for i in 0..block_len {
                out[(block_start + i) * channel_count + channel_index] = converted[i];
            }
        }
        if !progress((block_start + block_len) as u64, frames as u64) {
            return Err(crate::IoError::Cancelled);
        }
    }
    Ok(())
}

fn convert_planar_integer(
    channels: &[Vec<f32>],
    frames: usize,
    bits: u32,
    dither: AudDither,
    needs_dither: bool,
    progress: crate::export::ProgressFn<'_>,
) -> Result<Vec<i32>, crate::IoError> {
    let channel_count = channels.len();
    let mut out = vec![0i32; frames.saturating_mul(channel_count)];
    let block_size = if bits == 16 || bits == 24 {
        WAVPACK_BLOCK_SIZE
    } else {
        FLAC_BLOCK_SIZE
    };
    let mut state = DitherState::new();
    let mut converted = vec![0i32; frames];

    for block_start in (0..frames).step_by(block_size) {
        let block_len = (frames - block_start).min(block_size);
        for (channel_index, channel) in channels.iter().enumerate() {
            state.convert(
                &channel[block_start..block_start + block_len],
                bits,
                if needs_dither {
                    dither
                } else {
                    AudDither::None
                },
                &mut converted[..block_len],
            );
            for i in 0..block_len {
                out[(block_start + i) * channel_count + channel_index] = converted[i];
            }
        }
        if !progress((block_start + block_len) as u64, frames as u64) {
            return Err(crate::IoError::Cancelled);
        }
    }
    Ok(out)
}

/// Audacity-style sample conversion dither state.
#[derive(Clone, Debug)]
pub struct DitherState {
    rng: GlibcRand,
    phase: usize,
    triangle_state: f32,
    buffer: [f32; 8],
}

impl Default for DitherState {
    fn default() -> Self {
        Self::new()
    }
}

impl DitherState {
    /// Create a dither state with the deterministic glibc `rand()` seed of 1.
    pub fn new() -> Self {
        Self {
            rng: GlibcRand::new(1),
            phase: 0,
            triangle_state: 0.0,
            buffer: [0.0; 8],
        }
    }

    /// Convert normalized float samples to signed 16- or 24-bit integer samples.
    ///
    /// Each call reproduces `Dither::Apply`: triangle and shaped filters are
    /// reset for each conversion, while the random number stream is retained.
    pub fn convert(
        &mut self,
        source: &[f32],
        bits: u32,
        dither: AudDither,
        destination: &mut [i32],
    ) {
        assert!(bits == 16 || bits == 24);
        assert!(destination.len() >= source.len());

        if matches!(dither, AudDither::Triangle | AudDither::Shaped) {
            self.phase = 0;
            self.triangle_state = 0.0;
            self.buffer = [0.0; 8];
        }

        let scale = if bits == 16 {
            32_768.0f32
        } else {
            8_388_608.0f32
        };
        let (minimum, maximum) = if bits == 16 {
            (-32_768i32, 32_767i32)
        } else {
            (-8_388_608i32, 8_388_607i32)
        };

        for (index, &input) in source.iter().enumerate() {
            let sample = if input > 1.0 {
                1.0
            } else if input < -1.0 {
                -1.0
            } else {
                input
            };
            let scaled = sample * scale;
            let value = match dither {
                AudDither::None => scaled,
                AudDither::Rectangle => scaled - self.noise(),
                AudDither::Triangle => {
                    let r = self.noise();
                    let result = scaled + r - self.triangle_state;
                    self.triangle_state = r;
                    result
                }
                AudDither::Shaped => self.shaped(scaled),
            };
            let rounded = lrintf(value).clamp(minimum as i64, maximum as i64) as i32;
            destination[index] = rounded;
        }
    }

    fn noise(&mut self) -> f32 {
        self.rng.next() as f32 / 2_147_483_647.0f32 - 0.5f32
    }

    fn shaped(&mut self, sample: f32) -> f32 {
        const SHAPED_BS: [f32; 5] = [2.033, -2.165, 1.959, -1.590, 0.6149];

        let r = self.noise() + self.noise();
        let sample = if sample.is_nan() { 0.0 } else { sample };
        let xe = sample
            + self.buffer[self.phase] * SHAPED_BS[0]
            + self.buffer[(self.phase.wrapping_sub(1)) & 7] * SHAPED_BS[1]
            + self.buffer[(self.phase.wrapping_sub(2)) & 7] * SHAPED_BS[2]
            + self.buffer[(self.phase.wrapping_sub(3)) & 7] * SHAPED_BS[3]
            + self.buffer[(self.phase.wrapping_sub(4)) & 7] * SHAPED_BS[4];
        let result = xe + r;
        self.phase = (self.phase + 1) & 7;
        self.buffer[self.phase] = xe - lrintf(result) as f32;
        result
    }
}

fn lrintf(value: f32) -> i64 {
    if !value.is_finite() {
        return if value.is_sign_negative() {
            i64::MIN
        } else {
            i64::MAX
        };
    }
    let floor = value.floor();
    let fraction = value - floor;
    let rounded = if fraction < 0.5 {
        floor
    } else if fraction > 0.5 {
        floor + 1.0
    } else if (floor as i64) & 1 == 0 {
        floor
    } else {
        floor + 1.0
    };
    if rounded >= i64::MAX as f32 {
        i64::MAX
    } else if rounded <= i64::MIN as f32 {
        i64::MIN
    } else {
        rounded as i64
    }
}

#[derive(Clone, Debug)]
struct GlibcRand {
    /// The last 34 outputs of glibc's TYPE_3 recurrence `r[i] = r[i-31] + r[i-3]`.
    ring: [u32; 34],
    pos: usize,
}

impl GlibcRand {
    fn new(seed: u32) -> Self {
        let mut r: Vec<u32> = Vec::with_capacity(344);
        let seed = if seed == 0 { 1 } else { seed };
        r.push(seed);
        for i in 1..31 {
            let w = r[i - 1] as i32 as i64;
            let hi = w / 127_773;
            let lo = w % 127_773;
            let mut word = 16_807 * lo - 2_836 * hi;
            if word < 0 {
                word += 2_147_483_647;
            }
            r.push(word as u32);
        }
        for i in 31..34 {
            r.push(r[i - 31]);
        }
        for i in 34..344 {
            r.push(r[i - 31].wrapping_add(r[i - 3]));
        }
        let mut ring = [0u32; 34];
        ring.copy_from_slice(&r[310..344]);
        Self { ring, pos: 0 }
    }

    fn next(&mut self) -> u32 {
        // ring[pos] holds r[n-34]; r[n-31] is at pos+3, r[n-3] at pos+31.
        let v = self.ring[(self.pos + 3) % 34].wrapping_add(self.ring[(self.pos + 31) % 34]);
        self.ring[self.pos] = v;
        self.pos = (self.pos + 1) % 34;
        v >> 1
    }
}

struct FfiTags {
    names: Vec<CString>,
    #[allow(dead_code)] // owns the strings behind value_ptrs
    values: Vec<CString>,
    name_ptrs: Vec<*const c_char>,
    value_ptrs: Vec<*const c_char>,
}

impl FfiTags {
    fn new(tags: &Tags) -> Result<Self, crate::IoError> {
        let mut names = Vec::new();
        let mut values = Vec::new();
        for (name, value) in tags.entries() {
            names.push(CString::new(name).map_err(|_| {
                crate::IoError::Unsupported("tag names cannot contain NUL bytes".into())
            })?);
            values.push(CString::new(value).map_err(|_| {
                crate::IoError::Unsupported("tag values cannot contain NUL bytes".into())
            })?);
        }
        let name_ptrs = names.iter().map(|s| s.as_ptr()).collect();
        let value_ptrs = values.iter().map(|s| s.as_ptr()).collect();
        Ok(Self {
            names,
            values,
            name_ptrs,
            value_ptrs,
        })
    }

    fn len(&self) -> usize {
        self.names.len()
    }

    fn names_ptr(&self) -> *const *const c_char {
        if self.name_ptrs.is_empty() {
            std::ptr::null()
        } else {
            self.name_ptrs.as_ptr()
        }
    }

    fn values_ptr(&self) -> *const *const c_char {
        if self.value_ptrs.is_empty() {
            std::ptr::null()
        } else {
            self.value_ptrs.as_ptr()
        }
    }
}

fn path_cstring(path: &Path) -> Result<CString, crate::IoError> {
    let path = path
        .to_str()
        .ok_or_else(|| crate::IoError::Unsupported("export path is not valid UTF-8".into()))?;
    CString::new(path)
        .map_err(|_| crate::IoError::Unsupported("export path contains a NUL byte".into()))
}

fn check_result(result: c_int, error: &[c_char]) -> Result<(), crate::IoError> {
    if result == 0 {
        return Ok(());
    }
    // SAFETY: The shim writes a NUL-terminated error message into this fixed,
    // initialized buffer before returning.
    let message = unsafe { CStr::from_ptr(error.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    Err(crate::IoError::Encode(if message.is_empty() {
        format!("encoder failed ({result})")
    } else {
        message
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine() -> Vec<Vec<f32>> {
        let rate = 8_000usize;
        (0..2)
            .map(|channel| {
                (0..rate)
                    .map(|i| {
                        let phase = i as f32 * 440.0 * std::f32::consts::TAU / rate as f32;
                        phase.sin() * if channel == 0 { 0.5 } else { 0.4 }
                    })
                    .collect()
            })
            .collect()
    }

    fn temp_file(extension: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "au-intfmt-{}-{}.{}",
            std::process::id(),
            extension,
            extension
        ))
    }

    #[test]
    fn dither_rng_matches_glibc_seed_one() {
        let mut rng = GlibcRand::new(1);
        assert_eq!(rng.next(), 1_804_289_383);
        assert_eq!(rng.next(), 846_930_886);
        assert_eq!(rng.next(), 1_681_692_777);
    }

    #[test]
    fn conversion_is_repeatable_and_clips() {
        let source = [-2.0, -1.0, -0.5, 0.0, 0.5, 1.0, 2.0];
        let mut a = [0i32; 7];
        let mut b = [0i32; 7];
        DitherState::new().convert(&source, 16, AudDither::Shaped, &mut a);
        DitherState::new().convert(&source, 16, AudDither::Shaped, &mut b);
        assert_eq!(a, b);
        assert_eq!(a[0], -32_768);
        assert_eq!(a[6], 32_767);
    }

    #[test]
    fn empty_and_stereo_pcm_export_do_not_panic() {
        let path = temp_file("wav");
        let tags = Tags::new();
        let stereo = sine();
        export_pcm(&path, &stereo, 8_000, &PcmOptions::default(), &tags).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 44);
        let _ = std::fs::remove_file(&path);

        let empty = vec![Vec::<f32>::new(), Vec::<f32>::new()];
        let path = temp_file("empty.wav");
        export_pcm(&path, &empty, 8_000, &PcmOptions::default(), &tags).unwrap();
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pcm_encodings_write_and_tags_are_forwarded() {
        let mut tags = Tags::new();
        tags.set("TITLE", "intfmt test title");
        let path = temp_file("tagged.wav");
        let mut options = PcmOptions::default();
        options.encoding = SF_FORMAT_PCM_24;
        export_pcm(&path, &sine(), 8_000, &options, &tags).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.windows(4).any(|w| w == b"RIFF"));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn flac_and_wavpack_option_modes_encode() {
        let audio = sine();
        let tags = Tags::new();

        for bit_depth in [16, 24] {
            let path = temp_file(&format!("flac-{bit_depth}.flac"));
            let options = FlacOptions {
                bit_depth,
                level: 5,
                ..FlacOptions::default()
            };
            export_flac(&path, &audio, 8_000, &options, &tags).unwrap();
            assert!(std::fs::metadata(&path).unwrap().len() > 4);
            let _ = std::fs::remove_file(path);
        }

        for bit_depth in [16, 24, 32] {
            let path = temp_file(&format!("wv-{bit_depth}.wv"));
            let options = WavPackOptions {
                bit_depth,
                ..WavPackOptions::default()
            };
            export_wavpack(&path, &audio, 8_000, &options, &tags).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            assert!(bytes.starts_with(b"wvpk"));
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn pcm_progress_is_monotonic_and_output_matches() {
        let audio = sine();
        let tags = Tags::new();
        let original = temp_file("progress-original.wav");
        let with_progress = temp_file("progress-with.wav");

        export_pcm(&original, &audio, 8_000, &PcmOptions::default(), &tags).unwrap();
        let mut updates = Vec::new();
        let mut callback = |done, total| {
            updates.push((done, total));
            true
        };
        export_pcm_with_progress(
            &with_progress,
            &audio,
            8_000,
            &PcmOptions::default(),
            &tags,
            &mut callback,
        )
        .unwrap();

        assert!(!updates.is_empty());
        assert!(updates.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(updates.last(), Some(&(8_000, 8_000)));
        assert_eq!(
            std::fs::read(&original).unwrap(),
            std::fs::read(&with_progress).unwrap()
        );

        let _ = std::fs::remove_file(original);
        let _ = std::fs::remove_file(with_progress);
    }

    #[test]
    fn flac_and_wavpack_progress_output_matches_original() {
        let audio = sine();
        let tags = Tags::new();

        let flac_original = temp_file("progress-original.flac");
        let flac_progress = temp_file("progress-with.flac");
        let flac_opts = FlacOptions::default();
        export_flac(&flac_original, &audio, 8_000, &flac_opts, &tags).unwrap();
        let mut flac_updates = Vec::new();
        let mut flac_callback = |done, total| {
            flac_updates.push((done, total));
            true
        };
        export_flac_with_progress(
            &flac_progress,
            &audio,
            8_000,
            &flac_opts,
            &tags,
            &mut flac_callback,
        )
        .unwrap();
        assert!(flac_updates.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(flac_updates.last(), Some(&(8_000, 8_000)));
        assert_eq!(
            std::fs::read(&flac_original).unwrap(),
            std::fs::read(&flac_progress).unwrap()
        );

        let wavpack_original = temp_file("progress-original.wv");
        let wavpack_progress = temp_file("progress-with.wv");
        let wavpack_opts = WavPackOptions::default();
        export_wavpack(
            &wavpack_original,
            &audio,
            8_000,
            &wavpack_opts,
            &tags,
        )
        .unwrap();
        let mut wavpack_updates = Vec::new();
        let mut wavpack_callback = |done, total| {
            wavpack_updates.push((done, total));
            true
        };
        export_wavpack_with_progress(
            &wavpack_progress,
            &audio,
            8_000,
            &wavpack_opts,
            &tags,
            &mut wavpack_callback,
        )
        .unwrap();
        assert!(wavpack_updates.windows(2).all(|w| w[0].0 <= w[1].0));
        assert_eq!(wavpack_updates.last(), Some(&(8_000, 8_000)));
        assert_eq!(
            std::fs::read(&wavpack_original).unwrap(),
            std::fs::read(&wavpack_progress).unwrap()
        );

        let _ = std::fs::remove_file(flac_original);
        let _ = std::fs::remove_file(flac_progress);
        let _ = std::fs::remove_file(wavpack_original);
        let _ = std::fs::remove_file(wavpack_progress);
    }

    #[test]
    fn cancellation_during_conversion_leaves_no_file() {
        let frames = FLAC_BLOCK_SIZE + 1;
        let audio = vec![vec![0.25f32; frames], vec![-0.25f32; frames]];
        let tags = Tags::new();
        let path = temp_file("cancel.flac");
        let _ = std::fs::remove_file(&path);
        let mut calls = 0usize;
        let mut callback = |_, _| {
            calls += 1;
            false
        };

        let result = export_flac_with_progress(
            &path,
            &audio,
            8_000,
            &FlacOptions::default(),
            &tags,
            &mut callback,
        );

        assert!(matches!(result, Err(crate::IoError::Cancelled)));
        assert_eq!(calls, 1);
        assert!(!path.exists());
    }
}
