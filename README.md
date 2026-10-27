# ticoraw

A clean-room Rust decoder for Nikon **High Efficiency (HE)** and **High Efficiency\*
(HE\*)** NEF raw files.

In these modes Nikon cameras store the Bayer mosaic as an intoPIX **TicoRAW** codestream:
JPEG XS (ISO/IEC 21122) entropy coding and 5/3 wavelets over four Star-Tetrix colour
components, followed by a vendor transfer curve. `ticoraw` decodes that codestream to the
14-bit CFA values that Adobe's converters produce, and can write them out as an
uncompressed DNG that any raw processor opens.

The workspace has two crates:

- [`ticoraw`](https://github.com/lukaj99/ticoraw/blob/main/crates/ticoraw): the library, a
  pure functional core over `&[u8]` with no filesystem access and no `unsafe`;
- [`ticoraw-cli`](https://github.com/lukaj99/ticoraw/blob/main/crates/ticoraw-cli): the
  `ticoraw` command-line tool.

The format is documented in
[docs/FORMAT.md](https://github.com/lukaj99/ticoraw/blob/main/docs/FORMAT.md).

## Status

| Body | Mode | Status |
|---|---|---|
| Nikon Z6III | HE\* | **Bit-exact** against Adobe: 15 reference DNGs of 11 distinct NEFs match at every photosite |
| Nikon Z6III | HE | Decodes cleanly on a public sample; not yet verified bit-exact |
| Nikon Z9, Z8, Zf, Z5II, Z50II, ZR | HE, HE\* | Decode cleanly on public CC0 samples (see [docs/compat.md](https://github.com/lukaj99/ticoraw/blob/main/docs/compat.md)); not yet verified bit-exact |

Of the 15 reference DNGs, 14 were written by Adobe Lightroom and one by Topaz Photo AI,
whose CFA image is identical to Adobe's.

DNG colour profiles (white level and D65 colour matrix) are built in for all of the bodies
above. Only the Z6III's is verified against Adobe HE output; the others come from the
public LibRaw / dnglab tables. Other models decode, but their DNG gets an identity colour
matrix and `ticoraw decode` warns about it.

Classic NEF compression (lossless, lossy, uncompressed) is out of scope: use any existing
raw library for those. Codestreams outside the supported profile are rejected with an
`Unsupported` error rather than decoded wrongly.

## Install

Prebuilt binaries are intentionally not distributed, because of JPEG XS per-copy patent
licensing considerations (see the [patent notice](#patent-notice)). This project ships
source code only; build it yourself with a Rust toolchain (1.85 or later).

Straight from GitHub:

```sh
cargo install --git https://github.com/lukaj99/ticoraw ticoraw-cli
```

From a clone:

```sh
git clone https://github.com/lukaj99/ticoraw
cd ticoraw
cargo install --path crates/ticoraw-cli
```

Both install a binary called `ticoraw`.

As a library:

```toml
[dependencies]
ticoraw = { git = "https://github.com/lukaj99/ticoraw" }
```

## Command-line usage

```sh
# Print container and codestream facts without decoding pixels
ticoraw inspect DSC_0001.NEF
ticoraw inspect --json DSC_0001.NEF DSC_0002.NEF

# Decode to an uncompressed DNG (default) or a 16-bit PGM of the raw mosaic
ticoraw decode DSC_0001.NEF                          # writes DSC_0001.dng
ticoraw decode DSC_0001.NEF -o out.dng
ticoraw decode DSC_0001.NEF --format pgm -o out.pgm
ticoraw decode DSC_0001.NEF -o out.dng --force       # overwrite an existing file

# Compare a decode against a reference DNG (e.g. from Adobe DNG Converter or Lightroom)
ticoraw verify DSC_0001.NEF DSC_0001.dng

# List the NEFs in a directory with their compression; optionally decode each one
ticoraw scan ~/Pictures/import
ticoraw scan ~/Pictures/import --recursive --decode
```

| Command | Flags | What it does |
|---|---|---|
| `inspect <file.nef>...` | `--json` | For each file: camera model, raw dimensions, codec (TicoRAW or classic Nikon), MakerNote compression mode, raw strip location, black levels and, for TicoRAW strips, the codestream layout and transfer curve. `--json` prints one array with an object per file |
| `decode <file.nef>` | `-o, --output <path>`, `--format dng\|pgm`, `-f, --force` | Decodes the mosaic and writes a DNG (default) or a PGM. The default output is the input path with a `.dng` / `.pgm` extension |
| `verify <file.nef> <reference.dng>` | | Decodes the NEF and the DNG's full-resolution CFA image (lossless JPEG or uncompressed), then prints the mismatch count, the largest absolute difference and mismatches per 2x2 CFA site |
| `scan <dir>` | `--decode`, `-r, --recursive` | Lists each `.nef` / `.nrw` file (any case) as TicoRAW (HE/HE\*), classic Nikon or error, then prints counts. `--decode` also decodes every TicoRAW file and reports failures and throughput |

Every command takes `-h, --help`; `ticoraw -V` prints the version. Exit status is 0 on
success, 1 on failure (for `verify`, any mismatch) and 2 on usage errors.

Details worth knowing:

- **DNG output** keeps the CFA pattern, black and white levels, colour matrix, as-shot
  white balance, the NEF's `Orientation` and, when the MakerNote has one, the camera's
  recommended crop as `DefaultCropOrigin` / `DefaultCropSize`, and Adobe's
  `BaselineExposure` (+1 EV for HLG tone-mode frames on top of the per-body value; equal to
  Adobe's on every ground-truth DNG). `UniqueCameraModel` carries Adobe's camera name, so
  Lightroom and Camera Raw apply their own profile. Other raw processors use their own
  tone curves, so their rendering can still differ from Adobe's; the CFA values are
  identical.
- **PGM output** is the bare mosaic as big-endian 16-bit samples (maxval 16383 for 14-bit
  data, else 65535).
- **Writes are atomic.** `decode` writes to a temporary file next to the output, then
  renames it into place (`--force`) or hard-links it only if the output does not exist yet.
  A failed write never leaves a partial file or destroys an existing one. `decode` refuses
  to write over its own input (on Unix, also when the output is a hard link to it).
- **`scan` only reads.** It opens regular files only, reads just the start of each file
  (enough for the TIFF structures and the codestream header) unless `--decode` is given,
  and with `--recursive` skips symbolic links to directories.

## Library API

```rust
let nef = std::fs::read("DSC_0001.NEF")?;

// Metadata only: container fields, Nikon compression mode, codestream header.
let info = ticoraw::inspect_nef(&nef)?;
println!("{} {} {:?}", info.make, info.model, info.compression);

// Full decode: a 6064 x 4040 RGGB mosaic of u16 plus levels, colour, orientation and crop.
let raw = ticoraw::decode_nef(&nef)?;
println!("{}x{} orientation {}", raw.image.width, raw.image.height, raw.orientation);

// Write it as an uncompressed DNG.
std::fs::write("DSC_0001.dng", ticoraw::dng::encode_uncompressed_dng(&raw)?)?;
```

| Item | Purpose |
|---|---|
| `inspect_nef(&[u8]) -> Result<NefInspection>` | Parse the container and codestream headers without decoding pixels |
| `inspect_nef_prefix(prefix: &[u8], file_len: usize) -> Result<NefInspection>` | The same from just the first bytes of a file (in practice a megabyte or two); a prefix that is too short fails with `Truncated` |
| `decode_nef(&[u8]) -> Result<DecodedRaw>` | Decode a HE/HE\* NEF: `make`, `model`, `image: CfaImage`, `color: ColorInfo`, `orientation: u16`, `crop: Option<Crop>` |
| `decode_ticoraw(&[u8]) -> Result<CfaImage>` | Decode a bare TicoRAW codestream (SOC..EOC) |
| `NikonCompression::{from_code, code}` | Map the MakerNote 0x0051 compression code (13 = HE, 14 = HE\*, 3 = lossless) to and from the enum |
| `dng::encode_uncompressed_dng(&DecodedRaw) -> Result<Vec<u8>>` | Serialise a decode as an uncompressed CFA DNG |
| `dng::decode_dng_cfa(&[u8]) -> Result<DngCfa>` | Read the CFA image of an existing DNG (lossless JPEG or uncompressed), e.g. as ground truth |
| `dng::decode_lossless_jpeg(&[u8]) -> Result<LosslessJpeg>` | Decode one lossless JPEG (SOF3) stream to `width`, `height` and `samples` |
| `TicoError` | Every expected failure (`Truncated`, `Container`, `Header`, `Unsupported`, `Entropy`, `Jpeg`) as a value; malformed input never panics |

The crate root and `dng::{decode_dng_cfa, encode_uncompressed_dng}` are the stable API.
The lower-level stage modules (`bitstream`, `container`, `entropy`, `header`, `mct`,
`tiff`, `wavelet`) and `dng::decode_lossless_jpeg` are public for testing and for tools
that want to look inside the codestream; they may change in minor releases.

Hostile input is bounded before anything large is allocated, and every check failure is an
`Err`. A crafted stream that passes the checks can decode to garbage pixels (the inverse
transforms use wrapping arithmetic), but it cannot make the decoder panic.

### Features, `no_std`-ish use and WebAssembly

The `parallel` feature (on by default) decodes slices and runs the wavelet and colour
transforms on all cores with [rayon](https://crates.io/crates/rayon). With
`default-features = false` the crate is single-threaded and dependency-free. It still uses
`std` for `Vec` and `String`, but never touches the filesystem, threads or the clock, so it
builds for `wasm32-unknown-unknown`:

```toml
ticoraw = { git = "https://github.com/lukaj99/ticoraw", default-features = false }
```

```sh
cargo build -p ticoraw --no-default-features --target wasm32-unknown-unknown
```

## Performance

About **45 ms per 24.5 MP frame** on Apple Silicon with the `parallel` feature, in a
release build. Decoding is multi-threaded across the 64 slices of a frame, then across rows
for the wavelet and colour stages.

## Testing

```sh
cargo test --release
```

runs the unit tests (bit reader, header and transfer-curve vectors, wavelet perfect
reconstruction, Star-Tetrix invariants, DNG round trip), synthetic encoder round trips and
robustness tests on corrupted input. None of them needs camera files.

### Ground-truth suite

The bit-exactness claim is checked against pairs of NEFs and DNGs converted from them. The
suite is gated on an environment variable and is skipped when it is unset:

```sh
TICORAW_CORPUS=/path/to/folder cargo test --release
```

The suite discovers pairs on its own: every DNG in the folder whose `OriginalRawFileName`
tag (0xC68B) names a TicoRAW NEF in the same folder is decoded and compared. DNGs whose
NEF is missing or not HE/HE\* are skipped. Each pair must agree at every photosite, and the
CFA pattern and black and white levels `decode_nef` reports must match the ones the DNG
declares. The
folder is only read, never written. You can check a single pair with
`ticoraw verify <nef> <dng>`.

## Patent notice

**Please read this before using or distributing this software.**

- JPEG XS (ISO/IEC 21122) is covered by essential patents. A patent pool for JPEG XS is
  administered by Vectis IP (<https://www.jpegxspool.com/>), with Fraunhofer-Gesellschaft
  and intoPIX among the licensors.
- intoPIX describes TicoRAW as patented technology and holds patents on it.
- This project provides **source code** for interoperability and research purposes: so that
  photographers can read their own raw files and so that the format is documented. It does
  not grant, and cannot grant, any patent licence. The MIT and Apache-2.0 licences of this
  project cover its copyright only.
- For this reason the project does not distribute compiled binaries.
- **You are responsible** for determining whether you need a patent licence to use,
  build, or distribute this software or binaries made from it in your jurisdiction, and
  for obtaining one if so.
- This notice is not legal advice.

## Licence

Licensed under either of

- Apache License, Version 2.0
  ([LICENSE-APACHE](https://github.com/lukaj99/ticoraw/blob/main/LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT licence ([LICENSE-MIT](https://github.com/lukaj99/ticoraw/blob/main/LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 licence, shall be dual licensed
as above, without any additional terms or conditions.

## Acknowledgements

`ticoraw` is a clean-room implementation: no code was copied from the projects below, and
in particular none from LGPL-licensed sources. It would not exist without the public work
of:

- the authors of ISO/IEC 21122 and of **SVT-JPEG-XS**
  (<https://github.com/OpenVisualCloud/SVT-JPEG-XS>), for an open account of JPEG XS;
- **LibRaw PR #826** (<https://github.com/LibRaw/LibRaw/pull/826>) and **RdWing**, whose
  review established the exact transfer curve and verified it against Adobe;
- **darktable PR #22743** (<https://github.com/darktable-org/darktable/pull/22743>);
- **dnglab PR #835** (<https://github.com/dnglab/dnglab/pull/835>);
- Richter, Fößel, Descampe and Rouvroy, "Bayer CFA Pattern Compression With JPEG XS",
  IEEE TIP 30 (2021), <https://doi.org/10.1109/TIP.2021.3095421>.

See
[docs/FORMAT.md](https://github.com/lukaj99/ticoraw/blob/main/docs/FORMAT.md#21-sources-and-acknowledgements)
for what each contributed.

## Trademarks

Nikon is a trademark of Nikon Corporation. intoPIX and TicoRAW are trademarks of intoPIX
SA. Adobe, Lightroom and DNG are trademarks of Adobe Inc. Topaz Photo AI is a trademark of
Topaz Labs. All other names are the property of their respective owners. This project is
independent and is not affiliated with, endorsed by, or sponsored by any of them.
