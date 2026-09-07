#!/usr/bin/env bash
# Wrap eiviz-mac.app, eiviz-remote.app, eivizctl, and eiviz-headless in a
# component package. Apps install to /Applications; CLI tools to /usr/local/bin.
# BundleIsRelocatable must be false: otherwise Installer updates an existing
# jp.mikanseilaboratory.eiviz copy (zip extract, Downloads, build tree)
# and never creates /Applications/eiviz.app.
set -euo pipefail
DIR="${1:?directory containing eiviz-mac.app}"
OUT="${2:?output .pkg path}"
DIR="$(cd "$DIR" && pwd)"
APP="$DIR/eiviz-mac.app"
REMOTE="$DIR/eiviz-remote.app"
CTL="$DIR/eivizctl"
HEADLESS="$DIR/eiviz-headless"
EXAMPLE_ENV="$DIR/eiviz-headless.example.env"
if [[ ! -d "$APP" ]]; then
  echo "missing $APP" >&2
  exit 1
fi
if [[ ! -d "$REMOTE" ]]; then
  echo "missing $REMOTE" >&2
  exit 1
fi
if [[ ! -f "$CTL" ]]; then
  echo "missing $CTL" >&2
  exit 1
fi
if [[ ! -f "$HEADLESS" ]]; then
  echo "missing $HEADLESS" >&2
  exit 1
fi
VERSION="${EIVIZ_VERSION:-0.0.0}"
ROOT="$(mktemp -d "${TMPDIR:-/tmp}/eiviz-pkg.XXXXXX")"
PLIST="$(mktemp "${TMPDIR:-/tmp}/eiviz-pkg-plist.XXXXXX")"
cleanup() { rm -rf "$ROOT" "$PLIST"; }
trap cleanup EXIT
mkdir -p "$ROOT/Applications" "$ROOT/usr/local/bin"
cp -R "$APP" "$ROOT/Applications/eiviz.app"
cp -R "$REMOTE" "$ROOT/Applications/eiviz-remote.app"
cp "$CTL" "$ROOT/usr/local/bin/eivizctl"
cp "$HEADLESS" "$ROOT/usr/local/bin/eiviz-headless"
chmod 755 "$ROOT/usr/local/bin/eivizctl" "$ROOT/usr/local/bin/eiviz-headless"
if [[ -f "$EXAMPLE_ENV" ]]; then
  mkdir -p "$ROOT/usr/local/share/eiviz"
  cp "$EXAMPLE_ENV" "$ROOT/usr/local/share/eiviz/"
fi
pkgbuild --analyze --root "$ROOT" "$PLIST"
i=0
while /usr/libexec/PlistBuddy -c "Print :$i:BundleIsRelocatable" "$PLIST" >/dev/null 2>&1; do
  /usr/libexec/PlistBuddy -c "Set :$i:BundleIsRelocatable false" "$PLIST"
  i=$((i + 1))
done
pkgbuild \
  --root "$ROOT" \
  --component-plist "$PLIST" \
  --identifier jp.mikanseilaboratory.eiviz \
  --version "$VERSION" \
  --install-location / \
  "$OUT"
echo "pkg -> $OUT"
