// SPDX-License-Identifier: GPL-3.0-or-later
//! Finding a sound by name, the way the XDG sound theme spec says to.
//!
//! A theme is a directory under `sounds/` on the data path, made a theme by its `index.theme`:
//!
//! ```ini
//! [Sound Theme]
//! Name=wlRIX
//! Inherits=freedesktop
//! Directories=stereo
//!
//! [stereo]
//! OutputProfile=stereo
//! ```
//!
//! A name is looked for in the theme's directories, then each theme it inherits from, then the
//! stock `freedesktop` theme -- in every `sounds` directory on the data path at each step, so a
//! file in `~/.local/share/sounds/wlrix/stereo/` replaces one sound of an installed theme
//! without copying the rest. When the whole chain has nothing, the last dash-separated part of
//! the name is dropped and the search starts again: `dialog-error-serious` falls back to
//! `dialog-error`, then `dialog`.
//!
//! A file named `<name>.disabled` is the theme saying "this one is silent on purpose", and ends
//! the search with nothing to play rather than falling through to a parent.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The theme every chain ends in, per the spec.
pub const FALLBACK_THEME: &str = "freedesktop";

/// The output profile wlRIX plays through. The spec has others (`5.1`, ...) for themes that
/// ship surround versions; nothing here would know which to pick, and stereo is what every
/// theme has.
const PROFILE: &str = "stereo";

/// The extensions a sound may have, in the order the spec checks them. `.disabled` first, so a
/// theme can silence a name its parent has a file for.
const EXTENSIONS: &[&str] = &["disabled", "oga", "ogg", "wav"];

/// How deep `Inherits` is followed. A theme inheriting from itself, directly or round a loop, is
/// a mistake in someone's `index.theme`; it should cost a missing sound, not a hung daemon.
const MAX_DEPTH: usize = 16;

/// What looking a name up came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// A file to play.
    File(PathBuf),
    /// The theme marks this sound as silent.
    Disabled,
    /// No theme in the chain has anything by this name or any shortening of it.
    Missing,
}

/// Whether `name` is something a sound can be called.
///
/// Names arrive from any program on the session, and are joined onto a directory: one with a
/// `/` or a `..` in it would play whatever file it pointed at. The naming spec's names are
/// lowercase words and dashes; wlRIX allows the same plus dots and underscores, which some
/// themes use, and nothing else.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with(['.', '-'])
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        && !name.contains("..")
}

/// A theme's `index.theme`, as far as lookup needs it.
#[derive(Debug, Default)]
struct Index {
    name: Option<String>,
    inherits: Vec<String>,
    /// The subdirectories whose output profile is stereo.
    directories: Vec<String>,
}

impl Index {
    fn parse(text: &str) -> Self {
        let mut index = Index::default();
        let mut section = String::new();
        // Each directory group's profile, gathered first: the groups come after `Directories`
        // names them, and a group may be missing entirely.
        let mut listed = Vec::new();
        let mut profiles: HashMap<String, String> = HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line
                .strip_prefix('[')
                .and_then(|line| line.strip_suffix(']'))
            {
                section = name.to_owned();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let (key, value) = (key.trim(), value.trim());
            if section == "Sound Theme" {
                match key {
                    "Name" => index.name = Some(value.to_owned()),
                    "Inherits" => index.inherits = list(value),
                    "Directories" => listed = list(value),
                    _ => {}
                }
            } else if key == "OutputProfile" {
                profiles.insert(section.clone(), value.to_owned());
            }
        }
        // A directory with no profile is taken to be stereo, the spec's default.
        index.directories = listed
            .into_iter()
            .filter(|dir| profiles.get(dir).is_none_or(|profile| profile == PROFILE))
            .collect();
        index
    }
}

/// A comma-separated `index.theme` value. Some themes write `Inherits` with spaces after the
/// commas, and the spec does not say they may not.
fn list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A theme found on the data path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The directory name, which is what `sounds.toml` and `gtk-sound-theme-name` call it by.
    pub id: String,
    /// What `index.theme` calls it, for a settings panel to show.
    pub name: String,
}

/// Looks sounds up in one theme and its parents, remembering what it found.
pub struct Lookup {
    /// Every `sounds` directory, most specific first.
    roots: Vec<PathBuf>,
    /// The themes to search, in order: the chosen one, what it inherits, then `freedesktop`.
    /// Each with the directories to look in, gathered from every root that has the theme.
    chain: Vec<(String, Vec<String>)>,
    cache: HashMap<String, Found>,
}

impl Lookup {
    /// Prepare to look sounds up in `theme`, searching `roots`.
    pub fn new(theme: &str, roots: Vec<PathBuf>) -> Self {
        let mut lookup = Lookup {
            roots,
            chain: Vec::new(),
            cache: HashMap::new(),
        };
        lookup.add(theme, 0);
        if !lookup.chain.iter().any(|(id, _)| id == FALLBACK_THEME) {
            lookup.add(FALLBACK_THEME, 0);
        }
        lookup
    }

