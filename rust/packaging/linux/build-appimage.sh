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
ARCH="$(uname -m)"
PACKAGES_DIR="$RUST_DIR/target/packages"
TOOLS_DIR="$PACKAGES_DIR/tools"
APPDIR="$PACKAGES_DIR/Audacity-RS.AppDir"
mkdir -p "$TOOLS_DIR" "$PACKAGES_DIR"
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/scalable/apps"

install -m 0755 "$BIN" "$APPDIR/usr/bin/audacity-rs"
install -m 0755 "$WORKER" "$APPDIR/usr/bin/au-nyquist-worker"
install -m 0755 "$ENGINE" "$APPDIR/usr/bin/au-nyquist-engine"

shopt -s nullglob
for catalog in "$REPO_ROOT"/au3/locale/*.po; do
    [[ "$(basename -- "$catalog")" == "audacity.pot" ]] && continue
    language="$(basename -- "$catalog" .po)"
    catalog_dir="$APPDIR/usr/share/locale/$language/LC_MESSAGES"
    mkdir -p "$catalog_dir"
    if command -v msgfmt >/dev/null 2>&1; then
        msgfmt -o "$catalog_dir/audacity.mo" "$catalog"
    else
        install -m 0644 "$catalog" "$catalog_dir/audacity.po"
    fi
done

install -m 0644 "$SCRIPT_DIR/audacity-rs.desktop" "$APPDIR/audacity-rs.desktop"
install -m 0644 "$SCRIPT_DIR/audacity-rs.desktop" "$APPDIR/usr/share/applications/audacity-rs.desktop"
install -m 0644 "$REPO_ROOT/images/audacity.svg" "$APPDIR/audacity-rs.svg"
install -m 0644 "$REPO_ROOT/images/audacity.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/audacity-rs.svg"

cat > "$APPDIR/AppRun" <<'APPRUN'
#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd -- "$(dirname -- "$(readlink -f -- "$0")")" && pwd)"
export AU_NYQUIST_WORKER="$HERE/usr/bin/au-nyquist-engine"
exec "$HERE/usr/bin/audacity-rs" "$@"
APPRUN
chmod 0755 "$APPDIR/AppRun"

if command -v appimagetool >/dev/null 2>&1; then
    APPIMAGETOOL="$(command -v appimagetool)"
else
    APPIMAGETOOL="$TOOLS_DIR/appimagetool-${ARCH}.AppImage"
    if [[ ! -x "$APPIMAGETOOL" ]]; then
        URL="https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${ARCH}.AppImage"
        if command -v curl >/dev/null 2>&1; then
            curl -fL "$URL" -o "$APPIMAGETOOL"
        elif command -v wget >/dev/null 2>&1; then
            wget -O "$APPIMAGETOOL" "$URL"
        else
            echo "curl or wget is required to download appimagetool" >&2
            exit 1
        fi
        chmod 0755 "$APPIMAGETOOL"
    fi
fi

OUT="$PACKAGES_DIR/Audacity-RS-${VERSION}-${ARCH}.AppImage"
APPIMAGE_EXTRACT_AND_RUN=1 ARCH="$ARCH" "$APPIMAGETOOL" "$APPDIR" "$OUT"
chmod 0755 "$OUT"
printf '%s\n' "$OUT"
