// SPDX-License-Identifier: GPL-3.0-or-later
//! wlRIX's system sounds.
//!
//! A daemon with no window. Programs ask it for a sound by name -- `dialog-error`,
//! `wlrix-launch` -- and it decides whether that sound plays (`sounds.toml`: the theme, a master
//! switch, the muted list), finds it in the XDG sound theme, and plays it through PipeWire.
//! Keeping that decision in one place is the point: a component never reads the sound settings,
//! and a settings panel changes one file to change every sound on the desktop.
//!
//! [`daemon`] is the event loop. [`theme`] (finding a sound by name), [`decode`] and [`config`]
//! are testable without a session; [`player`] is the PipeWire side. [`client`] is what the other
//! components copy in to ask for a sound, and [`dbus`] is the interface everything else uses.

pub mod catalog;
pub mod client;
pub mod config;
pub mod daemon;
pub mod dbus;
pub mod decode;
pub mod pidfile;
pub mod player;
pub mod signals;
pub mod theme;
pub mod xdg;
