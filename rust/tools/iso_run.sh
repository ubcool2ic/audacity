#!/bin/sh
# Run a pipe comparison with its own /tmp (private mod-script-pipe FIFOs and X sockets) and its
# own Audacity config (HOME), so parallel agents' Audacity instances cannot collide with it.
# usage: tools/iso_run.sh python3 tools/compare_snap.py
ISO=${AU_ISO_HOME:-/workspace/isohome}
mkdir -p "$ISO/.config"
[ -d "$ISO/.config/audacity" ] || cp -r "$HOME/.config/audacity" "$ISO/.config/"
# PULSE_RUNTIME_PATH: with an empty private /tmp, libpulse would mint a random runtime dir name via
# rand(), shifting Audacity's process-global rand() state and breaking rand()-exact references
# (Paulstretch). A fixed path keeps Audacity's rand() sequence at the glibc default seed.
# NO_AT_BRIDGE: GTK would spawn an at-spi dbus-daemon outside our process group that outlives the run.
exec bwrap --dev-bind / / --tmpfs /tmp --setenv HOME "$ISO" --setenv AU_ISO_RUN 1 --setenv PULSE_RUNTIME_PATH /tmp/pulse-iso --setenv NO_AT_BRIDGE 1 \
  --setenv CARGO_HOME "${CARGO_HOME:-$HOME/.cargo}" --setenv RUSTUP_HOME "${RUSTUP_HOME:-$HOME/.rustup}" "$@"
