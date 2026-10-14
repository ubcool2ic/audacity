use au_core::AudioBuffer;
use au_io::ffmpeg::{get, status, ExportFormat, FfmpegStatus};
use au_io::import_file;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

fn unique_temp_dir() -> PathBuf {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "au-io-ffmpeg-present-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).expect("create temporary directory");
    path
}

fn freq(samples: &[f32], rate: u32) -> f32 {
    if samples.len() < 2 {
        return 0.0;
    }
    // Hysteresis so lossy-codec noise around zero is not counted as extra crossings.
    let threshold = 0.25 * samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let mut armed = false;
    let mut positive_crossings = 0usize;
    for &sample in samples {
        if sample < -threshold {
            armed = true;
        } else if armed && sample > threshold {
            armed = false;
            positive_crossings += 1;
        }
    }
    positive_crossings as f32 * rate as f32 / (samples.len() - 1) as f32
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples
        .iter()
        .map(|sample| (*sample as f64) * (*sample as f64))
        .sum::<f64>()
        / samples.len() as f64)
        .sqrt() as f32
}

fn sine_buffer(sample_rate: u32, duration_seconds: usize, frequency: f32) -> AudioBuffer {
    let length = sample_rate as usize * duration_seconds;
    let samples: Vec<f32> = (0..length)
        .map(|index| {
            (std::f32::consts::TAU * frequency * index as f32 / sample_rate as f32).sin() * 0.5
        })
        .collect();
    AudioBuffer::from_channels(sample_rate, vec![samples])
}

fn command_succeeded(output: Output, description: &str) {
    assert!(
        output.status.success(),
        "{description} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn cli_available(name: &str) -> bool {
    Command::new(name)
        .arg("-version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn assert_tone(audio: &AudioBuffer, expected_frequency: f32) {
    let measured = freq(&audio.channels[0], audio.sample_rate);
    assert!(
        (measured - expected_frequency).abs() / expected_frequency < 0.03,
        "expected about {expected_frequency} Hz, measured {measured} Hz"
    );
}

#[test]
fn m4a_round_trip_via_import_file() {
    if let FfmpegStatus::Missing { message } = status() {
        eprintln!("skip: {message}");
        return;
    }

    let dir = unique_temp_dir();
    let path = dir.join("round-trip.m4a");
    let sample_rate = 44_100;
    let length = sample_rate as usize * 2;
    let samples: Vec<f32> = (0..length)
        .map(|index| {
            (std::f32::consts::TAU * 1_000.0 * index as f32 / sample_rate as f32).sin() * 0.5
        })
        .collect();
    let original = AudioBuffer::from_channels(sample_rate, vec![samples.clone(), samples]);

    get()
        .expect("load FFmpeg")
        .export(
            &path,
            &original,
            ExportFormat::M4a,
            ExportFormat::M4a.default_bit_rate(),
        )
        .expect("export M4A");

    let decoded = import_file(&path).expect("import exported M4A");
    assert_eq!(decoded.channels.len(), 2);
    assert_eq!(decoded.sample_rate, 44_100);
    let duration = decoded.len() as f32 / decoded.sample_rate as f32;
    assert!((duration - 2.0).abs() < 0.1);

    assert_tone(&decoded, 1_000.0);
    let middle = &decoded.channels[0][decoded.len() / 4..decoded.len() * 3 / 4];
    let measured_rms = rms(middle);
    assert!(
        (measured_rms - 0.3536).abs() / 0.3536 < 0.20,
        "expected middle RMS near 0.3536, measured {measured_rms}"
    );
}

#[test]
fn audio_from_video_file() {
    if let FfmpegStatus::Missing { message } = status() {
        eprintln!("skip: {message}");
        return;
    }
    if !cli_available("ffmpeg") {
        eprintln!("skip: ffmpeg CLI is missing");
        return;
    }

    let dir = unique_temp_dir();
    let mp4 = dir.join("video.mp4");
    let mkv = dir.join("video.mkv");

    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x120:rate=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000",
            "-t",
            "2",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-shortest",
        ])
        .arg(&mp4)
        .output()
        .expect("run ffmpeg to generate MP4");
    command_succeeded(output, "generate MP4");

    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=160x120:rate=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000",
            "-t",
            "2",
            "-c:v",
            "mpeg4",
            "-c:a",
            "ac3",
            "-shortest",
        ])
        .arg(&mkv)
        .output()
        .expect("run ffmpeg to generate MKV");
    command_succeeded(output, "generate MKV");

    for path in [&mp4, &mkv] {
        let audio = import_file(path).expect("import audio from video container");
        assert_eq!(audio.sample_rate, 48_000);
        let duration = audio.len() as f32 / audio.sample_rate as f32;
        assert!((duration - 2.0).abs() < 0.15);
        assert_tone(&audio, 880.0);
        assert!(rms(&audio.channels[0]) > 0.05);
    }
}

#[test]
fn foreign_wma_and_ac3_decode() {
    if let FfmpegStatus::Missing { message } = status() {
        eprintln!("skip: {message}");
        return;
    }
    if !cli_available("ffmpeg") {
        eprintln!("skip: ffmpeg CLI is missing");
        return;
    }

    let dir = unique_temp_dir();
    let wma = dir.join("tone.wma");
    let ac3 = dir.join("tone.ac3");

    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=500:sample_rate=44100",
            "-t",
            "1",
            "-c:a",
            "wmav2",
            "-ac",
            "2",
        ])
        .arg(&wma)
        .output()
        .expect("run ffmpeg to generate WMA");
    command_succeeded(output, "generate WMA");

    let output = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=500:sample_rate=48000",
            "-t",
            "1",
            "-ar",
            "48000",
            "-c:a",
            "ac3",
            "-ac",
            "2",
        ])
        .arg(&ac3)
        .output()
        .expect("run ffmpeg to generate AC3");
    command_succeeded(output, "generate AC3");

    for path in [&wma, &ac3] {
        let audio = import_file(path).expect("decode foreign audio file");
        assert_eq!(audio.channels.len(), 2);
        assert_tone(&audio, 500.0);
    }
}

#[test]
fn our_exports_are_readable_by_ffmpeg_cli() {
    if let FfmpegStatus::Missing { message } = status() {
        eprintln!("skip: {message}");
        return;
    }
    if !cli_available("ffprobe") {
        eprintln!("skip: ffprobe CLI is missing");
        return;
    }

    let dir = unique_temp_dir();
    let audio = sine_buffer(48_000, 1, 440.0);
    let formats = [
        (ExportFormat::M4a, "aac"),
        (ExportFormat::Ac3, "ac3"),
        (ExportFormat::Wma, "wmav2"),
    ];

    for (format, codec) in formats {
        let path = dir.join(format!("tone.{}", format.extension()));
        get()
            .expect("load FFmpeg")
            .export(&path, &audio, format, format.default_bit_rate())
            .expect("export audio");

        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,sample_rate,channels",
                "-of",
                "csv=p=0",
            ])
            .arg(&path)
            .output()
            .expect("run ffprobe");
        command_succeeded(output.clone(), "probe exported audio");
        let report = String::from_utf8_lossy(&output.stdout);
        assert!(
            report.contains(codec),
            "expected codec {codec} in ffprobe output: {report}"
        );
        assert!(
            report.contains("48000"),
            "expected 48000 Hz in ffprobe output: {report}"
        );
    }
}
