// SPDX-License-Identifier: GPL-3.0-or-later
//! The sounds wlRIX plays, for a settings panel to list.
//!
//! Each entry is a sound name some part of the desktop asks for, the label to show beside its
//! switch, and the group it belongs under. A panel builds its list of switches from this over
//! `com.wlrix.Sound1.Events` rather than keeping its own, so a sound added to a component and
//! here appears in the panel with no change to the panel.
//!
//! **Keep this in step with the callers.** Adding a sound to a component without listing it here
//! leaves it playing with no way to turn it off but the master switch.

/// `(name, label, category)`.
pub type Event = (&'static str, &'static str, &'static str);

pub const EVENTS: &[Event] = &[
    // The session.
    ("desktop-login", "Log in", "Session"),
    ("desktop-logout", "Log out", "Session"),
    ("desktop-screen-lock", "Lock the screen", "Session"),
    // Windows, from the compositor.
    ("window-new", "Open a window", "Windows"),
    ("window-close", "Close a window", "Windows"),
    ("window-minimized", "Minimize a window", "Windows"),
    ("window-unminimized", "Restore a window", "Windows"),
    ("desktop-switch-left", "Switch desks left", "Windows"),
    ("desktop-switch-right", "Switch desks right", "Windows"),
    ("bell", "Refused action", "Windows"),
    // Icons and files, from the desktop and the file manager.
    ("wlrix-launch", "Launch an application", "Icons"),
    ("wlrix-open-folder", "Open a folder", "Icons"),
    ("wlrix-open-remote-folder", "Open a remote folder", "Icons"),
    ("wlrix-icon-shift", "Shift an icon's position", "Icons"),
    ("wlrix-drag-move", "Move by drag-and-drop", "Icons"),
    ("wlrix-drag-copy", "Copy by drag-and-drop", "Icons"),
    ("drag-fail", "Refuse a drop", "Icons"),
    ("file-trash", "Move to the dumpster", "Icons"),
    ("item-deleted", "Remove permanently", "Icons"),
    ("complete-copy", "Finish a long copy or move", "Icons"),
    // Devices, from the file manager.
    ("wlrix-volume-mount", "Mount a volume", "Devices"),
    ("device-added", "Device plugged in", "Devices"),
    ("device-removed", "Device removed", "Devices"),
];

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
}
