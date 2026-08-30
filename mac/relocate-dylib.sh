#!/usr/bin/env bash
# Point eiviz-mac at sibling dylibs via @rpath.
# Cargo's default LC_ID_DYLIB is an absolute CI path, which dyld cannot load
# on another machine.
set -euo pipefail
BIN="${1:?binary}"
DYLIB="${2:?dylib}"
DIR="$(cd "$(dirname "$DYLIB")" && pwd)"

rewrite_id() {
  local file="$1"
  local id="$2"
  install_name_tool -id "$id" "$file"
}

rewrite_dep() {
  local file="$1"
  local pattern="$2"
  local dest="$3"
  while IFS= read -r old; do
    [ -z "$old" ] && continue
    if [ "$old" != "$dest" ]; then
      install_name_tool -change "$old" "$dest" "$file"
    fi
  done <<EOF
$(otool -L "$file" | awk -v pat="$pattern" '$1 ~ pat { print $1 }')
EOF
}

rewrite_id "$DYLIB" "@rpath/libeiviz_mixer.dylib"
rewrite_dep "$BIN" "libeiviz_mixer\\.dylib" "@rpath/libeiviz_mixer.dylib"
install_name_tool -add_rpath "@executable_path" "$BIN" 2>/dev/null || true

if [ -f "$DIR/libndi.dylib" ]; then
  rewrite_id "$DIR/libndi.dylib" "@rpath/libndi.dylib"
  rewrite_dep "$DYLIB" "libndi" "@rpath/libndi.dylib"
  install_name_tool -add_rpath "@executable_path" "$DYLIB" 2>/dev/null || true
fi

refs="$(otool -L "$BIN" | awk '/libeiviz_mixer\.dylib/ { print $1 }')"
if [ -z "$refs" ]; then
  echo "eiviz-mac does not link libeiviz_mixer.dylib" >&2
  otool -L "$BIN" >&2
  exit 1
fi
while IFS= read -r old; do
  [ -z "$old" ] && continue
  if [ "$old" != "@rpath/libeiviz_mixer.dylib" ]; then
    echo "eiviz-mac still references libeiviz_mixer.dylib by a non-@rpath path:" >&2
    otool -L "$BIN" >&2
    exit 1
  fi
done <<EOF
$refs
EOF
