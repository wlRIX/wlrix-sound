// SPDX-License-Identifier: GPL-3.0-or-later
//! The sounds wlRIX plays, and the groups they are switched on and off in.
//!
//! IRIX's System & Desktop Sounds panel did not list sounds one by one. It had a checkbox per
//! kind of sound -- *Startup & Shutdown Tunes*, *Desktop Sounds*, *System Alerts Sounds*,
//! *Keyboard Bell* -- and `sounds.toml`'s `[play]` table is those checkboxes, plus *Window
//! Sounds*, which IRIX did not have because it played none. Every sound belongs to exactly one
//! [`Category`], and plays only while its category is on.
//!
//! [`EVENTS`] is every sound some part of wlRIX asks for, with a label, for
//! `com.wlrix.Sound1.Events`. A sound asked for by another program over the bus is not listed;
//! [`category_of`] places it by its name instead.
//!
//! **Keep this in step with the callers.** A sound added to a component and not listed here
//! still plays, in whatever category its name suggests, but a panel cannot show it.

/// A checkbox on the sound panel, and the sounds it switches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Logging in, logging out and locking. Off by default: the sounds are the Indy's power-on
    /// and power-off tunes, which an IRIX desktop never played at login.
    Startup,
    /// Icons, folders, launching, drag-and-drop, the dumpster, disks. IRIX's soundscheme proper.
    Desktop,
    /// Windows opening, closing, minimizing and restoring, and changing desks. A wlRIX addition,
    /// off by default: IRIX 6.5 played none of these.
    Windows,
    /// Dialogs and other things that want attention.
    Alerts,
    /// The bell: a refused action, or a terminal's `^G`.
    Bell,
}

impl Category {
    pub const ALL: [Category; 5] = [
        Category::Startup,
        Category::Desktop,
        Category::Windows,
        Category::Alerts,
        Category::Bell,
    ];

    /// The name `sounds.toml`'s `[play]` table and the bus use for it.
    pub fn id(self) -> &'static str {
        match self {
            Category::Startup => "startup",
            Category::Desktop => "desktop",
            Category::Windows => "windows",
            Category::Alerts => "alerts",
            Category::Bell => "bell",
        }
    }
}

/// `(name, label, category)`.
pub type Event = (&'static str, &'static str, Category);

pub const EVENTS: &[Event] = &[
    // From the session and the compositor's lock.
    ("desktop-login", "Log in", Category::Startup),
    ("desktop-logout", "Log out", Category::Startup),
    ("desktop-screen-lock", "Lock the screen", Category::Startup),
    // From the compositor.
    ("window-new", "Open a window", Category::Windows),
    ("window-close", "Close a window", Category::Windows),
    ("window-minimized", "Minimize a window", Category::Windows),
    ("window-unminimized", "Restore a window", Category::Windows),
    (
        "desktop-switch-left",
        "Switch desks left",
        Category::Windows,
    ),
    (
        "desktop-switch-right",
        "Switch desks right",
        Category::Windows,
    ),
    ("bell", "Refused action", Category::Bell),
    // From the desktop, the file manager and the Toolchest.
    ("wlrix-launch", "Launch an application", Category::Desktop),
    ("wlrix-open-folder", "Open a folder", Category::Desktop),
    (
        "wlrix-open-remote-folder",
        "Open a remote folder",
        Category::Desktop,
    ),
    (
        "wlrix-icon-shift",
        "Shift an icon's position",
        Category::Desktop,
    ),
    (
        "wlrix-drag-move",
        "Move by drag-and-drop",
        Category::Desktop,
    ),
    (
        "wlrix-drag-copy",
        "Copy by drag-and-drop",
        Category::Desktop,
    ),
    ("drag-fail", "Refuse a drop", Category::Desktop),
    ("file-trash", "Move to the dumpster", Category::Desktop),
    ("item-deleted", "Remove permanently", Category::Desktop),
    (
        "complete-copy",
        "Finish a long copy or move",
        Category::Desktop,
    ),
    ("wlrix-volume-mount", "Mount a volume", Category::Desktop),
    ("device-added", "Device plugged in", Category::Desktop),
    ("device-removed", "Device removed", Category::Desktop),
    // Dialogs: Wlrix.Avalonia.Dialogs' MessageDialog, as it opens, and any other program.
    ("dialog-error", "Error", Category::Alerts),
    ("dialog-error-serious", "Fatal error", Category::Alerts),
    ("dialog-warning", "Warning", Category::Alerts),
    ("dialog-question", "Question", Category::Alerts),
    ("dialog-information", "Information", Category::Alerts),
];

/// Which checkbox `name` answers to.
///
/// A listed sound has its category. Anything else is placed by the naming spec's own groupings,
/// so a program playing `battery-low` or `bell-terminal` is switched with its kind; what is left
/// is a desktop sound.
pub fn category_of(name: &str) -> Category {
    if let Some((_, _, category)) = EVENTS.iter().find(|(listed, _, _)| *listed == name) {
        return *category;
    }
    let starts = |prefixes: &[&str]| prefixes.iter().any(|prefix| name.starts_with(prefix));
    if starts(&["bell"]) {
        Category::Bell
    } else if starts(&["window-", "desktop-switch-"]) {
        Category::Windows
    } else if starts(&[
        "desktop-log",
        "desktop-screen-lock",
        "system-",
        "service-log",
    ]) {
        Category::Startup
    } else if starts(&[
        "dialog-",
        "battery-",
        "power-",
        "suspend-",
        "network-",
        "message-",
        "phone-",
        "alarm-",
        "software-update-",
        "completion-fail",
    ]) {
        Category::Alerts
    } else {
        Category::Desktop
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_valid_and_listed_once() {
        for (index, (name, _, _)) in EVENTS.iter().enumerate() {
            assert!(crate::theme::valid_name(name), "{name}");
            assert!(
                !EVENTS[..index].iter().any(|(other, _, _)| other == name),
                "{name} is listed twice"
            );
        }
    }

    #[test]
    fn a_listed_name_and_its_kind_agree() {
        // The rules are the fallback for what is not listed; a listed sound the rules would put
        // somewhere else is a sign one or the other is wrong.
        for (name, _, category) in EVENTS {
            let unlisted = match *name {
                // IRIX's own sounds have no prefix the spec knows.
                name if name.starts_with("wlrix-") => Category::Desktop,
                name => category_of(name),
            };
            assert_eq!(unlisted, *category, "{name}");
        }
    }

    #[test]
    fn an_unlisted_name_is_placed_by_its_kind() {
        assert_eq!(category_of("bell-terminal"), Category::Bell);
        assert_eq!(category_of("window-attention-active"), Category::Windows);
        assert_eq!(category_of("system-bootup"), Category::Startup);
        assert_eq!(category_of("battery-low"), Category::Alerts);
        assert_eq!(category_of("camera-shutter"), Category::Desktop);
    }
}
