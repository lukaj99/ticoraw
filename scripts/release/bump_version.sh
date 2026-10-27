#!/bin/sh
# Set the release version everywhere the repository records it, then check the result.
#
#   scripts/release/bump_version.sh [--dry-run] [--date YYYY-MM-DD] [--skip-notices] X.Y.Z
#
# Edits, whatever the old values were:
#   Cargo.toml                            [workspace.package] version
#   crates/*/Cargo.toml                   the version requirement of each path dependency on a
#                                         workspace crate (ticoraw = { path = ..., version = })
#   crates/ticoraw-py/Cargo.toml          [package] version (maturin takes the wheel version
#                                         from it; the crate is its own workspace)
#   crates/ticoraw-wasm/npm/package.json  "version" (the npm package)
#   web/package.json, web/package-lock.json  "version" of the web converter package
#   CHANGELOG.md                          moves the [Unreleased] entries under
#                                         "## [X.Y.Z] - DATE" and adds the release link
# then regenerates the three lockfiles (Cargo.lock, crates/ticoraw-py/Cargo.lock and
# fuzz/Cargo.lock; `cargo update --workspace` changes only the workspace crates' own
# entries), regenerates THIRD-PARTY-NOTICES.md (needs cargo-about) and runs
# check_version.sh vX.Y.Z.
#
#   --dry-run       print the edits as a diff and the commands it would run; change nothing
#   --date          the release date for CHANGELOG.md (default: today, UTC)
#   --skip-notices  leave THIRD-PARTY-NOTICES.md alone (regenerate it before committing:
#                   scripts/release/third_party_notices.sh)
#
# Needs cargo and jq. It does not commit, tag or push.
set -eu

usage() {
  echo "usage: $0 [--dry-run] [--date YYYY-MM-DD] [--skip-notices] X.Y.Z"
}

