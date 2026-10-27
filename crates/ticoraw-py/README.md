# ticoraw for Python

Read Nikon **High Efficiency (HE)** and **High Efficiency\* (HE\*)** NEF raw files from
Python, and convert them to DNG files that every raw processor opens.

The decoder is [ticoraw](https://github.com/lukaj99/ticoraw), written in Rust. Its output is
**bit-exact**: every photosite equals what Adobe's converters produce, on the Nikon Z 9, Z 8,
Z f, Z6III, Z 5 II, Z 50 II and ZR.

> **No prebuilt wheels yet.** JPEG XS and TicoRAW are covered by patents, so compiled
> packages (wheels) are not published to PyPI, pending patent-licensing review. Build it
> from source as shown below. See the [patent notice](#patent-notice).

## Install from source

You need Python 3.9 or later and a Rust toolchain from <https://rustup.rs> (1.85 or later).

```sh
git clone https://github.com/lukaj99/ticoraw
pip install ./ticoraw/crates/ticoraw-py
```

## Convert a NEF to DNG

```python
import ticoraw

ticoraw.to_dng("DSC_0001.NEF", "DSC_0001.dng")
```

The DNG keeps the camera's EXIF, GPS, XMP and MakerNote, has a preview, and stores the raw
data with lossless compression (about the size of an Adobe DNG). Convert a whole folder:

```python
from pathlib import Path
import ticoraw

for nef in Path("import").glob("*.NEF"):
    try:
        ticoraw.to_dng(nef, nef.with_suffix(".dng"))
    except ticoraw.Unsupported:
        print(f"{nef.name}: not an HE/HE* file, skipped")
```

## Work with the raw data in numpy

```python
import numpy as np
import ticoraw

raw = ticoraw.read("DSC_0001.NEF")
mosaic = raw.raw_image                  # numpy uint16, shape (height, width), not copied
print(raw.model, mosaic.shape, raw.raw_pattern.tolist())   # NIKON Z6_3 (4040, 6064) [[0, 1], [3, 2]]

# Black-subtract and normalise, channel by channel like rawpy.
black = np.array(raw.black_level_per_channel, dtype=np.float32)[raw.raw_pattern]
tiled = np.tile(black, (raw.height // 2, raw.width // 2))
linear = (mosaic - tiled) / (raw.white_level - tiled)

# The camera's recommended crop, as (x, y, width, height).
x, y, w, h = raw.crop
visible = mosaic[y : y + h, x : x + w]
```

The attribute names follow [rawpy](https://pypi.org/project/rawpy/) where the meaning is
the same, so code written for rawpy mostly carries over:

| Attribute | Meaning |
|---|---|
| `raw_image` | The whole CFA mosaic, `uint16`, `(height, width)`: the 14-bit values Adobe stores, no black subtracted |
| `width`, `height` | Mosaic size in photosites |
| `raw_pattern`, `color_desc` | 2x2 `uint8` indices into `color_desc` (`b"RGBG"`), as in rawpy: `[[0, 1], [3, 2]]` for RGGB |
| `cfa_pattern` | The same pattern as DNG `CFAPattern` codes (0 = R, 1 = G, 2 = B): `((0, 1), (1, 2))` |
| `black_level_per_channel` | Black level per `color_desc` channel (R, G, B, G2) |
| `white_level` | Saturation level |
| `color_matrix` | XYZ (D65) to camera, 3x3 `float64` (DNG `ColorMatrix1`; rawpy calls it `rgb_xyz_matrix`); `None` for unknown bodies |
| `camera_whitebalance` | As-shot multipliers (R, G, B, G2), G = 1; `None` if the file has none |
| `make`, `model` | TIFF `Make` and `Model` |
| `orientation` | TIFF `Orientation` (1 = normal, 6 / 8 = portrait) |
| `crop` | The camera's recommended crop `(x, y, width, height)`, or `None` |
| `baseline_exposure` | EV a raw processor should add (DNG `BaselineExposure`), or `None` |

## API

| Function | What it does |
|---|---|
| `ticoraw.read(source) -> RawImage` | Decode a NEF |
| `ticoraw.inspect(source) -> dict` | Camera, size, compression mode and codestream facts, without decoding pixels |
| `ticoraw.to_dng(source, dest=None, *, compression="lossless", preview=True, xmp=None)` | Write a DNG to `dest`, or return it as `bytes` when `dest` is `None`. `compression="uncompressed"` writes one plain 16-bit strip; `preview=False` skips the full-size preview; `xmp` (`str` or `bytes`) replaces the NEF's XMP packet, for example with a sidecar |

`source` is a path (`str` or `os.PathLike`) or the file's contents (`bytes` or `bytearray`).
The GIL is released while files are read, decoded and written, so threads run in parallel.
Decoding a 24 MP frame takes a few tens of milliseconds.

Errors are exceptions: missing files raise the usual `OSError` subclasses
(`FileNotFoundError`, ...), and bad or unsupported files raise a subclass of
`ticoraw.TicoError`:

| Exception | Raised when |
|---|---|
| `ticoraw.Truncated` | The file ends early (`what` and `offset` say where) |
| `ticoraw.Container` | Not a Nikon TIFF/NEF, or a required tag is missing |
| `ticoraw.Header` | The codestream header is malformed |
| `ticoraw.Unsupported` | A classic (non-HE) NEF, or a codestream outside the supported profile |
| `ticoraw.Entropy` | The compressed pixel data is corrupt |
| `ticoraw.Jpeg` | A lossless JPEG stream is malformed |
| `ticoraw.Dng` | The DNG cannot be written (for example, it would exceed 4 GiB) |

The package ships type hints (`py.typed`).

## Development

```sh
cd crates/ticoraw-py
python -m venv .venv && . .venv/bin/activate
pip install maturin pytest numpy        # rawpy too, for the cross-checks
maturin develop --release
pytest
```

The tests need no camera files. Two environment variables add real decodes:

- `TICORAW_CONFORMANCE_DIR`: a folder filled by `conformance/fetch.sh`; every decode must
  hash to the Adobe-verified SHA-256 in `conformance/manifest.toml`.
- `TICORAW_CORPUS`: a folder of NEFs and DNGs converted from them; with rawpy installed,
  each DNG's mosaic (read by LibRaw) must equal the decode at every photosite.

This crate is its own Cargo workspace (like `fuzz/`), so the main workspace builds and
tests without Python. CI (`.github/workflows/python.yml`) builds abi3 wheels for Linux
(manylinux x86_64, aarch64), macOS (arm64, x86_64) and Windows (x64), tests each one, and
keeps them only as short-lived workflow artifacts. Its PyPI publish job is gated, pending
patent-licensing review: it runs only from a manual run with `publish` ticked **and** with
the repository variable `ALLOW_BINARY_DISTRIBUTION` set to `true`, which it is not.

## Patent notice

JPEG XS (ISO/IEC 21122) is covered by essential patents, licensed through a pool
administered by Vectis IP, and intoPIX describes TicoRAW as patented. This project provides
source code for interoperability and research; its MIT and Apache-2.0 licences cover
copyright only and grant no patent licence. You are responsible for determining whether you
need a patent licence to use, build or distribute this software or binaries made from it in
your jurisdiction. This is not legal advice. See the
[main README](https://github.com/lukaj99/ticoraw#patent-notice).

## Licence

MIT OR Apache-2.0. The DNG preview encoder,
[`jpeg-encoder`](https://crates.io/crates/jpeg-encoder), is `(MIT OR Apache-2.0) AND IJG`:
this software is based in part on the work of the Independent JPEG Group.

Nikon is a trademark of Nikon Corporation; intoPIX and TicoRAW are trademarks of intoPIX SA;
Adobe and DNG are trademarks of Adobe Inc. This project is not affiliated with any of them.
