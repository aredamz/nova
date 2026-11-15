#!/usr/bin/env bash
# No-root alternative to `apt install libfontconfig1-dev libfreetype-dev`:
# create the unversioned .so symlinks the linker wants, inside the repo.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p .link-shim
for pair in "libfreetype.so:libfreetype.so.6" "libfontconfig.so:libfontconfig.so.1"; do
  want="${pair%%:*}"; have="${pair##*:}"
  src="$(ldconfig -p | awk -v n="$have" '$1==n {print $NF; exit}')"
  [[ -n "$src" ]] || { echo "missing runtime library $have (install fontconfig/freetype)"; exit 1; }
  ln -sf "$src" ".link-shim/$want"
  echo "$want -> $src"
done
