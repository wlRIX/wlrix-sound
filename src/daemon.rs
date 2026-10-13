// SPDX-License-Identifier: GPL-3.0-or-later
//! The daemon: taking requests, deciding whether they play, and playing them.
//!
//! Requests arrive two ways -- datagrams on `$XDG_RUNTIME_DIR/wlrix-sound.sock` from wlRIX's own
//! components, and calls on `com.wlrix.Sound1` from anything else -- and end up in the same
//! place, [`Daemon::play`]. There the request is checked against `sounds.toml` (the master
//! switch, the muted list), dropped if the same sound started a moment ago, looked up in the
//! theme, decoded (once; decoded sounds are kept), and handed to PipeWire.
//!
//! A `play-wait` request is answered with `done` when its sound has finished -- or at once, when
//! it is not going to play at all.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction};

use crate::config::Config;
use crate::dbus::{self, Request};
use crate::decode::{self, Pcm};
use crate::player::Player;
use crate::theme::{self, Found, Lookup};

/// A sound asked for again within this long of starting is dropped. Twelve icons dropped at
/// once are one *shift*, not twelve on top of each other.
const REPEAT_WINDOW: Duration = Duration::from_millis(60);

/// The longest request accepted on the socket. A name is at most 128 bytes.
const MAX_REQUEST: usize = 256;

pub struct Daemon {
    config: Config,
    lookup: Lookup,
    /// Decoded sounds by file, so each is read once. Emptied on a reload, which may have changed
    /// the theme or found the files replaced.
    decoded: HashMap<PathBuf, Rc<Pcm>>,
    /// Names already reported as missing or broken, so each is said once per reload rather than
    /// once per click.
    reported: Vec<String>,
    player: Player,
    socket: UnixDatagram,
    /// Who to tell when each playing sound finishes.
    waiters: HashMap<u64, Vec<SocketAddr>>,
    /// The last sound started, and when, for [`REPEAT_WINDOW`].
    last: Option<(String, Instant)>,
    handle: LoopHandle<'static, Daemon>,
    exit: bool,
}

/// Run until asked to stop.
pub fn run() -> Result<(), String> {
    let (sender, requests) = calloop::channel::channel();
    // The bus first: a second copy of this program fails here, before it touches the socket the
    // first one is listening on.
    let _bus = match dbus::start(sender) {
        Ok(bus) => Some(bus),
        Err(dbus::StartError::Taken) => {
            return Err(format!(
                "{} is already owned -- another wlrix-sound is running",
                dbus::BUS_NAME
            ));
        }
        Err(dbus::StartError::NoBus(why)) => {
            eprintln!("wlrix-sound: no session bus ({why}); serving the socket only");
            None
        }
    };

    let socket_path = crate::xdg::runtime_dir()
        .ok_or("XDG_RUNTIME_DIR is not set")?
        .join(crate::client::SOCKET_NAME);
    let socket = bind(&socket_path)?;
    let _socket_guard = RemoveOnDrop(socket_path);
    let _pidfile = crate::pidfile::write();

    let mut event_loop: EventLoop<'static, Daemon> =
        EventLoop::try_new().map_err(|err| format!("could not create the event loop: {err}"))?;
    let handle = event_loop.handle();

    let config = Config::load();
    let lookup = new_lookup(config.theme());
    let player = Player::new()?;
    let pipewire_fd = player.loop_fd()?;

    let mut daemon = Daemon {
        config,
        lookup,
        decoded: HashMap::new(),
        reported: Vec::new(),
        player,
        socket: socket
            .try_clone()
            .map_err(|err| format!("could not share the socket: {err}"))?,
        waiters: HashMap::new(),
        last: None,
        handle: handle.clone(),
        exit: false,
    };

    handle
        .insert_source(
            Generic::new(socket, Interest::READ, Mode::Level),
            |_, _, daemon: &mut Daemon| {
                daemon.on_socket();
                Ok(PostAction::Continue)
            },
        )
        .map_err(|err| format!("could not listen on the socket: {err}"))?;
    handle
        .insert_source(requests, |event, _, daemon: &mut Daemon| {
            if let calloop::channel::Event::Msg(request) = event {
                match request {
                    Request::Play(name) => daemon.play(&name, None, None),
                    Request::Preview(theme, name) => {
                        daemon.play(&name, Some(theme.as_deref()), None)
                    }
                }
            }
        })
        .map_err(|err| format!("could not listen to the bus: {err}"))?;
    // PipeWire's loop, run whenever it has something. `iterate` both reads the fd and runs the
    // callbacks, so calloop only has to say "there is something there".
    handle
        .insert_source(
            Generic::new(pipewire_fd, Interest::READ, Mode::Level),
            |_, _, daemon: &mut Daemon| {
                daemon.player.iterate();
                daemon.reap();
                Ok(PostAction::Continue)
            },
        )
        .map_err(|err| format!("could not watch the PipeWire loop: {err}"))?;

    let (reload_ping, reload_source) = calloop::ping::make_ping()
        .map_err(|err| format!("could not create the reload ping: {err}"))?;
    handle
        .insert_source(reload_source, |_, _, daemon: &mut Daemon| daemon.reload())
        .map_err(|err| format!("could not watch for reloads: {err}"))?;
    crate::signals::forward_reload_to_loop(reload_ping);

    let (quit_ping, quit_source) = calloop::ping::make_ping()
        .map_err(|err| format!("could not create the quit ping: {err}"))?;
    handle
        .insert_source(quit_source, |_, _, daemon: &mut Daemon| daemon.exit = true)
        .map_err(|err| format!("could not watch for a stop: {err}"))?;
    crate::signals::forward_to_loop(quit_ping);

    while !daemon.exit {
        event_loop
            .dispatch(None, &mut daemon)
            .map_err(|err| format!("the event loop failed: {err}"))?;
    }
    Ok(())
}

