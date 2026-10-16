#!/usr/bin/env bash
# Cross-builds the Windows x64 app on Linux (mingw-w64) and packages a portable
# zip plus an MSI (wixl/msitools). Codec DLLs come from MSYS2's mingw64 repo.
# Output: rust/target/packages/Audacity-RS-<ver>-win64-portable.zip and Audacity-RS-<ver>-x64.msi
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
RUST_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
REPO_ROOT="$(cd "$RUST_DIR/.." && pwd)"
TARGET=x86_64-pc-windows-gnu
VERSION="$(grep -m1 '^version' "$RUST_DIR/Cargo.toml" | cut -d'"' -f2)"
DEPS="${MSYS_DEPS:-$RUST_DIR/target/msys2-mingw64}"
OUT="$RUST_DIR/target/packages"
STAGE="$RUST_DIR/target/win-stage/Audacity-RS"
REPO=https://repo.msys2.org/mingw/mingw64
PKGS="lame libvorbis libogg opus wavpack libsndfile flac libid3tag soundtouch libsoxr mpg123 libwinpthread libgcc libstdc++ libgomp"

fetch_deps() {
    mkdir -p "$DEPS"; cd "$DEPS"
    [ -f index.html ] || curl -sfL "$REPO/" -o index.html
    for p in $PKGS; do
        local re; re="$(printf '%s' "$p" | sed 's/+/\\+/g')"
        local f; f="$(grep -oE "mingw-w64-x86_64-$re-[0-9][^\"]*-any\.pkg\.tar\.zst\"" index.html | tr -d '"' | sort -V | tail -1)"
        [ -n "$f" ] || { echo "no MSYS2 package for $p" >&2; exit 1; }
        [ -f "$f" ] || curl -sfL -o "$f" "$REPO/$(printf '%s' "$f" | sed 's/+/%2B/g')"
        tar --zstd -xf "$f"
    done
    cd - >/dev/null
}

[ -d "$DEPS/mingw64/lib" ] || fetch_deps

if [ "${SKIP_BUILD:-0}" != 1 ]; then
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS="-L $DEPS/mingw64/lib" \
    CXXFLAGS_x86_64_pc_windows_gnu="${CXXFLAGS_x86_64_pc_windows_gnu:--I/tmp/wininc}" \
        cargo build --manifest-path "$RUST_DIR/Cargo.toml" --release --workspace --features au-app/gui --target $TARGET
fi

rm -rf "$STAGE"; mkdir -p "$STAGE" "$OUT"
BIN="$RUST_DIR/target/$TARGET/release"
for exe in audacity-rs au-nyquist-engine au-nyquist-worker au-plugin-editor au-plugin-scan; do
    install -m 0755 "$BIN/$exe.exe" "$STAGE/"
done
# Runtime DLLs the exe imports, resolved recursively from the MSYS2 tree.
need() { x86_64-w64-mingw32-objdump -p "$1" | awk '/DLL Name:/{print $3}'; }
queue=("$STAGE"/*.exe)
while [ ${#queue[@]} -gt 0 ]; do
    f="${queue[0]}"; queue=("${queue[@]:1}")
    for d in $(need "$f"); do
        src="$DEPS/mingw64/bin/$d"
        if [ -f "$src" ] && [ ! -f "$STAGE/$d" ]; then cp "$src" "$STAGE/"; queue+=("$STAGE/$d"); fi
    done
done
cp -r "$REPO_ROOT/share/nyquist-runtime" "$STAGE/nyquist"; rm -f "$STAGE/nyquist/CMakeLists.txt"
mkdir -p "$STAGE/plug-ins"; cp "$REPO_ROOT"/share/nyquist-plug-ins/*.ny "$STAGE/plug-ins/"
for po in "$REPO_ROOT"/au3/locale/*.po; do
    lang="$(basename "$po" .po)"; mkdir -p "$STAGE/locale/$lang/LC_MESSAGES"
    msgfmt -o "$STAGE/locale/$lang/LC_MESSAGES/audacity.mo" "$po" 2>/dev/null || cp "$po" "$STAGE/locale/$lang/LC_MESSAGES/audacity.po"
done
cat > "$STAGE/README-Windows.txt" <<TXT
Audacity-RS $VERSION for Windows (x64), portable build.
Run audacity-rs.exe. Settings go to %APPDATA%\\audacity\\audacity.cfg and %APPDATA%\\audacity-rs.
Optional: FFmpeg 7 (avformat-61.dll etc.) next to the exe or in "C:\\Program Files\\FFmpeg for Audacity"
for M4A/AAC/WMA/AC3. Nyquist effects use lib-nyquist-effects.dll from an installed Audacity 3.7.x.
TXT

ZIP="$OUT/Audacity-RS-$VERSION-win64-portable.zip"
rm -f "$ZIP"; (cd "$(dirname "$STAGE")" && zip -qr "$ZIP" "$(basename "$STAGE")")
echo "zip: $ZIP"

if command -v wixl >/dev/null && command -v wixl-heat >/dev/null; then
    W="$RUST_DIR/target/win-stage/wix"; mkdir -p "$W"
    (cd "$(dirname "$STAGE")" && find Audacity-RS -type f | sort | wixl-heat -p Audacity-RS/ --component-group AppFiles --directory-ref INSTALLDIR --var var.SourceDir) > "$W/files.wxs"
    sed -e "s/@VERSION@/$VERSION/" "$SCRIPT_DIR/audacity-rs-wixl.wxs" > "$W/main.wxs"
    MSI="$OUT/Audacity-RS-$VERSION-x64.msi"
    wixl -a x64 -D SourceDir="$STAGE" -o "$MSI" "$W/main.wxs" "$W/files.wxs"
    echo "msi: $MSI"
else
    echo "wixl/wixl-heat not installed (apt install wixl); skipping MSI" >&2
fi
