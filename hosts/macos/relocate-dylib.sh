#!/usr/bin/env bash
# Point eiviz-mac at sibling dylibs via @rpath.
# Cargo's default LC_ID_DYLIB is an absolute CI path, which dyld cannot load
# on another machine.
set -euo pipefail
BIN="${1:?binary}"
shift
if [ "$#" -lt 1 ]; then
  echo "usage: relocate-dylib.sh BIN DYLIB [DYLIB...]" >&2
  exit 1
fi

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

DIR=""
for DYLIB in "$@"; do
  DIR="$(cd "$(dirname "$DYLIB")" && pwd)"
  name="$(basename "$DYLIB")"
  rewrite_id "$DYLIB" "@rpath/$name"
  pattern="$(printf '%s' "$name" | sed 's/\./\\./g')"
  rewrite_dep "$BIN" "$pattern" "@rpath/$name"
  install_name_tool -add_rpath "@executable_path" "$DYLIB" 2>/dev/null || true
  install_name_tool -add_rpath "@loader_path" "$DYLIB" 2>/dev/null || true
done

install_name_tool -add_rpath "@executable_path" "$BIN" 2>/dev/null || true
install_name_tool -add_rpath "@loader_path" "$BIN" 2>/dev/null || true

if [ -n "$DIR" ] && [ -f "$DIR/libndi.dylib" ]; then
  rewrite_id "$DIR/libndi.dylib" "@rpath/libndi.dylib"
  for DYLIB in "$@"; do
    rewrite_dep "$DYLIB" "libndi" "@rpath/libndi.dylib"
  done
fi
if [ -n "$DIR" ] && [ -f "$DIR/libndi.6.dylib" ]; then
  rewrite_id "$DIR/libndi.6.dylib" "@rpath/libndi.6.dylib"
fi

require_rpath() {
  local name="$1"
  local pattern
  pattern="$(printf '%s' "$name" | sed 's/\./\\./g')"
  refs="$(otool -L "$BIN" | awk -v pat="$pattern" '$1 ~ pat { print $1 }')"
  if [ -z "$refs" ]; then
    echo "eiviz-mac does not link $name" >&2
    otool -L "$BIN" >&2
    exit 1
  fi
  while IFS= read -r old; do
    [ -z "$old" ] && continue
    if [ "$old" != "@rpath/$name" ]; then
      echo "eiviz-mac still references $name by a non-@rpath path:" >&2
      otool -L "$BIN" >&2
      exit 1
    fi
  done <<EOF
$refs
EOF
}

require_rpath "libeiviz_mixer.dylib"
if otool -L "$BIN" | grep -q 'libeiviz_remote\.dylib'; then
  require_rpath "libeiviz_remote.dylib"
fi
