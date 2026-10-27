//! Bit-exact ground truth against Adobe-written DNGs.
//!
//! Set `TICORAW_CORPUS` to a directory holding NEFs and DNGs converted from them. Every DNG
//! whose IFD0 names its source (`OriginalRawFileName`, tag 0xC68B) is paired with that NEF in
//! the same directory; pairs whose NEF is absent or not TicoRAW encoded are skipped. Each
//! remaining pair must decode to the DNG's CFA mosaic with zero sample mismatches, and the
//! colour metadata `decode_nef` reports must agree with the levels the DNG declares.
//!
//! ```text
//! TICORAW_CORPUS=/path/to/photos cargo test --release --test ground_truth -- --nocapture
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use ticoraw::RawCodec;
use ticoraw::tiff::{self, Tiff};

const ORIGINAL_RAW_FILE_NAME: u16 = 0xC68B;
/// Upper bound on concurrently decoded pairs; each holds a few hundred MB at 24 MP.
const MAX_WORKERS: usize = 4;

/// What happened to one DNG in the corpus.
enum Outcome {
    /// Not a ground-truth pair for this decoder; the reason is printed.
    Skipped(String),
    Compared(Comparison),
}

struct Comparison {
    nef: String,
    width: usize,
    height: usize,
    mismatches: usize,
    /// (x, y, decoded, truth) of the first differing sample in raster order.
    first_mismatch: Option<(usize, usize, u16, u16)>,
    /// Disagreements between `DecodedRaw::color` and the DNG's declared levels.
    metadata: Vec<String>,
    nef_ms: u128,
    dng_ms: u128,
}

impl Comparison {
    fn passed(&self) -> bool {
        self.mismatches == 0 && self.metadata.is_empty()
    }
}

