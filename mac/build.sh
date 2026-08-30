#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export EIVIZ_MIXER_LIBDIR="$ROOT/mixer/target/release"
export LIBCLANG_PATH="${LIBCLANG_PATH:-}"
cd "$ROOT/mixer"
cargo build --release --locked
cd "$ROOT/mac"
swift build -c release
BIN="$(swift build -c release --show-bin-path)"
cp -f "$EIVIZ_MIXER_LIBDIR/libeiviz_mixer.dylib" "$BIN/"
echo "eiviz-mac -> $BIN/eiviz-mac"
