//! Writes the seed corpora for every fuzz target from synthetic inputs, so no camera file
//! is ever committed or needed:
//!
//! ```text
//! cargo run --release --manifest-path fuzz/Cargo.toml --example gen_seeds -- fuzz/corpus
//! ```
//!
//! Each target gets `<out>/<target>/seed-*` files. The output is deterministic; rerunning
//! it over an existing corpus only rewrites the seeds.

use std::path::{Path, PathBuf};
use std::{env, fs, io};

use ticoraw_fuzz::synth::encoder::{self, Options};
use ticoraw_fuzz::synth::tiff::{self, DngLayout, NefSpec, Order};
use ticoraw_fuzz::synth::{Rng, ljpeg};

struct Corpus {
    root: PathBuf,
    count: usize,
}

impl Corpus {
    fn add(&mut self, target: &str, name: &str, bytes: &[u8]) -> io::Result<()> {
        let dir = self.root.join(target);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(format!("seed-{name}")), bytes)?;
        self.count += 1;
        Ok(())
    }
}

/// Small codestreams covering every coding mode, slicing and padding variant.
fn codestreams() -> Vec<(String, Vec<u8>)> {
    let with = |w, h, hsl, f: &dyn Fn(&mut Options)| {
        let mut o = Options::new(w, h, hsl);
        f(&mut o);
        o
    };
    let cases: Vec<(&str, Options)> = vec![
        ("smallest", Options::new(16, 4, 1)),
        ("mixed", Options::new(64, 48, 2)),
        ("tall-slice", Options::new(18, 8, 16)),
        ("odd-width", Options::new(90, 44, 3)),
        ("raw", with(32, 16, 2, &|o| o.raw_percent = 100)),
        (
            "significance",
            with(32, 16, 2, &|o| (o.modes, o.raw_percent) = (vec![2], 0)),
        ),
        (
            "vertical",
            with(32, 16, 2, &|o| (o.modes, o.raw_percent) = (vec![1], 0)),
        ),
        (
            "vertical-significance",
            with(32, 16, 2, &|o| (o.modes, o.raw_percent) = (vec![3], 0)),
        ),
        (
            "run-mode",
            with(32, 16, 2, &|o| {
                (o.modes, o.run_mode, o.raw_percent) = (vec![2, 3], true, 10);
            }),
        ),
        ("deadzone", with(32, 16, 2, &|o| o.deadzone = true)),
        ("padded", with(34, 20, 3, &|o| o.padding = true)),
        ("coarse", with(32, 16, 2, &|o| o.q_max = 24)),
        ("lossless", with(32, 16, 2, &|o| o.q_max = 0)),
    ];
    cases
        .into_iter()
        .enumerate()
        .map(|(k, (name, o))| {
            (
                name.to_owned(),
                encoder::encode_random(&o, 100 + k as u64).codestream,
            )
        })
        .collect()
}

fn first_slice(cs: &[u8]) -> usize {
    ticoraw::header::parse_main_header(cs)
        .expect("synthetic header parses")
        .first_slice
}

/// NEF specs: both byte orders, known and unknown bodies, HE and HE*, missing tags.
fn nef_specs(width: usize, height: usize) -> Vec<(&'static str, NefSpec)> {
    let base = NefSpec::new(width, height);
    let with = |f: &dyn Fn(&mut NefSpec)| {
        let mut s = base.clone();
        f(&mut s);
        s
    };
    vec![
        ("z6iii-be", base.clone()),
        (
            "unknown-le-he",
            with(&|s| {
                s.order = Order::Little;
                s.model = "NIKON Z 99".into();
                s.compression_code = Some(13);
            }),
        ),
        ("no-makernote", with(&|s| s.maker_note = false)),
        (
            "sparse-makernote",
            with(&|s| {
                s.black_level = None;
                s.wb = None;
                s.compression_code = None;
                s.cfa_pattern = None;
            }),
        ),
    ]
}

