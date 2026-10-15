# wlrix-sound

System sounds for wlRIX. Programs ask this daemon for a sound by name and it plays it through PipeWire, from an
[XDG sound theme](https://specifications.freedesktop.org/sound-theme-spec/latest/). By default that is the IRIX Indigo
Magic soundscheme from `wlrix-sounds`.

- **License:** GPL-3.0-or-later
- **Started by:** `wlrix-session`, first among its apps.
- **Settings:** `~/.config/wlrix/sounds.toml`, written by `wlrix-settings-daemon` (namespace `sounds`).

## Why a daemon

The decision about whether a sound plays belongs in one place. The daemon owns it: the theme, a master switch, and a
list of muted sounds. Components only ask for a sound, never read the sound settings, and stay unchanged when a setting
changes. A settings panel changes one file and every sound on the desktop follows.

It also means one PipeWire client instead of one per component, and that the compositor never touches audio.

## Asking for a sound

**From a wlRIX component:** send a datagram to `$XDG_RUNTIME_DIR/wlrix-sound.sock`.

```text
play <name>        play a sound; no reply
play-wait <name>   play a sound; the daemon answers "done" when it has finished
```

`src/client.rs` does this using only `std`. It is copied into each Rust component as `src/sound.rs` and kept in step by
hand. The C# apps have the same thing as `Wlrix.Common.SystemSound`. A send never blocks, and with no daemon running it
just fails silently, which is why the compositor can call it from its event loop.

**From anything else:** D-Bus.

```text
bus name   com.wlrix.Sound
object     /com/wlrix/Sound
interface  com.wlrix.Sound1

Play(s name)                  play a sound, if sounds.toml allows it
Preview(s theme, s name)      play from a theme ("" = the configured one), ignoring the switches
Events() -> a(sss)            every sound the desktop plays: name, label, category
ListThemes() -> a(ss)         every installed sound theme: id, display name
```

`Events` and `ListThemes` are what a settings panel builds its list of switches and its theme menu from. The theme in
use and the muted list are settings, read and written through `com.wlrix.Settings`.

**From a shell:**

```sh
wlrix-sound --play wlrix-launch
busctl --user call com.wlrix.Sound /com/wlrix/Sound com.wlrix.Sound1 Preview ss freedesktop bell
```

## Names

Names follow the [sound naming spec](https://specifications.freedesktop.org/sound-naming-spec/latest/) wherever it has
one. IRIX concepts the spec has no name for use a `wlrix-` prefix. `src/catalog.rs` is the list of sounds the desktop
plays, and it is what `Events` returns:

| Sound                                         | Played by                    | When                                                   |
|-----------------------------------------------|------------------------------|--------------------------------------------------------|
| `desktop-login`, `desktop-logout`             | wlrix-session                | the session starts; the session is logged out          |
| `desktop-screen-lock`                         | wlrix-compositor             | the screen locker has locked                           |
| `window-new`, `window-close`                  | wlrix-compositor             | a window maps; a window goes away (no `wlrix` sound)   |
| `window-minimized`, `window-unminimized`      | wlrix-compositor             | a window is minimized; restored                        |
| `desktop-switch-left`, `desktop-switch-right` | wlrix-compositor             | changing desks, by direction in the desk order         |
| `bell`                                        | compositor, desktop          | an action is refused                                   |
| `wlrix-launch`                                | desktop, Files, Toolchest    | a program is started                                   |
| `wlrix-open-folder`, `wlrix-open-remote-folder` | desktop, Files             | a folder is opened                                     |
| `wlrix-icon-shift`                            | wlrix-desktop                | desktop icons are dragged to a new place               |
| `wlrix-drag-move`, `wlrix-drag-copy`, `drag-fail` | Files                    | a drop moves, copies or links, or is refused           |
| `file-trash`, `item-deleted`                  | desktop, Files               | moved to the trash; deleted outright                   |
| `wlrix-drag-to-trash`, `trash-empty`          | desktop, Files               | icons dropped on the Dumpster; the Dumpster emptied    |
| `complete-copy`                               | Files                        | a copy or move that took a while has finished          |
| `dialog-error`, `-error-serious`, `-question`, `-warning`, `-information` | `MessageDialog` | a message dialog opens                   |
| `wlrix-volume-mount`, `device-added`, `device-removed` | Files               | a volume is mounted; a drive appears or goes           |

## Settings

```toml
# ~/.config/wlrix/sounds.toml
enabled = true          # play system sounds at all
theme = "wlrix"         # by directory name under share/sounds
output = ""             # a PipeWire sink by node name; empty follows the default output
disabled = []           # sounds that never play, by name

[play]                  # whole kinds of sound, as IRIX's System & Desktop Sounds panel had them
startup = false         # login, logout, lock
desktop = true          # icons, folders, launching, drag-and-drop, the dumpster, disks
windows = false         # minimize, restore, desk switches, open/close
alerts = true           # dialogs
bell = true             # the bell
```

Every sound belongs to one `[play]` category (`src/catalog.rs`), and an unlisted one is placed by its name. The
defaults are a default IRIX 6.5 desktop's. It played neither the Indy's power-on and power-off tunes, which the `wlrix`
theme uses for login and logout, nor any window sounds. `Wlrix.Settings.Sound` is the panel for all of this.

Every key is optional, and `data/sounds.toml` documents each one. `SIGHUP` re-reads the file, and the
settings daemon sends it on every change, finding the process through `$XDG_RUNTIME_DIR/wlrix-sound.pid`.

## Theme lookup

The lookup follows the spec. A name is searched for in the theme, then each theme it `Inherits`, then `freedesktop`,
across every `sounds` directory on the XDG data path at each step. A file in `~/.local/share/sounds/wlrix/stereo/`
replaces one sound of the installed theme without copying the rest.

If no theme in that chain has the name, its last dash-separated part is dropped and the search starts again:
`dialog-error-serious` falls back to `dialog-error`. A `<name>.disabled` file silences a sound on purpose. Only `stereo`
directories are used, and `.oga`, `.ogg` and `.wav` files.

## Playback

Each sound gets a short-lived PipeWire stream tagged `media.role=Event`, `media.name=<sound>` and `event.id=<sound>`,
which is how libcanberra's streams look. A mixer shows system sounds as such, and a session manager's policy for event
sounds applies.

Up to four sounds play at once, and anything beyond that is dropped. A sound asked for again within 60 ms of starting is
also dropped. Decoded sounds are kept in memory until the next reload.

If PipeWire is not running, sounds are dropped. One line goes to the log, and the daemon tries again on a later sound.

## Building

```sh
just release && sudo just install   # bin/wlrix-sound, share/dbus-1/services/com.wlrix.Sound.service
```

Needs the PipeWire development headers (`libpipewire-0.3-dev` and `libspa-0.2-dev` on Debian, `pipewire` on Arch) and
clang for the bindings.

## Testing

`cargo test` covers the theme lookup (inheritance, the `freedesktop` fallback, shortening, `.disabled`, user overrides,
loops), decoding, the config, the request parser and the stale-socket check. Playback and the bus are checked by hand:

```sh
XDG_DATA_DIRS=$STAGE/usr/share:/usr/share ./target/release/wlrix-sound &
./target/release/wlrix-sound --play wlrix-launch   # returns once it has played
pw-dump | grep -A1 '"media.role": "Event"'          # while one is playing
```
