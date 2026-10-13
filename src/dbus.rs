// SPDX-License-Identifier: GPL-3.0-or-later
//! `com.wlrix.Sound`: system sounds, for programs on the session bus.
//!
//! ```text
//! bus name   com.wlrix.Sound
//! object     /com/wlrix/Sound
//! interface  com.wlrix.Sound1
//!
//! Play(s name)                  play a sound, if sounds.toml allows it
//! Preview(s theme, s name)      play a sound from a theme ("" for the configured one), whatever
//!                               sounds.toml says -- for a settings panel's "test" button
//! Events() -> a(sss)            every sound the desktop plays: name, label, category -- one
//!                               of startup, desktop, windows, alerts, bell, sounds.toml's
//!                               [play] keys
//! ListThemes() -> a(ss)         every installed sound theme: id, display name
//! ```
//!
//! wlRIX's own components do not use this: they send to the daemon's datagram socket instead
//! (see [`crate::client`]), which never blocks and needs no bus connection. This is the public
//! face, for a settings panel and for any other program that wants the desktop's sounds.
//!
//! Which theme is in use and what is muted are not here. They are settings, read and written
//! through `com.wlrix.Settings` like every other, and this daemon follows the file.
//!
//! # Threads
//!
//! Calls are answered on zbus's own thread. Anything that plays is handed to the main loop as a
//! [`Request`], since PipeWire lives there; the two lists are answered on the spot.

use zbus::fdo;

pub const BUS_NAME: &str = "com.wlrix.Sound";
pub const OBJECT_PATH: &str = "/com/wlrix/Sound";

/// What the bus asks the main loop to do.
#[derive(Debug)]
pub enum Request {
    Play(String),
    /// Theme (`None` for the configured one) and name.
    Preview(Option<String>, String),
}

struct Service {
    sender: calloop::channel::Sender<Request>,
}

impl Service {
    fn send(&self, request: Request) -> fdo::Result<()> {
        self.sender
            .send(request)
            .map_err(|_| fdo::Error::Failed("wlrix-sound is shutting down".to_owned()))
    }
}

fn checked(name: &str, what: &str) -> fdo::Result<()> {
    if crate::theme::valid_name(name) {
        Ok(())
    } else {
        Err(fdo::Error::InvalidArgs(format!(
            "{name:?} is not a {what} name"
        )))
    }
}

#[zbus::interface(name = "com.wlrix.Sound1")]
impl Service {
    fn play(&self, name: String) -> fdo::Result<()> {
        checked(&name, "sound")?;
        self.send(Request::Play(name))
    }

    fn preview(&self, theme: String, name: String) -> fdo::Result<()> {
        checked(&name, "sound")?;
        let theme = if theme.is_empty() {
            None
        } else {
            checked(&theme, "theme")?;
            Some(theme)
        };
        self.send(Request::Preview(theme, name))
    }

    #[zbus(out_args("events"))]
    fn events(&self) -> Vec<(String, String, String)> {
        crate::catalog::EVENTS
            .iter()
            .map(|(name, label, category)| {
                (
                    (*name).to_owned(),
                    (*label).to_owned(),
                    category.id().to_owned(),
                )
            })
            .collect()
    }

    #[zbus(out_args("themes"))]
    fn list_themes(&self) -> Vec<(String, String)> {
        crate::theme::installed(&crate::xdg::sound_dirs())
            .into_iter()
            .map(|theme| (theme.id, theme.name))
            .collect()
    }
}

/// Serve on the session bus.
///
/// `Err` when the name is already owned, which means another copy of this program is running.
pub fn start(
    sender: calloop::channel::Sender<Request>,
) -> Result<zbus::blocking::Connection, StartError> {
    let connection = zbus::blocking::connection::Builder::session()
        .map_err(|err| StartError::NoBus(err.to_string()))?
        // Served before the name is requested, so a caller that calls the instant the name
        // appears never finds an empty connection behind it.
        .serve_at(OBJECT_PATH, Service { sender })
        .map_err(|err| StartError::NoBus(format!("could not serve {OBJECT_PATH}: {err}")))?
        .build()
        .map_err(|err| StartError::NoBus(err.to_string()))?;
    let reply = connection
        .request_name_with_flags(BUS_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .map_err(|err| match err {
            // zbus reports `DoNotQueue`'s refusal as an error rather than a reply.
            zbus::Error::NameTaken => StartError::Taken,
            err => StartError::NoBus(format!("could not request {BUS_NAME}: {err}")),
        })?;
    if !matches!(reply, zbus::fdo::RequestNameReply::PrimaryOwner) {
        return Err(StartError::Taken);
    }
    Ok(connection)
}

/// Why the bus could not be served.
pub enum StartError {
    /// Another `wlrix-sound` has the name. Fatal: two daemons would play every sound twice.
    Taken,
    /// No usable session bus. Not fatal -- the components' socket still works.
    NoBus(String),
}
