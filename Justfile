#!/usr/bin/env just --justfile
name := 'wlrix-sound'

rootdir := ''
prefix := '/usr'

base-dir := absolute_path(clean(rootdir / prefix))
bin-dir := base-dir / 'bin'
dbusdir := base-dir / 'share' / 'dbus-1' / 'services'

bin-src := 'target' / 'release' / name
bin-dst := bin-dir / name

# The activation file carries an absolute path to the binary, so it is a template with @BINDIR@
# substituted at install time -- a staged or non-/usr install has to point at where the binary
# actually landed. The binary's *runtime* path, not where DESTDIR puts it.
dbus-src := 'data' / 'com.wlrix.Sound.service.in'
dbus-dst := dbusdir / 'com.wlrix.Sound.service'
runtime-bindir := clean(prefix / 'bin')

default:
  @just --list

release:
  cargo build --release

lint:
  cargo clippy --all-targets -- -D warnings

test:
  cargo test

# Install the daemon and its bus activation file.
#
# There is deliberately no `.desktop` entry: the Toolchest builds its menu by scanning them, and
# the daemon is not something a person starts -- `wlrix-session` starts it. The activation file
# is for a program that calls `com.wlrix.Sound` outside a wlRIX session, or before the session
# has started the daemon.
#
# Deliberately does not build: this is normally run as root, and building as root leaves a target
# directory nobody can write to afterwards.
#
#     just release && sudo just install
[doc("Install the daemon (build first; run as root)")]
install:
  #!/usr/bin/env bash
  set -euo pipefail
  if [ ! -x '{{bin-src}}' ]; then
    echo "no release build -- run 'just release' first" >&2
    exit 1
  fi
  install -Dm0755 '{{bin-src}}' '{{bin-dst}}'
  echo "installed {{bin-dst}}"
  install -d '{{dbusdir}}'
  sed 's|@BINDIR@|{{runtime-bindir}}|g' '{{dbus-src}}' > '{{dbus-dst}}'
  chmod 0644 '{{dbus-dst}}'
  echo "installed {{dbus-dst}}"
  echo
  echo "wlrix-session starts this by name off PATH. Settings go in"
  echo "~/.config/wlrix/sounds.toml; see data/sounds.toml for every key. The sounds"
  echo "themselves come from wlrix-sounds."

[doc("Remove what install put down")]
uninstall:
  #!/usr/bin/env bash
  set -euo pipefail
  rm -f '{{bin-dst}}' '{{dbus-dst}}'
  echo "removed {{bin-dst}} and {{dbus-dst}}"

clean:
  cargo clean
