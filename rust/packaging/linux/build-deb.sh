#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../../.." && pwd)"
RUST_DIR="$REPO_ROOT/rust"
SKIP_BUILD=0

if [[ "${1:-}" == "--skip-build" ]]; then
    SKIP_BUILD=1
elif [[ $# -gt 0 ]]; then
    echo "Usage: $0 [--skip-build]" >&2
    exit 2
fi

if [[ "$SKIP_BUILD" -eq 0 ]]; then
    cargo build --manifest-path "$RUST_DIR/Cargo.toml" --release -p au-app --features gui
    cargo build --manifest-path "$RUST_DIR/Cargo.toml" --release -p au-plugin-host --bin au-nyquist-worker
    cargo build --manifest-path "$RUST_DIR/Cargo.toml" --release -p au-nyquist --bin au-nyquist-engine
fi

BIN="$RUST_DIR/target/release/audacity-rs"
WORKER="$RUST_DIR/target/release/au-nyquist-worker"
ENGINE="$RUST_DIR/target/release/au-nyquist-engine"
for file in "$BIN" "$WORKER" "$ENGINE"; do
    if [[ ! -f "$file" ]]; then
        echo "Missing build output: $file" >&2
        exit 1
    fi
done

metadata="$(cd "$RUST_DIR" && cargo metadata --no-deps --format-version 1)"
VERSION=""
if command -v jq >/dev/null 2>&1; then
    VERSION="$(printf '%s' "$metadata" | jq -r '[.packages[] | select(.name == "au-app")][0].version // empty')"
fi
if [[ -z "$VERSION" ]] && command -v python3 >/dev/null 2>&1; then
    VERSION="$(printf '%s' "$metadata" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next((p["version"] for p in d["packages"] if p["name"]=="au-app"), ""))')"
fi
VERSION="${VERSION:-0.0.0}"
ARCH="$(dpkg --print-architecture)"
PACKAGES_DIR="$RUST_DIR/target/packages"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

install -D -m 0755 "$BIN" "$STAGE/usr/lib/audacity-rs/audacity-rs"
install -D -m 0755 "$WORKER" "$STAGE/usr/lib/audacity-rs/au-nyquist-worker"
install -D -m 0755 "$ENGINE" "$STAGE/usr/lib/audacity-rs/au-nyquist-engine"
mkdir -p "$STAGE/usr/bin"
ln -s ../lib/audacity-rs/audacity-rs "$STAGE/usr/bin/audacity-rs"

shopt -s nullglob
for catalog in "$REPO_ROOT"/au3/locale/*.po; do
    [[ "$(basename -- "$catalog")" == "audacity.pot" ]] && continue
    language="$(basename -- "$catalog" .po)"
    catalog_dir="$STAGE/usr/lib/audacity-rs/locale/$language/LC_MESSAGES"
    mkdir -p "$catalog_dir"
    if command -v msgfmt >/dev/null 2>&1; then
        msgfmt -o "$catalog_dir/audacity.mo" "$catalog"
    else
        install -m 0644 "$catalog" "$catalog_dir/audacity.po"
    fi
done

install -D -m 0644 "$SCRIPT_DIR/audacity-rs.desktop" "$STAGE/usr/share/applications/audacity-rs.desktop"
install -D -m 0644 "$REPO_ROOT/images/audacity.svg" "$STAGE/usr/share/icons/hicolor/scalable/apps/audacity-rs.svg"
install -D -m 0644 /dev/stdin "$STAGE/usr/share/doc/audacity-rs/copyright" <<'COPYRIGHT'
Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/
Upstream-Name: audacity-rs
License: GPL-2.0-or-later
 This program is free software: you can redistribute it and/or modify it
 under the terms of the GNU General Public License, version 2 or later.
 .
 The complete license text is available at https://www.gnu.org/licenses/old-licenses/gpl-2.0.html
COPYRIGHT

if command -v strip >/dev/null 2>&1; then
    strip --strip-unneeded "$STAGE/usr/lib/audacity-rs/audacity-rs" || true
    strip --strip-unneeded "$STAGE/usr/lib/audacity-rs/au-nyquist-worker" || true
    strip --strip-unneeded "$STAGE/usr/lib/audacity-rs/au-nyquist-engine" || true
fi

mkdir -p "$STAGE/DEBIAN" "$PACKAGES_DIR"
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: audacity-rs
Version: $VERSION
Architecture: $ARCH
Maintainer: audacity-rs developers <noreply@example.invalid>
Depends: libc6, libgl1, libx11-6, libxkbcommon0, libasound2 | libasound2t64
Recommends: ffmpeg, liblilv-0-0
Description: Rust port of Audacity
 Audacity-RS is an audio editor and recorder built in Rust.
CONTROL

OUT="$PACKAGES_DIR/audacity-rs_${VERSION}_${ARCH}.deb"
dpkg-deb --root-owner-group --build "$STAGE" "$OUT"
printf '%s\n' "$OUT"
