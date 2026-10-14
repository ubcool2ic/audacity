//! au-ai: AI music features for the Rust Audacity port.
//! - Stem separation (`StemSeparator`, Demucs subprocess backend; ONNX/ort backend planned)
//! - Instrumental / karaoke export
//! - Pluggable text-to-music (`MusicGenerator`): offline procedural, OpenRouter Lyria 3, fal ACE-Step
//! Initial code generated with openai/gpt-6-luna via OpenRouter, reviewed by hand. See docs/AI_MUSIC.md.
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    io::{Cursor, Read},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("backend error: {0}")]
    Backend(String),
    #[error("WAV error: {0}")]
    Wav(#[from] hound::Error),
}

/// Interleaved f32 stems.
#[derive(Clone, Debug, PartialEq)]
pub struct Stems {
    pub sample_rate: u32,
    pub channels: u16,
    pub vocals: Vec<f32>,
    pub drums: Vec<f32>,
    pub bass: Vec<f32>,
    pub other: Vec<f32>,
}

impl Stems {
    /// Everything except vocals.
    pub fn instrumental(&self) -> Vec<f32> {
        self.drums
            .iter()
            .zip(&self.bass)
            .zip(&self.other)
            .map(|((&d, &b), &o)| d + b + o)
            .collect()
    }
    pub fn karaoke(&self) -> Vec<f32> {
        self.instrumental()
    }
}

pub trait StemSeparator {
    fn name(&self) -> &str;
    fn separate(&self, input: &Path) -> Result<Stems, AiError>;
}

/// Runs `python -m demucs` (MIT-licensed) as a subprocess.
pub struct DemucsSeparator {
    pub python: String,
    pub model: String,
    pub out_dir: PathBuf,
}

impl StemSeparator for DemucsSeparator {
    fn name(&self) -> &str {
        "Demucs"
    }
    fn separate(&self, input: &Path) -> Result<Stems, AiError> {
        let status = Command::new(&self.python)
            .args(["-m", "demucs", "-n", &self.model, "-d", "cpu", "-o"])
            .arg(&self.out_dir)
            .arg(input)
            .status()?;
        if !status.success() {
            return Err(AiError::Backend(format!("Demucs exited with status {status}")));
        }
        let stem = input
            .file_stem()
            .ok_or_else(|| AiError::Backend("input path has no file stem".into()))?;
        let dir = self.out_dir.join(&self.model).join(stem);
        let (vocals, sample_rate, channels) = read_wav(&dir.join("vocals.wav"))?;
        let mut rest = Vec::new();
        for n in ["drums", "bass", "other"] {
            let (s, sr, ch) = read_wav(&dir.join(format!("{n}.wav")))?;
            if sr != sample_rate || ch != channels {
                return Err(AiError::Backend("Demucs stems have mismatched formats".into()));
            }
            rest.push(s);
        }
        let other = rest.pop().unwrap();
        let bass = rest.pop().unwrap();
        let drums = rest.pop().unwrap();
        Ok(Stems { sample_rate, channels, vocals, drums, bass, other })
    }
}

pub fn read_wav(path: &Path) -> Result<(Vec<f32>, u32, u16), AiError> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 2_f32.powi(spec.bits_per_sample as i32 - 1);
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    Ok((samples, spec.sample_rate, spec.channels))
}

