//! Lossless JPEG round trip: the synthetic T.81 encoder writes a frame with fuzzed size,
//! component count, precision and predictor, which `decode_lossless_jpeg` must read back
//! exactly. Optionally the same pixels go into a tiled or striped compression-7 DNG, which
//! `decode_dng_cfa` must also reproduce.
#![no_main]

use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;
use ticoraw_fuzz::synth::ljpeg;
use ticoraw_fuzz::synth::tiff::{self, DngLayout};

#[derive(Debug, Arbitrary)]
struct Input {
    /// `1 + rows % 32`, `1 + cols % 32`, `1 + nf % 4` components.
    rows: u8,
    cols: u8,
    nf: u8,
    /// `2 + precision % 15` bits (2..=16).
    precision: u8,
    /// `1 + predictor % 7`.
    predictor: u8,
    samples: Vec<u16>,
    dng: Option<Dng>,
}

#[derive(Debug, Arbitrary)]
struct Dng {
    /// CFA `2 + 2 * (width % 32)` by `1 + height % 32`.
    width: u8,
    height: u8,
    strips: bool,
    /// Tile width `2 * (1 + tile_width % 16)`; tile height or rows per strip `1 + rows % 16`.
    tile_width: u8,
    rows: u8,
    black_level: [u16; 4],
    white_level: u16,
}

fn fill(src: &[u16], n: usize, mask: u16) -> Vec<u16> {
    let mut v: Vec<u16> = src.iter().map(|s| s & mask).cycle().take(n).collect();
    v.resize(n, 0);
    v
}

fuzz_target!(|input: Input| {
    let rows = 1 + input.rows as usize % 32;
    let cols = 1 + input.cols as usize % 32;
    let nf = 1 + input.nf as usize % 4;
    let precision = 2 + input.precision % 15;
    let predictor = 1 + input.predictor % 7;
    let mask = (((1u32 << precision) - 1) & 0xFFFF) as u16;
    let samples = fill(&input.samples, rows * cols * nf, mask);
    let j = ljpeg::encode(&samples, rows, cols, nf, precision, predictor);
    let got = ticoraw::dng::decode_lossless_jpeg(&j).expect("a valid SOF3 stream decodes");
    assert_eq!((got.width, got.height), (cols * nf, rows));
    assert!(
        got.samples == samples,
        "lossless JPEG round trip changed samples"
    );

    if let Some(d) = &input.dng {
        let (w, h) = (2 + 2 * (d.width as usize % 32), 1 + d.height as usize % 32);
        let pixels = fill(&input.samples, w * h, u16::MAX);
        let r = 1 + d.rows as usize % 16;
        let layout = if d.strips {
            DngLayout::Strips(r)
        } else {
            DngLayout::Tiles(2 * (1 + d.tile_width as usize % 16), r)
        };
        let black = d.black_level.map(u32::from);
        let file = tiff::build_ljpeg_dng(&pixels, w, h, layout, black, d.white_level.into());
        let cfa = ticoraw::dng::decode_dng_cfa(&file).expect("a valid LJPEG DNG decodes");
        assert_eq!((cfa.image.width, cfa.image.height), (w, h));
        assert!(
            cfa.image.pixels == pixels,
            "LJPEG DNG round trip changed pixels"
        );
        assert_eq!(cfa.black_level, d.black_level);
        assert_eq!(cfa.white_level, d.white_level);
    }
});
