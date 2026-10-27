#!/bin/sh
# Assembles the npm package `ticoraw` in crates/ticoraw-wasm/pkg, or in the directory given:
#
#   web/        wasm-pack --target web: browsers, Web Workers, Deno and bundlers (ESM)
#   node/       wasm-pack --target nodejs: Node.js (CommonJS, loads the .wasm synchronously)
#   worker.js   the Web Worker pool, with its types in index.d.ts
#   package.json, README.md, LICENSE-MIT, LICENSE-APACHE
#
# The package is built and tested but NOT published: compiled decoders stay unpublished
# pending patent-licensing review (see README.md). package.json is "private" so that a stray
# `npm publish` fails.
#
# Environment:
#   WASM_PACK                         wasm-pack binary (default: wasm-pack on PATH)
#   CARGO_PROFILE_RELEASE_OPT_LEVEL   default 3 (README.md compares it with "s")
#
# No LTO: cargo does not apply it to a crate that is also an rlib (the wasm-bindgen tests
# link it as one), and a fat-LTO cdylib measured 5% larger and no faster.
set -eu

crate=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
out=${1:-$crate/pkg}
wasm_pack=${WASM_PACK:-wasm-pack}
export CARGO_PROFILE_RELEASE_OPT_LEVEL="${CARGO_PROFILE_RELEASE_OPT_LEVEL:-3}"

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$crate/../../Cargo.toml" | head -n 1)
if ! grep -q "\"version\": \"$version\"" "$crate/npm/package.json"; then
  echo "build.sh: npm/package.json must have the workspace version $version" >&2
  exit 1
fi

mkdir -p "$out"
out=$(CDPATH='' cd -- "$out" && pwd)

for target in web nodejs; do
  case $target in
  web) dir=web ;;
  *) dir=node ;;
  esac
  "$wasm_pack" build --release --no-pack --target "$target" \
    --out-dir "$out/$dir" --out-name ticoraw "$crate"
  # wasm-pack writes a catch-all .gitignore, which would also hide the files from npm.
  rm -f "$out/$dir/.gitignore"
done

# The root package is "type": "module"; the nodejs glue is CommonJS.
printf '{ "type": "commonjs" }\n' >"$out/node/package.json"

cp "$crate/npm/package.json" "$crate/README.md" "$crate/LICENSE-MIT" \
  "$crate/LICENSE-APACHE" "$crate/js/worker.js" "$crate/js/index.d.ts" "$out/"

wasm="$out/web/ticoraw_bg.wasm"
raw=$(wc -c <"$wasm" | tr -d ' ')
gz=$(gzip -9 -c "$wasm" | wc -c | tr -d ' ')
echo "build.sh: $out (ticoraw_bg.wasm: $raw bytes, $gz gzipped;" \
  "opt-level $CARGO_PROFILE_RELEASE_OPT_LEVEL)"
