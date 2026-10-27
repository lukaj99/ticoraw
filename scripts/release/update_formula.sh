#!/bin/sh
# Point the Homebrew formula at a release: set its `url` to the GitHub tag tarball and
# its `sha256` to that tarball's digest.
#
#   scripts/release/update_formula.sh 0.2.0                      download the tag tarball
#   scripts/release/update_formula.sh 0.2.0 --tarball FILE       hash a local copy instead
#   scripts/release/update_formula.sh 0.2.0 --formula PATH       edit another formula file
#
# The tag must already be pushed for the download to work. The formula builds from
# source; this script never touches compiled binaries.
set -eu

repo_url=https://github.com/lukaj99/ticoraw

usage() {
  echo "usage: $0 VERSION [--tarball FILE] [--formula PATH]"
}

if [ "$#" -lt 1 ]; then
  usage >&2
  exit 2
fi
case "$1" in
-h | --help)
  usage
  exit 0
  ;;
esac
version=${1#v}
shift

root=$(cd "$(dirname "$0")/../.." && pwd)
formula="$root/packaging/homebrew/ticoraw.rb"
tarball=
while [ "$#" -gt 0 ]; do
  case "$1" in
  --tarball)
    [ "$#" -ge 2 ] || {
      usage >&2
      exit 2
    }
    tarball=$2
    shift 2
    ;;
  --formula)
    [ "$#" -ge 2 ] || {
      usage >&2
      exit 2
    }
    formula=$2
    shift 2
    ;;
  *)
    usage >&2
    exit 2
    ;;
  esac
done

if ! printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'; then
  echo "error: '$version' is not a version (X.Y.Z or X.Y.Z-pre)" >&2
  exit 1
fi
if [ ! -f "$formula" ]; then
  echo "error: no formula at $formula" >&2
  exit 1
fi

url="$repo_url/archive/refs/tags/v$version.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

if [ -z "$tarball" ]; then
  tarball="$tmp/ticoraw-$version.tar.gz"
  echo "downloading $url" >&2
  curl --fail --silent --show-error --location --proto '=https' --output "$tarball" "$url"
fi
if [ ! -f "$tarball" ]; then
  echo "error: no tarball at $tarball" >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  sha256=$(sha256sum "$tarball" | awk '{print $1}')
else
  sha256=$(shasum -a 256 "$tarball" | awk '{print $1}')
fi

# Rewrite only the first top-level `url` and `sha256` lines (two-space indent), so the
# `head` line and anything nested are left alone. sed -i differs between GNU and BSD,
# so write a copy and move it into place.
awk -v url="$url" -v sha="$sha256" '
  !done_url && /^  url "/ { print "  url \"" url "\""; done_url = 1; next }
  !done_sha && /^  sha256 "/ { print "  sha256 \"" sha "\""; done_sha = 1; next }
  { print }
  END { if (!done_url || !done_sha) exit 1 }
' "$formula" >"$tmp/formula.rb" || {
  echo "error: $formula has no top-level url or sha256 line" >&2
  exit 1
}
cat "$tmp/formula.rb" >"$formula"

echo "$formula: url $url" >&2
echo "$formula: sha256 $sha256" >&2
