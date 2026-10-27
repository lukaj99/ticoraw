# Packaging and releases

This folder is for maintainers. It has the release procedure, the package-manager
manifests and the config used to generate `THIRD-PARTY-NOTICES.md`.

| Path | What it is | Status |
|---|---|---|
| `homebrew/ticoraw.rb` | Homebrew formula that **builds from source** | Usable: a source build ships no compiled decoder |
| `scoop/ticoraw.json` | Scoop manifest for the Windows binary archives | **Template, gated** |
| `winget/*.yaml` | winget manifests (version, installer, locale) for the Windows binary archives | **Template, gated** |
| `about/about.toml`, `about/about.hbs` | cargo-about config and template for `THIRD-PARTY-NOTICES.md` | Used by `scripts/release/third_party_notices.sh` |

The scripts live in `scripts/release/`, the workflows in `.github/workflows/release.yml`
(releases) and `.github/workflows/deny.yml` (cargo-deny, notices, script checks).

## The patent gate

JPEG XS (ISO/IEC 21122) is covered by essential patents licensed through a pool, per
decoder copy, and intoPIX describes TicoRAW as patented (see the patent notice in the
top-level README). **Publishing compiled decoders is on hold, pending patent-licensing
review.** That covers prebuilt binaries, Homebrew bottles, Scoop and winget packages,
Python wheels, npm packages and hosted web apps.

What the release workflow does today:

| Step | When it runs |
|---|---|
| Check the tag, run every test, build, package and smoke-test the CLI for 8 targets | Every release run |
| Upload the archives as workflow artifacts (7 days), write `SHA256SUMS`, attest provenance | **Only** when the repository variable `ALLOW_BINARY_DISTRIBUTION` is `true` (public workflow artifacts can be downloaded by any signed-in user) |
| Create or update the GitHub Release, **source code only**, notes from CHANGELOG.md | Every release run |
| Attach the binary archives and `SHA256SUMS` to the Release | **Only** when run by hand with `attach_binaries = yes` **and** the repository variable `ALLOW_BINARY_DISTRIBUTION` is `true` |
| Publish the crates (source) to crates.io | Only when run by hand with `publish_crates = yes` |

`ALLOW_BINARY_DISTRIBUTION` is not set. Do not set it until the licensing review is done
and its outcome allows distribution. A tag push can never attach binaries, whatever the
variable says. The attach job also runs in the `binary-distribution` environment, so you
can add required reviewers to it (Settings, Environments) as a third lock.

Workflow artifacts of a public repository can be downloaded by any signed-in GitHub user
while they are kept. If the review concludes that this also counts as distribution, make
the `build` job conditional on the same gate.

The Homebrew formula is allowed because Homebrew builds it on the user's machine. Keep it
that way: no `bottle do` block, and do not submit it to homebrew-core, which builds and
hosts bottles.

## Release procedure

### 1. Prepare the release commit

1. Move the `## [Unreleased]` entries in `CHANGELOG.md` under `## [X.Y.Z] - YYYY-MM-DD`
   and add the `[X.Y.Z]: https://github.com/lukaj99/ticoraw/releases/tag/vX.Y.Z` link.
2. Set `version = "X.Y.Z"` in `[workspace.package]` in the root `Cargo.toml`, and the
   `ticoraw` dependency version in `crates/ticoraw-cli/Cargo.toml` and
   `crates/ticoraw-dng/Cargo.toml` (and any other crate that depends on a workspace crate).
   Run `cargo check --workspace` to update `Cargo.lock`.
3. Check it: `scripts/release/check_version.sh vX.Y.Z` must print `X.Y.Z`. It fails if a
   crate version, an internal dependency requirement or the changelog section is off.
4. Regenerate the notices if dependencies changed:
   `scripts/release/third_party_notices.sh` (see below), then
   `cargo deny check`.
5. Run every CI check from CONTRIBUTING.md plus the full ground-truth suite on the
   conformance folder. Only release when every pair is bit-exact.
6. Merge to `main`.

### 2. Tag

```sh
git tag -s vX.Y.Z -m "ticoraw X.Y.Z"
git push origin vX.Y.Z
```

The push starts `release.yml`, which:

1. `verify`: checks the tag against the workspace (`check_version.sh`), runs rustfmt,
   clippy, the release-script tests and `cargo publish --workspace --dry-run`;
2. `test`, `msrv`, `conformance`: the test suite on Linux, macOS and Windows, the 1.85
   and wasm32 builds, and the public conformance samples downloaded afresh;
3. `build`: the `ticoraw` CLI for x86_64 and aarch64 Linux (glibc and static musl),
   macOS and Windows, each smoke-tested and packaged as
   `ticoraw-X.Y.Z-<target>.tar.gz` (`.zip` on Windows) holding the binary, `README.md`,
   `LICENSE-MIT`, `LICENSE-APACHE` and `THIRD-PARTY-NOTICES.md`, with a build provenance
   attestation;
4. `checksums`: `SHA256SUMS` over all archives;
5. `release`: creates the GitHub Release (or updates its notes) with the CHANGELOG.md
   section, marked as a prerelease for `X.Y.Z-pre` versions, and a note that it is
   source only.

