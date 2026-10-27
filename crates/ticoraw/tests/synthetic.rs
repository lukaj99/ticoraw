//! Corpus-independent tests: synthetic TicoRAW codestreams, NEF and DNG containers built by
//! test-only encoders, forward transforms that must invert the crate's inverses, and
//! no-panic robustness over deterministic mutations and garbage.

mod common;

use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, Once, PoisonError};

use common::encoder::{self, Encoded, FQ, LEVELS, NIKON_EXT, Options};
use common::tiff::{self, DngLayout, NefSpec, Order, TiffWriter, Value};
use common::{Rng, ljpeg, transforms};
use ticoraw::header::TransferCurve;
use ticoraw::{
    CfaImage, NikonCompression, RawCodec, TicoError, dng, entropy, header, mct, wavelet,
};

/// (CFA width, CFA height, precincts per slice).
const FRAMES: &[(usize, usize, usize)] = &[
    (64, 48, 2),   // 32x24 components, 12 precincts in 6 full slices
    (100, 60, 4),  // band widths 13/12/6/3/2 (not multiples of 4); last slice 3 of 4
    (90, 44, 3),   // odd component width 45; last slice 2 of 3
    (200, 120, 7), // 30 precincts; last slice 2 of 7
    (134, 36, 5),  // component width 67; last slice 4 of 5
    (16, 4, 1),    // smallest legal frame: 8x2 components, zero-width bands
    (18, 8, 16),   // one slice taller than the frame
];

fn variants(w: usize, h: usize, hsl: usize) -> Vec<(&'static str, Options)> {
    let base = Options::new(w, h, hsl);
    let with = |f: &dyn Fn(&mut Options)| {
        let mut o = base.clone();
        f(&mut o);
        o
    };
    vec![
        ("mixed", base.clone()),
        ("raw", with(&|o| o.raw_percent = 100)),
        (
            "zero-prediction",
            with(&|o| (o.modes, o.raw_percent) = (vec![0], 0)),
        ),
        (
            "significance",
            with(&|o| (o.modes, o.raw_percent) = (vec![2], 0)),
        ),
        (
            "vertical",
            with(&|o| (o.modes, o.raw_percent) = (vec![1], 0)),
        ),
        (
            "vertical+significance",
            with(&|o| (o.modes, o.raw_percent) = (vec![3], 0)),
        ),
        (
            "run-mode",
            with(&|o| (o.modes, o.run_mode, o.raw_percent) = (vec![2, 3], true, 10)),
        ),
        ("deadzone", with(&|o| o.deadzone = true)),
        ("padded", with(&|o| o.padding = true)),
        ("coarse", with(&|o| o.q_max = 24)),
        ("lossless", with(&|o| o.q_max = 0)),
    ]
}

/// The decoder's expected output, built from the crate's public stage functions.
fn expected_image(e: &Encoded) -> CfaImage {
    let (w, h) = (e.comp_width, e.comp_height);
    let mut planes: [Vec<i32>; 4] = e
        .coefficients
        .clone()
        .map(|p| p.into_iter().map(|v| v << FQ).collect());
    for (plane, &levels) in planes.iter_mut().zip(&LEVELS) {
        if levels == 0 {
            continue;
        }
        let mut scratch = Vec::new();
        for (y, row) in plane.chunks_mut(w).enumerate() {
            wavelet::synthesize_row(row, if y % 2 == 0 { levels } else { 1 }, &mut scratch);
        }
        let mut out = vec![0; w * h];
        wavelet::synthesize_columns(plane, w, &mut out);
        *plane = out;
    }
    mct::inverse_star_tetrix(&mut planes, w, h);
    let curve = TransferCurve::from_extension(&NIKON_EXT).unwrap();
    CfaImage {
        width: 2 * w,
        height: 2 * h,
        pixels: mct::assemble_rggb(&planes, w, h, &curve),
    }
}

fn first_mismatch(a: &CfaImage, b: &CfaImage) -> Option<(usize, usize, u16, u16)> {
    a.pixels
        .iter()
        .zip(&b.pixels)
        .position(|(x, y)| x != y)
        .map(|i| (i % a.width, i / a.width, a.pixels[i], b.pixels[i]))
}

#[test]
fn band_layout_matches_decoder_layout() {
    for &(w, h, hsl) in FRAMES {
        let e = encoder::encode_random(&Options::new(w, h, hsl), 7);
        let hdr = header::parse_main_header(&e.codestream).unwrap();
        let layout = entropy::Layout::new(&hdr).unwrap();
        let (_, packets) = encoder::band_layout(w / 2);
        assert_eq!(layout.bands.len(), 25);
        assert_eq!(layout.packets, packets, "{w}x{h}");
        for (i, (got, want)) in layout.bands.iter().zip(&e.bands).enumerate() {
            let got_spec = (got.component, got.width, got.lines, got.row, got.x);
            let want_spec = (want.component, want.width, want.lines, want.row, want.x);
            assert_eq!(got_spec, want_spec, "{w}x{h} band {i}");
            assert_eq!(
                (got.gain, got.priority),
                e.weights[i],
                "{w}x{h} band {i} WGT"
            );
        }
        assert_eq!((layout.precincts, layout.precincts_per_slice), (h / 4, hsl));
        assert_eq!(layout.slices(), (h / 4).div_ceil(hsl));
    }
}

