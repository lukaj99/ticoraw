# Converting photos

`ticoraw convert` turns HE and HE\* NEFs into complete DNG files. This page shows what it
can do, with examples. The [CLI reference](reference/cli.md) has the exact help text of
every command.

```sh
ticoraw convert [options] <file|dir>...
```

You can name any mix of files and folders. Folders are searched for files ending in
`.nef` or `.nrw`, in any case (`.NEF`, `.Nef`, ...).

## Examples

```sh
# One photo
ticoraw convert DSC_0001.NEF

# A few photos
ticoraw convert DSC_0001.NEF DSC_0002.NEF DSC_0003.NEF

# A whole card, including its subfolders
ticoraw convert ~/Pictures/MyCard -r

# The same, writing the DNGs to another folder
ticoraw convert ~/Pictures/MyCard -r -o ~/Pictures/MyCard-DNG

# Only photos converted since last time
ticoraw convert ~/Pictures/MyCard -r --skip-existing

# Use Lightroom-style sidecars (DSC_0001.xmp) for ratings and keywords
ticoraw convert ~/Pictures/MyCard -r --with-sidecar

# Machine-readable output for a script
ticoraw convert ~/Pictures/MyCard -r --json > convert-log.jsonl
```

## Options

| Option | What it does |
|---|---|
| `-r`, `--recursive` | Also look in subfolders. Symbolic links to folders are skipped |
| `-o`, `--output <dir>` | Write the DNGs under `<dir>` instead of next to each NEF. The folder is created if needed |
| `--skip-existing` | Leave a NEF alone if its DNG already exists |
| `-f`, `--force` | Replace DNGs that already exist |
| `--with-sidecar` | Use the `.xmp` sidecar next to each NEF as the DNG's XMP (see [Sidecars](#sidecars)) |
| `--uncompressed` | Store the raw data uncompressed (2 bytes per photosite): simpler to read, but much larger |
| `--no-preview` | Leave out the embedded preview (the camera's mid-size JPEG, or a rendered one for HLG frames). The small thumbnail is always kept |
| `-j`, `--jobs <n>` | How many files to convert at once (see [Speed](#speed)) |
| `--json` | Print one JSON object per line instead of text (see [For scripts](#for-scripts)) |

Without `--skip-existing` or `--force`, an existing DNG is never replaced: that file is
reported as failed and the others are still converted.

## Where the DNGs go

**Without `-o`**, each DNG is written next to its NEF, with the same name and a `.dng`
extension: `DCIM/100NZ6_3/DSC_0001.NEF` gets `DCIM/100NZ6_3/DSC_0001.dng`.

**With `-o <dir>`**, files found inside a folder you named keep their path relative to that
folder. Files you named directly go straight into `<dir>`:

```text
ticoraw convert MyCard -r -o Out          MyCard/DCIM/100NZ6_3/DSC_0001.NEF
                                       -> Out/DCIM/100NZ6_3/DSC_0001.dng

ticoraw convert MyCard/DCIM/100NZ6_3/DSC_0001.NEF -o Out
                                       -> Out/DSC_0001.dng
```

If two NEFs would end up with the same DNG name (compared ignoring upper and lower case, as
macOS and Windows do), the second one fails with a message instead of overwriting the first.
A file named twice is converted once.

## Which files are converted

- **HE and HE\* NEFs** are converted.
- **Classic NEFs** (lossless, lossy or uncompressed) are **skipped**, with a note: your
  editor already opens them. A skipped file is not a failure.
- **Other files** with a `.nef` or `.nrw` name that are not Nikon raw files fail with a
  message, such as `truncated TIFF header at byte 0`.

## What's inside each DNG

The DNG is meant as a drop-in replacement for one made by Adobe DNG Converter:

- the **raw mosaic**, with exactly the values Adobe's converters produce, stored as
  lossless JPEG tiles (or uncompressed with `--uncompressed`);
- the camera's **EXIF** data (date, exposure, lens, ...), the **GPS** position when there is
  one, the **XMP** packet and the Nikon **MakerNote**, copied unchanged;
- a **preview**: the camera's own JPEG for normal (SDR) photos, or a half-size rendering for
  HLG photos, whose camera previews DNG cannot hold; plus a small thumbnail;
- the colour matrix, as-shot white balance, orientation, the camera's crop and
  `BaselineExposure`, all matching Adobe's;
- the NEF's file name as `OriginalRawFileName`.

The [ticoraw-dng README](https://github.com/lukaj99/ticoraw/blob/main/crates/ticoraw-dng/README.md)
lists every part in detail, including what is not written.

## Sidecars

Lightroom, Bridge and Capture One can save your edits, ratings and keywords next to a raw
file in a **sidecar** file, `DSC_0001.xmp` for `DSC_0001.NEF`.

With `--with-sidecar`, ticoraw looks for `<name>.xmp` (in any case) next to each NEF and,
when it finds one, writes it into the DNG **instead of** the NEF's own XMP packet. NEFs
without a sidecar keep their own XMP. Without the flag, sidecars are ignored.

darktable's sidecars are named differently (`DSC_0001.NEF.xmp`) and are not used.

A sidecar must be an XMP packet (it must contain `<x:xmpmeta` or `<?xpacket`), or that
photo fails with a message.

## File sizes

The DNGs are larger than the NEFs. HE and HE\* compression is lossy (the camera keeps very
high quality but discards some data), while a DNG stores the decoded values with
**lossless** compression, which cannot be as small. Measured on the 13 public conformance
samples:

| Camera, mode | NEF | DNG (default) | DNG (`--uncompressed`, about) |
|---|---|---|---|
| Z6III HE, 24.5 MP | 13.4 MB | 28.4 MB | 49 MB |
| Z f HE\*, 24.5 MP | 19.6 MB | 28.1 MB | 49 MB |
| ZR HE\* (HLG), 24.5 MP | 15.8 MB | 19.0 MB | 49 MB |
| Z 9 HE, 45.7 MP | 24.8 MB | 46.0 MB | 92 MB |
| Z 9 HE\*, 45.7 MP | 35.6 MB | 46.7 MB | 92 MB |

As a rule of thumb, a default DNG is 1.2 to 2.2 times the size of its NEF; HE files grow
more than HE\* files, because they were compressed harder. For a Z6III HE\* frame, the DNG
is about the size of Adobe DNG Converter's compressed DNG. `--uncompressed` uses 2 bytes per
photosite, plus the preview.

## Speed

Each file is decoded and compressed on every core of your computer, and a few files are
converted at the same time. On an Apple M5 Pro, the 13 public samples (251 MB of NEFs, 24
to 46 megapixels each) convert in 0.7 seconds, about 18 files per second.

`--jobs` sets how many files are converted at once. The default is up to 4, and at most one
per 2 GB of memory, which gives nearly all the speed. Each file in progress needs several
hundred megabytes of memory, so lower `--jobs` (for example `-j 1`) on a computer that is
short of memory.

## Privacy and safety

- **Everything stays on your computer.** ticoraw makes no network connections, uploads
  nothing and collects no telemetry.
- **Your NEFs are only read**, never changed, moved or deleted.
- **Writes are atomic.** Each DNG is written to a temporary file next to its final place,
  then renamed into place. If anything fails part-way, you never get a half-written DNG,
  and an existing file is never destroyed.
- **No accidental overwrites.** Existing DNGs are kept unless you pass `--force`, and ticoraw
  refuses to write over its own input.

## For scripts

`--json` prints [JSON Lines](https://jsonlines.org/): a `start` event, one event per file
(`converted`, `skipped` or `failed`), then a `summary`.

```json
{"event":"start","ticoraw_version":"0.1.0","files":1,"jobs":1}
{"event":"converted","input":"MyCard/DCIM/100NZ6_3/DSC_0001.NEF","output":"Out/DCIM/100NZ6_3/DSC_0001.dng","sidecar":null,"bytes_in":13429248,"bytes_out":28355588,"seconds":0.075,"warning":null}
{"event":"summary","files":1,"converted":1,"skipped":0,"failed":0,"bytes_in":13429248,"bytes_out":28355588,"seconds":0.076,"files_per_second":13.099,"mb_per_second":175.904}
```

The exit status is 0 when no file failed (skipped files are not failures), 1 when any
file failed, and 2 when the command line itself is wrong.

## Other commands

| Command | Use it to |
|---|---|
| `ticoraw scan <dir> -r` | List which NEFs are HE/HE\* and which are classic, without converting |
| `ticoraw decode <file.nef>` | Convert one file, optionally to a 16-bit PGM of the raw mosaic (`--format pgm`) |
| `ticoraw inspect <file.nef>` | Show the camera, size, compression mode and codestream details |
| `ticoraw verify <file.nef> <reference.dng>` | Compare ticoraw's decode with a DNG from another converter, photosite by photosite |
| `ticoraw report <file.nef>` | Print a privacy-safe report to paste into a bug report |

Their options are in the [CLI reference](reference/cli.md).
