#!/bin/sh
# Check that a release tag matches the workspace, then print the version.
#
#   scripts/release/check_version.sh v0.2.0
#
# Fails unless the tag is vMAJOR.MINOR.PATCH (with an optional -prerelease), every
# workspace crate has that version, every dependency between workspace crates asks for
# exactly that version ("^0.2.0", as `cargo publish` needs), and CHANGELOG.md has a
# section for it. Needs cargo and jq.
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

metadata=$(cargo metadata --no-deps --format-version 1 --locked)
status=0

mismatched=$(printf '%s' "$metadata" | jq -r --arg v "$version" \
  '.packages[] | select(.version != $v) | "\(.name) \(.version)"')
if [ -n "$mismatched" ]; then
  echo "error: workspace crates whose version is not $version:" >&2
  printf '%s\n' "$mismatched" | sed 's/^/  /' >&2
  status=1
fi

stale=$(printf '%s' "$metadata" | jq -r --arg req "^$version" '
  [.packages[].name] as $members
  | .packages[] | .name as $from
  | .dependencies[]
  | select(.kind == null and (.name as $n | $members | index($n)) and .req != $req)
  | "\($from) -> \(.name) \(.req)"')
if [ -n "$stale" ]; then
  echo "error: workspace dependencies that do not ask for ^$version:" >&2
  printf '%s\n' "$stale" | sed 's/^/  /' >&2
  status=1
fi

if ! "$here/changelog_section.sh" "$version" CHANGELOG.md >/dev/null; then
  status=1
fi

if [ "$status" -ne 0 ]; then
  exit "$status"
fi
printf '%s\n' "$version"
