# ticoraw (command-line tool)

`ticoraw` converts Nikon **High Efficiency (HE)** and **High Efficiency\* (HE\*)** NEF raw
files to complete DNGs, and inspects, verifies and scans them. Its pixels are
**bit-exact** against Adobe DNG Converter on every Nikon body that records HE/HE\*
([compatibility](https://github.com/lukaj99/ticoraw/blob/main/docs/compat.md)).

## Install

Build it from source with Rust 1.85 or later (get Rust from <https://rustup.rs>):

```sh
cargo install --git https://github.com/lukaj99/ticoraw ticoraw-cli
```

or from a clone:

```sh
git clone https://github.com/lukaj99/ticoraw
cd ticoraw
cargo install --path crates/ticoraw-cli
```

Both install a binary called `ticoraw`. Prebuilt binaries and package-manager installs are
not available yet: compiled decoders are on hold pending a patent-licensing review (see
the [patent notice](https://github.com/lukaj99/ticoraw#patent-notice)). A Homebrew formula
that builds from source is ready in
[`packaging/homebrew`](https://github.com/lukaj99/ticoraw/blob/main/packaging/homebrew/ticoraw.rb),
but its tap is not public yet.

## Commands

```sh
# Convert a card or folder to DNGs, keeping the folder structure
ticoraw convert --recursive /Volumes/NIKON/DCIM -o ~/Pictures/DNG
ticoraw convert --recursive --skip-existing /Volumes/NIKON/DCIM -o ~/Pictures/DNG  # resume
ticoraw convert DSC_0001.NEF DSC_0002.NEF                  # DNGs next to the NEFs

# Convert one file, or write its raw mosaic as a 16-bit PGM
ticoraw decode DSC_0001.NEF                                # writes DSC_0001.dng
ticoraw decode DSC_0001.NEF --format pgm -o mosaic.pgm

# Describe files without decoding pixels
ticoraw inspect DSC_0001.NEF
ticoraw inspect --json DSC_0001.NEF DSC_0002.NEF

# Compare a decode with a reference DNG, e.g. from Adobe DNG Converter
ticoraw verify DSC_0001.NEF DSC_0001.dng

# List the NEFs in a folder; optionally decode every HE/HE* file
ticoraw scan --recursive ~/Pictures/import
ticoraw scan --decode ~/Pictures/import

# A privacy-safe JSON report to paste into a bug report
ticoraw report DSC_0001.NEF
```

| Command | Options | What it does |
|---|---|---|
| `convert <file\|dir>...` | `-o, --output <dir>`, `-r, --recursive`, `--skip-existing`, `-f, --force`, `--uncompressed`, `--no-preview`, `--with-sidecar`, `-j, --jobs <n>`, `--json` | Converts every HE/HE\* NEF given, or found as a `.nef` / `.nrw` file (any case) in a given folder, to a complete DNG. Classic NEFs are skipped with a note. Prints one line per file and a summary with throughput |
| `decode <file.nef>` | `-o, --output <path>`, `--format dng\|pgm`, `--uncompressed`, `--no-preview`, `-f, --force` | Converts one file to a complete DNG (default) or a PGM. The default output is the input path with a `.dng` / `.pgm` extension |
| `inspect <file.nef>...` | `--json` | Camera model, raw dimensions, codec (TicoRAW or classic Nikon), MakerNote compression mode, raw strip location, black levels and, for TicoRAW strips, the codestream layout and transfer curve. `--json` prints one array with an object per file |
| `verify <file.nef> <reference.dng>` | | Decodes the NEF and the DNG's full-resolution CFA image (lossless JPEG or uncompressed), then prints the mismatch count, the largest absolute difference and mismatches per 2x2 CFA site |
| `scan <dir>` | `--decode`, `-r, --recursive` | Lists each `.nef` / `.nrw` file as TicoRAW (HE/HE\*), classic Nikon or error, then prints counts. `--decode` also decodes every TicoRAW file and reports failures and throughput |
| `report <file.nef>` | | Prints a JSON report for a bug report: ticoraw version, OS and architecture, camera make, model and firmware, compression mode, dimensions, codec, the codestream's main header in hex, and the decode outcome |

Every command takes `-h, --help`; `ticoraw -V` prints the version. Exit status is 0 on
success, 1 on failure (for `convert`, any failed file; for `verify`, any mismatch) and 2
on usage errors.

## Details worth knowing

- **What the DNG holds.** The mosaic in lossless JPEG tiles (`--uncompressed` writes one
  plain strip, about three times larger), the camera's EXIF, GPS, XMP and MakerNote, a
  thumbnail, a preview at least 1024 pixels on its long side (`--no-preview` leaves it
  out), the as-shot white balance and the NEF's file name as `OriginalRawFileName`. Colour
  matrix, levels, `BaselineExposure`, default crop, orientation and `UniqueCameraModel`
  equal Adobe's. See
  the [ticoraw-dng README](https://github.com/lukaj99/ticoraw/blob/main/crates/ticoraw-dng/README.md).
- **Sidecars.** `convert --with-sidecar` uses the `<name>.xmp` file next to each NEF (as
  Lightroom, Bridge or Capture One write it) as the DNG's XMP instead of the NEF's own.
- **Where DNGs go.** Next to each NEF, or with `-o` under that folder. Files found in a
  given folder keep their path relative to it, so a card's `100NCZ_6` folders reappear
  under the output folder.
- **Several files at once.** `convert` runs up to 4 files at a time by default (at most one
  per 2 GB of memory); each file already uses every core. `-j` changes that.
- **JSON Lines.** `convert --json` prints one JSON object per line: a `start` event, then
  `converted`, `skipped` or `failed` for each file, then a `summary`.
- **Writes are atomic.** DNGs and PGMs are written to a temporary file next to the output,
  then renamed into place (`--force`) or hard-linked only if the output does not exist
  yet. A failed write never leaves a partial file or destroys an existing one. The tool
  never writes over its input (on Unix, also when the output is a hard link to it).
- **Inputs are only read.** `scan` opens regular files only and, without `--decode`, reads
  just the start of each file. With `--recursive`, symbolic links to folders are skipped.
- **Unknown bodies.** A model without a built-in colour profile still decodes, with a
  warning: its DNG gets an identity colour matrix, so colours will be off.
- **PGM output** is the bare mosaic as big-endian 16-bit samples (maxval 16383 for 14-bit
  data, else 65535).
- **Privacy.** `report` leaves out serial numbers, GPS position, lens serial, owner,
  artist, copyright, dates, the file name and path, and all pixels.

## Speed

On an Apple M5 Pro, `ticoraw convert` turned the 13 public conformance samples (251 MB of
NEFs) into DNGs in 0.7 to 0.8 s, 14 to 19 files per second across runs. One 24.5 MP frame
decodes in about 35 ms and is fully converted, file I/O included, in about 75 ms.

## Licence

MIT OR Apache-2.0, at your option. The DNG preview encoder,
[`jpeg-encoder`](https://crates.io/crates/jpeg-encoder), is `(MIT OR Apache-2.0) AND IJG`:
this software is based in part on the work of the Independent JPEG Group. See the
[patent notice](https://github.com/lukaj99/ticoraw#patent-notice) before distributing
binaries.