#[test]
fn decodes_synthetic_codestreams_in_every_coding_mode() {
    let mut checked = 0;
    for &(w, h, hsl) in FRAMES {
        for (name, opts) in variants(w, h, hsl) {
            for seed in 0..3 {
                let e = encoder::encode_random(&opts, seed * 1000 + w as u64);
                let got = ticoraw::decode_ticoraw(&e.codestream).unwrap_or_else(|err| {
                    panic!("{w}x{h} hsl {hsl} {name} seed {seed}: decode failed: {err}")
                });
                let want = expected_image(&e);
                assert_eq!((got.width, got.height), (w, h));
                if let Some((x, y, g, x_want)) = first_mismatch(&got, &want) {
                    panic!(
                        "{w}x{h} hsl {hsl} {name} seed {seed}: pixel ({x},{y}) = {g}, want {x_want}"
                    );
                }
                checked += 1;
            }
        }
    }
    assert_eq!(checked, FRAMES.len() * 11 * 3);
}

#[test]
fn corrupted_entropy_data_is_detected() {
    // Guards the comparison above against being vacuous: flipping a bit of entropy-coded
    // data must change the decoded image or fail the decode.
    let e = encoder::encode_random(&Options::new(64, 48, 2), 13);
    let want = expected_image(&e);
    let first_slice = header::parse_main_header(&e.codestream)
        .unwrap()
        .first_slice;
    let mut rng = Rng::new(14);
    let (mut changed, trials) = (0, 200);
    for _ in 0..trials {
        let mut cs = e.codestream.clone();
        let at = first_slice + 6 + rng.below((cs.len() - first_slice - 8) as u64) as usize;
        cs[at] ^= 1 << rng.below(8);
        if ticoraw::decode_ticoraw(&cs).map_or(true, |got| got != want) {
            changed += 1;
        }
    }
    assert!(
        changed * 10 >= trials * 9,
        "{changed} of {trials} flips detected"
    );
}

#[test]
fn synthetic_coefficients_are_not_trivial() {
    // Guards the encoder itself: quantised, signed, non-zero data in every component.
    let e = encoder::encode_random(&Options::new(100, 60, 4), 1);
    for (c, plane) in e.coefficients.iter().enumerate() {
        assert!(plane.iter().any(|&v| v > 0), "component {c}");
        assert!(plane.iter().any(|&v| v < 0), "component {c}");
        assert!(plane.iter().any(|&v| v != 0 && v % 2 != 0), "component {c}");
    }
    let coarse = Options {
        q_max: 24,
        ..Options::new(100, 60, 4)
    };
    let e = encoder::encode_random(&coarse, 2);
    // Uniform dequantisation of a single plane above GTLI g leaves non-power-of-two steps.
    assert!(
        e.coefficients
            .iter()
            .flatten()
            .any(|&v| v != 0 && (v.unsigned_abs() & (v.unsigned_abs() - 1)) != 0)
    );
}

#[test]
fn dequantisation_reference_values() {
    // s = gcli - gtli + 1; r = 0; while v > 0 { r += v; v >>= s }
    assert_eq!(encoder::dequantise(1, 4, 3, false), 10);
    assert_eq!(encoder::dequantise(5, 6, 3, false), 40 + (40 >> 4));
    assert_eq!(encoder::dequantise(5, 6, 0, false), 5);
    assert_eq!(encoder::dequantise(0, 6, 3, false), 0);
    assert_eq!(encoder::dequantise(1, 4, 3, true), 12);
}

#[test]
fn forward_5_3_inverts_exactly() {
    let mut rng = Rng::new(53);
    for n in 1..80 {
        for _ in 0..4 {
            let x: Vec<i32> = (0..n).map(|_| rng.range(-40_000, 40_000) as i32).collect();
            let (l, h) = transforms::analyze_1d(&x);
            assert_eq!((l.len(), h.len()), (n - n / 2, n / 2));
            let mut out = vec![0; n];
            wavelet::synthesize_1d(&l, &h, &mut out);
            assert_eq!(out, x, "n = {n}");
        }
    }
    for levels in 1..=8u8 {
        for n in [1, 2, 3, 8, 9, 45, 67, 100, 257] {
            let x: Vec<i32> = (0..n).map(|_| rng.range(-9999, 9999) as i32).collect();
            let mut row = x.clone();
            transforms::analyze_row(&mut row, levels);
            let mut scratch = Vec::new();
            wavelet::synthesize_row(&mut row, levels, &mut scratch);
            assert_eq!(row, x, "n = {n}, levels = {levels}");
        }
    }
    for (w, h) in [(1, 2), (5, 2), (7, 6), (16, 10), (33, 24)] {
        let img: Vec<i32> = (0..w * h).map(|_| rng.range(-5000, 5000) as i32).collect();
        let coef = transforms::analyze_columns(&img, w);
        let mut out = vec![0; w * h];
        wavelet::synthesize_columns(&coef, w, &mut out);
        assert_eq!(out, img, "{w}x{h}");
    }
}