#[test]
fn nef_decodes_bit_exact_to_adobe_dng() {
    let Some(dir) = std::env::var_os("TICORAW_CORPUS").map(PathBuf::from) else {
        println!("ground_truth: TICORAW_CORPUS is not set; skipping");
        return;
    };
    if !dir.is_dir() {
        println!(
            "ground_truth: TICORAW_CORPUS={} is not a directory; skipping",
            dir.display()
        );
        return;
    }

    let (dngs, nefs) = scan(&dir);
    let results = run_bounded(&dngs, |dng| check(dng, &nefs));

    let mut compared = 0;
    let mut failures = Vec::new();
    let w = dngs
        .iter()
        .zip(&results)
        .filter(|(_, r)| matches!(r, Ok(Outcome::Compared(_))))
        .map(|(d, _)| file_name(d).len())
        .max()
        .unwrap_or(0);
    println!(
        "\n{:<w$} {:<16} {:>11} {:>10} {:>7} {:>7}",
        "dng", "nef", "samples", "mismatches", "nef ms", "dng ms"
    );
    for (dng, result) in dngs.iter().zip(&results) {
        let name = file_name(dng);
        match result {
            Ok(Outcome::Skipped(why)) => println!("{name:<w$} skipped: {why}"),
            Ok(Outcome::Compared(c)) => {
                compared += 1;
                println!(
                    "{name:<w$} {:<16} {:>11} {:>10} {:>7} {:>7}",
                    c.nef,
                    c.width * c.height,
                    c.mismatches,
                    c.nef_ms,
                    c.dng_ms
                );
                if !c.passed() {
                    failures.push(describe_failure(&name, c));
                }
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    println!("{compared} pair(s) compared, {} failed\n", failures.len());

    assert!(
        failures.is_empty(),
        "ground truth failures:\n{}",
        failures.join("\n")
    );
    assert!(
        compared > 0,
        "TICORAW_CORPUS={} holds no DNG paired with a TicoRAW NEF",
        dir.display()
    );
}

/// Every `.dng` in `dir` (sorted) and a lowercase-name index of every `.nef`.
fn scan(dir: &Path) -> (Vec<PathBuf>, HashMap<String, PathBuf>) {
    let mut dngs = Vec::new();
    let mut nefs = HashMap::new();
    let entries = std::fs::read_dir(dir).expect("read TICORAW_CORPUS");
    for entry in entries {
        let path = entry.expect("read corpus entry").path();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        match ext.as_deref() {
            Some("dng") => dngs.push(path),
            Some("nef") => {
                nefs.insert(file_name(&path).to_lowercase(), path);
            }
            _ => {}
        }
    }
    dngs.sort();
    (dngs, nefs)
}

/// Applies `f` to every item on at most `MAX_WORKERS` scoped threads, preserving order.
fn run_bounded<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = std::thread::available_parallelism()
        .map_or(1, std::num::NonZero::get)
        .min(MAX_WORKERS)
        .min(items.len().max(1));
    let next = AtomicUsize::new(0);
    let done = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    let r = f(item);
                    done.lock().expect("result lock").push((i, r));
                }
            });
        }
    });
    let mut done = done.into_inner().expect("result lock");
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// Pairs one DNG with its source NEF and compares them. `Err` is a test failure.
fn check(dng_path: &Path, nefs: &HashMap<String, PathBuf>) -> Result<Outcome, String> {
    let dng = std::fs::read(dng_path).map_err(|e| format!("read: {e}"))?;
    let Some(nef_name) = original_raw_file_name(&dng)? else {
        return Ok(Outcome::Skipped("no OriginalRawFileName".into()));
    };
    let Some(nef_path) = nefs.get(&nef_name.to_lowercase()) else {
        return Ok(Outcome::Skipped(format!("source {nef_name} not in corpus")));
    };
    let nef = std::fs::read(nef_path).map_err(|e| format!("read {nef_name}: {e}"))?;
    let container = match ticoraw::container::parse_nef(&nef) {
        Ok(c) => c,
        Err(e) => return Ok(Outcome::Skipped(format!("{nef_name} is not a NEF ({e})"))),
    };
    if container.codec != RawCodec::TicoRaw {
        return Ok(Outcome::Skipped(format!("{nef_name} is not TicoRAW")));
    }
    if !has_cfa_ifd(&dng)? {
        return Ok(Outcome::Skipped("DNG holds no CFA mosaic".into()));
    }

    let t = Instant::now();
    let raw = ticoraw::decode_nef(&nef).map_err(|e| format!("decode {nef_name}: {e}"))?;
    let nef_ms = t.elapsed().as_millis();
    let t = Instant::now();
    let truth = ticoraw::dng::decode_dng_cfa(&dng).map_err(|e| format!("decode DNG: {e}"))?;
    let dng_ms = t.elapsed().as_millis();

    let (ours, theirs) = (&raw.image, &truth.image);
    if (ours.width, ours.height) != (theirs.width, theirs.height) {
        return Err(format!(
            "{nef_name} decodes to {}x{} but the DNG mosaic is {}x{}",
            ours.width, ours.height, theirs.width, theirs.height
        ));
    }
    let width = ours.width;
    let mut mismatches = 0;
    let mut first_mismatch = None;
    for (i, (&a, &b)) in ours.pixels.iter().zip(&theirs.pixels).enumerate() {
        if a != b {
            mismatches += 1;
            first_mismatch.get_or_insert((i % width, i / width, a, b));
        }
    }

    let mut metadata = Vec::new();
    let color = &raw.color;
    if color.cfa_pattern != truth.cfa_pattern {
        metadata.push(format!(
            "CFA pattern {:?} != DNG {:?}",
            color.cfa_pattern, truth.cfa_pattern
        ));
    }
    if color.black_level != truth.black_level {
        metadata.push(format!(
            "black level {:?} != DNG {:?}",
            color.black_level, truth.black_level
        ));
    }
    if color.white_level != truth.white_level {
        metadata.push(format!(
            "white level {} != DNG {}",
            color.white_level, truth.white_level
        ));
    }

    // Rendering metadata the DNG writer emits must agree with Adobe's.
    let adobe = rendering_metadata(&dng)?;
    let ours_ev = raw
        .baseline_exposure
        .map(|(n, d)| (i64::from(n), i64::from(d)));
    let same_ratio = |a: Option<(i64, i64)>, b: Option<(i64, i64)>| match (a, b) {
        (Some((an, ad)), Some((bn, bd))) => an * bd == bn * ad,
        (None, None) => true,
        // An absent BaselineExposure means 0 EV.
        (Some((n, _)), None) | (None, Some((n, _))) => n == 0,
    };
    if !same_ratio(ours_ev, adobe.baseline_exposure) {
        metadata.push(format!(
            "baseline exposure {ours_ev:?} != DNG {:?}",
            adobe.baseline_exposure
        ));
    }
    if adobe
        .orientation
        .is_some_and(|o| o != u64::from(raw.orientation))
    {
        metadata.push(format!(
            "orientation {} != DNG {:?}",
            raw.orientation, adobe.orientation
        ));
    }
    let our_crop = raw.crop.map(|c| [c.x, c.y, c.width, c.height]);
    if adobe.crop.is_some() && our_crop != adobe.crop {
        metadata.push(format!("crop {our_crop:?} != DNG {:?}", adobe.crop));
    }

    Ok(Outcome::Compared(Comparison {
        nef: nef_name,
        width,
        height: ours.height,
        mismatches,
        first_mismatch,
        metadata,
        nef_ms,
        dng_ms,
    }))
}