/// Bind the request socket at `path`, replacing a stale one.
///
/// A socket file left by a daemon that was killed -- `wlrix-session` stops its apps with
/// `SIGKILL` -- refuses connections, and is removed. One that accepts is a live daemon, which
/// should not happen once the bus name has been taken, but would mean two daemons playing
/// everything twice if it did.
fn bind(path: &Path) -> Result<UnixDatagram, String> {
    if path.exists() {
        let live = UnixDatagram::unbound()
            .and_then(|probe| probe.connect(path))
            .is_ok();
        if live {
            return Err(format!(
                "{} is answering -- another wlrix-sound is running",
                path.display()
            ));
        }
        std::fs::remove_file(path)
            .map_err(|err| format!("could not remove the stale {}: {err}", path.display()))?;
    }
    let socket = UnixDatagram::bind(path)
        .map_err(|err| format!("could not bind {}: {err}", path.display()))?;
    socket
        .set_nonblocking(true)
        .map_err(|err| format!("could not make the socket non-blocking: {err}"))?;
    Ok(socket)
}

/// A lookup for `theme` over the whole data path, saying so when the theme is not installed.
fn new_lookup(theme: &str) -> Lookup {
    let lookup = Lookup::new(theme, crate::xdg::sound_dirs());
    if lookup.chain().next() != Some(theme) {
        eprintln!(
            "wlrix-sound: no sound theme {theme:?} is installed; using {}",
            theme::FALLBACK_THEME
        );
    }
    lookup
}

