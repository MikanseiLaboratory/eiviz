#!/usr/bin/env bash
# Assemble eiviz-mac.app and eiviz-remote.app so NDI/Bonjour and local-network TCC see a real bundle.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DIR="${1:?directory containing eiviz-mac}"
DIR="$(cd "$DIR" && pwd)"
chmod +x "$ROOT/hosts/macos/relocate-dylib.sh"

assemble() {
  local bin="$1"
  local plist="$2"
  local app="$DIR/${bin}.app"
  rm -rf "$app"
  mkdir -p "$app/Contents/MacOS"
  cp "$DIR/eiviz-mac" "$app/Contents/MacOS/$bin"
  cp "$DIR/libeiviz_mixer.dylib" "$app/Contents/MacOS/"
  cp "$DIR/libeiviz_remote.dylib" "$app/Contents/MacOS/"
  if [ -f "$DIR/libndi.dylib" ]; then
    cp "$DIR/libndi.dylib" "$app/Contents/MacOS/"
  fi
  if [ -f "$DIR/libndi.6.dylib" ]; then
    cp "$DIR/libndi.6.dylib" "$app/Contents/MacOS/"
  fi
  cp "$plist" "$app/Contents/Info.plist"
  if [ -n "${EIVIZ_VERSION:-}" ]; then
    /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $EIVIZ_VERSION" "$app/Contents/Info.plist"
    /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $EIVIZ_VERSION" "$app/Contents/Info.plist"
  fi
  "$ROOT/hosts/macos/relocate-dylib.sh" "$app/Contents/MacOS/$bin" "$app/Contents/MacOS/libeiviz_mixer.dylib" "$app/Contents/MacOS/libeiviz_remote.dylib"
  echo "${bin}.app -> $app"
}

assemble eiviz-mac "$ROOT/hosts/macos/Sources/EivizMac/Info.plist"
assemble eiviz-remote "$ROOT/hosts/macos/Sources/EivizMac/Info-Remote.plist"