#[test]
fn forward_star_tetrix_inverts_exactly() {
    let mut rng = Rng::new(4);
    for (w, h) in [
        (1, 1),
        (1, 2),
        (2, 1),
        (5, 4),
        (8, 2),
        (13, 7),
        (32, 24),
        (45, 22),
    ] {
        let original: [Vec<i32>; 4] = std::array::from_fn(|_| {
            (0..w * h)
                .map(|_| rng.range(-20_000, 20_000) as i32)
                .collect()
        });
        let mut planes = original.clone();
        transforms::forward_star_tetrix(&mut planes, w, h);
        assert_ne!(planes, original);
        mct::inverse_star_tetrix(&mut planes, w, h);
        assert_eq!(planes, original, "{w}x{h}");
    }
}

#[test]
fn forward_pipeline_inverts_through_crate_stages() {
    // Image planes -> forward Star-Tetrix -> forward 5/3 per component, then the crate's
    // inverse stages in decode order must restore the planes exactly.
    let mut rng = Rng::new(9);
    for &(w, h) in &[(8, 2), (32, 24), (45, 22), (67, 18)] {
        let original: [Vec<i32>; 4] = std::array::from_fn(|_| {
            (0..w * h)
                .map(|_| rng.range(-16_000, 16_000) as i32)
                .collect()
        });
        let mut planes = original.clone();
        transforms::forward_star_tetrix(&mut planes, w, h);
        for (p, &levels) in planes.iter_mut().zip(&LEVELS) {
            *p = transforms::analyze_plane(p, w, levels);
        }
        for (p, &levels) in planes.iter_mut().zip(&LEVELS) {
            if levels == 0 {
                continue;
            }
            let mut scratch = Vec::new();
            for (y, row) in p.chunks_mut(w).enumerate() {
                wavelet::synthesize_row(row, if y % 2 == 0 { levels } else { 1 }, &mut scratch);
            }
            let mut out = vec![0; w * h];
            wavelet::synthesize_columns(p, w, &mut out);
            *p = out;
        }
        mct::inverse_star_tetrix(&mut planes, w, h);
        assert_eq!(planes, original, "{w}x{h}");
    }
}

fn sample_codestream() -> Encoded {
    encoder::encode_random(&Options::new(100, 60, 4), 42)
}

#[test]
fn nef_wrapper_inspects_and_decodes() {
    let e = sample_codestream();
    let spec = NefSpec::new(100, 60);
    let nef = tiff::build_nef(&spec, &e.codestream);

    let info = ticoraw::inspect_nef(&nef).unwrap();
    assert_eq!(info.make, "NIKON CORPORATION");
    assert_eq!(info.model, "NIKON Z6_3");
    assert_eq!(
        (info.width, info.height, info.bits_per_sample),
        (100, 60, 14)
    );
    assert_eq!(info.codec, RawCodec::TicoRaw);
    assert_eq!(info.compression, Some(NikonCompression::HighEfficiencyStar));
    assert_eq!(info.strip_len, e.codestream.len());
    assert_eq!(
        &nef[info.strip_offset..info.strip_offset + info.strip_len],
        &e.codestream[..]
    );
    assert_eq!(info.black_level, Some([1008, 1009, 1010, 1011]));
    let cs = info.codestream.unwrap();
    assert_eq!((cs.component_width, cs.component_height), (50, 30));
    assert_eq!((cs.slices, cs.precincts, cs.bands), (4, 15, 25));
    assert_eq!(cs.unsupported, None);
    assert_eq!(
        (cs.curve.black, cs.curve.gain, cs.curve.knee),
        (1008, 84429, 16255)
    );
    assert_eq!(
        (cs.header.pih.wf, cs.header.pih.hf, cs.header.pih.hsl),
        (100, 60, 4)
    );
    assert_eq!(cs.header.pih.lcod as usize, e.codestream.len());

    let raw = ticoraw::decode_nef(&nef).unwrap();
    assert_eq!(raw.make, "NIKON CORPORATION");
    assert_eq!(raw.model, "NIKON Z6_3");
    assert_eq!(raw.image, expected_image(&e));
    assert_eq!(raw.color.cfa_pattern, [0, 1, 1, 2]);
    assert_eq!(raw.color.black_level, [1008, 1009, 1010, 1011]);
    assert_eq!(raw.color.white_level, 15892);
    assert!(raw.color.color_matrix.is_some());
    assert_eq!(
        raw.color.as_shot_neutral,
        Some([(512, 924), (1, 1), (512, 717)])
    );
}

