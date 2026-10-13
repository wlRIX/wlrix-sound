// SPDX-License-Identifier: GPL-3.0-or-later
//! The hand-edited settings file.
//!
//! ```toml
//! # ~/.config/wlrix/sounds.toml
//! enabled = true                             # play system sounds at all
//! theme = "wlrix"                            # the XDG sound theme, by directory name
//! disabled = ["bell"]                       # sounds that never play, by name
//! ```
//!
//! Read from the user's config directory first, then `/etc/wlrix`; the first file found wins
//! outright rather than merging, as everywhere else in wlRIX. Unknown keys are an error, for the
//! same reason the compositor rejects them -- a silently ignored typo in a config file is a bad
//! afternoon.
//!
//! `wlrix-settings-daemon` writes this file and validates a candidate with `--check-config`. Its
//! schema (`src/schema/table.rs`, the `sounds.toml` block) carries the same defaults, kept in
//! step by hand.

use serde::Deserialize;

/// The theme when the file does not name one: the IRIX soundscheme, from `wlrix-sounds`.
pub const DEFAULT_THEME: &str = "wlrix";

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Whether any sound plays. Defaults to yes.
    #[serde(default)]
    enabled: Option<bool>,
    /// The sound theme's directory name under `share/sounds`.
    #[serde(default)]
    theme: Option<String>,
    /// Sound names that never play. Matched exactly against the name a program asked for, before
    /// any of the theme's fallbacks -- muting `dialog-error` does not mute `dialog-error-serious`.
    /// Missing and empty are the same, as with every list in wlRIX's config files.
    #[serde(default)]
    disabled: Vec<String>,
}

impl Config {
    pub fn enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn theme(&self) -> &str {
        self.theme
            .as_deref()
            .filter(|theme| !theme.is_empty())
            .unwrap_or(DEFAULT_THEME)
    }

    /// Whether `name` has been muted.
    pub fn is_disabled(&self, name: &str) -> bool {
        self.disabled.iter().any(|muted| muted == name)
    }

    /// Load the first config file that exists. No file at all is not an error -- the defaults are
    /// the IRIX soundscheme, on.
    pub fn load() -> Self {
        for path in crate::xdg::config_paths() {
            let text = match std::fs::read_to_string(&path) {
                Ok(text) => text,
                // Not-found is the ordinary case; only real errors are worth a line.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(err) => {
                    eprintln!("wlrix-sound: could not read {}: {err}", path.display());
                    continue;
                }
            };
            match toml::from_str::<Self>(&text) {
                Ok(config) => return config,
                Err(err) => {
                    eprintln!("wlrix-sound: {} is not valid: {err}", path.display());
                    return Self::default();
                }
            }
        }
        Self::default()
    }
}

/// Parse a candidate config file, for `--check-config`.
///
/// `wlrix-settings-daemon` writes a temporary file and runs this against it before renaming it
/// into place. Deliberately not [`Config::load`], which reports and carries on with defaults.
pub fn check(path: &std::path::Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("could not read {}: {err}", path.display()))?;
    toml::from_str::<Config>(&text)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Config {
        toml::from_str(text).expect("the test config should parse")
    }

    #[test]
    fn an_empty_file_is_all_defaults() {
        let config = config("");
        assert!(config.enabled());
        assert_eq!(config.theme(), DEFAULT_THEME);
        assert!(!config.is_disabled("wlrix-launch"));
    }

    #[test]
    fn a_full_file_is_read() {
        let config = config(
            "enabled = false\n\
             theme = \"freedesktop\"\n\
             disabled = [\"bell\"]\n",
        );
        assert!(!config.enabled());
        assert_eq!(config.theme(), "freedesktop");
        assert!(config.is_disabled("bell"));
        assert!(!config.is_disabled("wlrix-launch"));
    }

    #[test]
    fn an_empty_theme_is_the_default() {
        assert_eq!(config("theme = \"\"\n").theme(), DEFAULT_THEME);
    }

    #[test]
    fn a_typo_is_refused_rather_than_ignored() {
        assert!(toml::from_str::<Config>("enable = false\n").is_err());
    }
}
