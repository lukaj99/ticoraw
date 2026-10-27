# ticoraw

**ticoraw converts Nikon HE and HE\* raw files to DNG files that almost every raw editor
opens: free, private, and bit-exact.**

Nikon's newer cameras can save raw photos in two space-saving modes, called **High
Efficiency** (HE) and **High Efficiency\*** (HE\*). Many photo editors cannot open these
files yet. ticoraw reads them on your own computer and writes standard DNG raw files,
which those editors do open.

- **Bit-exact.** The raw values ticoraw decodes are identical, photosite for photosite,
  to what Adobe's converters produce. This is checked on every Nikon body that writes
  HE or HE\*.
- **Complete DNGs.** Your EXIF data, GPS position, XMP, Nikon MakerNote and a preview are
  kept. The raw data is stored with lossless compression.
- **Private.** Everything runs on your computer. Nothing is uploaded, and there is no
  telemetry.
- **Safe for your files.** Your original NEFs are only ever read, never changed.
- **Open source.** MIT or Apache-2.0, at your choice.

## Supported cameras

| Body | HE | HE\* |
|---|---|---|
| Nikon Z 9 | Bit-exact | Bit-exact |
| Nikon Z 8 | Bit-exact | Bit-exact |
| Nikon Z f | Bit-exact | Bit-exact |
| Nikon Z6III | Bit-exact | Bit-exact |
| Nikon Z 5 II | Bit-exact | Bit-exact |
| Nikon Z 50 II | Bit-exact | Bit-exact |
| Nikon ZR | No public sample yet | Bit-exact |

"Bit-exact" means checked against Adobe DNG Converter 18.7 on public sample files, with
0 differing photosites. The [compatibility matrix](compat.md) has the details.

## Where to start

| You are... | Start here |
|---|---|
| A photographer whose editor won't open HE files | [My editor can't open my HE files](photographers/index.md) |
| Someone who converts many files or writes scripts | [Converting photos](converting.md) and the [CLI reference](reference/cli.md) |
| A developer adding HE support to an app | [Library guides](libraries/index.md) for Rust, C, Python and JavaScript |
| Someone who implements or audits decoders | The [format specification](format.md) and the [conformance kit](conformance.md) |

## How to get it today

Today you **build ticoraw from its source code**. One command does it, and the
[install guide](photographers/install.md) walks you through it on macOS, Windows and
Linux, even if you have never used a terminal.

Ready-made downloads, Python and npm packages, and a converter that runs in your web
browser are **coming, but not available yet**. They are waiting on a review of patent
licensing for the compression format inside HE files. [Patents and
downloads](patents.md) explains this calmly and in plain words.

## About this project

ticoraw is an independent, clean-room implementation. It is not affiliated with, endorsed
by, or sponsored by Nikon, intoPIX or Adobe. Nikon is a trademark of Nikon Corporation.
intoPIX and TicoRAW are trademarks of intoPIX SA. Adobe, Lightroom and DNG are trademarks
of Adobe Inc.

The source code is at [github.com/lukaj99/ticoraw](https://github.com/lukaj99/ticoraw).
Questions are welcome in
[Discussions](https://github.com/lukaj99/ticoraw/discussions).
