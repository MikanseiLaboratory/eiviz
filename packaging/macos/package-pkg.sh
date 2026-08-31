#!/usr/bin/env bash
# Wrap eiviz-mac.app in a component package that installs to /Applications.
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
pkgbuild \
  --root "$ROOT" \
  --identifier jp.mikanseilaboratory.eiviz \
  --version "$VERSION" \
  --install-location /Applications \
  "$OUT"
echo "pkg -> $OUT"