#[test]
fn nef_wrapper_variants() {
    let e = sample_codestream();
    let mut spec = NefSpec::new(100, 60);
    spec.order = Order::Little;
    spec.model = "NIKON Z 99".into(); // no built-in colour profile
    spec.compression_code = Some(13);
    spec.black_level = None;
    spec.wb = None;
    spec.cfa_pattern = None;
    let nef = tiff::build_nef(&spec, &e.codestream);
    let info = ticoraw::inspect_nef(&nef).unwrap();
    assert_eq!(info.compression, Some(NikonCompression::HighEfficiency));
    assert_eq!(info.black_level, None);
    let raw = ticoraw::decode_nef(&nef).unwrap();
    assert_eq!(raw.image, expected_image(&e));
    // Without MakerNote black levels the curve's black offset is used.
    assert_eq!(raw.color.black_level, [1008; 4]);
    assert_eq!(raw.color.white_level, 16383);
    assert_eq!(raw.color.color_matrix, None);
    assert_eq!(raw.color.as_shot_neutral, None);
    assert_eq!(raw.color.cfa_pattern, [0, 1, 1, 2]);

    for (code, want) in [
        (3, NikonCompression::Lossless),
        (99, NikonCompression::Other(99)),
    ] {
        spec.compression_code = Some(code);
        let info = ticoraw::inspect_nef(&tiff::build_nef(&spec, &e.codestream)).unwrap();
        assert_eq!(info.compression, Some(want));
    }
    assert_eq!(
        NikonCompression::HighEfficiencyStar.to_string(),
        "High Efficiency* (HE*)"
    );
    assert_eq!(
        NikonCompression::Other(99).to_string(),
        "Nikon compression code 99"
    );

    spec.maker_note = false;
    let nef = tiff::build_nef(&spec, &e.codestream);
    assert_eq!(ticoraw::inspect_nef(&nef).unwrap().compression, None);
    assert_eq!(ticoraw::decode_nef(&nef).unwrap().image, expected_image(&e));
}

#[test]
fn nef_container_errors() {
    let e = sample_codestream();

    let mut classic = NefSpec::new(100, 60);
    classic.compression_code = Some(3);
    let nef = tiff::build_nef(&classic, &[0x55; 64]);
    let info = ticoraw::inspect_nef(&nef).unwrap();
    assert_eq!(info.codec, RawCodec::Classic);
    assert!(info.codestream.is_none());
    assert!(matches!(
        ticoraw::decode_nef(&nef),
        Err(TicoError::Unsupported(_))
    ));

    let nef = tiff::build_nef(&NefSpec::new(102, 60), &e.codestream);
    assert!(matches!(
        ticoraw::decode_nef(&nef),
        Err(TicoError::Container(_))
    ));

    let mut canon = NefSpec::new(100, 60);
    canon.make = "Canon".into();
    let nef = tiff::build_nef(&canon, &e.codestream);
    assert!(matches!(
        ticoraw::inspect_nef(&nef),
        Err(TicoError::Container(_))
    ));

    // Ng = 8 parses but is outside the supported profile.
    let mut cs = e.codestream.clone();
    assert_eq!(cs[59], 4);
    cs[59] = 8;
    let info = ticoraw::inspect_nef(&tiff::build_nef(&NefSpec::new(100, 60), &cs)).unwrap();
    assert!(info.codestream.unwrap().unsupported.is_some());
    assert!(matches!(
        ticoraw::decode_ticoraw(&cs),
        Err(TicoError::Unsupported(_))
    ));
}

#[test]
fn decoded_nef_round_trips_through_uncompressed_dng() {
    let e = sample_codestream();
    let raw = ticoraw::decode_nef(&tiff::build_nef(&NefSpec::new(100, 60), &e.codestream)).unwrap();
    let back = dng::decode_dng_cfa(&dng::encode_uncompressed_dng(&raw).unwrap()).unwrap();
    assert_eq!(back.image, raw.image);
    assert_eq!(back.cfa_pattern, raw.color.cfa_pattern);
    assert_eq!(back.black_level, raw.color.black_level);
    assert_eq!(back.white_level, raw.color.white_level);
}

#[test]
fn lossless_jpeg_round_trips() {
    let mut rng = Rng::new(3);
    for predictor in 1..=7u8 {
        for (precision, nf, rows, cols) in
            [(16, 1, 5, 7), (14, 2, 9, 4), (12, 2, 1, 13), (16, 2, 6, 6)]
        {
            let max = (1u64 << precision) - 1;
            let samples: Vec<u16> = (0..rows * cols * nf)
                .map(|i| match i % 5 {
                    0 => 0,
                    1 => max as u16,
                    _ => rng.below(max + 1) as u16,
                })
                .collect();
            let j = ljpeg::encode(&samples, rows, cols, nf, precision, predictor);
            let got = dng::decode_lossless_jpeg(&j).unwrap();
            assert_eq!((got.width, got.height), (cols * nf, rows));
            assert_eq!(
                got.samples, samples,
                "predictor {predictor} precision {precision} nf {nf}"
            );
        }
    }
}

#[test]
fn lossless_jpeg_dng_tiles_and_strips() {
    let mut rng = Rng::new(11);
    let (w, h) = (70, 46);
    let pixels: Vec<u16> = (0..w * h).map(|_| rng.below(16384) as u16).collect();
    for layout in [
        DngLayout::Tiles(32, 16),
        DngLayout::Tiles(70, 46),
        DngLayout::Strips(10),
    ] {
        let d = tiff::build_ljpeg_dng(&pixels, w, h, layout, [1008, 1009, 1010, 1011], 15892);
        let cfa = dng::decode_dng_cfa(&d).unwrap();
        assert_eq!((cfa.image.width, cfa.image.height), (w, h));
        assert_eq!(cfa.image.pixels, pixels, "{layout:?}");
        assert_eq!(cfa.cfa_pattern, [0, 1, 1, 2]);
        assert_eq!(cfa.black_level, [1008, 1009, 1010, 1011]);
        assert_eq!(cfa.white_level, 15892);
    }
}

