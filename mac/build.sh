#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ARCH="${EIVIZ_MAC_ARCH:-}"
SWIFT_ARCH=()
if [ -n "$ARCH" ]; then
  export EIVIZ_MIXER_LIBDIR="$ROOT/mixer/target/${ARCH}/release"
  case "$ARCH" in
    x86_64-apple-darwin) SWIFT_ARCH=(--arch x86_64) ;;
    aarch64-apple-darwin) SWIFT_ARCH=(--arch arm64) ;;
    *)
      echo "unknown EIVIZ_MAC_ARCH=$ARCH" >&2
      exit 1
      ;;
  esac
else
  export EIVIZ_MIXER_LIBDIR="$ROOT/mixer/target/release"
fi
export LIBCLANG_PATH="${LIBCLANG_PATH:-}"
cd "$ROOT/mixer"
if [ -n "$ARCH" ]; then
  cargo build --release --locked --target "$ARCH"
else
  cargo build --release --locked
fi
cd "$ROOT/mac"
swift build -c release "${SWIFT_ARCH[@]}"
BIN="$(swift build -c release "${SWIFT_ARCH[@]}" --show-bin-path)"
cp -f "$EIVIZ_MIXER_LIBDIR/libeiviz_mixer.dylib" "$BIN/"
echo "eiviz-mac -> $BIN/eiviz-mac"
file "$BIN/eiviz-mac" "$BIN/libeiviz_mixer.dylib"
