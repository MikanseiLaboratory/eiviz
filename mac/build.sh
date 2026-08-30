#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCH="${EIVIZ_MAC_ARCH:-}"
SWIFT_ARGS=(-c release)
if [ -n "$ARCH" ]; then
  export EIVIZ_MIXER_LIBDIR="$ROOT/mixer/target/${ARCH}/release"
  case "$ARCH" in
    x86_64-apple-darwin) SWIFT_ARGS+=(--arch x86_64) ;;
    aarch64-apple-darwin) SWIFT_ARGS+=(--arch arm64) ;;
    *)
      echo "unknown EIVIZ_MAC_ARCH=$ARCH" >&2
      exit 1
      ;;
  esac
else
  export EIVIZ_MIXER_LIBDIR="$ROOT/mixer/target/release"
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
cd "$ROOT/mixer"
if [ -n "$ARCH" ]; then
  cargo build --release --locked --target "$ARCH"
else
  cargo build --release --locked
fi
DYLIB="$EIVIZ_MIXER_LIBDIR/libeiviz_mixer.dylib"
install_name_tool -id "@rpath/libeiviz_mixer.dylib" "$DYLIB"
if [ -f "$EIVIZ_MIXER_LIBDIR/deps/libeiviz_mixer.dylib" ]; then
  install_name_tool -id "@rpath/libeiviz_mixer.dylib" "$EIVIZ_MIXER_LIBDIR/deps/libeiviz_mixer.dylib"
fi
cd "$ROOT/mac"
swift build "${SWIFT_ARGS[@]}"
BIN="$(swift build "${SWIFT_ARGS[@]}" --show-bin-path)"
cp -f "$DYLIB" "$BIN/"
if [ -f "$EIVIZ_MIXER_LIBDIR/libndi.dylib" ]; then
  cp -f "$EIVIZ_MIXER_LIBDIR/libndi.dylib" "$BIN/"
fi
chmod +x "$ROOT/mac/relocate-dylib.sh"
"$ROOT/mac/relocate-dylib.sh" "$BIN/eiviz-mac" "$BIN/libeiviz_mixer.dylib"
echo "eiviz-mac -> $BIN/eiviz-mac"
file "$BIN/eiviz-mac" "$BIN/libeiviz_mixer.dylib"
otool -L "$BIN/eiviz-mac"