/// A lossless JPEG with one restart interval per row (DRI + RSTn), which the synthetic
/// encoder cannot write directly: each row is encoded as its own one-row frame, whose scan
/// is exactly what follows a restart marker.
fn ljpeg_with_restarts(samples: &[u16], rows: usize, cols: usize, nf: usize) -> Vec<u8> {
    let scan_start = |j: &[u8]| {
        let sos = j.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        sos + 2 + u16::from_be_bytes([j[sos + 2], j[sos + 3]]) as usize
    };
    let full = ljpeg::encode(samples, rows, cols, nf, 14, 1);
    let sos = full.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    let mut out = full[..sos].to_vec();
    out.extend_from_slice(&[0xFF, 0xDD, 0x00, 0x04]);
    out.extend_from_slice(&(cols as u16).to_be_bytes());
    out.extend_from_slice(&full[sos..scan_start(&full)]);
    let stride = cols * nf;
    for y in 0..rows {
        if y > 0 {
            out.extend_from_slice(&[0xFF, 0xD0 + ((y - 1) % 8) as u8]);
        }
        let row = ljpeg::encode(&samples[y * stride..(y + 1) * stride], 1, cols, nf, 14, 1);
        out.extend_from_slice(&row[scan_start(&row)..row.len() - 2]);
    }
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

fn main() -> io::Result<()> {
    let root = env::args_os()
        .nth(1)
        .map_or_else(|| Path::new("fuzz/corpus").to_path_buf(), PathBuf::from);
    let mut c = Corpus { root, count: 0 };
    let mut rng = Rng::new(2026);

    // Codestreams, whole and cut to the main header (plus a little precinct data).
    let streams = codestreams();
    for (name, cs) in &streams {
        c.add("decode_ticoraw", name, cs)?;
        let fs = first_slice(cs);
        c.add("decode_ticoraw", &format!("{name}-header"), &cs[..fs])?;
    }
    let (_, mixed) = &streams[1];
    c.add(
        "decode_ticoraw",
        "mixed-cut",
        &mixed[..first_slice(mixed) + 64],
    )?;

    // NEFs around a small and a mid-size codestream, plus a non-TicoRAW strip.
    let small = encoder::encode_random(&Options::new(32, 16, 2), 7).codestream;
    let mut nefs = Vec::new();
    for (name, spec) in nef_specs(32, 16) {
        nefs.push((name.to_owned(), tiff::build_nef(&spec, &small)));
    }
    nefs.push((
        "z6iii-64x48".into(),
        tiff::build_nef(&NefSpec::new(64, 48), mixed),
    ));
    nefs.push((
        "classic-strip".into(),
        tiff::build_nef(&NefSpec::new(32, 16), &rng.bytes(96)),
    ));
    for (name, nef) in &nefs {
        c.add("decode_nef", name, nef)?;
        c.add("inspect_nef", name, nef)?;
        let mut whole = tiff_len_header(0);
        whole.extend_from_slice(nef);
        c.add("inspect_nef_prefix", name, &whole)?;
    }
    // Header-only prefixes: TIFF structures, MakerNote and the codestream main header.
    for (name, nef) in &nefs[..2] {
        let info = ticoraw::inspect_nef(nef).expect("synthetic NEF inspects");
        let cut = info.strip_offset + first_slice(&nef[info.strip_offset..]);
        let mut prefix = tiff_len_header((nef.len() - cut) as u32);
        prefix.extend_from_slice(&nef[..cut]);
        c.add("inspect_nef_prefix", &format!("{name}-header"), &prefix)?;
    }

    // DNGs: lossless-JPEG tiles and strips, and our own uncompressed writer's output.
    let (w, h) = (24, 12);
    let pixels: Vec<u16> = (0..w * h).map(|_| rng.below(16384) as u16).collect();
    let black = [1008, 1009, 1010, 1011];
    for (name, layout) in [
        ("ljpeg-tiles", DngLayout::Tiles(16, 8)),
        ("ljpeg-one-tile", DngLayout::Tiles(24, 12)),
        ("ljpeg-strips", DngLayout::Strips(5)),
    ] {
        let d = tiff::build_ljpeg_dng(&pixels, w, h, layout, black, 15892);
        c.add("decode_dng_cfa", name, &d)?;
    }
    for (name, nef) in &nefs[..2] {
        let raw = ticoraw::decode_nef(nef).expect("synthetic NEF decodes");
        let d = ticoraw::dng::encode_uncompressed_dng(&raw).expect("DNG encodes");
        c.add("decode_dng_cfa", &format!("uncompressed-{name}"), &d)?;
    }

    // Lossless JPEG streams: every predictor, several precisions and component counts.
    for predictor in 1..=7u8 {
        for (precision, nf, rows, cols) in
            [(14, 2, 4, 6), (16, 1, 5, 7), (12, 2, 1, 13), (8, 3, 3, 3)]
        {
            let max = (1u64 << precision) - 1;
            let samples: Vec<u16> = (0..rows * cols * nf)
                .map(|_| rng.below(max + 1) as u16)
                .collect();
            let j = ljpeg::encode(&samples, rows, cols, nf, precision, predictor);
            let name = format!("p{predictor}-{precision}bit-{nf}c-{rows}x{cols}");
            c.add("decode_lossless_jpeg", &name, &j)?;
        }
    }
    let samples: Vec<u16> = (0..6 * 5 * 2).map(|_| rng.below(16384) as u16).collect();
    let restarts = ljpeg_with_restarts(&samples, 6, 5, 2);
    let decoded = ticoraw::dng::decode_lossless_jpeg(&restarts).expect("restart seed decodes");
    assert_eq!(decoded.samples, samples, "restart seed round trip");
    c.add("decode_lossless_jpeg", "restarts", &restarts)?;

    // The structure-aware targets read their input through `Arbitrary`; any bytes work,
    // so give them a few deterministic random starts of different lengths.
    for target in ["codestream_structured", "dng_roundtrip", "ljpeg_roundtrip"] {
        for (k, len) in [16usize, 48, 128, 512].into_iter().enumerate() {
            c.add(target, &format!("random-{k}"), &rng.bytes(len))?;
        }
    }

    println!("wrote {} seeds under {}", c.count, c.root.display());
    Ok(())
}

fn tiff_len_header(extra: u32) -> Vec<u8> {
    ticoraw_fuzz::file_len_header(extra).to_vec()
}