/// Removes the socket file on a clean exit.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl Daemon {
    /// Read every waiting request off the socket.
    fn on_socket(&mut self) {
        let mut buffer = [0_u8; MAX_REQUEST];
        loop {
            let (length, from) = match self.socket.recv_from(&mut buffer) {
                Ok(received) => received,
                Err(err) if err.kind() == ErrorKind::WouldBlock => return,
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) => {
                    eprintln!("wlrix-sound: could not read a request: {err}");
                    return;
                }
            };
            let Ok(text) = std::str::from_utf8(&buffer[..length]) else {
                continue;
            };
            match parse(text) {
                Some((name, wait)) => {
                    let reply_to = wait.then_some(from);
                    self.play(name, None, reply_to);
                }
                None => {
                    // Answered anyway, so a caller waiting on a typo is not left waiting.
                    if text.starts_with("play-wait ") {
                        self.reply(&from);
                    }
                }
            }
        }
    }

    /// Play `name`.
    ///
    /// `preview` is `Some` for a settings panel's test button: it plays from that theme (or the
    /// configured one, for `Some(None)`) and ignores the switches in `sounds.toml`, since the
    /// point of pressing the button is to hear the sound.
    fn play(&mut self, name: &str, preview: Option<Option<&str>>, reply_to: Option<SocketAddr>) {
        let started = self.start(name, preview);
        match (started, reply_to) {
            (Some(id), Some(address)) => self.waiters.entry(id).or_default().push(address),
            (None, Some(address)) => self.reply(&address),
            _ => {}
        }
    }

    /// Start `name` playing, returning its voice, or `None` when it is not going to play.
    fn start(&mut self, name: &str, preview: Option<Option<&str>>) -> Option<u64> {
        if !theme::valid_name(name) {
            return None;
        }
        if preview.is_none() && (!self.config.enabled() || self.config.is_disabled(name)) {
            return None;
        }
        let now = Instant::now();
        if self
            .last
            .as_ref()
            .is_some_and(|(last, at)| last == name && now.duration_since(*at) < REPEAT_WINDOW)
        {
            return None;
        }

        let found = match preview {
            Some(Some(theme)) if theme != self.config.theme() => {
                Lookup::new(theme, crate::xdg::sound_dirs()).find(name)
            }
            _ => self.lookup.find(name),
        };
        let path = match found {
            Found::File(path) => path,
            Found::Disabled => return None,
            Found::Missing => {
                self.report(name, || {
                    format!("no sound for {name:?} in any theme -- nothing plays for it")
                });
                return None;
            }
        };
        let pcm = match self.decoded.get(&path) {
            Some(pcm) => Rc::clone(pcm),
            None => match decode::decode(&path) {
                Ok(pcm) => {
                    let pcm = Rc::new(pcm);
                    self.decoded.insert(path.clone(), Rc::clone(&pcm));
                    pcm
                }
                Err(err) => {
                    self.report(name, || format!("could not decode {err}"));
                    return None;
                }
            },
        };

        match self.player.play(name, pcm, self.config.output()) {
            Ok(id) => {
                self.last = Some((name.to_owned(), now));
                self.arm_deadline();
                Some(id)
            }
            Err(err) => {
                // Usually "PipeWire is not running", which would otherwise be said per click.
                self.report(&format!("{name}\0player"), || format!("{name}: {err}"));
                None
            }
        }
    }

    /// Say something about `key` once per reload.
    fn report(&mut self, key: &str, message: impl FnOnce() -> String) {
        if !self.reported.iter().any(|reported| reported == key) {
            eprintln!("wlrix-sound: {}", message());
            self.reported.push(key.to_owned());
        }
    }

    /// Make sure the loop wakes when the next sound runs out of time to finish. A sound whose
    /// stream never drains -- there is no sink, say -- would otherwise hold its voice and its
    /// waiters until PipeWire next had something to say.
    fn arm_deadline(&mut self) {
        let Some(deadline) = self.player.next_deadline() else {
            return;
        };
        let _ = self.handle.insert_source(
            Timer::from_deadline(deadline),
            |_, _, daemon: &mut Daemon| {
                daemon.reap();
                TimeoutAction::Drop
            },
        );
    }

    /// Let go of finished sounds and answer whoever was waiting on them.
    fn reap(&mut self) {
        for id in self.player.reap() {
            for address in self.waiters.remove(&id).unwrap_or_default() {
                self.reply(&address);
            }
        }
    }

    fn reply(&self, address: &SocketAddr) {
        // An unnamed sender cannot be answered, and asked for no answer.
        if address.is_unnamed() {
            return;
        }
        if let Err(err) = self.socket.send_to_addr(b"done", address) {
            // Gone already is the usual reason, and the caller's own business.
            if err.kind() != ErrorKind::ConnectionRefused {
                eprintln!("wlrix-sound: could not answer a request: {err}");
            }
        }
    }

    /// Re-read `sounds.toml` and start the theme lookup afresh: the theme may have changed, or
    /// been installed since.
    fn reload(&mut self) {
        self.config = Config::load();
        self.lookup = new_lookup(self.config.theme());
        self.decoded.clear();
        self.reported.clear();
        eprintln!(
            "wlrix-sound: reloaded: theme {}, sounds {}",
            self.config.theme(),
            if self.config.enabled() { "on" } else { "off" }
        );
    }
}

/// Split a socket request into the sound name and whether a reply is wanted.
fn parse(text: &str) -> Option<(&str, bool)> {
    let text = text.trim_end_matches(['\n', '\r']);
    let (verb, name) = text.split_once(' ')?;
    let wait = match verb {
        "play" => false,
        "play-wait" => true,
        _ => return None,
    };
    theme::valid_name(name).then_some((name, wait))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_parsed() {
        assert_eq!(parse("play bell"), Some(("bell", false)));
        assert_eq!(
            parse("play-wait desktop-logout\n"),
            Some(("desktop-logout", true))
        );
        assert_eq!(parse("play ../../etc/passwd"), None);
        assert_eq!(parse("stop bell"), None);
        assert_eq!(parse("play"), None);
        assert_eq!(parse("play "), None);
    }

    #[test]
    fn a_live_socket_is_not_replaced_and_a_stale_one_is() {
        let path =
            std::env::temp_dir().join(format!("wlrix-sound-test-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let first = bind(&path).expect("a fresh path binds");
        assert!(bind(&path).is_err(), "a live socket must not be taken over");
        drop(first);
        // The file is still there, but nobody answers it.
        assert!(path.exists());
        let second = bind(&path).expect("a stale socket is replaced");
        drop(second);
        std::fs::remove_file(&path).unwrap();
    }
}
