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
- [Benchmarks](#benchmarks)
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

The repository holds these crates:

| Crate | What it is |
|---|---|
| `crates/ticoraw` | The decoder library: pure functions over `&[u8]`. Modules `tiff`, `container`, `header`, `bitstream`, `entropy`, `wavelet`, `mct`, `dng` |
| `crates/ticoraw-dng` | The complete DNG writer: lossless JPEG tiles, metadata carry-over, previews. Also I/O-free |
| `crates/ticoraw-cli` | The `ticoraw` command-line tool, which owns all file I/O |
| `crates/ticoraw-capi` | The C ABI (`include/ticoraw.h`); the only crate with `unsafe`, confined to `src/ffi.rs` |
| `crates/ticoraw-wasm` | WebAssembly / JavaScript bindings (wasm-bindgen) and the npm package layout |
| `crates/ticoraw-py` | Python bindings (pyo3 + maturin). **Its own Cargo workspace**, so the main one builds without Python |
| `fuzz` | cargo-fuzz targets. **Its own Cargo workspace**, built with nightly Rust |

The first five are members of the main workspace, so `cargo build --workspace` and the
checks below cover them. Each crate's README explains its API.

[docs/FORMAT.md](docs/FORMAT.md) describes the format, and
[docs/superpowers/specs/2026-10-27-ticoraw-design.md](docs/superpowers/specs/2026-10-27-ticoraw-design.md)
explains how the code is organised and why.

Optional extras, used by CI:

```sh
rustup toolchain install 1.85                  # to check the MSRV locally
rustup target add wasm32-unknown-unknown       # to check the WebAssembly build
```

### The checks CI runs

Run these before you open a pull request. The CI workflow (`.github/workflows/ci.yml`) runs
the same commands (the tests on Linux, macOS and Windows), and a pull request needs all of
them green:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p ticoraw --all-targets --no-default-features -- -D warnings
cargo test --workspace --release
cargo test -p ticoraw --release --no-default-features
cargo +1.85 check --workspace --all-targets --locked
cargo build -p ticoraw --no-default-features --target wasm32-unknown-unknown
cargo build -p ticoraw-dng --no-default-features --target wasm32-unknown-unknown
cargo build -p ticoraw-wasm --target wasm32-unknown-unknown
cargo check --manifest-path fuzz/Cargo.toml --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

`cargo fmt --all` fixes formatting for you. Tests run in release mode because decoding
full frames in a debug build is slow.

Other workflows also run on pull requests: `conformance.yml` (public samples, below),
`bench.yml` (the instruction-count gate, below), `capi.yml` (the C header and C tests, also
under sanitizers) and `deny.yml` (cargo-deny, the third-party notices and the release
scripts). `python.yml` and `wasm.yml` run when the crates they build change. `fuzz.yml`
runs nightly.

### Python bindings

`crates/ticoraw-py` is its own Cargo workspace. You need Python 3.9 or later:

```sh
cd crates/ticoraw-py
python -m venv .venv && . .venv/bin/activate
pip install maturin pytest numpy
maturin develop --release        # builds and installs `ticoraw` into the venv
pytest
cargo fmt --check && cargo clippy --all-targets -- -D warnings
```

### WebAssembly bindings

You need the `wasm32-unknown-unknown` target, [wasm-pack](https://github.com/drager/wasm-pack)
and Node.js 18 or later:

```sh
rustup target add wasm32-unknown-unknown
cargo test -p ticoraw-wasm                    # option parsing and error mapping, natively
wasm-pack test --node crates/ticoraw-wasm     # the JavaScript API in a real wasm instance
crates/ticoraw-wasm/scripts/build.sh          # assembles the npm package in crates/ticoraw-wasm/pkg
```

Both crates' READMEs list their conformance and worker scripts.

## Running the tests

The tests come in layers. The first three need no camera files, so they run everywhere,
including CI and your laptop.

| Layer | Where | What it checks |
|---|---|---|
| **Unit tests** | beside each module in `crates/*/src` | Small fixed inputs and outputs: bit reader, header parsing of the real Nikon header bytes, transfer-curve vectors, 5/3 wavelet perfect reconstruction, Star-Tetrix invariants, DNG round trip, lossless JPEG, CLI argument parsing |
| **Synthetic round trips** | `crates/ticoraw/tests/synthetic.rs`, `crates/ticoraw-dng/tests`, `crates/ticoraw-cli/tests/cli.rs` | A test-only encoder (`tests/common`) writes TicoRAW codestreams, NEF and DNG containers from known data; the decoder must reproduce the input exactly, and every DNG must read back unchanged. This reaches code paths real cameras rarely use |
| **Robustness** | `crates/ticoraw/tests/synthetic.rs` | Deterministic bit flips, truncations, length corruptions and garbage, plus every crash the fuzzers found (`fuzz/artifacts-regressions`). Every public entry point must return `Err`, never panic or hang |
| **Conformance** (env-gated, in CI) | `crates/ticoraw/tests/conformance.rs` | The 13 public CC0 samples in [`conformance/manifest.toml`](conformance/manifest.toml): every decode must hash to the Adobe-verified SHA-256, with the expected levels, `BaselineExposure`, `Orientation`, crop and camera name |
| **Ground truth** (env-gated) | `crates/ticoraw/tests/ground_truth.rs`, `crates/ticoraw-cli/tests/cli.rs` | Real NEFs against DNGs converted from them by Adobe software: every photosite, the CFA pattern, black and white levels, `BaselineExposure`, `Orientation` and the default crop must match |

### Conformance

The **Conformance** workflow (`.github/workflows/conformance.yml`) runs the conformance
layer on every pull request and push to `main`, from a cached copy of the samples, and
downloads them afresh every week. The pull request template has a checkbox for it: a pull
request needs it green. To run it yourself (about 250 MB of downloads, cached):

```sh
conformance/fetch.sh ~/.cache/ticoraw-conformance
TICORAW_CONFORMANCE_DIR=~/.cache/ticoraw-conformance \
  cargo test -p ticoraw --release --test conformance -- --nocapture
```

Without `TICORAW_CONFORMANCE_DIR` the test only checks the manifest itself.
[conformance/README.md](conformance/README.md) explains the kit and how to add a sample.

### Ground truth

The ground-truth layer runs only when `TICORAW_CORPUS` names a folder of NEFs and DNGs
made from them; without it those tests print a note and pass:

```sh
TICORAW_CORPUS=/path/to/folder cargo test --workspace --release
TICORAW_CORPUS=/path/to/folder cargo test --release --test ground_truth -- --nocapture
```

The suite pairs files through each DNG's `OriginalRawFileName` tag, so file names do not
matter. The folder is only ever read. To check one pair by hand:
`ticoraw verify <file.NEF> <file.dng>`.

### Building a ground-truth folder yourself

The conformance layer already checks the 13 public samples against Adobe's hashes. To
compare against Adobe's DNGs directly, at every photosite:

1. Download the samples with `conformance/fetch.sh <folder>`, which checks each SHA-256.
   They are also listed, with links, in [docs/compat.md](docs/compat.md#samples).
2. Convert each one with Adobe DNG Converter, uncompressed:
   `"Adobe DNG Converter" -u -p0 -d <folder> <file.NEF>` (macOS and Windows only).
3. Put the NEFs and DNGs in one folder and point `TICORAW_CORPUS` at it.

All 13 pairs must report 0 mismatching photosites. If you cannot run Adobe DNG Converter,
say so in your pull request and the maintainer will run the suite for you.

Never commit these files; see [privacy](#privacy-what-never-goes-in-the-repository).

## The bit-exact rule

`ticoraw`'s whole value is that it produces **exactly** the CFA values Adobe's converters
produce, on every body we list. So:

> **No change merges unless conformance stays green:** the Conformance workflow passes,
> and every ground-truth pair still matches at every photosite, with the same levels and
> metadata.

- The Conformance workflow checks the 13 public samples on every pull request; you do not
  need to do anything for it.
- If your change touches decoding (anything in `container`, `header`, `bitstream`,
  `entropy`, `wavelet`, `mct`, the colour profiles in `lib.rs`, or the DNG writers and
  reader), also run the ground-truth suite on a folder of Adobe DNGs and paste the summary
  in your pull request, or ask the maintainer to run it.
- A change that makes any pair worse is not merged, even if it fixes something else.
  Speed-ups must produce identical output.
- Changes to the CLI, docs or tests alone do not need the corpus run.

## Benchmarks

Speed matters for whole memory cards, so decoding has benchmarks in
`crates/ticoraw/benches`:

```sh
cargo bench -p ticoraw --bench decode                          # criterion, all cores
RAYON_NUM_THREADS=1 cargo bench -p ticoraw --bench decode      # one core
TICORAW_BENCH_NEF=/path/to/file.NEF cargo bench -p ticoraw --bench decode -- nef
```

They decode a synthetic 24.5 MP frame at the HE and HE\* rates, and each stage on its own,
so they need no camera files. Throughput is in photosites per second.

The **instruction-count gate** in `.github/workflows/bench.yml` runs on every pull request.
Wall-clock times on shared CI machines vary too much to compare, so it counts the
instructions each decoder stage executes under Valgrind (with
[gungraun](https://crates.io/crates/gungraun)), for the base commit and for your branch on
the same runner. It **fails when any benchmark executes more than 5% more instructions**.
It runs on Linux only; to run it locally there, install Valgrind and the runner at the
version in `Cargo.lock`:

```sh
cargo install gungraun-runner --version <gungraun version in Cargo.lock>
cargo bench -p ticoraw --no-default-features --bench instructions
```

If a slowdown is the price of a fix, say so in the pull request.

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

- **Functional core, imperative shell.** `ticoraw` and `ticoraw-dng` never touch the
  filesystem, the clock, threads (except through the `parallel` feature's helpers) or the
  network. They take byte slices and return values. File I/O belongs in `ticoraw-cli` and
  in the bindings that accept paths (Python). This is what lets the libraries build for
  `wasm32-unknown-unknown`.
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
- **No `unsafe`.** The workspace sets `unsafe_code = "forbid"`. The one exception is the C
  ABI in `crates/ticoraw-capi/src/ffi.rs`, where every `unsafe` block needs a
  `// SAFETY:` comment.
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
- **Dependencies.** The decoder has no dependencies without the `parallel` feature, and we
  want to keep it that way. `cargo deny check` (see `deny.toml`) must pass. A new dependency anywhere needs a good reason, an issue to
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

Releases are made by the maintainer, following
[packaging/README.md](packaging/README.md). In short: run
`scripts/release/bump_version.sh X.Y.Z` (try `--dry-run` first), which moves the
`[Unreleased]` entries in CHANGELOG.md under the new version, sets the version in every
manifest (the main workspace, the Python crate and the npm and web `package.json` files),
regenerates the lockfiles and runs `scripts/release/check_version.sh`; then push a signed
`vX.Y.Z` tag. The project follows
[Semantic Versioning](https://semver.org/); before 1.0, a minor version (0.x) may change
the API.

The tag starts `.github/workflows/release.yml`. It checks the tag, runs every test on
Linux, macOS and Windows plus the MSRV, wasm32 and fresh conformance checks, and creates a
**source-only** GitHub Release with the changelog section as its notes.

**The patent gate.** Compiled decoders are not distributed, pending a patent-licensing
review (see the [patent notice](README.md#patent-notice)). The release workflow still
builds the CLI for 8 targets and smoke-tests each build, so the release path stays
working. But its archives are uploaded, attested or attached to a Release only on a
manual run with `attach_binaries = yes` **and** with the repository variable
`ALLOW_BINARY_DISTRIBUTION` set to `true`; a tag push never uploads them. The variable is
unset. The C, Python and WebAssembly workflows build and test; they upload their compiled
outputs as short-lived workflow artifacts only when the variable is `true` (any signed-in
user can download those), and their publish jobs (PyPI, npm) need a manual run **and**
the variable. The docs website deploys the web converter at `/convert/` only from a
manual run of `pages.yml` with `include_converter = yes` **and** the variable; a push to
`main` deploys the docs alone. Publishing crates (source) to crates.io is a separate
manual step.

If you add a workflow step that uploads, publishes or deploys anything compiled, guard it
the same way: `vars.ALLOW_BINARY_DISTRIBUTION == 'true'` on every upload, plus a manual
`workflow_dispatch` input for anything that publishes or deploys. Pull requests that
weaken the gate are not merged.

## Licence of contributions

`ticoraw` is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
the user's option. Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0 licence, shall be
dual licensed as above, without any additional terms or conditions.
