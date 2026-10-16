#!/bin/sh
# Run a pipe comparison with its own /tmp (private mod-script-pipe FIFOs and X sockets) and its
# own Audacity config (HOME), so parallel agents' Audacity instances cannot collide with it.
# usage: tools/iso_run.sh python3 tools/compare_snap.py
ISO=${AU_ISO_HOME:-/workspace/isohome}
mkdir -p "$ISO/.config"
[ -d "$ISO/.config/audacity" ] || cp -r "$HOME/.config/audacity" "$ISO/.config/"
exec bwrap --dev-bind / / --tmpfs /tmp --setenv HOME "$ISO" \
  --setenv CARGO_HOME "${CARGO_HOME:-$HOME/.cargo}" --setenv RUSTUP_HOME "${RUSTUP_HOME:-$HOME/.rustup}" "$@"
