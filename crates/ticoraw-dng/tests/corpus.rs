//! Round trip on real files: every HE/HE\* NEF in `TICORAW_CORPUS` is converted with both
//! compressions, and the DNG's CFA image must equal `ticoraw::decode_nef`'s at every
//! photosite. The DNG must also carry the NEF's MakerNote and EXIF, and a thumbnail that is
//! not all black.
//!
//! ```text
//! TICORAW_CORPUS=/path/to/nefs cargo test -p ticoraw-dng --release --test corpus -- --nocapture
//! ```
//!
//! `TICORAW_DNG_LIMIT=n` stops after `n` HE/HE\* files; `TICORAW_DNG_OUT=dir` also writes
//! each lossless-JPEG DNG there (as `<nef stem>.dng`) for inspection with other tools.

use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ticoraw::RawCodec;
use ticoraw::tiff::{self, Tiff};
use ticoraw_dng::{Compression, Options};

/// Enough of a NEF to classify it without reading the raw strip.
const PREFIX: usize = 4 << 20;

#[test]
fn corpus_round_trips_through_dng() {
    let Some(dir) = std::env::var_os("TICORAW_CORPUS").map(PathBuf::from) else {
        println!("corpus: TICORAW_CORPUS is not set; skipping");
        return;
    };
    let limit = std::env::var("TICORAW_DNG_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX);
    let out_dir = std::env::var_os("TICORAW_DNG_OUT").map(PathBuf::from);
    let mut nefs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("TICORAW_CORPUS must be a readable directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("nef") || e.eq_ignore_ascii_case("nrw"))
        })
        .collect();
    nefs.sort();

    println!(
        "\n{:<28} {:>9} {:>11} {:>8} {:>8} {:>11} {:>8}  preview",
        "nef", "nef MB", "ljpeg MB", "ms", "check", "uncomp MB", "ms"
    );
    let mut checked = 0;
    let mut failures = Vec::new();
    for path in &nefs {
        if checked >= limit {
            break;
        }
        if !is_ticoraw(path) {
            continue;
        }
        checked += 1;
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let nef = std::fs::read(path).unwrap();
        match check(&nef, &name, out_dir.as_deref()) {
            Ok(line) => println!("{line}"),
            Err(e) => {
                println!("{name:<28} FAILED: {e}");
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    println!(
        "corpus: {checked} HE/HE* files, {} failures",
        failures.len()
    );
    assert!(failures.is_empty(), "{failures:#?}");
}

fn is_ticoraw(path: &Path) -> bool {
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let len = f.metadata().map_or(0, |m| m.len() as usize);
    let mut prefix = Vec::with_capacity(PREFIX.min(len));
    if f.by_ref()
        .take(PREFIX as u64)
        .read_to_end(&mut prefix)
        .is_err()
    {
        return false;
    }
    let info = match ticoraw::inspect_nef_prefix(&prefix, len) {
        // The raw strip starts past the prefix: read the whole file.
        Err(ticoraw::TicoError::Truncated { .. }) => match std::fs::read(path) {
            Ok(all) => ticoraw::inspect_nef(&all),
            Err(_) => return false,
        },
        other => other,
    };
    info.is_ok_and(|i| i.codec == RawCodec::TicoRaw)
}

fn mb(n: usize) -> f64 {
    n as f64 / 1e6
}

fn check(nef: &[u8], name: &str, out_dir: Option<&Path>) -> Result<String, String> {
    let raw = ticoraw::decode_nef(nef).map_err(|e| e.to_string())?;
    let mut line = format!("{name:<28} {:>9.2}", mb(nef.len()));
    let mut preview = String::new();
    for compression in [Compression::LosslessJpeg, Compression::Uncompressed] {
        let opts = Options {
            compression,
            original_raw_file_name: Some(name.to_owned()),
            ..Options::default()
        };
        let t0 = Instant::now();
        let dng = ticoraw_dng::convert(nef, &opts).map_err(|e| e.to_string())?;
        let convert_ms = t0.elapsed().as_millis();
        let t1 = Instant::now();
        let back = ticoraw::dng::decode_dng_cfa(&dng).map_err(|e| e.to_string())?;
        let check_ms = t1.elapsed().as_millis();
        if back.image != raw.image {
            let n = back
                .image
                .pixels
                .iter()
                .zip(&raw.image.pixels)
                .filter(|(a, b)| a != b)
                .count();
            return Err(format!("{compression:?}: {n} mismatching photosites"));
        }
        if (back.black_level, back.white_level, back.cfa_pattern)
            != (
                raw.color.black_level,
                raw.color.white_level,
                raw.color.cfa_pattern,
            )
        {
            return Err(format!("{compression:?}: levels or CFA pattern differ"));
        }
        if compression == Compression::LosslessJpeg {
            preview = check_metadata(nef, &dng)?;
            if let Some(dir) = out_dir {
                let stem = Path::new(name).file_stem().unwrap().to_string_lossy();
                std::fs::write(dir.join(format!("{stem}.dng")), &dng).map_err(|e| e.to_string())?;
            }
            let _ = write!(
                line,
                " {:>11.2} {convert_ms:>8} {check_ms:>8}",
                mb(dng.len())
            );
        } else {
            let _ = write!(line, " {:>11.2} {convert_ms:>8}", mb(dng.len()));
        }
    }
    Ok(format!("{line}  {preview}"))
}

/// Checks the carried MakerNote and EXIF and the thumbnail; describes the preview.
fn check_metadata(nef: &[u8], dng: &[u8]) -> Result<String, String> {
    let e = |e: ticoraw::TicoError| e.to_string();
    let meta = ticoraw::container::parse_metadata(nef).map_err(e)?;
    let t = Tiff::parse(dng, 0).map_err(e)?;
    let ifd0 = t.ifd(t.first_ifd).map_err(e)?;
    if let Some(r) = meta.maker_note {
        let private = ifd0
            .get(0xC634)
            .map(|p| t.bytes(p))
            .ok_or("no DNGPrivateData")?;
        if !private.starts_with(b"Adobe\0MakN") || private[20..] != nef[r] {
            return Err("DNGPrivateData does not hold the NEF MakerNote".into());
        }
    }
    if meta.exif_ifd.is_some() {
        let exif = tiff::first_uint(&t, &ifd0, 0x8769)
            .map_err(e)?
            .ok_or("no EXIF IFD")?;
        let exif = t.ifd(exif as u32).map_err(e)?;
        if exif.get(0x9003).is_none() || exif.get(0x927C).is_some() {
            return Err("EXIF lacks DateTimeOriginal or still holds the MakerNote".into());
        }
    }
    let strip = tiff::require_uint(&t, &ifd0, 0x111).map_err(e)? as usize;
    let len = tiff::require_uint(&t, &ifd0, 0x117).map_err(e)? as usize;
    let thumb = &dng[strip..strip + len];
    let mean = thumb.iter().map(|&v| f64::from(v)).sum::<f64>() / len as f64;
    // Night frames are legitimately dark; only an all-black thumbnail is a failure.
    if thumb.iter().all(|&v| v == 0) {
        return Err("thumbnail is black".into());
    }
    let subs = t.uints(ifd0.get(0x14A).ok_or("no SubIFDs")?).map_err(e)?;
    let prev = t.ifd(*subs.get(1).ok_or("no preview")? as u32).map_err(e)?;
    let w = tiff::require_uint(&t, &prev, 0x100).map_err(e)?;
    let h = tiff::require_uint(&t, &prev, 0x101).map_err(e)?;
    let bytes = tiff::require_uint(&t, &prev, 0x117).map_err(e)?;
    let at = tiff::require_uint(&t, &prev, 0x111).map_err(e)? as usize;
    let copied = meta
        .jpeg_previews
        .iter()
        .any(|r| nef[r.clone()] == dng[at..at + bytes as usize]);
    Ok(format!(
        "{w}x{h} {} ({:.2} MB), thumbnail mean {mean:.0}",
        if copied { "camera JPEG" } else { "rendered" },
        mb(bytes as usize)
    ))
}
