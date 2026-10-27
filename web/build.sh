#!/bin/sh
# Builds the web converter into web/dist, or into the directory given:
#
#   index.html, icon.svg, src/   the page (plain HTML, CSS and ES modules, no bundler)
#   pkg/web/                     crates/ticoraw-wasm built with wasm-pack --target web
#   pkg/worker.js                the Web Worker pool from crates/ticoraw-wasm/js
#   LICENSE-*, THIRD-PARTY-NOTICES.md
#
# Every URL in the page is relative, so the folder works from any path on a static host.
#
# PATENT GATE: the output contains the compiled decoder. Do not publish or deploy it, and do
# not upload it as a CI artifact, unless the repository variable ALLOW_BINARY_DISTRIBUTION
# is 'true' (pending patent-licensing review; see the README's patent notice).
#
# Environment:
#   WASM_PACK   wasm-pack binary (default: wasm-pack on PATH)
#   BASE_PATH   where the site's files live, ending in / (default ./: next to the page,
#               which needs no <base> element). Set an absolute path such as
#               /ticoraw/convert/ only if the page is also served from a URL without the
#               trailing slash; build.sh then adds <base href="$BASE_PATH">.
#   DOCS_URL    where "More questions and help" points (default: SUPPORT.md on GitHub)
#   BUILD_ID    shown in the footer (default: the short git commit, or "local")
set -eu

web=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
root=$(dirname -- "$web")
crate=$root/crates/ticoraw-wasm
out=${1:-$web/dist}
wasm_pack=${WASM_PACK:-wasm-pack}
base=${BASE_PATH:-./}
docs=${DOCS_URL:-https://github.com/lukaj99/ticoraw/blob/main/SUPPORT.md}
build=${BUILD_ID:-$(git -C "$root" rev-parse --short HEAD 2>/dev/null || echo local)}

die() {
  echo "build.sh: $*" >&2
  exit 1
}

# These end up in index.html through sed: allow only characters that are safe in both.
case $base in
*/) ;;
*) die "BASE_PATH must end with /" ;;
esac
case $base in
*[!A-Za-z0-9._~/-]*) die "BASE_PATH may contain only letters, digits and . _ ~ / -" ;;
esac
case $docs in
https://*) ;;
*) die "DOCS_URL must start with https://" ;;
esac
case $docs in
*[!A-Za-z0-9._~/:#?=%+-]*) die "DOCS_URL contains a character this script does not allow" ;;
esac
case $build in
*[!A-Za-z0-9._-]*) die "BUILD_ID may contain only letters, digits and . _ -" ;;
esac

# Only ever replace a directory this script made. A file or a symbolic link in its place is
# refused too (find lists nothing inside a file, so it would pass the emptiness check below).
if [ -L "$out" ]; then
  die "$out is a symbolic link; pass the directory itself"
fi
if [ -e "$out" ] && [ ! -d "$out" ]; then
  die "$out exists and is not a directory; remove it or pick another directory"
fi
if [ -e "$out" ] && [ ! -f "$out/.ticoraw-web" ]; then
  if [ -n "$(find "$out" -mindepth 1 -maxdepth 1 2>/dev/null | head -n 1)" ]; then
    die "$out exists and was not made by build.sh; remove it or pick another directory"
  fi
fi
rm -rf "$out"
mkdir -p "$out/src" "$out/pkg"
out=$(CDPATH='' cd -- "$out" && pwd)
: >"$out/.ticoraw-web"

export CARGO_PROFILE_RELEASE_OPT_LEVEL="${CARGO_PROFILE_RELEASE_OPT_LEVEL:-3}"
"$wasm_pack" build --release --no-pack --target web \
  --out-dir "$out/pkg/web" --out-name ticoraw "$crate"
rm -f "$out/pkg/web/.gitignore"
# package.json makes Node read pkg/ as ES modules (the end-to-end tests use that).
cp "$crate/js/worker.js" "$crate/js/index.d.ts" "$crate/npm/package.json" "$out/pkg/"

cp "$web/src/app.js" "$web/src/core.js" "$web/src/files.js" "$web/src/zip.js" \
  "$web/src/style.css" "$out/src/"
cp "$web/icon.svg" "$root/LICENSE-MIT" "$root/LICENSE-APACHE" \
  "$root/THIRD-PARTY-NOTICES.md" "$out/"
if [ "$base" = ./ ]; then
  base_tag=''
else
  base_tag="<base href=\"$base\" />"
fi
sed -e "s|^\( *\)<!-- %BASE%.*-->$|\1$base_tag|" \
  -e "s|%DOCS_URL%|$docs|" \
  -e "s|%BUILD%|$build|" \
  "$web/index.html" >"$out/index.html"

wasm="$out/pkg/web/ticoraw_bg.wasm"
echo "build.sh: $out (ticoraw_bg.wasm: $(wc -c <"$wasm" | tr -d ' ') bytes; build $build)"
