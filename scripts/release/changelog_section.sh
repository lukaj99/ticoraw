#!/bin/sh
# Print the CHANGELOG.md section for one version, for use as GitHub release notes.
#
#   scripts/release/changelog_section.sh 0.2.0 [CHANGELOG.md]
#   scripts/release/changelog_section.sh v0.2.0          (a leading "v" is ignored)
#
# The section is everything between the "## [0.2.0]" heading (Keep a Changelog format)
# and the next "## " heading, without the heading itself, without link reference
# definitions ("[0.2.0]: https://...") and without leading or trailing blank lines.
# Exits 1 if the version has no section or the section is empty.
set -eu

usage() {
  echo "usage: $0 VERSION [CHANGELOG]"
}

if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then
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
changelog=${2:-CHANGELOG.md}

if [ -z "$version" ]; then
  usage >&2
  exit 2
fi
if [ ! -r "$changelog" ]; then
  echo "error: cannot read $changelog" >&2
  exit 1
fi

# awk compares strings, not regular expressions, so dots in the version match literally.
awk -v version="$version" '
  BEGIN { heading = "## [" version "]"; found = 0; in_section = 0; blanks = 0; printed = 0 }
  /^## / {
    if (in_section) { in_section = 0; exit }
    if (substr($0, 1, length(heading)) == heading) { in_section = 1; found = 1 }
    next
  }
  !in_section { next }
  /^\[[^]]*\]: / { next }
  /^[ \t]*$/ { if (printed) blanks++; next }
  {
    while (blanks > 0) { print ""; blanks-- }
    print
    printed = 1
  }
  END {
    if (!found) { printf "error: no \"## [%s]\" section in the changelog\n", version > "/dev/stderr"; exit 1 }
    if (!printed) { printf "error: the \"## [%s]\" section is empty\n", version > "/dev/stderr"; exit 1 }
  }
' "$changelog"
