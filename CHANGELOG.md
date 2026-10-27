# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-10-27

### Added

- **`ticoraw-dng`**, a new crate that writes complete DNGs (`convert(&nef, &Options)`, or
  `write` for a decode plus `CarriedMetadata`). It follows Adobe's layout and does no I/O:
  - the CFA mosaic as 256x256 lossless JPEG tiles (SOF3, per-tile Huffman tables,
    encoded in parallel), or one uncompressed strip; output is about Adobe's size;
  - the NEF's EXIF (with its Interoperability IFD) and GPS IFDs copied verbatim, its XMP
    packet (or a replacement, such as a sidecar), and the Nikon MakerNote wrapped in
    `DNGPrivateData`;
  - a 256-pixel thumbnail and a preview: for SDR frames the camera's smallest embedded
    JPEG with a long side of at least 1024 pixels, copied unchanged; for HLG frames, whose
    previews are HEVC, a half-size rendering from the raw data;
  - Adobe's `UniqueCameraModel`, the D65 colour matrix, `AsShotNeutral`,
    `BaselineExposure`, `LensInfo`, `CameraSerialNumber` and optional
    `OriginalRawFileName`.
- **`ticoraw convert <files|dirs>...`**: batch conversion to complete DNGs, next to each
  NEF or under `-o`, mirroring the given folders. Options `-r`, `--skip-existing`,
  `--force`, `--uncompressed`, `--no-preview`, `--with-sidecar` (use `<name>.xmp` as the
  DNG's XMP), `-j/--jobs` and `--json` (JSON Lines). Classic NEFs are skipped with a note;
  a progress line shows on a terminal and a summary reports throughput. By default up to 4
  files run at once (on Linux and macOS also at most one per 2 GB of memory), since each
  file already uses every core.
- **`ticoraw report <file>`**: a privacy-safe JSON bug report (version, OS, camera,
  firmware, compression mode, codestream header in hex, decode outcome) with no serial
  numbers, GPS, owner, dates, file names, paths or pixels. Tests check that none of these
  leak.
- **C API**: the new `ticoraw-capi` crate, a stable C ABI with a generated header
  (`include/ticoraw.h`), `tr_decode_nef`, `tr_nef_to_dng`, `tr_inspect_json`, versioned
  options and thread-local error messages. Tested from C on Linux, macOS and Windows and
  under ASan, UBSan and LSan. Source only.
- **Python bindings** (`crates/ticoraw-py`, its own Cargo workspace, pyo3 and maturin, abi3
  for Python 3.9 and later): `ticoraw.read` with a zero-copy numpy mosaic and rawpy-style
  names, `ticoraw.inspect`, `ticoraw.to_dng` and one exception class per error. Build
  from source; PyPI wheels are pending patent-licensing review.
- **WebAssembly / JavaScript bindings** (`crates/ticoraw-wasm`, wasm-bindgen): `toDng`,
  `decode`, `inspect`, `inspectPrefix`, `decodeDng` and `version` for browsers and
  Node.js, plus a Web Worker pool (`ticoraw/worker`). Build from source; the npm package
  is pending patent-licensing review.
- **Conformance kit** (`conformance/`): a manifest of the 13 public CC0 HE/HE\* samples
  with their Adobe-verified CFA hashes and DNG metadata, `fetch.sh` to download and check
  them, a test that compares every decode, and a CI workflow on pull requests, pushes to
  `main` and a weekly fresh download.
- **Fuzzing** (`fuzz/`): nine cargo-fuzz targets with synthetic seeds and dictionaries,
  run nightly in CI, with replayed crash reproducers; CI also checks that the targets
  compile.
- **Benchmarks**: criterion wall-clock benchmarks of the full decode and each stage, and an
  instruction-count gate in CI that fails a pull request running more than 5% more
  instructions than its base.
- **Release pipeline** (`release.yml`, `packaging/`, `scripts/release/`): tag checks
  against the version and this changelog, the full test suite, fresh conformance, and a
  source-only GitHub Release. The CLI is built and smoke-tested for 8 targets, but
  compiled decoders are uploaded or attached only on a manual run with the repository
  variable `ALLOW_BINARY_DISTRIBUTION` set to `true`, which it is not (pending
  patent-licensing review). crates.io publishing is manual. A Homebrew formula builds
  from source.
- **Supply-chain checks**: cargo-deny (advisories, licences, sources) and a generated
  `THIRD-PARTY-NOTICES.md`, checked in CI.
- **Community files**: CONTRIBUTING, SECURITY, SUPPORT, a Code of Conduct, issue forms,
  a pull request checklist and `docs/SAMPLES.md` on contributing CC0 samples safely.
- `container::parse_metadata` and `NefMetadata`: the byte ranges of the EXIF and GPS IFDs,
  XMP packet, MakerNote and embedded JPEG previews. `ticoraw::unique_camera_model`
  returns Adobe's name for a body.
- `DecodedRaw::baseline_exposure`, written to DNGs as `BaselineExposure`: Adobe's per-body
  baseline plus one stop for frames shot in Nikon's HLG tone mode (MakerNote ColorSpace
  BT.2100 or an HLG Picture Control). Matches Adobe's value on every ground-truth DNG
  (Z6III: +0.25 EV SDR, +1.25 EV HLG); unknown bodies get only the HLG stop.
