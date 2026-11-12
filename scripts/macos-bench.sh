#!/usr/bin/env bash
# Runs a session under eiviz-headless on macOS and records the 5 s stats line.
# Usage: scripts/macos-bench.sh [session.eivz] [seconds]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SESSION="${1:-$ROOT/headless/tests/fixtures/bars.eivz}"
SECONDS_TO_RUN="${2:-60}"
OUT="$ROOT/target/macos-bench/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"
# A checkout that keeps rustup beside the repo can use it; a normal ~/.rustup
# install must keep working, so only point at the sibling tree when it exists.
if [ -z "${RUSTUP_HOME:-}" ] && [ -d "$ROOT/../.rustup" ]; then
  export RUSTUP_HOME="$ROOT/../.rustup"
fi
if [ -z "${CARGO_HOME:-}" ] && [ -d "$ROOT/../.cargo" ]; then
  export CARGO_HOME="$ROOT/../.cargo"
  export PATH="$CARGO_HOME/bin:$PATH"
fi
# Only override NDI_SDK_DIR when the packaged SDK really is unpacked there: pointing
# it at a path that does not exist stops grafton-ndi from searching /Library itself.
if [ -z "${NDI_SDK_DIR:-}" ] && [ -d "/tmp/ndi_sdk/NDI_SDK_Component.pkg/Payload/NDI SDK for Apple" ]; then
  export NDI_SDK_DIR="/tmp/ndi_sdk/NDI_SDK_Component.pkg/Payload/NDI SDK for Apple"
fi
LIBCLANG_PATH="${LIBCLANG_PATH:-$(cd "$(dirname "$(xcrun --find clang)")/../lib" && pwd)}"
export LIBCLANG_PATH

cargo build -p eiviz-headless --release --locked --bins >"$OUT/build.log" 2>&1
"$ROOT/target/release/eiviz-headless" run --session "$SESSION" --bind 127.0.0.1:0 --renderer metal \
  >"$OUT/stdout.log" 2>"$OUT/stderr.log" &
PID=$!
cleanup() {
  kill "$PID" 2>/dev/null || true
  wait "$PID" 2>/dev/null || true
}
trap cleanup EXIT
ready=0
for _ in $(seq 1 60); do
  if grep -q 'ready ws=' "$OUT/stderr.log"; then
    ready=1
    break
  fi
  sleep 1
done
if [ "$ready" -ne 1 ]; then
  echo "eiviz-headless did not become ready; see $OUT/stderr.log" >&2
  exit 1
fi
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
