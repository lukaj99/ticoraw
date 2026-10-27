# Library guides

ticoraw is also a library you can build into your own software: a raw processor, a photo
manager, a web app or a data pipeline. Every binding wraps the same Rust core, so they all
decode bit-exactly. The Rust, C and JavaScript (WebAssembly) bindings work on bytes in
memory, with no file access of their own; the Python package also accepts file paths, and
`ticoraw.to_dng(nef, dest)` writes the DNG to `dest` itself.

| Language | What you get | Get it today | Packages |
|---|---|---|---|
| [Rust](rust.md) | `ticoraw` (decoder) and `ticoraw-dng` (DNG writer) | Git dependency in `Cargo.toml` | crates.io: planned |
| [C and C++](c.md) | A stable C ABI with one header, `ticoraw.h` | Build the static or shared library with `cargo` | Prebuilt libraries: pending licensing |
| [Python](python.md) | `ticoraw.read()` with numpy arrays, `ticoraw.to_dng()` | `pip install` from a clone (builds with maturin) | PyPI wheels: pending licensing |
| [JavaScript](javascript.md) | `toDng()` and `decode()` in WebAssembly, for browsers and Node.js | Build with `wasm-pack` | npm: pending licensing |

"Pending licensing" means these compiled packages are built and tested in CI, but not
published until a patent-licensing review is done. Until then, each guide shows how to
build from source. See [Patents and downloads](../patents.md).

## What every binding offers

- **Convert** a HE/HE\* NEF to a complete DNG (lossless tiles, EXIF, GPS, XMP, MakerNote,
  preview).
- **Decode** it to the raw CFA mosaic: 16-bit samples holding the 14-bit values Adobe's
  converters store, plus the CFA pattern, black and white levels, colour matrix, as-shot
  white balance, orientation, crop and baseline exposure.
- **Inspect** a file (camera, size, compression mode, codestream facts) without decoding
  pixels.
- **Typed errors** for every expected failure, including a classic (non-HE) NEF, which is
  reported as "unsupported". Malformed input never crashes the decoder.

## Licence

All of it is MIT OR Apache-2.0, at your option. The DNG writer's preview encoder,
[`jpeg-encoder`](https://crates.io/crates/jpeg-encoder), is `(MIT OR Apache-2.0) AND IJG`:
if you distribute a build, say that "this software is based in part on the work of the
Independent JPEG Group".
