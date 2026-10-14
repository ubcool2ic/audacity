use au_core::AudioBuffer;
use au_io::ffmpeg::{status, FfmpegStatus};
use au_io::{export_wav, import_file, IoError};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn unique_temp_dir() -> PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "au-io-ffmpeg-absent-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).expect("create temporary directory");
    path
}

#[test]
fn app_works_without_ffmpeg() {
    let dir = unique_temp_dir();
    let empty_lib_dir = dir.join("empty-libraries");
    fs::create_dir_all(&empty_lib_dir).expect("create empty library directory");
    std::env::set_var("AU_FFMPEG_LIBDIR", &empty_lib_dir);

    let message = match status() {
        FfmpegStatus::Missing { message } => message,
        FfmpegStatus::Available { version } => {
            panic!("expected FFmpeg to be unavailable, found {version}")
        }
    };
    assert!(message.contains("FFmpeg"));
    assert!(message.contains("apt install"));

    let dummy_file = dir.join("x.m4a");
    fs::write(&dummy_file, b"not an actual audio file").expect("write dummy M4A file");
    match import_file(&dummy_file) {
        Err(IoError::FfmpegMissing(message)) => {
            assert!(message.to_string().contains("libavformat"));
        }
        Err(error) => panic!("expected FfmpegMissing, got {error}"),
        Ok(_) => panic!("expected importing M4A to fail without FFmpeg"),
    }

    let sample_rate = 44_100;
    let samples = vec![0.25_f32; (sample_rate as usize) / 10];
    let original = AudioBuffer::from_channels(sample_rate, vec![samples]);
    let wav_file = dir.join("built-in.wav");
    export_wav(&wav_file, &original, 16).expect("export WAV without FFmpeg");

    let decoded = import_file(&wav_file).expect("import WAV without FFmpeg");
    assert_eq!(decoded.len(), original.len());
}