- `MakerNote::{color_space, hlg_picture_control, is_hlg}`.
- DNG `UniqueCameraModel` uses Adobe's camera names (`Nikon Z 6 3`, ...) so Adobe software
  applies its own camera profile to ticoraw DNGs; every name matches Adobe DNG Converter.
- The ground-truth suite also checks `BaselineExposure`, `Orientation` and the default crop
  against each Adobe DNG.

### Changed

- `ticoraw decode` now writes a complete DNG (lossless JPEG tiles, metadata, preview,
  `OriginalRawFileName`) instead of an uncompressed one; `--uncompressed` and
  `--no-preview` are new, and `--format pgm` is unchanged.
- Bit-exactness verified on every HE/HE*-capable body: public CC0 HE and HE* samples from
  the Z9, Z8, Zf, Z6III, Z5II, Z50II and ZR match Adobe DNG Converter 18.7 at every
  photosite, with matching levels, colour matrices, camera names and baseline exposures.
  The colour profiles of these bodies are now confirmed against Adobe instead of taken
  from public tables.
- Z9 white level is 15892 (Adobe's HE value), not 15520; per-body `BaselineExposure`
  values come from Adobe DNG Converter output.
- The README is now a beginner-first landing page; the library and CLI references moved to
  `crates/ticoraw/README.md` and `crates/ticoraw-cli/README.md`.

### Fixed

- The conformance test reads `manifest.toml` correctly from a checkout with CRLF line
  endings (Windows), and `.gitattributes` now keeps text files LF everywhere.
- `ticoraw convert` reports a classic NEF next to an existing DNG as skipped (classic),
  not as FAILED with exit status 1: files are classified before the existing-output
  policy applies.
- An interrupted `ticoraw convert` or `decode` (Ctrl-C or SIGTERM) removes its in-flight
  temporary files. Only a hard kill, crash or power loss can leave a stale
  `.<name>.dng.ticoraw-<pid>.tmp`; the final `.dng` is never partial.
- The `convert` progress line takes its width from the terminal itself rather than only
  from `$COLUMNS`, which shells do not export, so it no longer wraps and leaves stale
  lines on narrow terminals.
- The `-j/--jobs` documentation now says the 2 GB-per-file memory cap applies on Linux and
  macOS; on Windows only the core-count cap (at most 4) applies.
- Python: `ticoraw.to_dng(source, dest)` writes to a temporary file and renames it over
  `dest`, so a failed write no longer destroys an existing DNG.
- Python and WebAssembly: an empty `xmp` is treated as no replacement, as in the C API,
  instead of writing an empty XMP packet and dropping the NEF's own XMP.
- WebAssembly: `decode`, `toDng` and `inspect` given an `ArrayBuffer` instead of a
  `Uint8Array` no longer decode an empty slice and report a misleading `Truncated` error.
- WebAssembly: `TicorawWorker` rejects a job whose options cannot be structured-cloned,
  instead of hanging forever with the worker slot held.
- Web converter (folder mode): two files that map to the same DNG name no longer
  overwrite each other's output while the page reports both; a failed write no longer
  leaves a 0-byte `.dng` under the final name; losing folder permission
  (`NotAllowedError`) mid-run stops the run instead of converting the remaining files
  and failing them with a `TypeError`.
- Web converter: keyboard focus is no longer dropped to the page body after every run
  and after Stop; ZIP entry names no longer keep backslashes from file names (APPNOTE
  forbids them and Windows tools read them as folder separators); `web/build.sh` refuses
  an output path that is a regular file instead of deleting it.
- Docs: `--uncompressed` DNGs are about twice the size of the default ones (1.7 to 2.6
  times on the public samples), not about three times; the web converter's preview option
  no longer claims a full-size preview (the embedded one is the camera's mid-size JPEG);
  the 13 public samples are 10.7 to 45.7 megapixels, not 24 to 46; the library guide
  no longer says every binding has no file access (the Python package reads and writes
  paths).
- Docs: the README quick start lists the build prerequisites (Apple's command line tools
  on macOS, a C compiler on Linux, the MSVC Build Tools on Windows) and tells every
  platform to open a new terminal after installing Rust; the troubleshooting page quotes
  the error a Rust older than 1.85 really prints (``feature `edition2024` is required``)
  and says that nightly is not needed.
- Release: `scripts/release/check_version.sh` also checks the Python crate's version, the
  three lockfiles (`Cargo.lock`, `crates/ticoraw-py/Cargo.lock`, `fuzz/Cargo.lock`) and the
  npm and web `package.json` / `package-lock.json` versions, which a version bump used to
  miss, breaking the Python and WebAssembly CI. The new
  `scripts/release/bump_version.sh X.Y.Z` (with `--dry-run`) updates all of them, the
  changelog and the notices in one step, and packaging/README.md documents it.
- Workflow and manifest comments no longer say that compiled builds are always kept as
  workflow artifacts: they are uploaded only when `ALLOW_BINARY_DISTRIBUTION` is `true`.
- `THIRD-PARTY-NOTICES.md` (with the IJG statement) now ships in the Python wheels and the
  npm package, and covers the crates that only the wheels link (numpy, pyo3, ndarray and
  others).

### Security

- `dng::decode_dng_cfa`: hostile 64-bit `TileByteCounts` could overflow their sum (a
  panic with overflow checks on, a wrapped sum without). The sum now saturates and the
  file fails with an error. Found by fuzzing; the reproducer is replayed as a regression test.
- `ticoraw-dng`: the EXIF, GPS and Interoperability fields copied from the NEF have a total
  size budget, so one crafted NEF can no longer make `convert` use memory and write output
  far larger than the input.
- `ticoraw convert --with-sidecar` reads only a regular sidecar file, and at most a fixed
  number of bytes: a FIFO no longer hangs the conversion and a symlink to `/dev/zero` no
  longer uses up all memory.
- `ticoraw scan`, `inspect` and `decode` sanitise the camera's Make and Model strings
  before printing them, so a crafted file cannot inject terminal escape sequences.
- Python: `ticoraw.to_dng(source, dest)` refuses a `dest` that is a hard link to the
  source NEF instead of overwriting the NEF.
- The supply-chain checks (cargo-deny) also cover `crates/ticoraw-py/Cargo.lock`, whose
  crates are linked into the wheels.
- Patent gate: the docs website (`pages.yml`) deploys the compiled web converter at
  `/convert/` only from a manual run with `include_converter = yes` **and** the repository
  variable `ALLOW_BINARY_DISTRIBUTION` set to `true`; a push to `main` deploys the docs
  alone, whatever the variable says. Likewise the release workflow now attests, uploads
  and checksums the CLI archives only on a manual run with `attach_binaries = yes` and the
  variable, never on a tag push. This matches the documented two-key rule.

## [0.1.0] - 2026-10-27

### Added

- `ticoraw` library: clean-room decoder for Nikon High Efficiency (HE) and High
  Efficiency\* (HE\*) NEF raw files (JPEG XS / intoPIX TicoRAW codestreams).
  - NEF container and Nikon MakerNote parsing (`inspect_nef`, `container::parse_nef`),
    and `inspect_nef_prefix` / `container::parse_nef_prefix` to classify a file from its
    first bytes only.
  - `NikonCompression` with `from_code` / `code` for the MakerNote 0x0051 compression
    code (13 = HE, 14 = HE\*, 3 = lossless).
  - JPEG XS main-header parsing including the TicoRAW vendor extensions (CAP signature,
    13 PIH vendor bytes, 3-byte CDT entries).
  - Entropy decoding for the `Ppoc = 1` packet order: long packet headers, significance,
    zero and vertical GCLI prediction (including `D = 3` at slice starts, read as zero
    prediction), raw GCLIs at `Br` bits, bit-plane data, separate signs and
    uniform/deadzone dequantisation.
  - Reversible 5/3 inverse wavelet with frame-wide vertical synthesis across slices.
  - Inverse Star-Tetrix with Nikon's fixed geometry and the vendor transfer curve.
  - `decode_nef` and `decode_ticoraw`, bit-exact against Adobe for Nikon Z6III HE\*
    files (15 reference DNGs of 11 distinct NEFs). Public HE/HE\* samples of the Z9, Z8,
    Zf, Z6III, Z5II, Z50II and ZR decode cleanly (`docs/compat.md`).
  - `DecodedRaw::orientation` (TIFF Orientation) and `DecodedRaw::crop` (MakerNote
    0x0045 crop area).
  - Built-in DNG colour profiles (white level and D65 colour matrix) for the Z6III
    (verified against Adobe) and the Z8, Z9, Zf, Z5II, Z50II and ZR (from the public
    LibRaw / dnglab tables, unverified).
  - Uncompressed CFA DNG writer (`dng::encode_uncompressed_dng`, returning `Result`),
    writing `Orientation` and `DefaultCropOrigin` / `DefaultCropSize`; `BaselineExposure`
    is not written. A DNG reader for ground-truth comparison (`dng::decode_dng_cfa`) and
    `dng::decode_lossless_jpeg`, which returns a `LosslessJpeg` with its width, height
    and samples.
  - `parallel` feature (default) using rayon; the crate builds for
    `wasm32-unknown-unknown` with `default-features = false`.
  - Rustdoc for every public item (`#![warn(missing_docs)]`); the stage modules are
    documented as unstable.
- `ticoraw` command-line tool with `inspect`, `decode`, `verify` and `scan` commands.
  - `decode` warns on stderr when the camera model has no built-in colour profile.
  - `scan` accepts `.nef` and `.nrw` in any case.
- Environment-gated ground-truth suite (`TICORAW_CORPUS`) that discovers every TicoRAW
  NEF + DNG pair in a folder through the DNG `OriginalRawFileName` tag.
- Format documentation (`docs/FORMAT.md`), a cross-camera compatibility report
  (`docs/compat.md`), design notes, CI and dual MIT / Apache-2.0 licensing, with the
  licence files included in each crate package.

### Security

- Malformed or hostile input returns a `TicoError` and never panics.
- Profile checks reject, before decoding, `Fs != 1`, subsampled components
  (`Sx` / `Sy != 1`), `Hf` not a multiple of 4, `Fq > 8`, `Bw > 20`, `Br` outside 1..=8,
  and codestreams smaller than one byte per 16 photosites, so a tiny crafted input cannot
  trigger a huge allocation. GCLIs above `Bw` are entropy errors.
- The inverse 5/3 wavelet and Star-Tetrix use wrapping arithmetic: hostile input can
  decode to garbage but cannot overflow-panic.
- IFD dimensions are checked before allocation; DNG tile and strip sizes use checked
  products bounded by the file length; SubIFD fan-out is capped; TIFF offset arithmetic
  is checked.
- CLI output is written atomically (temporary file, then rename or no-clobber hard link),
  and `decode` refuses to overwrite its input, including through a hard link on Unix.
- `scan` refuses non-regular files and, without `--decode`, reads only a prefix of each
  file.

[0.2.0]: https://github.com/lukaj99/ticoraw/releases/tag/v0.2.0
[0.1.0]: https://github.com/lukaj99/ticoraw/releases/tag/v0.1.0
