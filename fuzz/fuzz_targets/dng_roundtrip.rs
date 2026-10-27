//! DNG writer round trip: an arbitrary `DecodedRaw` (any make/model strings, levels,
//! matrices, orientation, crop, exposure) written by `encode_uncompressed_dng` must read
//! back through `decode_dng_cfa` with the same mosaic, CFA pattern and levels.
#![no_main]

use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;
use ticoraw::{CfaImage, ColorInfo, Crop, DecodedRaw, TicoError};

#[derive(Debug, Arbitrary)]
struct Input {
    /// `width % 65`, `height % 65` (0 included: the reader must reject an empty image).
    width: u8,
    height: u8,
    pixels: Vec<u16>,
    /// Hand the writer one pixel too few: it must refuse.
    short_pixels: bool,
    make: String,
    model: String,
    cfa_pattern: [u8; 4],
    black_level: [u16; 4],
    white_level: u16,
    color_matrix: Option<[(i32, i32); 9]>,
    as_shot_neutral: Option<[(u32, u32); 3]>,
    orientation: u16,
    crop: Option<[u16; 4]>,
    baseline_exposure: Option<(i32, i32)>,
}

fuzz_target!(|input: Input| {
    let (width, height) = (input.width as usize % 65, input.height as usize % 65);
    let n = width * height;
    let mut pixels: Vec<u16> = input.pixels.iter().copied().cycle().take(n).collect();
    pixels.resize(n, 0);
    let short = input.short_pixels && n > 0;
    if short {
        pixels.pop();
    }
    let raw = DecodedRaw {
        make: input.make,
        model: input.model,
        image: CfaImage {
            width,
            height,
            pixels,
        },
        color: ColorInfo {
            cfa_pattern: input.cfa_pattern,
            black_level: input.black_level,
            white_level: input.white_level,
            color_matrix: input.color_matrix,
            as_shot_neutral: input.as_shot_neutral,
        },
        orientation: input.orientation,
        crop: input.crop.map(|[x, y, w, h]| Crop {
            x: x.into(),
            y: y.into(),
            width: w.into(),
            height: h.into(),
        }),
        baseline_exposure: input.baseline_exposure,
    };
    if short {
        assert!(matches!(
            ticoraw::dng::encode_uncompressed_dng(&raw),
            Err(TicoError::Container(_))
        ));
    } else if n == 0 {
        let dng = ticoraw::dng::encode_uncompressed_dng(&raw).expect("an empty image encodes");
        assert!(ticoraw::dng::decode_dng_cfa(&dng).is_err());
    } else {
        ticoraw_fuzz::assert_dng_round_trip(&raw);
    }
});