    /// The themes this lookup searches, in order.
    pub fn chain(&self) -> impl Iterator<Item = &str> {
        self.chain.iter().map(|(id, _)| id.as_str())
    }

    fn add(&mut self, theme: &str, depth: usize) {
        if depth > MAX_DEPTH || !valid_name(theme) || self.chain.iter().any(|(id, _)| id == theme) {
            return;
        }
        // The first `index.theme` found describes the theme; a user's copy of one overrides the
        // system's, the same rule icon themes follow.
        let Some(index) = self
            .roots
            .iter()
            .find_map(|root| std::fs::read_to_string(root.join(theme).join("index.theme")).ok())
            .map(|text| Index::parse(&text))
        else {
            // Not installed. Not worth a line per sound -- the caller says so once.
            return;
        };
        self.chain.push((theme.to_owned(), index.directories));
        for parent in index.inherits {
            self.add(&parent, depth + 1);
        }
    }

    /// Find the file for `name`.
    pub fn find(&mut self, name: &str) -> Found {
        if let Some(found) = self.cache.get(name) {
            return found.clone();
        }
        let found = self.search(name);
        self.cache.insert(name.to_owned(), found.clone());
        found
    }

    fn search(&self, name: &str) -> Found {
        if !valid_name(name) {
            return Found::Missing;
        }
        let mut candidate = name;
        loop {
            if let Some(found) = self.search_exact(candidate) {
                return found;
            }
            match candidate.rsplit_once('-') {
                Some((shorter, _)) if !shorter.is_empty() => candidate = shorter,
                _ => return Found::Missing,
            }
        }
    }

    fn search_exact(&self, name: &str) -> Option<Found> {
        for (theme, directories) in &self.chain {
            for directory in directories {
                for root in &self.roots {
                    let dir = root.join(theme).join(directory);
                    if let Some(found) = in_dir(&dir, name) {
                        return Some(found);
                    }
                }
            }
        }
        None
    }
}

/// Look for `name` with each extension in one directory.
fn in_dir(dir: &Path, name: &str) -> Option<Found> {
    for extension in EXTENSIONS {
        let path = dir.join(format!("{name}.{extension}"));
        // `exists` follows links, which is the point: most of a theme's names are links to a
        // handful of files, and a dangling one is as good as absent.
        if path.exists() {
            return Some(if *extension == "disabled" {
                Found::Disabled
            } else {
                Found::File(path)
            });
        }
    }
    None
}

