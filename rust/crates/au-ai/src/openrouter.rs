//! OpenRouter helpers: key loading and Lyria SSE parsing (generated with gpt-6-luna, reviewed).
use crate::AiError;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde_json::Value;
use std::io::BufRead;

/// Parse `NAME=value` / `export NAME="value"` from a shell env file.
pub fn parse_env_key(contents: &str, name: &str) -> Option<String> {
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else { continue };
        if key.trim() != name {
            continue;
        }
        let v = value.trim();
        let v = if v.len() >= 2 && ((v.starts_with('"') && v.ends_with('"')) || (v.starts_with('\'') && v.ends_with('\''))) {
            &v[1..v.len() - 1]
        } else {
            v
        };
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_owned());
        }
    }
    None
}

/// Key from `~/.config/openrouter/env` if present, else `$OPENROUTER_API_KEY`.
pub fn load_openrouter_key() -> Option<String> {
    if let Some(home) = std::env::var_os("HOME") {
        let path = std::path::PathBuf::from(home).join(".config/openrouter/env");
        if let Ok(c) = std::fs::read_to_string(path) {
            if let Some(k) = parse_env_key(&c, "OPENROUTER_API_KEY") {
                return Some(k);
            }
        }
    }
    std::env::var("OPENROUTER_API_KEY").ok().filter(|k| !k.trim().is_empty())
}

/// Parse an OpenRouter audio SSE stream → (bytes, format, cost_usd, text).
pub fn parse_lyria_sse(reader: impl BufRead) -> Result<(Vec<u8>, Option<String>, Option<f64>, String), AiError> {
    let (mut enc, mut format, mut cost, mut text) = (String::new(), None, None, String::new());
    for line in reader.lines() {
        let line = line?;
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim_start();
        if data == "[DONE]" {
            continue;
        }
        let chunk: Value = serde_json::from_str(data).map_err(|e| AiError::Backend(format!("invalid SSE JSON: {e}")))?;
        if let Some(err) = chunk.get("error") {
            let msg = err.get("message").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| err.to_string());
            return Err(AiError::Backend(msg));
        }
        if let Some(c) = chunk.pointer("/usage/cost").and_then(Value::as_f64) {
            cost = Some(c);
        }
        for choice in chunk.get("choices").and_then(Value::as_array).into_iter().flatten() {
            let Some(delta) = choice.get("delta") else { continue };
            if let Some(audio) = delta.get("audio").and_then(Value::as_object) {
                if let Some(d) = audio.get("data").and_then(Value::as_str) {
                    enc.push_str(d);
                }
                if let Some(f) = audio.get("format").and_then(Value::as_str) {
                    format = Some(f.to_owned());
                }
            }
            if let Some(c) = delta.get("content").and_then(Value::as_str) {
                text.push_str(c);
            }
        }
    }
    if enc.is_empty() {
        return Err(AiError::Backend("no audio data received".into()));
    }
    let bytes = STANDARD.decode(enc).map_err(|e| AiError::Backend(format!("invalid base64 audio: {e}")))?;
    Ok((bytes, format, cost, text))
}

pub fn sniff_audio_mime(b: &[u8]) -> &'static str {
    if b.starts_with(b"ID3") || (b.len() >= 2 && b[0] == 0xFF && b[1] & 0xE0 == 0xE0) {
        "audio/mpeg"
    } else if b.starts_with(b"RIFF") {
        "audio/wav"
    } else if b.starts_with(b"fLaC") {
        "audio/flac"
    } else if b.starts_with(b"OggS") {
        "audio/ogg"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod sse_tests {
    use super::*;

    #[test]
    fn env_key() {
        let c = "# x\nnoequals\nexport OTHER=no\nexport OPENROUTER_API_KEY=\"secret\"\n";
        assert_eq!(parse_env_key(c, "OPENROUTER_API_KEY").as_deref(), Some("secret"));
    }

    #[test]
    fn split_audio_sse() {
        let s = concat!(
            "data: {\"choices\":[{\"delta\":{\"audio\":{\"data\":\"SUQz\",\"format\":\"mp3\"},\"content\":\"<in\"}}]}\n",
            "\n: keepalive\n",
            "data: {\"choices\":[{\"delta\":{\"audio\":{\"data\":\"YWJj\"},\"content\":\"strumental>\"}}],\"usage\":{\"cost\":0.04}}\n",
            "data: [DONE]\n"
        );
        let (a, f, c, t) = parse_lyria_sse(std::io::Cursor::new(s)).unwrap();
        assert_eq!(a, b"ID3abc");
        assert_eq!(f.as_deref(), Some("mp3"));
        assert_eq!(c, Some(0.04));
        assert_eq!(t, "<instrumental>");
    }

    #[test]
    fn error_chunk() {
        let s = "data: {\"error\":{\"message\":\"boom\"}}\n";
        assert!(matches!(parse_lyria_sse(std::io::Cursor::new(s)), Err(AiError::Backend(m)) if m == "boom"));
    }

    #[test]
    fn sniff() {
        assert_eq!(sniff_audio_mime(b"ID3abc"), "audio/mpeg");
        assert_eq!(sniff_audio_mime(&[0xFF, 0xFB]), "audio/mpeg");
        assert_eq!(sniff_audio_mime(b"RIFFxxxx"), "audio/wav");
        assert_eq!(sniff_audio_mime(b"OggS"), "audio/ogg");
        assert_eq!(sniff_audio_mime(b"zz"), "application/octet-stream");
    }
}
