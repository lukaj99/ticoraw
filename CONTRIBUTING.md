# Contributing to ticoraw

Thank you for helping. Contributions of every size are welcome: a typo fix, a clearer
error message, a test, a camera sample, a bug report or a new feature. You do not need to
know anything about JPEG XS or wavelets to start; plenty of useful work is in the CLI,
the docs and the tests.

If you are new to open source, the usual flow is: open or pick an issue, fork the
repository, make your change on a branch, run the checks below, and open a pull request.
If you are unsure about anything, ask in the issue or in
[Discussions](https://github.com/lukaj99/ticoraw/discussions). There are no silly
questions here.

Please read the [Code of Conduct](CODE_OF_CONDUCT.md) before taking part. Security
problems go through [SECURITY.md](SECURITY.md), never a public issue.

## Contents

- [Ways to help](#ways-to-help)
- [Development setup](#development-setup)
- [Running the tests](#running-the-tests)
- [The bit-exact rule](#the-bit-exact-rule)
- [Clean-room rules](#clean-room-rules)
- [Coding style](#coding-style)
- [Privacy: what never goes in the repository](#privacy-what-never-goes-in-the-repository)
- [Commits and pull requests](#commits-and-pull-requests)
- [How releases work](#how-releases-work)
- [Licence of contributions](#licence-of-contributions)

## Ways to help

- **Try it on your files** and report what happens, good or bad, through the
  [issue forms](https://github.com/lukaj99/ticoraw/issues/new/choose).
- **Contribute a CC0 sample** from a camera, firmware or mode we have not verified yet.
  [docs/SAMPLES.md](docs/SAMPLES.md) walks you through it, including what personal data a
  NEF contains.
- **Improve the docs.** If something in the README or [docs/FORMAT.md](docs/FORMAT.md)
  confused you, that is a bug worth fixing.
- **Write code.** For anything bigger than a small fix, open an issue first and describe
  what you plan, so we can agree on the approach before you spend time on it.

## Development setup

You need a Rust toolchain from [rustup](https://rustup.rs/). The project builds with Rust
**1.85** or later (the minimum supported Rust version, MSRV); develop on current stable.

```sh
git clone https://github.com/<you>/ticoraw    # your fork
cd ticoraw
cargo build --workspace
cargo run -p ticoraw-cli -- --help             # run the CLI from source
```

The workspace has two crates:

| Crate | What it is |
|---|---|
| `crates/ticoraw` | The library: a pure decoder over `&[u8]`. Modules `tiff`, `container`, `header`, `bitstream`, `entropy`, `wavelet`, `mct`, `dng` |
| `crates/ticoraw-cli` | The `ticoraw` command-line tool, which owns all file I/O |

[docs/FORMAT.md](docs/FORMAT.md) describes the format, and
[docs/superpowers/specs/2026-10-27-ticoraw-design.md](docs/superpowers/specs/2026-10-27-ticoraw-design.md)
explains how the code is organised and why.

Optional extras, used by CI:

```sh
rustup toolchain install 1.85                  # to check the MSRV locally
rustup target add wasm32-unknown-unknown       # to check the WebAssembly build
```

### The checks CI runs

Run these before you open a pull request. CI runs the same commands on Linux, macOS and
Windows, and a pull request needs all of them green:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p ticoraw --all-targets --no-default-features -- -D warnings
cargo test --workspace --release
cargo test -p ticoraw --release --no-default-features
cargo +1.85 check --workspace --all-targets --locked
cargo build -p ticoraw --no-default-features --target wasm32-unknown-unknown
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

`cargo fmt --all` fixes formatting for you. Tests run in release mode because decoding
full frames in a debug build is slow.

## Running the tests

The tests come in layers. The first three need no camera files, so they run everywhere,
including CI and your laptop.

| Layer | Where | What it checks |
|---|---|---|
| **Unit tests** | beside each module in `crates/*/src` | Small fixed inputs and outputs: bit reader, header parsing of the real Nikon header bytes, transfer-curve vectors, 5/3 wavelet perfect reconstruction, Star-Tetrix invariants, DNG round trip, lossless JPEG, CLI argument parsing |
| **Synthetic round trips** | `crates/ticoraw/tests/synthetic.rs` | A test-only encoder (`tests/common`) writes TicoRAW codestreams, NEF and DNG containers from known data; the decoder must reproduce the input exactly. This reaches code paths real cameras rarely use |
| **Robustness** | `crates/ticoraw/tests/synthetic.rs` | Deterministic bit flips, truncations, length corruptions and garbage. Every public entry point must return `Err`, never panic or hang |
| **Ground truth** (env-gated) | `crates/ticoraw/tests/ground_truth.rs`, `crates/ticoraw-cli/tests/cli.rs` | Real NEFs against DNGs converted from them by Adobe software: every photosite, the CFA pattern, black and white levels, `BaselineExposure`, `Orientation` and the default crop must match |

The ground-truth layer runs only when `TICORAW_CORPUS` names a folder of NEFs and DNGs
made from them; without it those tests print a note and pass:

```sh
TICORAW_CORPUS=/path/to/folder cargo test --workspace --release
TICORAW_CORPUS=/path/to/folder cargo test --release --test ground_truth -- --nocapture
```

The suite pairs files through each DNG's `OriginalRawFileName` tag, so file names do not
matter. The folder is only ever read. To check one pair by hand:
`ticoraw verify <file.NEF> <file.dng>`.

### Building a conformance folder yourself

The public conformance set is the 13 CC0 samples listed in
[docs/compat.md](docs/compat.md#samples), one or two per body and mode:

1. Download each sample from the link in that table and check its SHA-256
   (`shasum -a 256 <file>` on macOS, `sha256sum <file>` on Linux).
2. Convert each one with Adobe DNG Converter, uncompressed:
   `"Adobe DNG Converter" -u -p0 -d <folder> <file.NEF>` (macOS and Windows only).
3. Put the NEFs and DNGs in one folder and point `TICORAW_CORPUS` at it.

All 13 pairs must report 0 mismatching photosites. If you cannot run Adobe DNG Converter,
say so in your pull request and the maintainer will run the suite for you.

Never commit these files; see [privacy](#privacy-what-never-goes-in-the-repository).

## The bit-exact rule

`ticoraw`'s whole value is that it produces **exactly** the CFA values Adobe's converters
produce, on every body we list. So:

> **No change merges unless conformance stays green:** every ground-truth pair still
> matches at every photosite, with the same levels and metadata.

- If your change touches decoding (anything in `container`, `header`, `bitstream`,
  `entropy`, `wavelet`, `mct`, the colour profiles in `lib.rs`, or the DNG writer and
  reader), run the ground-truth suite on the conformance folder and paste the summary in
  your pull request, or ask the maintainer to run it.
- A change that makes any pair worse is not merged, even if it fixes something else.
  Speed-ups must produce identical output.
- Changes to the CLI, docs or tests alone do not need the corpus run.
- A future CI job will run this conformance check automatically on public samples.

## Clean-room rules

`ticoraw` is a clean-room implementation, and keeping it that way protects the project
and its MIT / Apache-2.0 licence. Before contributing code that touches the decoder or the
DNG writer:

- **Never read, copy or translate code** from **Adobe** (including the DNG SDK),
  **intoPIX**, **LibRaw**, **dnglab**, or **darktable** (including rawspeed). That covers
  their source files, patches in their pull requests, and decompiled or disassembled
  binaries. If you have worked on one of those decoders yourself, mention it in your pull
  request so we can discuss it.
- **Adobe tools are black-box oracles only.** You may run Adobe DNG Converter, Lightroom
  or Camera Raw on a file and compare their output with ours. Never disassemble,
  decompile, debug or trace them.
- **What you may use:** published specifications (ISO/IEC 21122 for JPEG XS, TIFF 6.0,
  the DNG 1.7 specification, EXIF 2.3, ITU-T T.81 for lossless JPEG), published papers,
  this repository's [FORMAT.md](docs/FORMAT.md), and your own observations of real files.
- **Say where knowledge came from.** If a change rests on a new fact about the format,
  cite the spec section or describe the experiment that established it (for example
  "verified against Adobe DNG Converter 18.7 on samples 3 and 4"), and update FORMAT.md.

When in doubt, ask in an issue before you read something.

## Coding style

The existing code is the best guide; match what is around your change.

- **Functional core, imperative shell.** The library never touches the filesystem, the
  clock, threads (except through the `parallel` feature's helpers in `lib.rs`) or the
  network. It takes byte slices and returns values. All I/O belongs in `ticoraw-cli`. This
  is what lets the library build for `wasm32-unknown-unknown`.
- **Typed errors, no panics on hostile input.** Every expected failure is a
  [`TicoError`](crates/ticoraw/src/error.rs) value (`Truncated`, `Container`, `Header`,
  `Unsupported`, `Entropy`, `Jpeg`). On data from a file, do not use `unwrap`, `expect`,
  unchecked indexing or arithmetic that can overflow. Read through the bounds-checked
  helpers (`BitReader`, `slice::get`, checked arithmetic). Use `Unsupported` for valid
  input outside the profile and the other variants for malformed input.
- **Bound before you allocate.** Check sizes against the input length before allocating
  anything proportional to a header field, so a tiny file cannot request gigabytes.
- **Parse once at the boundary.** Validate a header once (as `entropy::Layout::new` does),
  then let the inner loops trust the validated types.
- **No `unsafe`.** The workspace sets `unsafe_code = "forbid"`.
- **Pedantic clippy.** The workspace enables `clippy::pedantic` and CI denies warnings.
  Prefer fixing a lint over silencing it; if an `#[allow]` is right, add a one-line comment
  saying why.
- **MSRV 1.85.** Do not use newer language features, for example `let` chains
  (`if let Some(x) = a && b`).
- **Documentation.** The library is built with `#![warn(missing_docs)]`; every public item
  needs a doc comment. The crate root and `dng::{decode_dng_cfa, encode_uncompressed_dng}`
  are the stable API; the stage modules are documented as unstable.
- **Tests without mocks.** Test pure functions with fixed inputs and expected outputs.
  Add a robustness case when you add a new parser.
- **Dependencies.** The library has no dependencies without the `parallel` feature, and we
  want to keep it that way. A new dependency anywhere needs a good reason, an issue to
  discuss it, a permissive licence (MIT, Apache-2.0, BSD or Zlib) and active maintenance.

## Privacy: what never goes in the repository

- **No raw files or DNGs** of any kind, including the public samples and anything Adobe
  software wrote. Tests build their inputs synthetically; real files stay outside the
  repository and are reached through `TICORAW_CORPUS`.
- **No personal data:** camera or lens serial numbers, GPS coordinates, people's names
  from file metadata, personal file names, or absolute paths from your machine (such as
  your home directory) in code, tests, docs or commit messages.
- When quoting `ticoraw inspect` output, shorten the `path` field.

## Commits and pull requests

- **Conventional commits:** `type(scope): description`, in the imperative mood, for
  example `fix(cli): name the input file when inspect fails` or
  `docs(samples): explain the serial number`. Common types are `feat`, `fix`, `docs`,
  `test`, `refactor`, `perf`, `build`, `ci` and `chore`; scopes are usually a module or
  the `cli`.
- Keep each pull request to one topic. Small pull requests are reviewed faster.
- Add an entry under `## [Unreleased]` in [CHANGELOG.md](CHANGELOG.md) for anything a user
  or library caller would notice.
- Update the README, FORMAT.md or compat.md when behaviour or knowledge changes.
- Fill in the pull request template. It is a checklist of everything above.

Review is done by the maintainer and can take a few days. Feedback is about the code, not
you, and it is normal for a pull request to go through a round or two of changes.

## How releases work

Releases are made by the maintainer:

1. Move the `## [Unreleased]` entries in CHANGELOG.md under a new version heading, with
   the date. The project follows [Semantic Versioning](https://semver.org/); before 1.0, a
   minor version (0.x) may change the API.
2. Bump `version` in the root `Cargo.toml` (`[workspace.package]`) and the `ticoraw`
   dependency version in `crates/ticoraw-cli/Cargo.toml`.
3. Run every CI check plus the full ground-truth suite on the conformance folder. A
   release is only cut when every pair is bit-exact.
4. Tag the commit `vX.Y.Z` and publish a GitHub release with the changelog section as its
   notes.

Releases are **source only**. The project does not distribute compiled binaries because
of JPEG XS patent licensing considerations (see the patent notice in the README), so
please do not add release jobs that build or upload binaries.

## Licence of contributions

`ticoraw` is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
the user's option. Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0 licence, shall be
dual licensed as above, without any additional terms or conditions.
