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

/// Config dir for persisted settings: `%APPDATA%\audacity-rs` on Windows,
/// else `$XDG_CONFIG_HOME/audacity-rs` or `$HOME/.config/audacity-rs`.
pub fn app_config_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("APPDATA").map(|p| std::path::PathBuf::from(p).join("audacity-rs"))
    }
    #[cfg(not(windows))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            let p = std::path::PathBuf::from(xdg);
            if !p.as_os_str().is_empty() {
                return Some(p.join("audacity-rs"));
            }
        }
        home_dir().map(|h| h.join(".config/audacity-rs"))
    }
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// Key lookup order: (a) non-empty `OPENROUTER_API_KEY` env var; (b) saved
/// `openrouter.key` in [`app_config_dir`] (written by Preferences via [`save_key`]);
/// (c) `<app_config_dir>/openrouter.env` via [`parse_env_key`]; (d)
/// `<home_dir>/.config/openrouter/env` via [`parse_env_key`].
pub fn load_openrouter_key() -> Option<String> {
    if let Ok(k) = std::env::var("OPENROUTER_API_KEY") {
        if !k.trim().is_empty() {
            return Some(k);
        }
    }
    if let Some(k) = load_saved_key() {
        return Some(k);
    }
    if let Some(dir) = app_config_dir() {
        let path = dir.join("openrouter.env");
        if let Ok(c) = std::fs::read_to_string(path) {
            if let Some(k) = parse_env_key(&c, "OPENROUTER_API_KEY") {
                return Some(k);
            }
        }
    }
    if let Some(home) = home_dir() {
        let path = home.join(".config/openrouter/env");
        if let Ok(c) = std::fs::read_to_string(path) {
            if let Some(k) = parse_env_key(&c, "OPENROUTER_API_KEY") {
                return Some(k);
            }
        }
    }
    None
}

fn saved_key_path() -> Option<std::path::PathBuf> {
    app_config_dir().map(|d| d.join("openrouter.key"))
}

/// Persist `key` to `openrouter.key` in [`app_config_dir`]. An empty/whitespace
/// key deletes the file (ignoring `NotFound`). On Windows the bytes are
/// protected with DPAPI (`CryptProtectData`, current user, no UI); on unix the
/// file is created with mode `0o600` and truncated.
pub fn save_key(key: &str) -> std::io::Result<()> {
    let Some(path) = saved_key_path() else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "no config dir"));
    };
    if key.trim().is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    } else {
        std::fs::create_dir_all(path.parent().expect("key path has parent"))?;
        #[cfg(windows)]
        {
            let blob = dpapi_protect(key.trim().as_bytes())?;
            std::fs::write(&path, blob)
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&path)?;
            use std::io::Write;
            f.write_all(key.trim().as_bytes())?;
            f.flush()
        }
    }
}

fn load_saved_key() -> Option<String> {
    let path = saved_key_path()?;
    #[cfg(windows)]
    {
        let blob = std::fs::read(path).ok()?;
        let plain = dpapi_unprotect(&blob).ok()?;
        let s = String::from_utf8_lossy(&plain).trim().to_owned();
        if s.is_empty() { None } else { Some(s) }
    }
    #[cfg(not(windows))]
    {
        let c = std::fs::read_to_string(path).ok()?;
        let s = c.trim().to_owned();
        if s.is_empty() { None } else { Some(s) }
    }
}

/// True when a non-empty saved key exists and can be loaded.
pub fn has_saved_key() -> bool {
    load_saved_key().is_some()
}

#[cfg(windows)]
fn dpapi_protect(plain: &[u8]) -> std::io::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN,
    };
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: plain.len() as u32,
            pbData: plain.as_ptr() as *mut u8,
        };
        let mut output = std::mem::zeroed::<CRYPT_INTEGER_BLOB>();
        let ok = CryptProtectData(
            &mut input,
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        );
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let out = slice.to_vec();
        LocalFree(output.pbData as _);
        Ok(out)
    }
}

#[cfg(windows)]
fn dpapi_unprotect(blob: &[u8]) -> std::io::Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: blob.len() as u32,
            pbData: blob.as_ptr() as *mut u8,
        };
        let mut output = std::mem::zeroed::<CRYPT_INTEGER_BLOB>();
        let ok = CryptUnprotectData(
            &mut input,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            0,
            &mut output,
        );
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let out = slice.to_vec();
        LocalFree(output.pbData as _);
        Ok(out)
    }
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

    #[cfg(unix)]
    #[test]
    fn save_load_roundtrip_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("audacity-rs-test-{}-{}", std::process::id(), "openrouter"));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        // Ensure env lookup does not shadow the saved key.
        std::env::remove_var("OPENROUTER_API_KEY");
        assert!(!has_saved_key());
        save_key("sk-test-secret").expect("save");
        let path = dir.join("audacity-rs/openrouter.key");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert!(has_saved_key());
        assert_eq!(load_saved_key().as_deref(), Some("sk-test-secret"));
        assert_eq!(load_openrouter_key().as_deref(), Some("sk-test-secret"));
        save_key("").expect("delete");
        assert!(!path.exists());
        assert!(!has_saved_key());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