// ---------------------------------------------------------------------------------------
// Robustness: the library must return Ok or Err, never panic.

/// Message and location of the most recent panic on any thread (rayon workers included).
static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);
/// Serialises the probing tests so `LAST_PANIC` belongs to the probe that reads it.
static PROBE_LOCK: Mutex<()> = Mutex::new(());
/// Set while a probe runs: the hook then records instead of printing.
static PROBING: AtomicBool = AtomicBool::new(false);

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !PROBING.load(Ordering::SeqCst) {
                return default(info);
            }
            let msg = info
                .payload()
                .downcast_ref::<&str>()
                .map(ToString::to_string)
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_default();
            let at = info
                .location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_default();
            *LAST_PANIC.lock().unwrap_or_else(PoisonError::into_inner) =
                Some(format!("{msg} at {at}"));
        }));
    });
}

/// Collects panicking inputs; holds `PROBE_LOCK` for its lifetime.
struct Failures {
    found: Vec<String>,
    _guard: MutexGuard<'static, ()>,
}

impl Failures {
    fn new() -> Self {
        install_panic_hook();
        Self {
            found: Vec::new(),
            _guard: PROBE_LOCK.lock().unwrap_or_else(PoisonError::into_inner),
        }
    }

    /// Runs `f(input)`, recording the panic message and location if it panicked.
    fn check(&mut self, f: &dyn Fn(&[u8]), input: &[u8], what: impl FnOnce() -> String) {
        PROBING.store(true, Ordering::SeqCst);
        let r = panic::catch_unwind(AssertUnwindSafe(|| f(input)));
        PROBING.store(false, Ordering::SeqCst);
        if r.is_err() {
            let p = LAST_PANIC
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .unwrap_or_else(|| "unknown panic".into());
            self.found.push(format!("{}: {p}", what()));
        }
    }

