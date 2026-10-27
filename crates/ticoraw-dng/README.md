# ticoraw-dng

Writes complete DNG files from Nikon **High Efficiency (HE)** and **High Efficiency\*
(HE\*)** NEFs, using the bit-exact [`ticoraw`](https://github.com/lukaj99/ticoraw) decoder.
The output is meant as a drop-in replacement for an Adobe DNG Converter file: the camera
metadata survives, file browsers show a preview, and the raw data is losslessly compressed.

Like `ticoraw`, the crate does no I/O: bytes in, bytes out.

```rust
let nef = std::fs::read("DSC_0001.NEF")?;
let opts = ticoraw_dng::Options {
    original_raw_file_name: Some("DSC_0001.NEF".into()),
    ..ticoraw_dng::Options::default()
};
std::fs::write("DSC_0001.dng", ticoraw_dng::convert(&nef, &opts)?)?;
```

## What the DNG contains

| Part | Contents |
|---|---|
| IFD0 | 256-pixel 8-bit RGB thumbnail; `Make`, `Model`, `Software`; the NEF's `DateTime`, `Artist`, `Copyright`; `Orientation`; DNG 1.4 (backward 1.1); Adobe's `UniqueCameraModel`; D65 `ColorMatrix1`, `AsShotNeutral`, `BaselineExposure`; `LensInfo` and `CameraSerialNumber` from EXIF; `OriginalRawFileName` (optional); XMP; the Nikon MakerNote in `DNGPrivateData` |
| EXIF IFD | Every EXIF field of the NEF except the MakerNote, copied verbatim, with its Interoperability IFD; `ExifVersion` is added when the NEF lacks it |
| GPS IFD | Copied verbatim when present |
| SubIFD 0 | Full-resolution CFA mosaic, 16 bits per sample: 256x256 lossless JPEG tiles (default) or one uncompressed strip; CFA pattern, black and white levels, default crop, active area |
| SubIFD 1 | JPEG preview (unless disabled): the camera's own JPEG for SDR frames, or a half-size rendering for HLG frames |

The MakerNote is wrapped as the DNG specification describes (`Adobe\0`, `MakN`, length,
source byte order, original offset, then the MakerNote unchanged), so ExifTool and Adobe
software still read the Nikon fields. The DNG is written in the NEF's byte order, so
verbatim fields stay valid.

### Previews

- **SDR frames** embed JPEG previews of several sizes. The smallest one with a long side
  of at least 1024 pixels (the camera's mid-size preview) is copied unchanged: no cost, it
  shows the camera's rendering, and it keeps the DNG near Adobe's size.
- **HLG frames** only embed 10-bit HEVC previews, which DNG cannot carry. For them the
  preview is rendered from the raw data at half size: each 2x2 CFA cell becomes one pixel
  (no demosaic), white balanced with `AsShotNeutral`, converted to sRGB through the inverse
  of the D65 colour matrix, scaled by `BaselineExposure` and JPEG-encoded at quality 90.
- The IFD0 thumbnail is always that rendering, downscaled.

### Not written

Adobe's camera profile data (`ForwardMatrix`, `ProfileHueSatMap`, `ProfileLookTable`,
`ProfileToneCurve`, ...) is Adobe's and is not reproduced. Lightroom and Camera Raw apply
their own profile anyway, because `UniqueCameraModel` carries Adobe's camera name. Also not
written: a second (illuminant A) colour matrix, `NoiseProfile`, lens-correction opcode lists,
`RawDataUniqueID` and the raw-image digests.

## Options

| Field | Default | Meaning |
|---|---|---|
| `compression` | `LosslessJpeg` | `Uncompressed` writes one 16-bit strip (about three times larger) |
| `preview` | `Auto` | `None` skips SubIFD 1 (the IFD0 thumbnail is always written) |
| `carry_xmp` | `true` | Copy the NEF's XMP packet |
| `extra_xmp` | `None` | An XMP packet (for example a merged sidecar) that **replaces** the NEF's; written even when `carry_xmp` is `false` |
| `original_raw_file_name` | `None` | Written as `OriginalRawFileName` |

The lower-level `write(&DecodedRaw, &CarriedMetadata, &Options)` takes a decoded image and
the metadata to carry separately; `CarriedMetadata::from_nef` collects the latter.

## Lossless compression

Tiles are lossless JPEG (ITU-T T.81 SOF3) with two interleaved components and predictor 1,
so every photosite is predicted from its same-colour neighbour. Each tile gets its own
optimal Huffman tables (limited to 16-bit codes), and tiles are encoded in parallel with the
default `parallel` feature. A Z6III HE\* frame comes out at about Adobe's size.

Every DNG decodes back to exactly the mosaic `ticoraw::decode_nef` returns; the crate's
tests check this on synthetic files, and on real files when `TICORAW_CORPUS` points at a
directory of NEFs (see `tests/corpus.rs`).

## Licence

MIT OR Apache-2.0, like the rest of the workspace. The preview encoder dependency,
[`jpeg-encoder`](https://crates.io/crates/jpeg-encoder), is `(MIT OR Apache-2.0) AND IJG`:
binary distributions must state that the software is based in part on the work of the
Independent JPEG Group.
