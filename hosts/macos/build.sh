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
# SwiftPM searches the products directory before the Rust target directory. A dylib
# copied there by an earlier build would then be linked instead of the one just built,
# so clear the stale copies before linking.
BIN="$(swift build "${SWIFT_ARGS[@]}" --show-bin-path)"
rm -f "$BIN/libeiviz_mixer.dylib" "$BIN/libeiviz_remote.dylib" "$BIN"/libndi*.dylib
swift build "${SWIFT_ARGS[@]}"
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
echo "eiviz-mac -> $BIN/eiviz-mac"
echo "eiviz-remote -> $BIN/eiviz-remote"
file "$BIN/eiviz-mac" "$BIN/eiviz-remote" "$BIN/libeiviz_mixer.dylib" "$BIN/libeiviz_remote.dylib"
otool -L "$BIN/eiviz-mac"
otool -L "$BIN/eiviz-remote"