    fn assert_none(self, label: &str) {
        assert!(
            self.found.is_empty(),
            "{label}: {} panicking inputs, first ones:\n{}",
            self.found.len(),
            self.found
                .iter()
                .take(12)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

fn decode_cs(b: &[u8]) {
    let _ = ticoraw::decode_ticoraw(b);
    let _ = header::parse_main_header(b);
}

fn decode_nef_all(b: &[u8]) {
    let _ = ticoraw::inspect_nef(b);
    let _ = ticoraw::decode_nef(b);
}

fn decode_dng(b: &[u8]) {
    let _ = dng::decode_dng_cfa(b);
}

fn decode_ljpeg(b: &[u8]) {
    let _ = dng::decode_lossless_jpeg(b);
}

/// Single-byte mutations (bit flip, boundary value or random change) at positions drawn
/// uniformly from `regions`.
fn mutate(
    f: &dyn Fn(&[u8]),
    seed_input: &[u8],
    regions: &[Range<usize>],
    count: usize,
    seed: u64,
    failures: &mut Failures,
) {
    let mut rng = Rng::new(seed);
    let mut buf = seed_input.to_vec();
    for i in 0..count {
        let region = &regions[i % regions.len()];
        let at = region.start + rng.below(region.len() as u64) as usize;
        let old = buf[at];
        let new = match rng.below(4) {
            0 => old ^ (1 << rng.below(8)),
            1 => [0x00, 0xFF, 0x7F, 0x80, 0x01][rng.below(5) as usize],
            _ => old ^ rng.range(1, 255) as u8,
        };
        if new == old {
            continue;
        }
        buf[at] = new;
        failures.check(f, &buf, || format!("byte {at}: {old:#04x} -> {new:#04x}"));
        buf[at] = old;
    }
}

/// Every prefix up to 256 bytes, then every `step`-th prefix.
fn truncate(f: &dyn Fn(&[u8]), seed_input: &[u8], step: usize, failures: &mut Failures) {
    let n = seed_input.len();
    for len in (0..256.min(n)).chain((256..n).step_by(step)) {
        failures.check(f, &seed_input[..len], || {
            format!("truncated to {len} bytes")
        });
    }
}

fn garbage(
    f: &dyn Fn(&[u8]),
    prefixes: &[&[u8]],
    count: usize,
    seed: u64,
    failures: &mut Failures,
) {
    let mut rng = Rng::new(seed);
    for i in 0..count {
        let prefix = prefixes[i % prefixes.len()];
        let mut buf = prefix.to_vec();
        let n = rng.below(600) as usize;
        buf.extend(rng.bytes(n));
        failures.check(f, &buf, || {
            format!(
                "garbage #{i} ({} bytes, prefix {} bytes)",
                buf.len(),
                prefix.len()
            )
        });
    }
}

/// Two codestreams (plain, and padded with odd slicing) with their header length.
fn fuzz_codestreams() -> Vec<(Vec<u8>, usize)> {
    [
        Options::new(64, 48, 2),
        Options {
            padding: true,
            ..Options::new(90, 44, 3)
        },
    ]
    .iter()
    .enumerate()
    .map(|(k, opts)| {
        let cs = encoder::encode_random(opts, 77 + k as u64).codestream;
        let first_slice = header::parse_main_header(&cs).unwrap().first_slice;
        (cs, first_slice)
    })
    .collect()
}

#[test]
fn codestream_header_mutations_never_panic() {
    let mut failures = Failures::new();
    for (seed, (cs, first_slice)) in fuzz_codestreams().into_iter().enumerate() {
        mutate(
            &decode_cs,
            &cs,
            std::slice::from_ref(&(0..first_slice + 6)),
            3000,
            1 + seed as u64,
            &mut failures,
        );
    }
    failures.assert_none("codestream header");
}

#[test]
fn codestream_entropy_mutations_never_panic() {
    let mut failures = Failures::new();
    for (seed, (cs, first_slice)) in fuzz_codestreams().into_iter().enumerate() {
        let body = first_slice + 6..cs.len();
        mutate(
            &decode_cs,
            &cs,
            &[body],
            3000,
            3 + seed as u64,
            &mut failures,
        );
    }
    failures.assert_none("codestream entropy data");
}

#[test]
fn codestream_truncations_never_panic() {
    let mut failures = Failures::new();
    for (cs, _) in fuzz_codestreams() {
        truncate(&decode_cs, &cs, 7, &mut failures);
    }
    failures.assert_none("codestream truncation");
}

#[test]
fn nef_container_mutations_never_panic() {
    let e = encoder::encode_random(&Options::new(64, 48, 2), 5);
    let mut failures = Failures::new();
    for order in [Order::Big, Order::Little] {
        let mut spec = NefSpec::new(64, 48);
        spec.order = order;
        let nef = tiff::build_nef(&spec, &e.codestream);
        // The strip starts right after the 8-byte TIFF header; the IFDs follow it.
        let container = [0..8, 8 + e.codestream.len()..nef.len()];
        mutate(&decode_nef_all, &nef, &container, 2000, 10, &mut failures);
        truncate(&decode_nef_all, &nef, 11, &mut failures);
    }
    failures.assert_none("NEF container");
}

#[test]
fn garbage_never_panics() {
    let cs = encoder::encode_random(&Options::new(64, 48, 2), 8).codestream;
    let first_slice = header::parse_main_header(&cs).unwrap().first_slice;
    let mut failures = Failures::new();
    garbage(
        &decode_cs,
        &[&[], &cs[..4], &cs[..first_slice], &cs[..first_slice + 6]],
        2000,
        21,
        &mut failures,
    );
    let tiff_prefixes: &[&[u8]] = &[&[], b"II*\0\x08\0\0\0", b"MM\0*\0\0\0\x08", b"II*\0"];
    garbage(&decode_nef_all, tiff_prefixes, 2000, 22, &mut failures);
    garbage(&decode_dng, tiff_prefixes, 2000, 23, &mut failures);
    garbage(
        &decode_ljpeg,
        &[
            &[],
            &[0xFF, 0xD8],
            &[0xFF, 0xD8, 0xFF, 0xC4],
            &[0xFF, 0xD8, 0xFF, 0xC3],
        ],
        2000,
        24,
        &mut failures,
    );
    failures.assert_none("garbage");
}

fn sample_ljpeg() -> Vec<u8> {
    let mut rng = Rng::new(31);
    let samples: Vec<u16> = (0..8 * 12).map(|_| rng.below(16384) as u16).collect();
    ljpeg::encode(&samples, 4, 12, 2, 14, 1)
}

/// Offset of the first entropy-coded byte (just past the SOS segment).
fn ljpeg_scan_start(j: &[u8]) -> usize {
    let sos = j.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    sos + 2 + u16::from_be_bytes([j[sos + 2], j[sos + 3]]) as usize
}

#[test]
fn lossless_jpeg_header_mutations_never_panic() {
    let j = sample_ljpeg();
    let mut failures = Failures::new();
    mutate(
        &decode_ljpeg,
        &j,
        std::slice::from_ref(&(0..ljpeg_scan_start(&j))),
        3000,
        41,
        &mut failures,
    );
    failures.assert_none("lossless JPEG header");
}

#[test]
fn lossless_jpeg_scan_mutations_never_panic() {
    let j = sample_ljpeg();
    let mut failures = Failures::new();
    mutate(
        &decode_ljpeg,
        &j,
        std::slice::from_ref(&(ljpeg_scan_start(&j)..j.len())),
        3000,
        42,
        &mut failures,
    );
    truncate(&decode_ljpeg, &j, 1, &mut failures);
    failures.assert_none("lossless JPEG scan");
}

fn fuzz_dngs() -> Vec<Vec<u8>> {
    let mut rng = Rng::new(51);
    let (w, h) = (24, 12);
    let pixels: Vec<u16> = (0..w * h).map(|_| rng.below(16384) as u16).collect();
    vec![
        tiff::build_ljpeg_dng(&pixels, w, h, DngLayout::Tiles(16, 8), [0; 4], 16383),
        tiff::build_ljpeg_dng(&pixels, w, h, DngLayout::Strips(5), [0; 4], 16383),
    ]
}

/// The IFD (with its values) is written after the tiles; its offset is in the header.
fn dng_ifd_start(d: &[u8]) -> usize {
    u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize
}

#[test]
fn dng_container_mutations_never_panic() {
    let mut failures = Failures::new();
    for d in fuzz_dngs() {
        let ifd = dng_ifd_start(&d);
        mutate(
            &decode_dng,
            &d,
            &[0..8, ifd..d.len()],
            2000,
            52,
            &mut failures,
        );
    }
    failures.assert_none("DNG container");
}

#[test]
fn dng_tile_mutations_never_panic() {
    let mut failures = Failures::new();
    for d in fuzz_dngs() {
        let ifd = dng_ifd_start(&d);
        mutate(
            &decode_dng,
            &d,
            std::slice::from_ref(&(8..ifd)),
            2000,
            53,
            &mut failures,
        );
        truncate(&decode_dng, &d, 3, &mut failures);
    }
    failures.assert_none("DNG tiles");
}

// ---------------------------------------------------------------------------------------
// Exact reproducers for panics the fuzzing once found in the core; each must now be an
// error, not a panic.

fn assert_no_panic(f: &dyn Fn(&[u8]), inputs: &[(&str, &[u8])]) {
    let mut failures = Failures::new();
    for &(name, input) in inputs {
        failures.check(f, input, || name.to_owned());
    }
    failures.assert_none("reproducer");
}

/// The sample codestream with its CDT payload replaced by `payload`.
fn with_cdt(payload: &[u8]) -> Vec<u8> {
    let cs = sample_codestream().codestream;
    let at = 79; // SOC 2 + CAP 36 + PIH 41
    assert_eq!(&cs[at..at + 4], &[0xFF, 0x13, 0x00, 0x0E]);
    let mut out = cs[..at].to_vec();
    out.extend_from_slice(&[0xFF, 0x13]);
    out.extend_from_slice(&(2 + payload.len() as u16).to_be_bytes());
    out.extend_from_slice(payload);
    out.extend_from_slice(&cs[at + 16..]);
    out
}

#[test]
fn short_cdt_entries_are_errors() {
    assert_no_panic(
        &decode_cs,
        &[
            ("CDT of 1 byte per component", &with_cdt(&[14, 14, 14, 14])),
            ("empty CDT", &with_cdt(&[])),
        ],
    );
}

#[test]
fn short_lossless_jpeg_segments_are_errors() {
    let dri = [0xFF, 0xD8, 0xFF, 0xDD, 0x00, 0x02, 0xFF, 0xD9];
    let j = sample_ljpeg();
    let sos = j.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    let mut empty_sos = j[..sos].to_vec();
    empty_sos.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x02, 0x00, 0x00]);
    assert_no_panic(
        &decode_ljpeg,
        &[("empty DRI", &dri), ("empty SOS", &empty_sos)],
    );
}

#[test]
fn hostile_lossless_jpeg_parameters_are_errors() {
    let j = sample_ljpeg();
    let sof = j.windows(2).position(|w| w == [0xFF, 0xC3]).unwrap();
    let mut zero_precision = j.clone();
    zero_precision[sof + 4] = 0;
    // Every DHT symbol (SSSS) set to 200.
    let dht = j.windows(2).position(|w| w == [0xFF, 0xC4]).unwrap();
    let mut big_ssss = j.clone();
    for s in &mut big_ssss[dht + 21..dht + 21 + 17] {
        *s = 200;
    }
    assert_no_panic(
        &decode_ljpeg,
        &[
            ("SOF3 precision 0", &zero_precision),
            ("SSSS 200", &big_ssss),
        ],
    );
}

/// A compression-7 CFA DNG (24x4) with explicit tile or strip geometry over one stream.
fn dng_with_geometry(tile_width: u32, rows_per_strip: Option<u32>) -> Vec<u8> {
    let j = sample_ljpeg();
    let mut t = TiffWriter::new(Order::Little);
    let at = t.blob(&j);
    let mut entries = vec![
        (0x00FE, Value::Long(vec![0])),
        (0x0100, Value::Long(vec![24])),
        (0x0101, Value::Long(vec![4])),
        (0x0102, Value::Short(vec![16])),
        (0x0103, Value::Short(vec![7])),
        (0x0106, Value::Short(vec![32803])),
        (0x828E, Value::Byte(vec![0, 1, 1, 2])),
    ];
    match rows_per_strip {
        Some(rps) => entries.extend([
            (0x0111, Value::Long(vec![at])),
            (0x0116, Value::Long(vec![rps])),
            (0x0117, Value::Long(vec![j.len() as u32])),
        ]),
        None => entries.extend([
            (0x0142, Value::Long(vec![tile_width])),
            (0x0143, Value::Long(vec![4])),
            (0x0144, Value::Long(vec![at])),
            (0x0145, Value::Long(vec![j.len() as u32])),
        ]),
    }
    let ifd = t.ifd(&entries, 0);
    t.set_first_ifd(ifd);
    t.out
}

#[test]
fn dng_reference_geometry_decodes() {
    // The valid baselines the zero-geometry reproducers below differ from.
    let tiled = dng::decode_dng_cfa(&dng_with_geometry(24, None)).unwrap();
    let stripped = dng::decode_dng_cfa(&dng_with_geometry(0, Some(4))).unwrap();
    assert_eq!((tiled.image.width, tiled.image.height), (24, 4));
    assert_eq!(tiled.image, stripped.image);
}

#[test]
fn zero_dng_tile_geometry_is_an_error() {
    assert_no_panic(
        &decode_dng,
        &[
            ("TileWidth 0", &dng_with_geometry(0, None)),
            ("RowsPerStrip 0", &dng_with_geometry(0, Some(0))),
        ],
    );
}

#[test]
fn surplus_uncompressed_dng_strips_are_an_error() {
    let mut t = TiffWriter::new(Order::Little);
    let at = t.blob(&[0; 2 * 4 * 4]);
    let ifd = t.ifd(
        &[
            (0x00FE, Value::Long(vec![0])),
            (0x0100, Value::Long(vec![4])),
            (0x0101, Value::Long(vec![4])),
            (0x0102, Value::Short(vec![16])),
            (0x0103, Value::Short(vec![1])),
            (0x0106, Value::Short(vec![32803])),
            (0x0111, Value::Long(vec![at, at, at])),
            (0x0116, Value::Long(vec![4])),
            (0x0117, Value::Long(vec![32, 32, 32])),
            (0x828E, Value::Byte(vec![0, 1, 1, 2])),
        ],
        0,
    );
    t.set_first_ifd(ifd);
    assert_no_panic(&decode_dng, &[("3 strips of 4 rows for 4 rows", &t.out)]);
}

#[test]
fn extreme_coefficients_do_not_panic() {
    let opts = Options {
        gcli_max: 30,
        planes_max: 15,
        q_max: 30,
        modes: vec![0],
        raw_percent: 0,
        ..Options::new(64, 48, 2)
    };
    let streams: Vec<Vec<u8>> = (0..4)
        .map(|seed| encoder::encode_random(&opts, seed).codestream)
        .collect();
    let inputs: Vec<(&str, &[u8])> = streams.iter().map(|s| ("GCLI 30", &s[..])).collect();
    assert_no_panic(&decode_cs, &inputs);
}

#[test]
fn large_fq_does_not_panic() {
    // PIH byte 20 is Fq << 4 | Br; Fq = 15 shifts every coefficient by 15 bits.
    let mut cs = sample_codestream().codestream;
    assert_eq!(cs[62], 0x44);
    cs[62] = 0xF4;
    assert_no_panic(&decode_cs, &[("Fq 15", &cs)]);
}

#[test]
fn widest_accepted_profile_wraps_instead_of_panicking() {
    // Bw 20 / Fq 8 is the largest profile Layout accepts; GCLIs at Bw with full bit planes
    // drive the inverse transforms past i32, which must wrap (no overflow panic in debug).
    let opts = Options {
        gcli_max: 20,
        planes_max: 15,
        q_max: 6,
        modes: vec![0],
        raw_percent: 0,
        ..Options::new(256, 64, 2)
    };
    let streams: Vec<Vec<u8>> = (0..6)
        .map(|seed| {
            let mut cs = encoder::encode_random(&opts, seed).codestream;
            assert_eq!((cs[61], cs[62]), (18, 0x44), "PIH Bw / Fq offsets moved");
            cs[61] = 20;
            cs[62] = 0x84;
            cs
        })
        .collect();
    let inputs: Vec<(&str, &[u8])> = streams.iter().map(|s| ("Bw 20 Fq 8", &s[..])).collect();
    assert_no_panic(&decode_cs, &inputs);
}

#[test]
fn inline_signs_are_reported_unsupported() {
    // PIH byte 23 holds Lh/Rl/Qpih/Fs/Rm; clear Fs (bits 3..2).
    let mut cs = sample_codestream().codestream;
    cs[65] &= !0x0C;
    assert!(matches!(
        ticoraw::decode_ticoraw(&cs),
        Err(TicoError::Unsupported(_))
    ));
}

#[test]
fn oversized_frame_is_rejected_before_allocating() {
    // A tiny codestream that claims a 65532 x 65532 frame must fail fast, not allocate GBs.
    let mut cs = sample_codestream().codestream;
    cs[50..54].copy_from_slice(&[0xFF, 0xFC, 0xFF, 0xFC]);
    assert!(matches!(
        ticoraw::decode_ticoraw(&cs),
        Err(TicoError::Header(_))
    ));
}

#[test]
fn fuzz_regressions_never_panic() {
    // Every minimised crash the fuzzers ever found lives under fuzz/artifacts-regressions/
    // <target>/; replay each through every public entry point.
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/artifacts-regressions");
    let Ok(targets) = std::fs::read_dir(&root) else {
        return; // not present in a packaged crate
    };
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for dir in targets.filter_map(Result::ok) {
        for f in std::fs::read_dir(dir.path())
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
        {
            if f.path().is_file() {
                files.push((
                    f.path().display().to_string(),
                    std::fs::read(f.path()).unwrap(),
                ));
            }
        }
    }
    let inputs: Vec<(&str, &[u8])> = files.iter().map(|(n, b)| (n.as_str(), &b[..])).collect();
    let all = |b: &[u8]| {
        let _ = ticoraw::decode_nef(b);
        let _ = ticoraw::inspect_nef(b);
        let _ = ticoraw::decode_ticoraw(b);
        let _ = ticoraw::dng::decode_dng_cfa(b);
        let _ = ticoraw::dng::decode_lossless_jpeg(b);
    };
    assert_no_panic(&all, &inputs);
}
