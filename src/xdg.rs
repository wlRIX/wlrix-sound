// SPDX-License-Identifier: GPL-3.0-or-later
//! Where the daemon's settings, the sound themes and the request socket live.
//!
//! The settings convention is the compositor's, unchanged (see its `config.rs`): a hand-edited
//! file under `$XDG_CONFIG_HOME/wlrix/`, with `/etc/wlrix/` behind it for a system default.
//!
//! Sound themes are looked for the way the XDG sound theme spec says: under `sounds/` in
//! `$XDG_DATA_HOME`, then in each of `$XDG_DATA_DIRS` -- the same walk `wlrix-compositor`'s
//! `icon_image.rs` does for the minimized-window pictures.

use std::path::{Path, PathBuf};

/// The hand-edited settings file, relative to a config directory.
pub const CONFIG_NAME: &str = "wlrix/sounds.toml";
/// Consulted when the user has no config of their own, as the compositor does.
const SYSTEM_CONFIG_DIR: &str = "/etc";
/// What `$XDG_DATA_DIRS` means when it is unset or empty, per the base directory spec.
const DEFAULT_DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// `$HOME`, or `None` when even that is unset.
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

/// An absolute path from the environment, or `None`. The spec says a relative one is to be
/// ignored.
fn absolute_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
}

/// `$XDG_CONFIG_HOME`, or `~/.config` as the spec says to assume.
pub fn user_config_dir() -> Option<PathBuf> {
    absolute_env("XDG_CONFIG_HOME").or_else(|| home().map(|home| home.join(".config")))
}

/// Where to look for the settings file, most specific first.
pub fn config_paths() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = user_config_dir() {
        dirs.push(dir.join(CONFIG_NAME));
    }
    dirs.push(Path::new(SYSTEM_CONFIG_DIR).join(CONFIG_NAME));
    dirs
}

/// Every `sounds` directory a theme can be in, most specific first: the user's own data
/// directory, then each system one.
pub fn sound_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) =
        absolute_env("XDG_DATA_HOME").or_else(|| home().map(|home| home.join(".local/share")))
    {
        dirs.push(dir.join("sounds"));
    }
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|dirs| !dirs.is_empty())
        .unwrap_or_else(|| DEFAULT_DATA_DIRS.to_owned());
    for dir in system.split(':').map(PathBuf::from) {
        // Relative entries are to be ignored, as with the single-directory variables.
        if dir.is_absolute() {
            let dir = dir.join("sounds");
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }
    dirs
}

/// `$XDG_RUNTIME_DIR`, or `None`.
///
/// No fallback to the temp dir for the socket, unlike the pidfile: a socket anyone can reach in a
/// world-writable directory is one anyone can make the speakers play through.
pub fn runtime_dir() -> Option<PathBuf> {
    absolute_env("XDG_RUNTIME_DIR")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_file_is_always_a_candidate() {
        let paths = config_paths();
        assert!(paths.last().unwrap().ends_with("wlrix/sounds.toml"));
        assert!(paths.last().unwrap().starts_with("/etc"));
    }
}
