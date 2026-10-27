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
| Create or update the GitHub Release, **source code only**, notes from CHANGELOG.md | Every release run |
| Upload the archives as workflow artifacts (7 days), write `SHA256SUMS`, attest provenance, attach the archives and `SHA256SUMS` to the Release | **Only** when run by hand with `attach_binaries = yes` **and** the repository variable `ALLOW_BINARY_DISTRIBUTION` is `true` (public workflow artifacts can be downloaded by any signed-in user, so a tag push uploads nothing compiled) |
| Publish the crates (source) to crates.io | Only when run by hand with `publish_crates = yes` |

`ALLOW_BINARY_DISTRIBUTION` is not set. Do not set it until the licensing review is done
and its outcome allows distribution. A tag push can never upload or attach binaries,
whatever the variable says. The attach job also runs in the `binary-distribution`
environment, so you can add required reviewers to it (Settings, Environments) as a third
lock.

The same two keys guard the other compiled channels:

| Channel | Workflow | Needs |
|---|---|---|
| Release archives (upload, attest, attach) | `release.yml` | a manual run with `attach_binaries = yes` **and** the variable |
| PyPI wheels | `python.yml` | a manual run with `publish` ticked **and** the variable |
| npm package | `wasm.yml` | a manual run with `publish` ticked **and** the variable |
| Web converter at `/convert/` on the docs site | `pages.yml` | a manual run on `main` with `include_converter = yes` **and** the variable |
| CI workflow artifacts of the C library, wheels and npm package | `capi.yml`, `python.yml`, `wasm.yml` | the variable (they are uploaded only for the gated jobs and for testing) |

Every Pages deployment replaces the whole site. A push to `main` (or a manual run
without `include_converter = yes`) deploys the docs alone, so it takes a previously
deployed `/convert/` offline; the run says so in a notice. To keep the converter online,
run the "Docs website" workflow by hand again with `include_converter = yes` after such a
deployment.

The Homebrew formula is allowed because Homebrew builds it on the user's machine. Keep it
that way: no `bottle do` block, and do not submit it to homebrew-core, which builds and
hosts bottles.

## Release procedure

### 1. Prepare the release commit

1. Make sure `## [Unreleased]` in `CHANGELOG.md` lists everything in the release.
2. Bump the version everywhere with one command (preview it first with `--dry-run`):

   ```sh
   scripts/release/bump_version.sh --dry-run X.Y.Z
   scripts/release/bump_version.sh X.Y.Z      # --date YYYY-MM-DD for another release date
   ```

   It sets the version in all of these, whatever their old values:

   | File | What |
   |---|---|
   | `Cargo.toml` | `[workspace.package] version` |
   | `crates/ticoraw-cli`, `-dng`, `-capi`, `-wasm` `Cargo.toml` | the `version` of each path dependency on a workspace crate (`cargo publish` needs it) |
   | `crates/ticoraw-py/Cargo.toml` | `[package] version`: a separate workspace, and maturin takes the wheel version from it |
   | `crates/ticoraw-wasm/npm/package.json` | `"version"` of the npm package (`crates/ticoraw-wasm/scripts/build.sh` refuses to build without it) |
   | `web/package.json`, `web/package-lock.json` | `"version"` of the web converter package |
   | `CHANGELOG.md` | moves the `[Unreleased]` entries under `## [X.Y.Z] - YYYY-MM-DD` and adds the `[X.Y.Z]: .../releases/tag/vX.Y.Z` link |

   Then it regenerates the three lockfiles, `Cargo.lock`, `crates/ticoraw-py/Cargo.lock`
   and `fuzz/Cargo.lock` (`cargo update --workspace`, which changes only the workspace
   crates' own entries; CI builds the main and Python workspaces with `--locked`), regenerates
   `THIRD-PARTY-NOTICES.md`, which lists the crate versions (needs cargo-about, see below;
   `--skip-notices` leaves it for later), and runs the check in the next step. It does not
   commit, tag or push.
3. Check it: `scripts/release/check_version.sh vX.Y.Z` must print `X.Y.Z`. It fails if any
   of the files above has another version, if a lockfile is out of date or if the
   changelog section is missing or empty. The release workflow runs the same check on the
   tag.
4. If dependencies changed, also run `cargo deny check`.
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
   `LICENSE-MIT`, `LICENSE-APACHE` and `THIRD-PARTY-NOTICES.md` (attested and uploaded
   only on a gated manual run, see [the patent gate](#the-patent-gate));
4. `checksums` (gated the same way): `SHA256SUMS` over all archives;
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
4. To host the web converter, run the "Docs website" workflow (`pages.yml`) by hand on
   `main` with `include_converter = yes`. Later pushes to `main` redeploy the docs without
   it, so run it by hand again after each one (see [the patent gate](#the-patent-gate)).
5. Update the README's install section and the patent notice to match.

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
