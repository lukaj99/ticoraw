# ticoraw (Rust library)

A clean-room decoder for Nikon **High Efficiency (HE)** and **High Efficiency\* (HE\*)**
NEF raw files. Its output is **bit-exact**: every photosite equals the CFA image Adobe DNG
Converter writes, on every Nikon body that records HE/HE\* (Z 9, Z 8, Z f, Z6III, Z 5 II,
Z 50 II and ZR; see [docs/compat.md](https://github.com/lukaj99/ticoraw/blob/main/docs/compat.md)).

In these modes the camera stores the Bayer mosaic as an intoPIX **TicoRAW** codestream:
JPEG XS (ISO/IEC 21122) entropy coding and 5/3 wavelets over four Star-Tetrix colour
components, followed by a vendor transfer curve. `ticoraw` decodes that codestream to the
14-bit CFA values Adobe's converters produce. The format is documented in
[docs/FORMAT.md](https://github.com/lukaj99/ticoraw/blob/main/docs/FORMAT.md).

The library is a pure functional core over `&[u8]`: no filesystem access, no clock, no
`unsafe`. To write complete DNGs (lossless tiles, EXIF, XMP, MakerNote, previews), use
[`ticoraw-dng`](https://github.com/lukaj99/ticoraw/blob/main/crates/ticoraw-dng/README.md)
on top of it.

> **Source only.** The crate is not on crates.io yet; use it as a git dependency. Compiled
> decoders are not distributed, pending a patent-licensing review; see the
> [patent notice](https://github.com/lukaj99/ticoraw#patent-notice).

```toml
[dependencies]
ticoraw = { git = "https://github.com/lukaj99/ticoraw" }
```

## Example

```rust
let nef = std::fs::read("DSC_0001.NEF")?;

// Metadata only: container fields, Nikon compression mode, codestream header.
let info = ticoraw::inspect_nef(&nef)?;
println!("{} {} {:?}", info.make, info.model, info.compression);

// Full decode: a 6064 x 4040 RGGB mosaic of u16 plus levels, colour, orientation and crop.
let raw = ticoraw::decode_nef(&nef)?;
println!("{}x{} orientation {}", raw.image.width, raw.image.height, raw.orientation);

// A minimal uncompressed DNG (ticoraw-dng writes complete ones).
std::fs::write("DSC_0001.dng", ticoraw::dng::encode_uncompressed_dng(&raw)?)?;
```

## API

| Item | Purpose |
|---|---|
| `inspect_nef(&[u8]) -> Result<NefInspection>` | Parse the container and codestream headers without decoding pixels |
| `inspect_nef_prefix(prefix: &[u8], file_len: usize) -> Result<NefInspection>` | The same from just the first bytes of a file (in practice a megabyte or two); a prefix that is too short fails with `Truncated` |
| `decode_nef(&[u8]) -> Result<DecodedRaw>` | Decode a HE/HE\* NEF: `make`, `model`, `image: CfaImage`, `color: ColorInfo`, `orientation`, `crop`, `baseline_exposure` |
| `decode_ticoraw(&[u8]) -> Result<CfaImage>` | Decode a bare TicoRAW codestream (SOC..EOC) |
| `unique_camera_model(make, model) -> String` | Adobe's camera name for a body (`Nikon Z 6 3`, ...), as DNG `UniqueCameraModel` |
| `NikonCompression::{from_code, code}` | Map the MakerNote 0x0051 compression code (13 = HE, 14 = HE\*, 3 = lossless) to and from the enum |
| `dng::encode_uncompressed_dng(&DecodedRaw) -> Result<Vec<u8>>` | Serialise a decode as a minimal uncompressed CFA DNG |
| `dng::decode_dng_cfa(&[u8]) -> Result<DngCfa>` | Read the CFA image of an existing DNG (lossless JPEG or uncompressed), for example as ground truth |
| `dng::decode_lossless_jpeg(&[u8]) -> Result<LosslessJpeg>` | Decode one lossless JPEG (SOF3) stream to `width`, `height` and `samples` |
| `TicoError` | Every expected failure (`Truncated`, `Container`, `Header`, `Unsupported`, `Entropy`, `Jpeg`) as a value |

`ColorInfo` holds the CFA pattern, black and white levels, the D65 colour matrix and the
as-shot white balance. Colour matrices, white levels, Adobe camera names and baseline
exposures are built in for every body above, and each equals what Adobe writes. Other
models decode, but `color_matrix` is `None`; the DNG writers then fall back to an identity
matrix and the CLI warns.

The crate root and `dng::{decode_dng_cfa, encode_uncompressed_dng}` are the stable API.
The lower-level stage modules (`bitstream`, `container`, `entropy`, `header`, `mct`,
`tiff`, `wavelet`) and `dng::decode_lossless_jpeg` are public for testing and for tools
that want to look inside the codestream; they may change in minor releases.

## Untrusted input

Hostile input is bounded before anything large is allocated, and every check failure is
an `Err`. A crafted stream that passes the checks can decode to garbage pixels (the inverse
transforms use wrapping arithmetic), but it cannot make the decoder panic. The parsers are
fuzzed every night
([fuzz/README.md](https://github.com/lukaj99/ticoraw/blob/main/fuzz/README.md)).
Codestreams outside the supported profile fail with `Unsupported` rather than decoding
wrongly.

## Features, threads and WebAssembly

The `parallel` feature (on by default) decodes slices and runs the wavelet and colour
transforms on all cores with [rayon](https://crates.io/crates/rayon). The output is
identical either way. With `default-features = false` the crate is single-threaded and
has no dependencies. It still uses `std` for `Vec` and `String`, but never touches the
filesystem, threads or the clock, so it builds for `wasm32-unknown-unknown`:

```toml
ticoraw = { git = "https://github.com/lukaj99/ticoraw", default-features = false }
```

```sh
cargo build -p ticoraw --no-default-features --target wasm32-unknown-unknown
```

A 24.5 MP frame decodes in about 30 to 35 ms on all cores of an Apple M5 Pro, and in
about 285 ms on one core. Benchmarks live in `benches/`; see
[CONTRIBUTING.md](https://github.com/lukaj99/ticoraw/blob/main/CONTRIBUTING.md#benchmarks).

## Tests

```sh
cargo test -p ticoraw --release
```

runs the unit tests, synthetic encoder round trips and robustness tests on corrupted
input; none of them needs camera files. Two environment variables add real files, which
are only read:

- `TICORAW_CONFORMANCE_DIR`: a folder filled by
  [`conformance/fetch.sh`](https://github.com/lukaj99/ticoraw/blob/main/conformance/README.md).
  Every decode must match the Adobe-verified hashes and metadata in
  `conformance/manifest.toml`.
- `TICORAW_CORPUS`: a folder of NEFs and DNGs converted from them. Every DNG whose
  `OriginalRawFileName` names a TicoRAW NEF in the folder is compared at every photosite,
  with its CFA pattern, levels, `BaselineExposure`, `Orientation` and default crop.

## Licence

MIT OR Apache-2.0, at your option. See the
[patent notice](https://github.com/lukaj99/ticoraw#patent-notice) before distributing
binaries built from it.
