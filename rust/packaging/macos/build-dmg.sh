#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../../.." && pwd)"
RUST_DIR="$REPO_ROOT/rust"
UNIVERSAL=0

if [[ "${1:-}" == "--universal" ]]; then
    UNIVERSAL=1
elif [[ $# -gt 0 ]]; then
    echo "Usage: $0 [--universal]" >&2
    exit 2
fi

metadata="$(cd "$RUST_DIR" && cargo metadata --no-deps --format-version 1)"
VERSION=""
if command -v jq >/dev/null 2>&1; then
    VERSION="$(printf '%s' "$metadata" | jq -r '[.packages[] | select(.name == "au-app")][0].version // empty')"
fi
if [[ -z "$VERSION" ]] && command -v python3 >/dev/null 2>&1; then
    VERSION="$(printf '%s' "$metadata" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(next((p["version"] for p in d["packages"] if p["name"]=="au-app"), ""))')"
fi
VERSION="${VERSION:-0.0.0}"

PACKAGES_DIR="$RUST_DIR/target/packages"
mkdir -p "$PACKAGES_DIR"
STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT
APP="$STAGING/Audacity-RS.app"
MACOS="$APP/Contents/MacOS"
RESOURCES="$APP/Contents/Resources"
mkdir -p "$MACOS" "$RESOURCES"

build_target() {
    local target="$1"
    cargo build --manifest-path "$RUST_DIR/Cargo.toml" --release --target "$target" -p au-app --features gui
    cargo build --manifest-path "$RUST_DIR/Cargo.toml" --release --target "$target" -p au-plugin-host --bin au-nyquist-worker
}

if [[ "$UNIVERSAL" -eq 1 ]]; then
    build_target x86_64-apple-darwin
    build_target aarch64-apple-darwin
    lipo -create \
        "$RUST_DIR/target/x86_64-apple-darwin/release/audacity-rs" \
        "$RUST_DIR/target/aarch64-apple-darwin/release/audacity-rs" \
        -output "$MACOS/audacity-rs"
    lipo -create \
        "$RUST_DIR/target/x86_64-apple-darwin/release/au-nyquist-worker" \
        "$RUST_DIR/target/aarch64-apple-darwin/release/au-nyquist-worker" \
        -output "$MACOS/au-nyquist-worker"
    PACKAGE_ARCH="universal"
else
    HOST_ARCH="$(uname -m)"
    case "$HOST_ARCH" in
        arm64|aarch64) TARGET="aarch64-apple-darwin"; PACKAGE_ARCH="aarch64" ;;
        x86_64) TARGET="x86_64-apple-darwin"; PACKAGE_ARCH="x86_64" ;;
        *) echo "Unsupported macOS host architecture: $HOST_ARCH" >&2; exit 1 ;;
    esac
    build_target "$TARGET"
    install -m 0755 "$RUST_DIR/target/$TARGET/release/audacity-rs" "$MACOS/audacity-rs"
    install -m 0755 "$RUST_DIR/target/$TARGET/release/au-nyquist-worker" "$MACOS/au-nyquist-worker"
fi
chmod 0755 "$MACOS/audacity-rs" "$MACOS/au-nyquist-worker"

ICON_PLIST=""
ICONSET="$STAGING/audacity.iconset"
if command -v rsvg-convert >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
    mkdir -p "$ICONSET"
    for size in 16 32 128 256 512; do
        rsvg-convert -w "$size" -h "$size" "$REPO_ROOT/images/audacity.svg" > "$ICONSET/icon_${size}x${size}.png"
        double=$((size * 2))
        rsvg-convert -w "$double" -h "$double" "$REPO_ROOT/images/audacity.svg" > "$ICONSET/icon_${size}x${size}@2x.png"
    done
    if iconutil -c icns "$ICONSET" -o "$RESOURCES/audacity-rs.icns"; then
        ICON_PLIST='<key>CFBundleIconFile</key><string>audacity-rs</string>'
    else
        rm -f "$RESOURCES/audacity-rs.icns"
    fi
fi

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>org.audacityrs.app</string>
  <key>CFBundleName</key><string>Audacity-RS</string>
  <key>CFBundleDisplayName</key><string>Audacity-RS</string>
  <key>CFBundleExecutable</key><string>audacity-rs</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSMicrophoneUsageDescription</key><string>Audacity-RS records audio from your microphone.</string>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Audacity-RS Project</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Owner</string>
      <key>LSItemContentTypes</key><array><string>org.audacityrs.project</string></array>
      <key>CFBundleTypeExtensions</key><array><string>aup3</string></array>
    </dict>
  </array>
  <key>UTExportedTypeDeclarations</key>
  <array>
    <dict>
      <key>UTTypeIdentifier</key><string>org.audacityrs.project</string>
      <key>UTTypeDescription</key><string>Audacity-RS Project</string>
      <key>UTTypeConformsTo</key><array><string>public.database</string></array>
      <key>UTTypeTagSpecification</key>
      <dict><key>public.filename-extension</key><array><string>aup3</string></array></dict>
    </dict>
  </array>
  $ICON_PLIST
</dict>
</plist>
PLIST

if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
    SIGN_ARGS=(--force --options runtime --timestamp --sign "$MACOS_SIGN_IDENTITY")
    codesign "${SIGN_ARGS[@]}" "$MACOS/au-nyquist-worker"
    codesign "${SIGN_ARGS[@]}" --entitlements "$SCRIPT_DIR/entitlements.plist" "$APP"
fi

ln -s /Applications "$STAGING/Applications"
OUT="$PACKAGES_DIR/Audacity-RS-${VERSION}-${PACKAGE_ARCH}.dmg"
hdiutil create -volname Audacity-RS -srcfolder "$STAGING" -ov -format UDZO "$OUT"

if [[ -n "${MACOS_SIGN_IDENTITY:-}" ]]; then
    codesign --force --timestamp --sign "$MACOS_SIGN_IDENTITY" "$OUT"
fi

if [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
    xcrun notarytool submit "$OUT" --wait --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD"
    xcrun stapler staple "$OUT"
fi

printf '%s\n' "$OUT"
