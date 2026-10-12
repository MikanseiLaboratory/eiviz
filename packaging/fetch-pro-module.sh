#!/usr/bin/env bash
# Download the signed Pro module published by eiviz-pro as <public-tag>-pro.
# The public release tag v0.4.0 reads the private release v0.4.0-pro.
set -euo pipefail

tag="${1:?public release tag}"
target="${2:?rust target}"
runtime="${3:?module file name}"
dest="${4:?destination directory}"

if ! printf '%s' "${EIVIZ_PRO_MODULE_PUBLIC_KEY:-}" | grep -Eq '^[0-9a-fA-F]{64}$'; then
  echo "Set the EIVIZ_PRO_MODULE_PUBLIC_KEY secret to the 64 hex character module public key" >&2
  exit 1
fi
if [ -z "${EIVIZ_PRO_REPO_TOKEN:-}" ]; then
  echo "Set the EIVIZ_PRO_REPO_TOKEN secret to a token that can read MikanseiLaboratory/eiviz-pro releases" >&2
  exit 1
fi

pro_tag="${tag}-pro"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
GH_TOKEN="$EIVIZ_PRO_REPO_TOKEN" gh release download "$pro_tag" \
  --repo MikanseiLaboratory/eiviz-pro \
  --dir "$tmp" \
  --pattern "${target}-${runtime}" \
  --pattern "${target}-${runtime}.manifest" \
  --pattern "${target}-module-public-key.txt"

got="$(tr -d '[:space:]' < "$tmp/${target}-module-public-key.txt" | tr '[:upper:]' '[:lower:]')"
want="$(printf '%s' "$EIVIZ_PRO_MODULE_PUBLIC_KEY" | tr -d '[:space:]' | tr '[:upper:]' '[:lower:]')"
if [ "$got" != "$want" ]; then
  echo "The signed module was not produced by EIVIZ_PRO_MODULE_PUBLIC_KEY" >&2
  exit 1
fi

mkdir -p "$dest"
cp "$tmp/${target}-${runtime}" "$dest/$runtime"
cp "$tmp/${target}-${runtime}.manifest" "$dest/${runtime}.manifest"
printf '%s\n' "$runtime" > "$dest/eiviz-pro.required"
