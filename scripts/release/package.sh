#!/bin/sh
# Package a built `ticoraw` binary as a release archive and print the archive's path.
#
#   scripts/release/package.sh VERSION TARGET BINARY OUT_DIR
#   scripts/release/package.sh 0.2.0 x86_64-unknown-linux-gnu target/x86_64-unknown-linux-gnu/release/ticoraw dist
#
# The archive is OUT_DIR/ticoraw-VERSION-TARGET.tar.gz (.zip for Windows targets) and
# holds one directory, ticoraw-VERSION-TARGET/, with the binary, README.md, LICENSE-MIT,
# LICENSE-APACHE and THIRD-PARTY-NOTICES.md.
#
# Building and packaging is fine; publishing the archives is not, pending
# patent-licensing review. See packaging/README.md.
set -eu

if [ "$#" -ne 4 ]; then
  echo "usage: $0 VERSION TARGET BINARY OUT_DIR" >&2
  exit 2
fi
version=$1
target=$2
binary=$3
out_dir=$4

root=$(cd "$(dirname "$0")/../.." && pwd)
name="ticoraw-$version-$target"

if [ ! -f "$binary" ]; then
  echo "error: no binary at $binary" >&2
  exit 1
fi
for doc in README.md LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES.md; do
  if [ ! -f "$root/$doc" ]; then
    echo "error: $doc is missing from the repository root" >&2
    exit 1
  fi
done

mkdir -p "$out_dir"
out_dir=$(cd "$out_dir" && pwd)
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT INT TERM
mkdir "$stage/$name"

case "$binary" in
*.exe) exe=ticoraw.exe ;;
*) exe=ticoraw ;;
esac
cp "$binary" "$stage/$name/$exe"
chmod 755 "$stage/$name/$exe"
for doc in README.md LICENSE-MIT LICENSE-APACHE THIRD-PARTY-NOTICES.md; do
  cp "$root/$doc" "$stage/$name/$doc"
  chmod 644 "$stage/$name/$doc"
done

case "$target" in
*windows*)
  archive="$out_dir/$name.zip"
  rm -f "$archive"
  if command -v 7z >/dev/null 2>&1; then
    (cd "$stage" && 7z a -tzip -bso0 -bsp0 "$archive" "$name")
  elif command -v zip >/dev/null 2>&1; then
    (cd "$stage" && zip -qr -X "$archive" "$name")
  else
    echo "error: neither 7z nor zip is installed" >&2
    exit 1
  fi
  ;;
*)
  archive="$out_dir/$name.tar.gz"
  (cd "$stage" && tar -cf - "$name" | gzip -9n >"$archive")
  ;;
esac

printf '%s\n' "$archive"
