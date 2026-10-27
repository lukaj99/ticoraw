#!/bin/sh
# Tests for the release scripts. Run from anywhere: scripts/release/test.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
failures=0

fail() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}

# Expect "$2" (a file) to equal the output of changelog_section.sh for version "$1".
expect_section() {
  if ! "$here/changelog_section.sh" "$1" "$tmp/CHANGELOG.md" >"$tmp/out" 2>"$tmp/err"; then
    fail "changelog_section.sh $1 exited non-zero: $(cat "$tmp/err")"
  elif ! cmp -s "$tmp/out" "$2"; then
    fail "changelog_section.sh $1 printed:"
    diff -u "$2" "$tmp/out" >&2 || true
  fi
}

expect_error() {
  if "$here/changelog_section.sh" "$1" "$tmp/CHANGELOG.md" >/dev/null 2>&1; then
    fail "changelog_section.sh $1 should have failed"
  fi
}

cat >"$tmp/CHANGELOG.md" <<'EOF'
# Changelog

## [Unreleased]

### Added

- Not released yet.

## [0.10.0] - 2027-01-02

### Fixed

- Ten.

## [0.2.0] - 2026-12-01


### Added

- Two, with a [link][0.2.0].

### Changed

- Two again.


## [0.1.0] - 2026-10-27

## [0.0.1] - 2026-01-01

- Oldest.

[0.2.0]: https://github.com/lukaj99/ticoraw/releases/tag/v0.2.0
[0.1.0]: https://github.com/lukaj99/ticoraw/releases/tag/v0.1.0
EOF

cat >"$tmp/want-0.2.0" <<'EOF'
### Added

- Two, with a [link][0.2.0].

### Changed

- Two again.
EOF
expect_section 0.2.0 "$tmp/want-0.2.0"
expect_section v0.2.0 "$tmp/want-0.2.0"

printf '### Fixed\n\n- Ten.\n' >"$tmp/want-0.10.0"
expect_section 0.10.0 "$tmp/want-0.10.0"

# The last section stops before the link reference definitions.
printf -- '- Oldest.\n' >"$tmp/want-0.0.1"
expect_section 0.0.1 "$tmp/want-0.0.1"

printf '### Added\n\n- Not released yet.\n' >"$tmp/want-unreleased"
expect_section Unreleased "$tmp/want-unreleased"

expect_error 0.1.0 # empty section
expect_error 0.3.0 # no section
expect_error 0.2   # prefix of 0.2.0 and 0.20.x must not match
expect_error 0.1.0.1

# The real changelog must have a section for the current workspace version.
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$here/../../Cargo.toml" | head -n 1)
if ! "$here/changelog_section.sh" "$version" "$here/../../CHANGELOG.md" >/dev/null; then
  fail "CHANGELOG.md has no section for the workspace version $version"
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures release-script test(s) failed" >&2
  exit 1
fi
echo "release-script tests passed"
