# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.1.0]: https://github.com/lukaj99/ticoraw/releases/tag/v0.1.0
