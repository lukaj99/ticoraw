#!/bin/sh
# Regenerate THIRD-PARTY-NOTICES.md from the dependency graph with cargo-about, using
# packaging/about/about.toml and packaging/about/about.hbs.
#
#   scripts/release/third_party_notices.sh           rewrite THIRD-PARTY-NOTICES.md
#   scripts/release/third_party_notices.sh --check   exit 1 if it is out of date
#
# Needs cargo-about (cargo install --locked --features cli cargo-about@0.9.3). It runs
# offline after `cargo fetch`, so the output depends only on Cargo.lock and the config.
set -eu

about_version=0.9.3

usage() {
  echo "usage: $0 [--check]"
}

check=0
case "${1-}" in
"") ;;
--check) check=1 ;;
-h | --help)
  usage
  exit 0
  ;;
*)
  usage >&2
  exit 2
  ;;
esac

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"

if ! command -v cargo-about >/dev/null 2>&1; then
  echo "error: cargo-about not found; install it with:" >&2
  echo "  cargo install --locked --features cli cargo-about@$about_version" >&2
  exit 1
fi
found=$(cargo-about --version | awk '{print $2}')
if [ "$found" != "$about_version" ]; then
  echo "warning: cargo-about $found, expected $about_version; the output may differ" >&2
fi

tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT INT TERM

cargo fetch --locked --quiet
cargo about generate --workspace --locked --offline --fail \
  -c packaging/about/about.toml -o "$tmp" packaging/about/about.hbs

if [ "$check" -eq 1 ]; then
  if cmp -s "$tmp" THIRD-PARTY-NOTICES.md; then
    echo "THIRD-PARTY-NOTICES.md is up to date"
  else
    echo "error: THIRD-PARTY-NOTICES.md is out of date; run $0 and commit the result" >&2
    diff -u THIRD-PARTY-NOTICES.md "$tmp" >&2 || true
    exit 1
  fi
else
  cat "$tmp" >THIRD-PARTY-NOTICES.md
  echo "wrote THIRD-PARTY-NOTICES.md"
fi
