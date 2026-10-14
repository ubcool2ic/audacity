#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <package.deb|package.AppImage>" >&2
    exit 2
fi

PACKAGE="$(cd -- "$(dirname -- "$1")" && pwd)/$(basename -- "$1")"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

case "$PACKAGE" in
    *.deb)
        command -v dpkg-deb >/dev/null 2>&1 || { echo "dpkg-deb is required" >&2; exit 1; }
        dpkg-deb -x "$PACKAGE" "$TMP"
        APP="$TMP/usr/bin/audacity-rs"
        ;;
    *.AppImage|*.appimage)
        chmod 0755 "$PACKAGE"
        APP="$PACKAGE"
        ;;
    *)
        echo "Unsupported package: $PACKAGE" >&2
        exit 2
        ;;
esac

run_cli() {
    if [[ "$PACKAGE" == *.deb ]]; then
        "$APP" "$1"
    else
        APPIMAGE_EXTRACT_AND_RUN=1 "$APP" "$1"
    fi
}

run_cli --help
run_cli --selftest

gui_smoke() {
    local status=0
    if [[ "$PACKAGE" == *.deb ]]; then
        timeout 5s "$APP" || status=$?
    else
        APPIMAGE_EXTRACT_AND_RUN=1 timeout 5s "$APP" || status=$?
    fi
    if [[ "$status" -ne 0 && "$status" -ne 124 ]]; then
        echo "GUI smoke test failed with exit status $status" >&2
        return "$status"
    fi
}

if [[ -n "${DISPLAY:-}" ]]; then
    gui_smoke
elif command -v xvfb-run >/dev/null 2>&1; then
    if [[ "$PACKAGE" == *.deb ]]; then
        timeout 5s xvfb-run -a "$APP" || status=$?
    else
        APPIMAGE_EXTRACT_AND_RUN=1 timeout 5s xvfb-run -a "$APP" || status=$?
    fi
    status="${status:-0}"
    if [[ "$status" -ne 0 && "$status" -ne 124 ]]; then
        echo "GUI smoke test failed with exit status $status" >&2
        exit "$status"
    fi
fi

if [[ "$PACKAGE" == *.deb ]]; then
    ENGINE="$TMP/usr/lib/audacity-rs/au-nyquist-engine"
    if [[ ! -x "$ENGINE" ]]; then
        echo "Missing or non-executable Nyquist engine: $ENGINE" >&2
        exit 1
    fi
    if [[ ! -L "$APP" || "$(readlink -- "$APP")" != "../lib/audacity-rs/audacity-rs" ]]; then
        echo "audacity-rs launcher is not the expected relative symlink" >&2
        exit 1
    fi
    shopt -s nullglob
    catalogs=("$TMP"/usr/lib/audacity-rs/locale/*/LC_MESSAGES/audacity.{mo,po})
    if [[ "${#catalogs[@]}" -lt 10 ]]; then
        echo "Expected at least 10 packaged catalogs; found ${#catalogs[@]}" >&2
        exit 1
    fi
    LANG=de_DE.UTF-8 "$APP" --help
else
    EXTRACT_DIR="$TMP/appimage-extract"
    mkdir -p "$EXTRACT_DIR"
    (cd "$EXTRACT_DIR" && "$PACKAGE" --appimage-extract >/dev/null)
    APPDIR="$EXTRACT_DIR/squashfs-root"
    ENGINE="$APPDIR/usr/bin/au-nyquist-engine"
    if [[ ! -x "$ENGINE" ]]; then
        echo "Missing or non-executable Nyquist engine: $ENGINE" >&2
        exit 1
    fi
    shopt -s nullglob
    catalogs=("$APPDIR"/usr/share/locale/*/LC_MESSAGES/audacity.{mo,po})
    if [[ "${#catalogs[@]}" -lt 10 ]]; then
        echo "Expected at least 10 packaged catalogs; found ${#catalogs[@]}" >&2
        exit 1
    fi
    LANG=de_DE.UTF-8 "$APPDIR/usr/bin/audacity-rs" --help
fi
