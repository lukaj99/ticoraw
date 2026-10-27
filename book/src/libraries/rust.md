# Rust

Two crates do the work:

- **`ticoraw`** decodes HE/HE\* NEFs. It is a pure function from bytes to values: no file
  access, no `unsafe`, and no dependencies without its default `parallel` feature.
- **`ticoraw-dng`** writes complete DNGs from them: lossless JPEG tiles, EXIF, GPS, XMP,
  the MakerNote and a preview.

## Add the crates

The crates are not on crates.io yet, so depend on the Git repository. You need Rust 1.85 or
later.

```toml
[dependencies]
ticoraw = { git = "https://github.com/lukaj99/ticoraw" }
ticoraw-dng = { git = "https://github.com/lukaj99/ticoraw" }
```

This follows the `main` branch. To pin an exact commit, add `rev = "<commit hash>"` to both
lines. Either way, Cargo records the commit it used in `Cargo.lock`.

Every example below is a complete `src/main.rs` that reads `DSC_0001.NEF` from the current
directory. Each one has been compiled and run against this version of the crates.

## Convert a NEF to DNG

```rust
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let nef = std::fs::read("DSC_0001.NEF")?;

    let options = ticoraw_dng::Options {
        original_raw_file_name: Some("DSC_0001.NEF".into()),
        ..ticoraw_dng::Options::default()
    };
    let dng = ticoraw_dng::convert(&nef, &options)?;

    std::fs::write("DSC_0001.dng", dng)?;
    Ok(())
}
```

`Options::default()` gives lossless compression, an automatic preview and the NEF's own
XMP. This is what `ticoraw convert` does for each file, minus the atomic write.

## Inspect a file without decoding it

```rust
use std::error::Error;

use ticoraw::RawCodec;

fn main() -> Result<(), Box<dyn Error>> {
    let nef = std::fs::read("DSC_0001.NEF")?;
    let info = ticoraw::inspect_nef(&nef)?;

    println!("{} {}x{}", info.model, info.width, info.height);
    if let Some(mode) = info.compression {
        println!("mode: {mode}");
    }
    match info.codec {
        RawCodec::TicoRaw => println!("High Efficiency: ticoraw can decode it"),
        RawCodec::Classic => println!("classic NEF: other raw libraries already read it"),
    }
    Ok(())
}
```

For the Nikon ZR sample this prints:

```text
NIKON ZR 6064x4040
mode: High Efficiency* (HE*)
High Efficiency: ticoraw can decode it
```

To sort through many files quickly, read just the start of each one.
`inspect_nef_prefix` needs only the TIFF structures and the codestream header, which in
practice fit in the first megabyte or two:

```rust
use std::error::Error;
use std::fs::File;
use std::io::Read;

fn main() -> Result<(), Box<dyn Error>> {
    let file = File::open("DSC_0001.NEF")?;
    let file_len = usize::try_from(file.metadata()?.len())?;

    // The TIFF structures and the codestream header sit near the start of the file.
    let mut head = Vec::new();
    file.take(2 << 20).read_to_end(&mut head)?;

    let info = ticoraw::inspect_nef_prefix(&head, file_len)?;
    println!("{}: {:?}", info.model, info.codec);
    Ok(())
}
```

A prefix that is too short fails with `TicoError::Truncated`; read more and try again.

## Work with the raw mosaic

`decode_nef` returns a `DecodedRaw`: the CFA mosaic as `u16` samples, plus everything
needed to interpret it.

```rust
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let nef = std::fs::read("DSC_0001.NEF")?;
    let raw = ticoraw::decode_nef(&nef)?;

    let image = &raw.image;
    let color = &raw.color;
    println!(
        "{} {}x{}, CFA {:?}, black {:?}, white {}",
        raw.model, image.width, image.height, color.cfa_pattern, color.black_level,
        color.white_level
    );

    // Average of the red photosites, black level subtracted. The 2x2 CFA pattern is
    // row-major: index (y % 2) * 2 + (x % 2), with 0 = red, 1 = green, 2 = blue.
    let (mut sum, mut count) = (0u64, 0u64);
    for y in 0..image.height {
        for x in 0..image.width {
            let site = (y % 2) * 2 + x % 2;
            if color.cfa_pattern[site] == 0 {
                let value = image.pixels[y * image.width + x];
                sum += u64::from(value.saturating_sub(color.black_level[site]));
                count += 1;
            }
        }
    }
    println!("mean red: {:.1}", sum as f64 / count as f64);
    Ok(())
}
```

| `DecodedRaw` field | Meaning |
|---|---|
| `make`, `model` | TIFF `Make` and `Model`, for example `NIKON Z6_3` |
| `image` | `CfaImage { width, height, pixels }`: the whole mosaic, row-major, no crop, black subtraction or scaling |
| `color.cfa_pattern` | 2x2 pattern codes, row-major (0 = R, 1 = G, 2 = B); `[0, 1, 1, 2]` is RGGB |
| `color.black_level`, `color.white_level` | Per-site black levels and the saturation level |
| `color.color_matrix` | XYZ (D65) to camera matrix as rationals; `None` for bodies without a built-in profile |
| `color.as_shot_neutral` | As-shot white balance as camera-neutral rationals (R, G, B) |
| `orientation` | TIFF orientation (1 = normal, 6 and 8 = portrait) |
| `crop` | The camera's recommended crop, `Option<Crop { x, y, width, height }>` |
| `baseline_exposure` | Adobe's `BaselineExposure` in EV, as a rational, including +1 EV for HLG frames |