/// Every theme installed under `roots`, each once, sorted by display name.
pub fn installed(roots: &[PathBuf]) -> Vec<Installed> {
    let mut themes: Vec<Installed> = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let id = entry.file_name().to_string_lossy().into_owned();
            if !valid_name(&id) || themes.iter().any(|theme| theme.id == id) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path().join("index.theme")) else {
                continue;
            };
            let index = Index::parse(&text);
            // A theme with no stereo directory has nothing this daemon could play.
            if index.directories.is_empty() {
                continue;
            }
            themes.push(Installed {
                name: index.name.unwrap_or_else(|| id.clone()),
                id,
            });
        }
    }
    themes.sort_by_key(|theme| theme.name.to_lowercase());
    themes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A data path laid out in a temporary directory, removed when dropped.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("wlrix-sound-test-{label}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            Fixture { root }
        }

        fn sounds(&self, base: &str) -> PathBuf {
            self.root.join(base).join("sounds")
        }

        fn theme(&self, base: &str, id: &str, index: &str) {
            let dir = self.sounds(base).join(id);
            std::fs::create_dir_all(dir.join("stereo")).unwrap();
            std::fs::write(dir.join("index.theme"), index).unwrap();
        }

        fn file(&self, base: &str, id: &str, file: &str) -> PathBuf {
            let path = self.sounds(base).join(id).join("stereo").join(file);
            std::fs::write(&path, b"").unwrap();
            path
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    const STEREO: &str = "[Sound Theme]\nName=Test\nDirectories=stereo\n\n\
                          [stereo]\nOutputProfile=stereo\n";

    fn index(name: &str, inherits: &str) -> String {
        format!(
            "[Sound Theme]\nName={name}\nInherits={inherits}\nDirectories=stereo\n\n\
             [stereo]\nOutputProfile=stereo\n"
        )
    }

    #[test]
    fn a_name_is_found_in_the_theme() {
        let fixture = Fixture::new("found");
        fixture.theme("system", "wlrix", STEREO);
        let path = fixture.file("system", "wlrix", "bell.ogg");
        let mut lookup = Lookup::new("wlrix", vec![fixture.sounds("system")]);
        assert_eq!(lookup.find("bell"), Found::File(path));
    }

    #[test]
    fn a_missing_name_falls_through_to_the_parent_then_freedesktop() {
        let fixture = Fixture::new("inherit");
        fixture.theme("system", "child", &index("Child", "parent"));
        fixture.theme("system", "parent", STEREO);
        fixture.theme("system", "freedesktop", STEREO);
        let from_parent = fixture.file("system", "parent", "bell.oga");
        let from_fallback = fixture.file("system", "freedesktop", "camera-shutter.oga");
        let mut lookup = Lookup::new("child", vec![fixture.sounds("system")]);
        assert_eq!(
            lookup.chain().collect::<Vec<_>>(),
            ["child", "parent", "freedesktop"]
        );
        assert_eq!(lookup.find("bell"), Found::File(from_parent));
        assert_eq!(lookup.find("camera-shutter"), Found::File(from_fallback));
    }

    #[test]
    fn the_whole_chain_is_tried_before_a_name_is_shortened() {
        let fixture = Fixture::new("shorten");
        fixture.theme("system", "wlrix", STEREO);
        fixture.theme("system", "freedesktop", STEREO);
        let short = fixture.file("system", "wlrix", "dialog-error.ogg");
        let exact = fixture.file("system", "freedesktop", "dialog-error-serious.oga");
        let mut lookup = Lookup::new("wlrix", vec![fixture.sounds("system")]);
        assert_eq!(lookup.find("dialog-error-serious"), Found::File(exact));
        assert_eq!(lookup.find("dialog-error-unheard-of"), Found::File(short));
    }

    #[test]
    fn disabled_silences_rather_than_falling_through() {
        let fixture = Fixture::new("disabled");
        fixture.theme("system", "child", &index("Child", "parent"));
        fixture.theme("system", "parent", STEREO);
        fixture.file("system", "child", "bell.disabled");
        fixture.file("system", "parent", "bell.ogg");
        let mut lookup = Lookup::new("child", vec![fixture.sounds("system")]);
        assert_eq!(lookup.find("bell"), Found::Disabled);
    }

    #[test]
    fn a_users_file_replaces_one_sound_of_an_installed_theme() {
        let fixture = Fixture::new("override");
        fixture.theme("system", "wlrix", STEREO);
        fixture.file("system", "wlrix", "bell.ogg");
        let other = fixture.file("system", "wlrix", "drag-fail.ogg");
        // No index.theme in the user's copy: the system's describes the theme.
        std::fs::create_dir_all(fixture.sounds("user").join("wlrix/stereo")).unwrap();
        let mine = fixture.file("user", "wlrix", "bell.wav");
        let mut lookup = Lookup::new(
            "wlrix",
            vec![fixture.sounds("user"), fixture.sounds("system")],
        );
        assert_eq!(lookup.find("bell"), Found::File(mine));
        assert_eq!(lookup.find("drag-fail"), Found::File(other));
    }

    #[test]
    fn an_inheritance_loop_ends() {
        let fixture = Fixture::new("loop");
        fixture.theme("system", "a", &index("A", "b"));
        fixture.theme("system", "b", &index("B", "a"));
        let mut lookup = Lookup::new("a", vec![fixture.sounds("system")]);
        assert_eq!(lookup.chain().collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(lookup.find("bell"), Found::Missing);
    }

    #[test]
    fn directories_for_other_profiles_are_skipped() {
        let fixture = Fixture::new("profile");
        fixture.theme(
            "system",
            "surround",
            "[Sound Theme]\nName=S\nDirectories=stereo,5.1\n\n\
             [stereo]\nOutputProfile=stereo\n\n[5.1]\nOutputProfile=5.1\n",
        );
        std::fs::create_dir_all(fixture.sounds("system").join("surround/5.1")).unwrap();
        std::fs::write(fixture.sounds("system").join("surround/5.1/bell.ogg"), b"").unwrap();
        let mut lookup = Lookup::new("surround", vec![fixture.sounds("system")]);
        assert_eq!(lookup.find("bell"), Found::Missing);
    }

    #[test]
    fn a_path_is_not_a_name() {
        for name in ["../x", "a/b", "", ".hidden", "-x", "a..b", "a b"] {
            assert!(!valid_name(name), "{name:?} should be refused");
        }
        for name in ["bell", "dialog-error-serious", "wlrix-launch", "x_y.z"] {
            assert!(valid_name(name), "{name:?} should be allowed");
        }
    }

    #[test]
    fn installed_themes_are_listed_once_by_display_name() {
        let fixture = Fixture::new("installed");
        fixture.theme("user", "wlrix", &index("wlRIX", "freedesktop"));
        fixture.theme("system", "wlrix", &index("wlRIX (system)", "freedesktop"));
        fixture.theme("system", "freedesktop", &index("Default", ""));
        let themes = installed(&[fixture.sounds("user"), fixture.sounds("system")]);
        assert_eq!(
            themes,
            [
                Installed {
                    id: "freedesktop".into(),
                    name: "Default".into()
                },
                Installed {
                    id: "wlrix".into(),
                    name: "wlRIX".into()
                },
            ]
        );
    }
}
