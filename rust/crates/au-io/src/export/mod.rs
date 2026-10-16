//! Audacity-compatible exporters backed by the same codec libraries Audacity 3.7.3 links.

pub mod intfmt;
pub mod mp3;
pub mod ogg_opus;

use au_core::tags::Tags;
use std::path::Path;

/// One export target with its Audacity options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExportFormat {
    /// WAV / AIFF / other libsndfile container (header + encoding in the options).
    Pcm(intfmt::PcmOptions),
    /// FLAC.
    Flac(intfmt::FlacOptions),
    /// WavPack.
    WavPack(intfmt::WavPackOptions),
    /// MP3 (LAME).
    Mp3(mp3::MP3Options),
    /// Ogg Vorbis.
    Vorbis(ogg_opus::VorbisOptions),
    /// Opus.
    Opus(ogg_opus::OpusOptions),
}

impl ExportFormat {
    /// Default file extension.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Pcm(o) if o.header == intfmt::SF_FORMAT_AIFF => "aiff",
            Self::Pcm(_) => "wav",
            Self::Flac(_) => "flac",
            Self::WavPack(_) => "wv",
            Self::Mp3(_) => "mp3",
            Self::Vorbis(_) => "ogg",
            Self::Opus(_) => "opus",
        }
    }
}

impl ExportFormat {
    /// Format Audacity's Export2 picks for a file extension, with default options.
    /// WAV and FLAC are left to the caller's own exporters.
    pub fn from_extension(ext: &str) -> Option<ExportFormat> {
        Some(match ext.to_ascii_lowercase().as_str() {
            "aif" | "aiff" => Self::Pcm(intfmt::PcmOptions { header: intfmt::SF_FORMAT_AIFF, ..Default::default() }),
            "wv" => Self::WavPack(Default::default()),
            "mp3" => Self::Mp3(Default::default()),
            "ogg" => Self::Vorbis(Default::default()),
            "opus" => Self::Opus(Default::default()),
            _ => return None,
        })
    }
}

/// Progress callback for long imports/exports: `(frames_done, frames_total) -> keep_going`.
/// Returning `false` aborts with [`crate::IoError::Cancelled`]. `frames_total` is 0 when unknown.
pub type ProgressFn<'a> = &'a mut dyn FnMut(u64, u64) -> bool;

/// Exports planar float audio in the given format with metadata tags.
pub fn export_any(path: &Path, channels: &[Vec<f32>], sample_rate: u32, format: &ExportFormat, tags: &Tags) -> Result<(), crate::IoError> {
    match format {
        ExportFormat::Pcm(o) => intfmt::export_pcm(path, channels, sample_rate, o, tags),
        ExportFormat::Flac(o) => intfmt::export_flac(path, channels, sample_rate, o, tags),
        ExportFormat::WavPack(o) => intfmt::export_wavpack(path, channels, sample_rate, o, tags),
        ExportFormat::Mp3(o) => mp3::export_mp3(path, channels, sample_rate, o, tags),
        ExportFormat::Vorbis(o) => ogg_opus::export_vorbis(path, channels, sample_rate, o, tags),
        ExportFormat::Opus(o) => ogg_opus::export_opus(path, channels, sample_rate, o, tags),
    }
}
