#!/usr/bin/env bash
# Runs a session under eiviz-headless on macOS and records the 5 s stats line.
# Usage: scripts/macos-bench.sh [session.eivz] [seconds]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SESSION="${1:-$ROOT/headless/tests/fixtures/bars.eivz}"
SECONDS_TO_RUN="${2:-60}"
OUT="$ROOT/target/macos-bench/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
export RUSTUP_HOME="${RUSTUP_HOME:-$ROOT/../.rustup}"
export CARGO_HOME="${CARGO_HOME:-$ROOT/../.cargo}"
export PATH="$CARGO_HOME/bin:$PATH"
export NDI_SDK_DIR="${NDI_SDK_DIR:-/tmp/ndi_sdk/NDI_SDK_Component.pkg/Payload/NDI SDK for Apple}"
export LIBCLANG_PATH="${LIBCLANG_PATH:-$(cd "$(dirname "$(xcrun --find clang)")/../lib" && pwd)}"

cargo build -p eiviz-headless --release --locked --bins >"$OUT/build.log" 2>&1
"$ROOT/target/release/eiviz-headless" run --session "$SESSION" --bind 127.0.0.1:0 --renderer metal \
  >"$OUT/stdout.log" 2>"$OUT/stderr.log" &
PID=$!
cleanup() {
  kill "$PID" 2>/dev/null || true
  wait "$PID" 2>/dev/null || true
}
trap cleanup EXIT
for _ in $(seq 1 60); do
  grep -q 'ready ws=' "$OUT/stderr.log" && break
  sleep 1
done
for _ in $(seq 1 "$SECONDS_TO_RUN"); do
  sleep 1
  ps -o pid=,pcpu=,rss= -p "$PID" | awk -v t="$(date +%s)" '{print t ",cpu," $2 ",rss_kb," $3}' >>"$OUT/samples.csv"
done
grep -h 'eiviz stats' "$OUT/stdout.log" "$OUT/stderr.log" 2>/dev/null | tail -40 >"$OUT/stats.txt" || true
pmset -g therm >"$OUT/therm.txt" 2>/dev/null || true
cp "$SESSION" "$OUT/session.eivz"
echo "wrote $OUT"
echo "stats:"
cat "$OUT/stats.txt"
