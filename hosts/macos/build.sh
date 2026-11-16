#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# The Swift package targets macOS 14. Keep the Rust dylibs from drifting above it.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
ARCH="${EIVIZ_MAC_ARCH:-}"
SWIFT_ARGS=(-c release)
if [ -n "$ARCH" ]; then
  export EIVIZ_MIXER_LIBDIR="$ROOT/target/${ARCH}/release"
  case "$ARCH" in
    x86_64-apple-darwin) SWIFT_ARGS+=(--arch x86_64) ;;
    aarch64-apple-darwin) SWIFT_ARGS+=(--arch arm64) ;;
    *)
      echo "unknown EIVIZ_MAC_ARCH=$ARCH" >&2
      exit 1
      ;;
  esac
else
  export EIVIZ_MIXER_LIBDIR="$ROOT/target/release"
fi
# Empty LIBCLANG_PATH makes bindgen skip its default search and fail.
if [ -z "${LIBCLANG_PATH:-}" ]; then
  CLANG="$(xcrun --find clang 2>/dev/null || true)"
  if [ -n "$CLANG" ]; then
    LIBDIR="$(cd "$(dirname "$CLANG")/../lib" && pwd)"
    if [ -f "$LIBDIR/libclang.dylib" ]; then
      export LIBCLANG_PATH="$LIBDIR"
    fi
  fi
fi
cd "$ROOT"
if [ -n "$ARCH" ]; then
  cargo build -p eiviz_mixer -p eiviz_remote --release --locked --target "$ARCH"
else
  cargo build -p eiviz_mixer -p eiviz_remote --release --locked
fi
DYLIB="$EIVIZ_MIXER_LIBDIR/libeiviz_mixer.dylib"
REMOTE="$EIVIZ_MIXER_LIBDIR/libeiviz_remote.dylib"
# Cargo hardlinks the deps copy when the top-level dylib is (re)linked. Rewriting the
# install name of two hardlinked paths applies the patch twice to one inode, and the
# second rewrite corrupts the LC_ID string pool on current toolchains; dyld then
# refuses the library. Patch one path when the two are the same file.
patch_install_name() {
  local file="$1" id="$2" twin="$3"
  [ -f "$file" ] || return 0
  if [ -f "$twin" ] && [ "$file" -ef "$twin" ]; then
    return 0
  fi
  install_name_tool -id "$id" "$file"
}
patch_install_name "$DYLIB" "@rpath/libeiviz_mixer.dylib" "$EIVIZ_MIXER_LIBDIR/deps/libeiviz_mixer.dylib"
patch_install_name "$EIVIZ_MIXER_LIBDIR/deps/libeiviz_mixer.dylib" "@rpath/libeiviz_mixer.dylib" "$DYLIB"
patch_install_name "$REMOTE" "@rpath/libeiviz_remote.dylib" "$EIVIZ_MIXER_LIBDIR/deps/libeiviz_remote.dylib"
patch_install_name "$EIVIZ_MIXER_LIBDIR/deps/libeiviz_remote.dylib" "@rpath/libeiviz_remote.dylib" "$REMOTE"
cd "$ROOT/hosts/macos"
# Build only the shipped products. A plain `swift build` also builds the
# replay-core-checks test executable, whose release (WMO + explicit modules)
# build fails on current toolchains while its debug build — the one the checks
# actually run under — keeps working. SwiftPM searches the products directory
# before the Rust target directory, so a dylib copied there by an earlier build
# would be linked instead of the one just built; clear the stale copies first.
BIN="$(swift build "${SWIFT_ARGS[@]}" --show-bin-path)"
rm -f "$BIN/libeiviz_mixer.dylib" "$BIN/libeiviz_remote.dylib" "$BIN"/libndi*.dylib
swift build "${SWIFT_ARGS[@]}" --product eiviz-mac
swift build "${SWIFT_ARGS[@]}" --product eiviz-remote
BIN="$(swift build "${SWIFT_ARGS[@]}" --show-bin-path)"
cp -f "$DYLIB" "$BIN/"
cp -f "$REMOTE" "$BIN/"
if [ -f "$EIVIZ_MIXER_LIBDIR/libndi.dylib" ]; then
  cp -f "$EIVIZ_MIXER_LIBDIR/libndi.dylib" "$BIN/"
fi
if [ -f "$EIVIZ_MIXER_LIBDIR/libndi.6.dylib" ]; then
  cp -f "$EIVIZ_MIXER_LIBDIR/libndi.6.dylib" "$BIN/"
fi
chmod +x "$ROOT/hosts/macos/relocate-dylib.sh" "$ROOT/hosts/macos/package-app.sh"
"$ROOT/hosts/macos/relocate-dylib.sh" "$BIN/eiviz-mac" "$BIN/libeiviz_mixer.dylib" "$BIN/libeiviz_remote.dylib"
"$ROOT/hosts/macos/relocate-dylib.sh" "$BIN/eiviz-remote" "$BIN/libeiviz_mixer.dylib" "$BIN/libeiviz_remote.dylib"
"$ROOT/hosts/macos/package-app.sh" "$BIN"
# install_name_tool drops the (ad-hoc) code signature while rewriting load
# commands, and a broken signature makes the binaries start far slower and
# blocks TCC from showing the app by name. Re-sign everything the linker and
# the relocation touched, and the bundles package-app.sh assembled.
codesign --force --sign - "$DYLIB" "$REMOTE"
codesign --force --sign - "$BIN/eiviz-mac" "$BIN/eiviz-remote"
# libndi ships signed; the relocation above rewrites its load commands (and the
# copies in $BIN), so put an ad-hoc one back the same way.
if ls "$BIN"/libndi*.dylib >/dev/null 2>&1; then
  codesign --force --sign - "$BIN"/libndi*.dylib
fi
for app in "$BIN/eiviz-mac.app" "$BIN/eiviz-remote.app" "$BIN"/*.app; do
  [ -d "$app" ] || continue
  codesign --force --deep --sign - "$app"
done
echo "codesign status:"
codesign -v "$BIN/eiviz-mac"
codesign -v "$BIN/eiviz-remote"
codesign -v "$DYLIB"
codesign -v "$REMOTE"
echo "eiviz-mac -> $BIN/eiviz-mac"
echo "eiviz-remote -> $BIN/eiviz-remote"
file "$BIN/eiviz-mac" "$BIN/eiviz-remote" "$BIN/libeiviz_mixer.dylib" "$BIN/libeiviz_remote.dylib"
otool -L "$BIN/eiviz-mac"
otool -L "$BIN/eiviz-remote"
