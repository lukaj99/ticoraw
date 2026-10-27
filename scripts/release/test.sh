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

# bump_version.sh and check_version.sh, on a miniature copy of the repository layout: the
# main workspace, the separate crates/ticoraw-py and fuzz workspaces with their own
# lockfiles, the npm and web package files and the changelog. No registry dependencies,
# so cargo runs offline.
if command -v cargo >/dev/null 2>&1 && command -v jq >/dev/null 2>&1; then
  repo=$tmp/repo
  mkdir -p "$repo/scripts/release" "$repo/crates/ticoraw-wasm/npm" "$repo/web"
  for s in bump_version.sh check_version.sh changelog_section.sh; do
    cp "$here/$s" "$repo/scripts/release/$s"
  done
  crate() { # crate DIR NAME VERSION-LINE [EXTRA-TOML]
    mkdir -p "$repo/$1/src"
    : >"$repo/$1/src/lib.rs"
    printf '[package]\nname = "%s"\n%s\nedition = "2021"\n%s' "$2" "$3" "${4-}" >"$repo/$1/Cargo.toml"
  }
  cat >"$repo/Cargo.toml" <<'EOF'
[workspace]
resolver = "2"
members = ["crates/ticoraw", "crates/ticoraw-dng"]

[workspace.package]
version = "0.1.0"
repository = "https://github.com/lukaj99/ticoraw"
EOF
  crate crates/ticoraw ticoraw 'version.workspace = true'
  crate crates/ticoraw-dng ticoraw-dng 'version.workspace = true' '
[dependencies]
ticoraw = { path = "../ticoraw", version = "0.1.0", default-features = false }
'
  crate crates/ticoraw-py ticoraw-py 'version = "0.1.0"' '
[workspace]
members = ["."]

[dependencies]
ticoraw = { path = "../ticoraw" }
ticoraw-dng = { path = "../ticoraw-dng" }
'
  crate fuzz ticoraw-fuzz 'version = "0.0.0"' '
[workspace]
members = ["."]

[dependencies]
ticoraw = { path = "../crates/ticoraw" }
'
  for m in Cargo.toml crates/ticoraw-py/Cargo.toml fuzz/Cargo.toml; do
    cargo generate-lockfile --quiet --offline --manifest-path "$repo/$m"
  done
  printf '{\n  "name": "ticoraw",\n  "version": "0.1.0",\n  "private": true\n}\n' \
    >"$repo/crates/ticoraw-wasm/npm/package.json"
  printf '{\n  "name": "ticoraw-web",\n  "version": "0.1.0",\n  "devDependencies": {\n    "dep": "0.1.0"\n  }\n}\n' \
    >"$repo/web/package.json"
  cat >"$repo/web/package-lock.json" <<'EOF'
{
  "name": "ticoraw-web",
  "version": "0.1.0",
  "lockfileVersion": 3,
  "packages": {
    "": {
      "name": "ticoraw-web",
      "version": "0.1.0",
      "devDependencies": {
        "dep": "0.1.0"
      }
    },
    "node_modules/dep": {
      "version": "0.1.0"
    }
  }
}
EOF
  cat >"$repo/CHANGELOG.md" <<'EOF'
# Changelog

## [Unreleased]

### Fixed

- Something.

## [0.1.0] - 2026-10-27

- First.

