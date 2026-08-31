#!/usr/bin/env bash
# Wrap eiviz-mac.app in a component package that always installs to /Applications.
# BundleIsRelocatable must be false: otherwise Installer updates an existing
# jp.mikanseilaboratory.eiviz copy (zip extract, Downloads, build tree)
# and never creates /Applications/eiviz.app.
set -euo pipefail
DIR="${1:?directory containing eiviz-mac.app}"
OUT="${2:?output .pkg path}"
DIR="$(cd "$DIR" && pwd)"
APP="$DIR/eiviz-mac.app"
if [[ ! -d "$APP" ]]; then
  echo "missing $APP" >&2
  exit 1
fi
VERSION="${EIVIZ_VERSION:-0.0.0}"
ROOT="$(mktemp -d "${TMPDIR:-/tmp}/eiviz-pkg.XXXXXX")"
cleanup() { rm -rf "$ROOT"; }
trap cleanup EXIT
mkdir -p "$ROOT"
cp -R "$APP" "$ROOT/eiviz.app"
PLIST="$ROOT/component.plist"
pkgbuild --analyze --root "$ROOT" "$PLIST"
/usr/libexec/PlistBuddy -c "Set :0:BundleIsRelocatable false" "$PLIST"
pkgbuild \
  --root "$ROOT" \
  --component-plist "$PLIST" \
  --identifier jp.mikanseilaboratory.eiviz \
  --version "$VERSION" \
  --install-location /Applications \
  "$OUT"
echo "pkg -> $OUT"