## Handle errors

Every expected failure is a value. `ticoraw` returns `TicoError` (`Truncated`, `Container`,
`Header`, `Unsupported`, `Entropy`, `Jpeg`), and `ticoraw-dng` wraps it in
`ticoraw_dng::Error::Decode`. A classic NEF is `Unsupported`:

```rust
use std::error::Error;
use std::path::Path;

use ticoraw::TicoError;

/// Converts one NEF. Returns `Ok(false)` for a file that is not High Efficiency.
fn convert(nef_path: &Path) -> Result<bool, Box<dyn Error>> {
    let nef = std::fs::read(nef_path)?;
    let options = ticoraw_dng::Options {
        original_raw_file_name: nef_path.file_name().map(|n| n.to_string_lossy().into()),
        ..ticoraw_dng::Options::default()
    };
    match ticoraw_dng::convert(&nef, &options) {
        Ok(dng) => {
            std::fs::write(nef_path.with_extension("dng"), dng)?;
            Ok(true)
        }
        // A classic NEF, or a codestream outside the supported profile.
        Err(ticoraw_dng::Error::Decode(TicoError::Unsupported(why))) => {
            eprintln!("{}: skipped: {why}", nef_path.display());
            Ok(false)
        }
        Err(e) => Err(format!("{}: {e}", nef_path.display()).into()),
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    for arg in std::env::args_os().skip(1) {
        convert(Path::new(&arg))?;
    }
    Ok(())
}
```

Hostile input is bounded before anything large is allocated, and no input makes the decoder
panic. Note that `std::fs::write` replaces an existing file; write to a temporary file and
rename it, as the CLI does, if that matters to you.

## Choose DNG options, or change the data first

`convert` is three steps: `decode_nef`, `CarriedMetadata::from_nef` and `write`. Call them
yourself to look at the image or the metadata before writing:

```rust
use std::error::Error;

use ticoraw_dng::{CarriedMetadata, Compression, Options, Preview};

fn main() -> Result<(), Box<dyn Error>> {
    let nef = std::fs::read("DSC_0001.NEF")?;

    let options = Options {
        compression: Compression::Uncompressed, // one plain 16-bit strip
        preview: Preview::None,                 // the small thumbnail is still written
        extra_xmp: Some(std::fs::read("DSC_0001.xmp")?), // replaces the NEF's XMP
        original_raw_file_name: Some("DSC_0001.NEF".into()),
        ..Options::default()
    };

    // convert() is these three steps; call them yourself to look at or change the data.
    let raw = ticoraw::decode_nef(&nef)?;
    let carried = CarriedMetadata::from_nef(&nef)?;
    let dng = ticoraw_dng::write(&raw, &carried, &options)?;

    // Every DNG decodes back to exactly the mosaic it was written from.
    let back = ticoraw::dng::decode_dng_cfa(&dng)?;
    assert_eq!(back.image, raw.image);

    std::fs::write("DSC_0001.dng", dng)?;
    Ok(())
}
```

| `Options` field | Default | Meaning |
|---|---|---|
| `compression` | `Compression::LosslessJpeg` | `Compression::Uncompressed` writes one 16-bit strip (about three times larger) |
| `preview` | `Preview::Auto` | `Preview::None` skips the full-size preview; the thumbnail is always written |
| `carry_xmp` | `true` | Copy the NEF's XMP packet |
| `extra_xmp` | `None` | An XMP packet that **replaces** the NEF's, written even when `carry_xmp` is `false` |
| `original_raw_file_name` | `None` | Written as `OriginalRawFileName` |

`ticoraw::dng::decode_dng_cfa` also reads DNGs from other converters (lossless JPEG or
uncompressed), which is how `ticoraw verify` compares against Adobe.

## Threads and WebAssembly

The default `parallel` feature uses [rayon](https://crates.io/crates/rayon) to decode and
encode on every core. Turn it off for a single-threaded build with no dependencies in
`ticoraw`; the output is identical:

```toml
[dependencies]
ticoraw = { git = "https://github.com/lukaj99/ticoraw", default-features = false }
ticoraw-dng = { git = "https://github.com/lukaj99/ticoraw", default-features = false }
```

Without it, both crates build for `wasm32-unknown-unknown`. The
[JavaScript guide](javascript.md) shows the ready-made WebAssembly package.

## API stability

The crate roots (`ticoraw::{inspect_nef, inspect_nef_prefix, decode_nef, decode_ticoraw,
...}` and `ticoraw_dng::{convert, write, Options, ...}`) and
`ticoraw::dng::{decode_dng_cfa, encode_uncompressed_dng}` are the stable API. The
lower-level stage modules (`bitstream`, `container`, `entropy`, `header`, `mct`, `tiff`,
`wavelet`) are public for tools that want to look inside the codestream, and may change in
minor releases. Before 1.0, a minor version (0.x) may change the API.

Run `cargo doc --open` in your project for the full reference of every item.
