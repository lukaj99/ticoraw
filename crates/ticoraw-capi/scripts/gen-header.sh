#!/usr/bin/env bash
# Regenerates include/ticoraw.h from the crate with the pinned cbindgen version.
#
#   crates/ticoraw-capi/scripts/gen-header.sh          # rewrite the header
#   crates/ticoraw-capi/scripts/gen-header.sh --check  # fail if the committed header is stale
#
# Install the pinned tool with: cargo install cbindgen --version 0.29.4 --locked
set -euo pipefail

CBINDGEN_VERSION=0.29.4
crate_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
header="$crate_dir/include/ticoraw.h"
cbindgen=${CBINDGEN:-cbindgen}

found=$("$cbindgen" --version 2>/dev/null | awk '{print $2}') || true
if [ "$found" != "$CBINDGEN_VERSION" ]; then
  echo "gen-header: need cbindgen $CBINDGEN_VERSION, found '${found:-none}'" >&2
  echo "  cargo install cbindgen --version $CBINDGEN_VERSION --locked" >&2
  exit 2
fi

out=$header
if [ "${1:-}" = "--check" ]; then
  out=$(mktemp)
  trap 'rm -f "$out"' EXIT
fi

(cd "$crate_dir" && "$cbindgen" --quiet --config cbindgen.toml --crate ticoraw-capi --output "$out")

if [ "${1:-}" = "--check" ]; then
  if ! diff -u "$header" "$out"; then
    echo "gen-header: include/ticoraw.h is stale; run crates/ticoraw-capi/scripts/gen-header.sh" >&2
    exit 1
  fi
  echo "gen-header: include/ticoraw.h is up to date"
fi
