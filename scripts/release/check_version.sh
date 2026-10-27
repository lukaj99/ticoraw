#!/bin/sh
# Check that a release tag matches the repository, then print the version.
#
#   scripts/release/check_version.sh v0.2.0
#
# Fails unless the tag is vMAJOR.MINOR.PATCH (with an optional -prerelease) and:
# - every crate of the main workspace has that version, and every dependency between
#   workspace crates asks for exactly that version ("^0.2.0", as `cargo publish` needs);
# - the Python package (crates/ticoraw-py, its own workspace, whose version maturin puts on
#   the wheel) has that version;
# - Cargo.lock, crates/ticoraw-py/Cargo.lock and fuzz/Cargo.lock are up to date
#   (CI builds with --locked);
# - crates/ticoraw-wasm/npm/package.json, web/package.json and web/package-lock.json have
#   that version;
# - CHANGELOG.md has a section for it.
# scripts/release/bump_version.sh makes all of these edits. Needs cargo and jq.
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: $0 TAG" >&2
  exit 2
fi
tag=$1
here=$(cd "$(dirname "$0")" && pwd)
cd "$here/../.."

if ! printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'; then
  echo "error: tag '$tag' is not of the form vX.Y.Z or vX.Y.Z-pre" >&2
  exit 1
fi
version=${tag#v}
status=0

# cargo metadata of a workspace, failing if its Cargo.lock is out of date.
locked_metadata() {
  if ! cargo metadata --format-version 1 --locked --manifest-path "$1"; then
    echo "error: $(dirname "$1")/Cargo.lock is out of date (run scripts/release/bump_version.sh, or cargo update --workspace --manifest-path $1)" >&2
    return 1
  fi
}

# The workspace's own packages, without its dependencies.
# shellcheck disable=SC2016 # jq variables, not shell ones
members='.workspace_members as $m | [.packages[] | select(.id as $id | $m | index($id))]'

if metadata=$(locked_metadata Cargo.toml); then
  mismatched=$(printf '%s' "$metadata" | jq -r --arg v "$version" \
    "$members"' | .[] | select(.version != $v) | "\(.name) \(.version)"')
  if [ -n "$mismatched" ]; then
    echo "error: workspace crates whose version is not $version:" >&2
    printf '%s\n' "$mismatched" | sed 's/^/  /' >&2
    status=1
  fi

  stale=$(printf '%s' "$metadata" | jq -r --arg req "^$version" "$members"'
    | [.[].name] as $names
    | .[] | .name as $from
    | .dependencies[]
    | select(.kind == null and (.name as $n | $names | index($n)) and .req != $req)
    | "\($from) -> \(.name) \(.req)"')
  if [ -n "$stale" ]; then
    echo "error: workspace dependencies that do not ask for ^$version:" >&2
    printf '%s\n' "$stale" | sed 's/^/  /' >&2
    status=1
  fi
else
  status=1
fi

if metadata=$(locked_metadata crates/ticoraw-py/Cargo.toml); then
  py=$(printf '%s' "$metadata" | jq -r "$members"' | .[] | select(.name == "ticoraw-py") | .version')
  if [ "$py" != "$version" ]; then
    echo "error: crates/ticoraw-py/Cargo.toml has version '$py', not $version" >&2
    status=1
  fi
else
  status=1
fi

# The fuzz targets are never released (version 0.0.0); only their lockfile must be current.
if ! locked_metadata fuzz/Cargo.toml >/dev/null; then
  status=1
fi

for f in crates/ticoraw-wasm/npm/package.json web/package.json; do
  found=$(jq -r .version "$f")
  if [ "$found" != "$version" ]; then
    echo "error: $f has version '$found', not $version" >&2
    status=1
  fi
done
found=$(jq -r '[.version, .packages[""].version] | unique | join(" ")' web/package-lock.json)
if [ "$found" != "$version" ]; then
  echo "error: web/package-lock.json has version '$found', not $version" >&2
  status=1
fi

if ! "$here/changelog_section.sh" "$version" CHANGELOG.md >/dev/null; then
  status=1
fi

if [ "$status" -ne 0 ]; then
  exit "$status"
fi
printf '%s\n' "$version"
