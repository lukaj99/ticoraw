#!/bin/sh
# Regenerate crates/ticoraw-py/THIRD-PARTY-NOTICES.md, the notices shipped in every wheel
# and sdist, from this crate's own Cargo.lock with cargo-about. It reuses the main
# workspace's packaging/about/about.toml (accepted licences, the jpeg-encoder IJG
# clarification) with the template crates/ticoraw-py/about.hbs.
#
#   crates/ticoraw-py/scripts/third_party_notices.sh           rewrite the file
#   crates/ticoraw-py/scripts/third_party_notices.sh --check   exit 1 if it is out of date
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

crate=$(cd "$(dirname "$0")/.." && pwd)
root=$(cd "$crate/../.." && pwd)
cd "$crate"
out=THIRD-PARTY-NOTICES.md

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

cargo fetch --manifest-path Cargo.toml --locked --quiet
cargo about generate --manifest-path Cargo.toml --workspace --locked --offline --fail \
  -c "$root/packaging/about/about.toml" -o "$tmp" about.hbs

if [ "$check" -eq 1 ]; then
  if cmp -s "$tmp" "$out"; then
    echo "crates/ticoraw-py/$out is up to date"
  else
    echo "error: crates/ticoraw-py/$out is out of date; run $0 and commit the result" >&2
    diff -u "$out" "$tmp" >&2 || true
    exit 1
  fi
else
  cat "$tmp" >"$out"
  echo "wrote crates/ticoraw-py/$out"
fi