[0.1.0]: https://github.com/lukaj99/ticoraw/releases/tag/v0.1.0
EOF

  bump=$repo/scripts/release/bump_version.sh
  check=$repo/scripts/release/check_version.sh
  if [ "$("$check" v0.1.0 2>"$tmp/err")" != 0.1.0 ]; then
    fail "check_version.sh v0.1.0 on the fixture: $(cat "$tmp/err")"
  fi
  if "$check" v0.2.0 >/dev/null 2>&1; then
    fail "check_version.sh v0.2.0 passed before the bump"
  fi

  snapshot() { (cd "$repo" && find . -type f ! -path './target/*' -exec cksum {} + | sort); }
  snapshot >"$tmp/before"
  if ! "$bump" --dry-run --skip-notices 0.2.0 >"$tmp/dry" 2>"$tmp/err"; then
    fail "bump_version.sh --dry-run failed: $(cat "$tmp/err")"
  fi
  snapshot >"$tmp/after"
  if ! cmp -s "$tmp/before" "$tmp/after"; then
    fail "bump_version.sh --dry-run changed files"
  fi
  for f in Cargo.toml crates/ticoraw-dng/Cargo.toml crates/ticoraw-py/Cargo.toml \
    crates/ticoraw-wasm/npm/package.json web/package.json web/package-lock.json CHANGELOG.md; do
    grep -Fqx "+++ b/$f" "$tmp/dry" || fail "bump_version.sh --dry-run shows no edit of $f"
  done

  if ! "$bump" --skip-notices --date 2026-11-01 v0.2.0 >/dev/null 2>"$tmp/err"; then
    fail "bump_version.sh 0.2.0 failed: $(cat "$tmp/err")"
  fi
  if [ "$("$check" v0.2.0 2>"$tmp/err")" != 0.2.0 ]; then
    fail "check_version.sh v0.2.0 after the bump: $(cat "$tmp/err")"
  fi
  printf '### Fixed\n\n- Something.\n' >"$tmp/want-bumped"
  if ! "$here/changelog_section.sh" 0.2.0 "$repo/CHANGELOG.md" | cmp -s - "$tmp/want-bumped"; then
    fail "bump_version.sh did not move the Unreleased entries under 0.2.0"
  fi
  grep -Fqx '## [0.2.0] - 2026-11-01' "$repo/CHANGELOG.md" || fail "no dated 0.2.0 heading"
  grep -Fqx '## [Unreleased]' "$repo/CHANGELOG.md" || fail "the Unreleased heading is gone"
  grep -Fqx '[0.2.0]: https://github.com/lukaj99/ticoraw/releases/tag/v0.2.0' "$repo/CHANGELOG.md" ||
    fail "no 0.2.0 link"
  if [ "$(jq -r '.packages["node_modules/dep"].version' "$repo/web/package-lock.json")" != 0.1.0 ]; then
    fail "bump_version.sh changed a dependency version in web/package-lock.json"
  fi
  if "$bump" --skip-notices 0.2.0 >/dev/null 2>&1; then
    fail "bump_version.sh to the current version should fail"
  fi
  if "$bump" --skip-notices 0.3.0 >/dev/null 2>&1; then
    fail "bump_version.sh with an empty Unreleased section should fail"
  fi

  # Each place a release can miss must fail check_version.sh on its own.
  cp -R "$repo" "$tmp/bumped"
  expect_check_fails() { # DESCRIPTION FILE SED-SCRIPT
    rm -rf "$repo"
    cp -R "$tmp/bumped" "$repo"
    sed "$3" "$repo/$2" >"$tmp/edited"
    cat "$tmp/edited" >"$repo/$2"
    if cmp -s "$repo/$2" "$tmp/bumped/$2"; then
      fail "fixture edit for '$1' changed nothing"
    elif "$check" v0.2.0 >/dev/null 2>&1; then
      fail "check_version.sh missed $1"
    fi
  }
  expect_check_fails "a stale Python crate version" crates/ticoraw-py/Cargo.toml \
    's/^version = "0.2.0"/version = "0.1.0"/'
  expect_check_fails "a stale Python Cargo.lock" crates/ticoraw-py/Cargo.lock \
    's/^version = "0.2.0"/version = "0.1.0"/'
  expect_check_fails "a stale root Cargo.lock" Cargo.lock 's/^version = "0.2.0"/version = "0.1.0"/'
  expect_check_fails "a stale fuzz Cargo.lock" fuzz/Cargo.lock 's/^version = "0.2.0"/version = "0.1.0"/'
  expect_check_fails "a stale internal dependency" crates/ticoraw-dng/Cargo.toml \
    's/version = "0.2.0"/version = "0.1.0"/'
  expect_check_fails "a stale npm package.json" crates/ticoraw-wasm/npm/package.json \
    's/"0.2.0"/"0.1.0"/'
  expect_check_fails "a stale web package.json" web/package.json 's/"version": "0.2.0"/"version": "0.1.0"/'
  expect_check_fails "a stale web package-lock.json" web/package-lock.json \
    's/^      "version": "0.2.0"/      "version": "0.1.0"/'
else
  echo "skipping the bump_version.sh tests: cargo or jq not found" >&2
fi

if [ "$failures" -ne 0 ]; then
  echo "$failures release-script test(s) failed" >&2
  exit 1
fi
echo "release-script tests passed"
