#!/usr/bin/env bash
# Assemble eiviz-mac.app so NDI/Bonjour and local-network TCC see a real bundle.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="${1:?directory containing eiviz-mac}"
DIR="$(cd "$DIR" && pwd)"
APP="$DIR/eiviz-mac.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$DIR/eiviz-mac" "$APP/Contents/MacOS/"
cp "$DIR/libeiviz_mixer.dylib" "$APP/Contents/MacOS/"
if [ -f "$DIR/libndi.dylib" ]; then
  cp "$DIR/libndi.dylib" "$APP/Contents/MacOS/"
fi
if [ -f "$DIR/libndi.6.dylib" ]; then
  cp "$DIR/libndi.6.dylib" "$APP/Contents/MacOS/"
fi
cp "$ROOT/mac/Sources/EivizMac/Info.plist" "$APP/Contents/Info.plist"
chmod +x "$ROOT/mac/relocate-dylib.sh"
"$ROOT/mac/relocate-dylib.sh" "$APP/Contents/MacOS/eiviz-mac" "$APP/Contents/MacOS/libeiviz_mixer.dylib"
echo "eiviz-mac.app -> $APP"
