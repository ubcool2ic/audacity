// SPDX-License-Identifier: GPL-3.0-or-later
// Part of the Audacity Rust port. Derived from Audacity (c) Audacity Team, GPLv3.

//! Per-user configuration base directory, matching where Audacity keeps its
//! settings on each platform.

use std::path::PathBuf;

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// Base directory for per-user configuration:
/// `$XDG_CONFIG_HOME` (any platform, if set; used by tests), then
/// `%APPDATA%` on Windows, `~/Library/Application Support` on macOS,
/// `~/.config` elsewhere. Falls back to `.config` when nothing is set.
pub fn user_config_base() -> PathBuf {
    if let Some(x) = env_dir("XDG_CONFIG_HOME") {
        return x;
    }
    #[cfg(windows)]
    {
        if let Some(a) = env_dir("APPDATA") {
            return a;
        }
        if let Some(p) = env_dir("USERPROFILE") {
            return p.join("AppData").join("Roaming");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(h) = env_dir("HOME") {
            return h.join("Library").join("Application Support");
        }
    }
    env_dir("HOME").map(|h| h.join(".config")).unwrap_or_else(|| PathBuf::from(".config"))
}