dry_run=0
skip_notices=0
date=$(date -u +%Y-%m-%d)
new=
while [ "$#" -gt 0 ]; do
  case "$1" in
  --dry-run) dry_run=1 ;;
  --skip-notices) skip_notices=1 ;;
  --date)
    if [ "$#" -lt 2 ]; then
      usage >&2
      exit 2
    fi
    date=$2
    shift
    ;;
  -h | --help)
    usage
    exit 0
    ;;
  -*)
    usage >&2
    exit 2
    ;;
  *)
    if [ -n "$new" ]; then
      usage >&2
      exit 2
    fi
    new=${1#v}
    ;;
  esac
  shift
done
if [ -z "$new" ]; then
  usage >&2
  exit 2
fi
if ! printf '%s\n' "$new" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'; then
  echo "error: '$new' is not of the form X.Y.Z or X.Y.Z-pre" >&2
  exit 1
fi
if ! printf '%s\n' "$date" | grep -Eq '^[0-9]{4}-[0-9]{2}-[0-9]{2}$'; then
  echo "error: --date '$date' is not of the form YYYY-MM-DD" >&2
  exit 1
fi

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cd "$root"

# The value of `key = "..."` in one TOML section of a file.
toml_value() {
  awk -v section="[$2]" -v key="$3" '
    /^\[/ { in_section = ($0 == section); next }
    in_section && index($0, key " = \"") == 1 {
      value = substr($0, length(key) + 5); sub(/".*/, "", value); print value; exit
    }
  ' "$1"
}

old=$(toml_value Cargo.toml workspace.package version)
repository=$(toml_value Cargo.toml workspace.package repository)
if [ -z "$old" ] || [ -z "$repository" ]; then
  echo "error: no version or repository in [workspace.package] of Cargo.toml" >&2
  exit 1
fi
if [ "$old" = "$new" ]; then
  echo "error: the workspace version is already $new" >&2
  exit 1
fi
if grep -Fq "## [$new]" CHANGELOG.md; then
  echo "error: CHANGELOG.md already has a section for $new" >&2
  exit 1
fi
if ! "$here/changelog_section.sh" Unreleased CHANGELOG.md >/dev/null; then
  echo "error: add the release notes under ## [Unreleased] in CHANGELOG.md first" >&2
  exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

# Every edited file is first written to $tmp/new/<path>, so a dry run can show the diff
# and a real run changes nothing until every edit has succeeded.
files=
stage() {
  mkdir -p "$tmp/new/$(dirname "$1")"
  files="$files $1"
}

# [section] version = "..." -> new.
set_toml_version() {
  stage "$1"
  awk -v section="[$2]" -v new="$new" '
    /^\[/ { in_section = ($0 == section) }
    in_section && /^version = "[^"]*"/ { sub(/"[^"]*"/, "\"" new "\"") }
    { print }
  ' "$1" >"$tmp/new/$1"
}

set_toml_version Cargo.toml workspace.package
set_toml_version crates/ticoraw-py/Cargo.toml package

# Path dependencies on workspace crates: ticoraw-x = { path = "...", version = "..." }.
# crates/ticoraw-py/Cargo.toml was already staged by set_toml_version above, so its pins
# are rewritten on that staged copy.
for manifest in crates/*/Cargo.toml; do
  if [ "$manifest" = crates/ticoraw-py/Cargo.toml ]; then
    src="$tmp/new/$manifest"
  else
    stage "$manifest"
    src=$manifest
  fi
  awk -v new="$new" '
    /^ticoraw[A-Za-z0-9_-]* = [{].*path = / { sub(/version = "[^"]*"/, "version = \"" new "\"") }
    { print }
  ' "$src" >"$tmp/new/$manifest.part"
  mv "$tmp/new/$manifest.part" "$tmp/new/$manifest"
done

# "version" right after the package's own "name" (the top level of package.json, and the
# top level and packages[""] of package-lock.json); dependencies keep their versions.
set_json_version() {
  stage "$1"
  awk -v name="$2" -v new="$new" '
    after_name && /^[ \t]*"version": "[^"]*"/ { sub(/"version": "[^"]*"/, "\"version\": \"" new "\"") }
    { after_name = ($0 ~ "^[ \t]*\"name\": \"" name "\",[ \t]*$"); print }
  ' "$1" >"$tmp/new/$1"
}

set_json_version crates/ticoraw-wasm/npm/package.json ticoraw
set_json_version web/package.json ticoraw-web
set_json_version web/package-lock.json ticoraw-web

stage CHANGELOG.md
awk -v new="$new" -v date="$date" -v link="[$new]: $repository/releases/tag/v$new" '
  !headed && $0 == "## [Unreleased]" { print; print ""; print "## [" new "] - " date; headed = 1; next }
  !linked && /^\[[0-9][^]]*\]: / { print link; linked = 1 }
  { print }
  END {
    if (!headed) { print "error: no \"## [Unreleased]\" heading in CHANGELOG.md" > "/dev/stderr"; exit 1 }
    if (!linked) { print ""; print link }
  }
' CHANGELOG.md >"$tmp/new/CHANGELOG.md"

# Fail before touching anything if an edit missed its target.
for f in crates/ticoraw-wasm/npm/package.json web/package.json; do
  if [ "$(jq -r .version "$tmp/new/$f")" != "$new" ]; then
    echo "error: could not set the version in $f" >&2
    exit 1
  fi
done
if [ "$(jq -r '[.version, .packages[""].version] | unique | join(" ")' "$tmp/new/web/package-lock.json")" != "$new" ]; then
  echo "error: could not set the version in web/package-lock.json" >&2
  exit 1
fi

lock_manifests="Cargo.toml crates/ticoraw-py/Cargo.toml fuzz/Cargo.toml"

if [ "$dry_run" -eq 1 ]; then
  echo "Would bump $old to $new:"
  for f in $files; do
    diff -u --label "a/$f" --label "b/$f" "$f" "$tmp/new/$f" || true
  done
  for m in $lock_manifests; do
    echo "Would run: cargo update --workspace --offline --manifest-path $m"
  done
  if [ "$skip_notices" -eq 0 ]; then
    echo "Would run: scripts/release/third_party_notices.sh"
  fi
  echo "Would run: scripts/release/check_version.sh v$new"
  exit 0
fi

if [ "$skip_notices" -eq 0 ] && ! command -v cargo-about >/dev/null 2>&1; then
  echo "error: cargo-about is needed to regenerate THIRD-PARTY-NOTICES.md; install it with" >&2
  echo "  cargo install --locked --features cli cargo-about@0.9.3" >&2
  echo "or pass --skip-notices and regenerate the file before committing" >&2
  exit 1
fi

for f in $files; do
  if ! cmp -s "$f" "$tmp/new/$f"; then
    cat "$tmp/new/$f" >"$f"
    echo "updated $f"
  fi
done

# Only the workspace crates' own lock entries change, so this works offline; fall back to
# the network if the local registry cache is incomplete.
for m in $lock_manifests; do
  cargo update --workspace --quiet --offline --manifest-path "$m" ||
    cargo update --workspace --quiet --manifest-path "$m"
  echo "updated ${m%Cargo.toml}Cargo.lock"
done

if [ "$skip_notices" -eq 0 ]; then
  "$here/third_party_notices.sh"
else
  echo "warning: THIRD-PARTY-NOTICES.md not regenerated; run scripts/release/third_party_notices.sh" >&2
fi

"$here/check_version.sh" "v$new" >/dev/null
echo "Bumped $old to $new. Review the diff, then commit it as the release commit."