A version with a `-` (for example `v0.3.0-rc.1`) becomes a prerelease.

To re-run for an existing tag (for example after fixing a flaky job), use Actions,
Release, Run workflow, with `tag = vX.Y.Z` and both other inputs left at `no`.

### 3. Publish to crates.io (manual)

Actions, Release, Run workflow: `tag = vX.Y.Z`, `publish_crates = yes`. After the tests
pass, the `publish-crates` job runs `cargo publish` for `ticoraw`, `ticoraw-dng`,
`ticoraw-cli` and `ticoraw-capi` (when that crate exists), in that order, skipping any
crate whose version is already on crates.io, so a failed run can simply be repeated.

It needs the repository secret `CARGO_REGISTRY_TOKEN`: a crates.io API token with the
`publish-new` and `publish-update` scopes, limited to the `ticoraw*` crates. The job runs
in the `crates-io` environment, which can hold the secret and required reviewers.
crates.io versions cannot be deleted, only yanked, so publish only after the GitHub
release looks right.

### 4. Update the Homebrew formula

```sh
scripts/release/update_formula.sh X.Y.Z    # downloads the tag tarball, sets url and sha256
```

Then copy `packaging/homebrew/ticoraw.rb` into the tap repository (for example
`lukaj99/homebrew-ticoraw`, as `Formula/ticoraw.rb`) and commit it there. Users install
with `brew install lukaj99/ticoraw/ticoraw`, which builds from source with Homebrew's Rust.

To check a formula change before a release, against a local tarball of the tree:

```sh
git archive --format=tar.gz --prefix=ticoraw-X.Y.Z/ -o /tmp/ticoraw.tar.gz HEAD
brew tap-new --no-git local/ticoraw-test
cp packaging/homebrew/ticoraw.rb "$(brew --repository local/ticoraw-test)/Formula/"
scripts/release/update_formula.sh X.Y.Z --tarball /tmp/ticoraw.tar.gz \
  --formula "$(brew --repository local/ticoraw-test)/Formula/ticoraw.rb"
brew audit --new --formula local/ticoraw-test/ticoraw
# Optional: point `url` at file:///tmp/ticoraw.tar.gz, then
brew install --build-from-source local/ticoraw-test/ticoraw && brew test local/ticoraw-test/ticoraw
brew uninstall ticoraw; brew untap local/ticoraw-test
```

### 5. Binary channels (gated; not today)

Only once the licensing review allows distributing compiled decoders:

1. Set the repository variable `ALLOW_BINARY_DISTRIBUTION` to `true`.
2. Run the Release workflow by hand with `tag = vX.Y.Z` and `attach_binaries = yes`.
   It re-verifies `SHA256SUMS` and attaches the archives and `SHA256SUMS` to the Release.
3. Fill the Scoop and winget templates: replace `__VERSION__`, `__RELEASE_DATE__` and the
   `__SHA256_*__` values (from `SHA256SUMS`; winget wants upper case), drop the
   "TEMPLATE" comments, and submit them to a Scoop bucket and to winget-pkgs.
4. Update the README's install section and the patent notice to match.

## Verifying an archive

```sh
sha256sum --check --ignore-missing SHA256SUMS
gh attestation verify ticoraw-X.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo lukaj99/ticoraw
```

## THIRD-PARTY-NOTICES.md

Generated by [cargo-about](https://github.com/EmbarkStudios/cargo-about) from the
dependency graph of the workspace's normal (non-dev, non-build) dependencies:

```sh
cargo install --locked --features cli cargo-about@0.9.3
scripts/release/third_party_notices.sh           # rewrite it
scripts/release/third_party_notices.sh --check   # what CI runs
```

`about/about.toml` lists the accepted licences (keep it in step with `deny.toml`) and a
clarification for `jpeg-encoder`, whose IJG-licensed code carries its notice in
`src/fdct.rs` rather than in a licence file. When `jpeg-encoder` is upgraded, the
clarification's checksums must be updated: cargo-about prints a checksum mismatch, and the
new values are the SHA-256 of the quoted text (for the IJG entry, from the `start` string
to the end of the `end` string). Because of the IJG licence, every binary distribution
must carry the statement "this software is based in part on the work of the Independent
JPEG Group"; the template adds it whenever an IJG-licensed crate is in the graph, and the
release archives include the file.

## Supply-chain checks

`deny.toml` configures [cargo-deny](https://github.com/EmbarkStudios/cargo-deny):
RustSec advisories, permissive licences only, crates.io as the only source, and a warning
for duplicate crate versions. Run `cargo deny check` locally
(`cargo install --locked cargo-deny@0.20.2`). An ignored advisory needs a written reason
in `deny.toml`.

## Rolling back

- A bad GitHub Release: `gh release delete vX.Y.Z` (keeps the tag) or edit it; fix and
  re-run the workflow for the tag, or release `X.Y.Z+1`.
- A bad crate: `cargo yank --version X.Y.Z <crate>` for each published crate, then
  release a fixed patch version. Published versions cannot be replaced.
- Do not move or re-push a tag that has been published: tarball checksums (Homebrew)
  and attestations refer to it.
