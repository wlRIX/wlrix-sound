// SPDX-License-Identifier: GPL-3.0-or-later
//! wlRIX system sounds daemon.
//!
//! Plays the desktop's event sounds from an XDG sound theme. Started by `wlrix-session`. See
//! [`wlrix_sound::daemon`].

use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut rest = args.iter();
    if let Some(argument) = rest.next() {
        match argument.as_str() {
            "--help" | "-h" => {
                println!(
                    "wlrix-sound {}\n\n\
                     System sounds for the wlRIX desktop. Plays the sounds the desktop's\n\
                     components ask for, from an XDG sound theme, through PipeWire. Started by\n\
                     wlrix-session.\n\n\
                     Usage: wlrix-sound [options]\n\
                     \x20      wlrix-sound --play <name>\n\n\
                     Options:\n  \
                       --play <name>          ask the running daemon to play a sound and wait\n  \
                                              for it to finish, then exit\n  \
                       --check-config <path>  say whether that file would be accepted, exit\n  \
                       -h, --help             this message\n  \
                       -V, --version          print the version\n\n\
                     Settings live in ~/.config/wlrix/sounds.toml, or /etc/wlrix/sounds.toml.\n\
                     Themes are looked for under share/sounds on the XDG data path.",
                    env!("CARGO_PKG_VERSION")
                );
                return;
            }
            "--version" | "-V" => {
                println!("wlrix-sound {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            // Answers a question about a file rather than doing anything with it, so it needs no
            // bus and no sound server. `wlrix-settings-daemon` runs this against a candidate file
            // before renaming it into place.
            "--check-config" => {
                let Some(path) = rest.next() else {
                    eprintln!("wlrix-sound: --check-config needs a path");
                    std::process::exit(2);
                };
                if let Err(why) = wlrix_sound::config::check(std::path::Path::new(path)) {
                    eprintln!("{why}");
                    std::process::exit(1);
                }
                return;
            }
            // For trying a sound, or a theme, from a shell. Goes through the running daemon
            // exactly as a component does, settings and all.
            "--play" => {
                let Some(name) = rest.next() else {
                    eprintln!("wlrix-sound: --play needs a sound name");
                    std::process::exit(2);
                };
                if !wlrix_sound::client::play_and_wait(name, Duration::from_secs(10)) {
                    eprintln!("wlrix-sound: no answer -- is the daemon running?");
                    std::process::exit(1);
                }
                return;
            }
            other => {
                eprintln!("wlrix-sound: unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }

    if let Err(err) = wlrix_sound::daemon::run() {
        eprintln!("wlrix-sound: {err}");
        std::process::exit(1);
    }
}