fn describe_failure(dng: &str, c: &Comparison) -> String {
    let mut problems = Vec::new();
    if c.mismatches > 0 {
        let first = c
            .first_mismatch
            .map(|(x, y, a, b)| format!(", first at (x={x}, y={y}): decoded {a}, DNG {b}"))
            .unwrap_or_default();
        problems.push(format!(
            "{} of {} samples differ{first}",
            c.mismatches,
            c.width * c.height
        ));
    }
    problems.extend(c.metadata.iter().cloned());
    format!("{dng} vs {}: {}", c.nef, problems.join("; "))
}

/// IFD0's OriginalRawFileName, if the DNG records one.
fn original_raw_file_name(dng: &[u8]) -> Result<Option<String>, String> {
    let t = Tiff::parse(dng, 0).map_err(|e| format!("DNG: {e}"))?;
    let ifd0 = t.ifd(t.first_ifd).map_err(|e| format!("DNG IFD0: {e}"))?;
    Ok(ifd0
        .get(ORIGINAL_RAW_FILE_NAME)
        .map(|e| t.ascii(e))
        .filter(|n| !n.is_empty()))
}

/// Whether the DNG has a full-resolution CFA IFD (linear / demosaiced DNGs do not).
/// Adobe's rendering metadata for the frame.
struct RenderingMetadata {
    /// IFD0 `BaselineExposure` (0xC62A).
    baseline_exposure: Option<(i64, i64)>,
    /// IFD0 `Orientation` (0x112).
    orientation: Option<u64>,
    /// Raw IFD `DefaultCropOrigin` / `DefaultCropSize` as integers (x, y, width, height).
    crop: Option<[usize; 4]>,
}

fn rendering_metadata(dng: &[u8]) -> Result<RenderingMetadata, String> {
    let t = Tiff::parse(dng, 0).map_err(|e| format!("DNG: {e}"))?;
    let ifd0 = t.ifd(t.first_ifd).map_err(|e| format!("DNG IFD0: {e}"))?;
    let baseline_exposure = match ifd0.get(0xC62A) {
        Some(e) => t
            .rationals(e)
            .map_err(|e| format!("BaselineExposure: {e}"))?
            .first()
            .copied(),
        None => None,
    };
    let orientation =
        tiff::first_uint(&t, &ifd0, 0x112).map_err(|e| format!("Orientation: {e}"))?;
    let ifds = tiff::all_ifds(&t).map_err(|e| format!("DNG IFDs: {e}"))?;
    let tag = |ifd, tag| tiff::first_uint(&t, ifd, tag).ok().flatten();
    let raw = ifds
        .iter()
        .find(|ifd| tag(ifd, 0xFE) == Some(0) && tag(ifd, 0x106) == Some(32803));
    let pair = |ifd: &tiff::Ifd, tag: u16| -> Option<[usize; 2]> {
        let e = ifd.get(tag)?;
        let v: Vec<usize> = match e.typ {
            5 => t
                .rationals(e)
                .ok()?
                .iter()
                .map(|&(n, d)| (n / d.max(1)) as usize)
                .collect(),
            _ => t.uints(e).ok()?.iter().map(|&v| v as usize).collect(),
        };
        Some([*v.first()?, *v.get(1)?])
    };
    let crop = raw.and_then(|ifd| {
        let [x, y] = pair(ifd, 0xC61F)?;
        let [w, h] = pair(ifd, 0xC620)?;
        Some([x, y, w, h])
    });
    Ok(RenderingMetadata {
        baseline_exposure,
        orientation,
        crop,
    })
}

fn has_cfa_ifd(dng: &[u8]) -> Result<bool, String> {
    let t = Tiff::parse(dng, 0).map_err(|e| format!("DNG: {e}"))?;
    let ifds = tiff::all_ifds(&t).map_err(|e| format!("DNG IFDs: {e}"))?;
    let tag = |ifd, tag| tiff::first_uint(&t, ifd, tag).ok().flatten();
    Ok(ifds
        .iter()
        .any(|ifd| tag(ifd, 0xFE) == Some(0) && tag(ifd, 0x106) == Some(32803)))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
