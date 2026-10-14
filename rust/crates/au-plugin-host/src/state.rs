// SPDX-License-Identifier: GPL-3.0-or-later

use au_plugin_api::{PluginError, PluginFormat};
use std::path::{Path, PathBuf};

pub const PROCESSOR_STATE: &str = "ProcessorState";
pub const CONTROLLER_STATE: &str = "ControllerState";
pub const PARAMETERS: &str = "Parameters";
pub const CLAP_STATE: &str = "State";

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes bytes using the standard padded Base64 alphabet.
pub fn b64_encode(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);

    for chunk in bytes.chunks(3) {
        let a = chunk[0];
        let b = chunk.get(1).copied().unwrap_or(0);
        let c = chunk.get(2).copied().unwrap_or(0);

        encoded.push(BASE64_ALPHABET[(a >> 2) as usize] as char);
        encoded.push(BASE64_ALPHABET[(((a & 0x03) << 4) | (b >> 4)) as usize] as char);

        if chunk.len() > 1 {
            encoded.push(BASE64_ALPHABET[(((b & 0x0f) << 2) | (c >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }

        if chunk.len() > 2 {
            encoded.push(BASE64_ALPHABET[(c & 0x3f) as usize] as char);
        } else {
            encoded.push('=');
        }
    }

    encoded
}

/// Decodes standard padded Base64.
pub fn b64_decode(input: &str) -> Result<Vec<u8>, PluginError> {
    let bytes = input.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(PluginError::Unknown("invalid Base64 length".into()));
    }

    let mut decoded = Vec::with_capacity((bytes.len() / 4) * 3);

    for (chunk_index, chunk) in bytes.chunks_exact(4).enumerate() {
        let last_chunk = chunk_index + 1 == bytes.len() / 4;
        let padding = if chunk[3] == b'=' {
            if chunk[2] == b'=' {
                2
            } else {
                1
            }
        } else {
            0
        };

        if (!last_chunk && padding != 0)
            || (padding == 2 && (chunk[2] != b'=' || chunk[3] != b'='))
            || (padding == 1 && chunk[2] == b'=')
        {
            return Err(PluginError::Unknown("invalid Base64 padding".into()));
        }

        let a = decode_base64_char(chunk[0])
            .ok_or_else(|| PluginError::Unknown("invalid Base64 character".into()))?;
        let b = decode_base64_char(chunk[1])
            .ok_or_else(|| PluginError::Unknown("invalid Base64 character".into()))?;
        let c = if padding == 2 {
            0
        } else {
            decode_base64_char(chunk[2])
                .ok_or_else(|| PluginError::Unknown("invalid Base64 character".into()))?
        };
        let d = if padding != 0 {
            0
        } else {
            decode_base64_char(chunk[3])
                .ok_or_else(|| PluginError::Unknown("invalid Base64 character".into()))?
        };

        if (padding == 2 && b & 0x0f != 0) || (padding == 1 && c & 0x03 != 0) {
            return Err(PluginError::Unknown("invalid Base64 padding bits".into()));
        }

        decoded.push((a << 2) | (b >> 4));
        if padding < 2 {
            decoded.push((b << 4) | (c >> 2));
        }
        if padding == 0 {
            decoded.push((c << 6) | d);
        }
    }

    Ok(decoded)
}

fn decode_base64_char(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Formats parameter values in Audacity's `id=value;` representation.
pub fn params_to_string(params: &[(u32, f64)]) -> String {
    let mut sorted = params.to_vec();
    sorted.sort_by_key(|(id, _)| *id);

    let mut result = String::new();
    for (id, value) in sorted {
        result.push_str(&format!("{id}={value:.6};"));
    }
    result
}

/// Parses valid `id=value;` entries, skipping malformed tokens.
pub fn params_from_string(input: &str) -> Vec<(u32, f64)> {
    input
        .split(';')
        .filter_map(|token| {
            let (id, value) = token.split_once('=')?;
            let id = id.trim().parse::<u32>().ok()?;
            let value = value.trim().parse::<f64>().ok()?;
            Some((id, value))
        })
        .collect()
}

/// Returns the first value associated with `key`.
pub fn get<'a>(state: &'a [(String, String)], key: &str) -> Option<&'a str> {
    state
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// Builds an Audacity-compatible effect id.
pub fn effect_id(format: PluginFormat, path: &Path, uid: &str) -> String {
    let prefix = match format {
        PluginFormat::Vst3 => "VST3",
        PluginFormat::Clap => "CLAP",
        _ => return format!("{format:?}:{};{uid}", path.display()),
    };
    format!("{prefix}:{};{uid}", path.display())
}

/// Parses a VST3 or CLAP effect id, splitting at its final semicolon.
pub fn parse_effect_id(id: &str) -> Option<(PluginFormat, PathBuf, String)> {
    let (prefix, remainder) = id.split_once(':')?;
    let format = match prefix {
        "VST3" => PluginFormat::Vst3,
        "CLAP" => PluginFormat::Clap,
        _ => return None,
    };
    let (path, uid) = remainder.rsplit_once(';')?;
    if path.is_empty() || uid.is_empty() {
        return None;
    }
    Some((format, PathBuf::from(path), uid.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for bytes in [
            &b""[..],
            &b"f"[..],
            &b"fo"[..],
            &b"foo"[..],
            &b"\0\x01\xfe\xff"[..],
        ] {
            let encoded = b64_encode(bytes);
            assert_eq!(b64_decode(&encoded).unwrap(), bytes);
        }
    }

    #[test]
    fn params_string_round_trip() {
        let params = [(3, 1.0), (0, 0.5)];
        let encoded = params_to_string(&params);
        assert_eq!(encoded, "0=0.500000;3=1.000000;");
        assert_eq!(params_from_string(&encoded), vec![(0, 0.5), (3, 1.0)]);
    }

    #[test]
    fn effect_id_parse() {
        let id = effect_id(
            PluginFormat::Vst3,
            Path::new("/plugins/example.vst3"),
            "vendor.class",
        );
        assert_eq!(
            parse_effect_id(&id),
            Some((
                PluginFormat::Vst3,
                PathBuf::from("/plugins/example.vst3"),
                "vendor.class".to_owned()
            ))
        );

        let id = effect_id(
            PluginFormat::Clap,
            Path::new("/plugins/example.clap"),
            "org.example.plugin",
        );
        assert_eq!(
            parse_effect_id(&id),
            Some((
                PluginFormat::Clap,
                PathBuf::from("/plugins/example.clap"),
                "org.example.plugin".to_owned()
            ))
        );
    }
}