pub fn write_wav(path: &Path, samples: &[f32], sr: u32, ch: u16) -> Result<(), AiError> {
    let spec = hound::WavSpec { channels: ch, sample_rate: sr, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut w = hound::WavWriter::create(path, spec)?;
    for &s in samples {
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}

/// Separate `input` and write the vocal-free mix to `output` (32-bit float WAV).
pub fn export_instrumental(sep: &dyn StemSeparator, input: &Path, output: &Path) -> Result<Stems, AiError> {
    let stems = sep.separate(input)?;
    write_wav(output, &stems.instrumental(), stems.sample_rate, stems.channels)?;
    Ok(stems)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MusicRequest {
    pub prompt: String,
    pub lyrics: Option<String>,
    pub instrumental: bool,
    pub duration_secs: f32,
    pub seed: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct GeneratedAudio {
    pub bytes: Vec<u8>,
    pub mime: String,
    pub est_cost_usd: f64,
}

pub trait MusicGenerator {
    fn name(&self) -> &str;
    fn estimate_cost_usd(&self, req: &MusicRequest) -> f64;
    fn generate(&self, req: &MusicRequest) -> Result<GeneratedAudio, AiError>;
}

/// Free, offline, deterministic backend (I-V-vi-IV in C, 120 BPM). Used for tests and
/// as a no-network fallback. The "vocal" line is a vibrato sine placeholder, not singing.
pub struct ProceduralGenerator;

impl MusicGenerator for ProceduralGenerator {
    fn name(&self) -> &str {
        "Procedural (offline)"
    }
    fn estimate_cost_usd(&self, _: &MusicRequest) -> f64 {
        0.0
    }
    fn generate(&self, req: &MusicRequest) -> Result<GeneratedAudio, AiError> {
        const SR: u32 = 44_100;
        let tau = std::f32::consts::TAU;
        let frames = (req.duration_secs.max(0.0) * SR as f32) as usize;
        let chords: [[f32; 3]; 4] = [
            [261.63, 329.63, 392.00],
            [196.00, 246.94, 293.66],
            [220.00, 261.63, 329.63],
            [174.61, 220.00, 261.63],
        ];
        let mut cursor = Cursor::new(Vec::new());
        {
            let spec = hound::WavSpec { channels: 2, sample_rate: SR, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
            let mut w = hound::WavWriter::new(&mut cursor, spec)?;
            for i in 0..frames {
                let t = i as f32 / SR as f32;
                let chord = chords[((t / 2.0) as usize) % 4];
                let mut v = chord.iter().map(|&f| (tau * f * t).sin()).sum::<f32>() * 0.09;
                let beat = t % 0.5;
                v += (-(beat * 18.0)).exp() * (tau * (70.0 + 35.0 * beat) * t).sin() * 0.28;
                if !req.instrumental {
                    let vib = (tau * 5.0 * t).sin() * 4.0;
                    v += (tau * (440.0 + vib) * t).sin() * 0.08;
                }
                w.write_sample(v)?;
                w.write_sample(v)?;
            }
            w.finalize()?;
        }
        Ok(GeneratedAudio { bytes: cursor.into_inner(), mime: "audio/wav".into(), est_cost_usd: 0.0 })
    }
}

/// Google Lyria 3 via OpenRouter ($0.04 per 30 s clip, $0.08 per full song, Oct 2026).
pub struct OpenRouterLyria {
    pub api_key: String,
    pub model: String,
}

impl OpenRouterLyria {
    pub fn new_clip(api_key: impl Into<String>) -> Self {
        Self { api_key: api_key.into(), model: "google/lyria-3-clip-preview".into() }
    }
    pub fn new_pro(api_key: impl Into<String>) -> Self {
        Self { api_key: api_key.into(), model: "google/lyria-3-pro-preview".into() }
    }
}

impl MusicGenerator for OpenRouterLyria {
    fn name(&self) -> &str {
        "Google Lyria 3 (OpenRouter)"
    }
    fn estimate_cost_usd(&self, _: &MusicRequest) -> f64 {
        if self.model.contains("pro") { 0.08 } else { 0.04 }
    }
    fn generate(&self, req: &MusicRequest) -> Result<GeneratedAudio, AiError> {
        let content = if req.instrumental {
            format!("{}. Instrumental only, no vocals.", req.prompt)
        } else {
            match &req.lyrics {
                Some(l) => format!("{}\nLyrics:\n{}", req.prompt, l),
                None => format!("{} With vocals.", req.prompt),
            }
        };
        let resp: Value = ureq::post("https://openrouter.ai/api/v1/chat/completions")
            .set("Authorization", &format!("Bearer {}", self.api_key))
            .send_json(json!({
                "model": self.model,
                "modalities": ["audio", "text"],
                "messages": [{"role": "user", "content": content}]
            }))
            .map_err(|e| AiError::Http(e.to_string()))?
            .into_json()
            .map_err(|e| AiError::Http(e.to_string()))?;
        let msg = &resp["choices"][0]["message"];
        let audio = &msg["audio"];
        let encoded = audio["data"]
            .as_str()
            .or_else(|| audio.as_array().and_then(|a| a.first()).and_then(|v| v["data"].as_str()))
            .ok_or_else(|| AiError::Backend("response did not contain audio data".into()))?;
        let encoded = encoded
            .split_once(',')
            .filter(|(p, _)| p.starts_with("data:"))
            .map_or(encoded, |(_, d)| d);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|e| AiError::Backend(format!("invalid audio base64: {e}")))?;
        let mime = audio["format"]
            .as_str()
            .map(|f| if f.starts_with("audio/") { f.to_owned() } else { format!("audio/{f}") })
            .unwrap_or_else(|| "audio/mpeg".into());
        Ok(GeneratedAudio { bytes, mime, est_cost_usd: self.estimate_cost_usd(req) })
    }
}

/// ACE-Step (MIT) hosted on fal.ai ($0.0002 per generated second, Oct 2026).
pub struct FalAceStep {
    pub api_key: String,
}

impl MusicGenerator for FalAceStep {
    fn name(&self) -> &str {
        "ACE-Step (fal.ai)"
    }
    fn estimate_cost_usd(&self, req: &MusicRequest) -> f64 {
        0.0002 * req.duration_secs.max(0.0) as f64
    }
    fn generate(&self, req: &MusicRequest) -> Result<GeneratedAudio, AiError> {
        let lyrics = if req.instrumental { "[inst]".to_owned() } else { req.lyrics.clone().unwrap_or_default() };
        let resp: Value = ureq::post("https://fal.run/fal-ai/ace-step")
            .set("Authorization", &format!("Key {}", self.api_key))
            .send_json(json!({"tags": req.prompt, "lyrics": lyrics, "duration": req.duration_secs}))
            .map_err(|e| AiError::Http(e.to_string()))?
            .into_json()
            .map_err(|e| AiError::Http(e.to_string()))?;
        let url = resp["audio"]["url"]
            .as_str()
            .ok_or_else(|| AiError::Backend("response did not contain audio URL".into()))?;
        let mut bytes = Vec::new();
        ureq::get(url).call().map_err(|e| AiError::Http(e.to_string()))?.into_reader().read_to_end(&mut bytes)?;
        Ok(GeneratedAudio {
            bytes,
            mime: resp["audio"]["content_type"].as_str().unwrap_or("audio/wav").to_owned(),
            est_cost_usd: self.estimate_cost_usd(req),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(d: f32, inst: bool) -> MusicRequest {
        MusicRequest { prompt: "t".into(), lyrics: None, instrumental: inst, duration_secs: d, seed: Some(1) }
    }

    #[test]
    fn instrumental_sums_stems() {
        let s = Stems { sample_rate: 44_100, channels: 2, vocals: vec![9.0, 9.0], drums: vec![1.0, 2.0], bass: vec![3.0, 4.0], other: vec![5.0, 6.0] };
        assert_eq!(s.instrumental(), vec![9.0, 12.0]);
        assert_eq!(s.karaoke(), s.instrumental());
    }

    #[test]
    fn procedural_wav_roundtrip() {
        for inst in [true, false] {
            let a = ProceduralGenerator.generate(&req(1.0, inst)).unwrap();
            let r = hound::WavReader::new(Cursor::new(a.bytes)).unwrap();
            assert_eq!(r.spec().channels, 2);
            let secs = r.duration() as f32 / r.spec().sample_rate as f32;
            assert!((secs - 1.0).abs() <= 0.1);
        }
    }

    #[test]
    fn wav_file_roundtrip() {
        let p = std::env::temp_dir().join("au_ai_rt.wav");
        write_wav(&p, &[0.5, -0.5, 0.25, 0.0], 48_000, 2).unwrap();
        let (s, sr, ch) = read_wav(&p).unwrap();
        assert_eq!((s, sr, ch), (vec![0.5, -0.5, 0.25, 0.0], 48_000, 2));
    }

    #[test]
    fn cost_estimates() {
        let r = req(10.0, true);
        assert_eq!(ProceduralGenerator.estimate_cost_usd(&r), 0.0);
        assert_eq!(OpenRouterLyria::new_clip("").estimate_cost_usd(&r), 0.04);
        assert_eq!(OpenRouterLyria::new_pro("").estimate_cost_usd(&r), 0.08);
        assert!((FalAceStep { api_key: String::new() }.estimate_cost_usd(&r) - 0.002).abs() < 1e-10);
    }
}
