//! Support for the ticoraw fuzz targets and the seed generator (`examples/gen_seeds.rs`).
//!
//! The byte-level targets only require "no panic, no hang, bounded memory"; libFuzzer's
//! `-timeout` and `-rss_limit_mb` enforce the last two. The structure-aware targets build
//! valid inputs with [`synth`] and also check that they decode to what was encoded.

pub mod synth;

use synth::encoder::{Encoded, FQ, LEVELS, NIKON_EXT};
use ticoraw::header::TransferCurve;
use ticoraw::{CfaImage, DecodedRaw, dng, mct, wavelet};

/// Bytes at the front of an `inspect_nef_prefix` input that choose `file_len`.
pub const FILE_LEN_HEADER: usize = 8;

/// Splits an `inspect_nef_prefix` input into `(prefix, file_len)`.
///
/// The top two bits of the big-endian `u64` header pick how `file_len` relates to the
/// prefix length `n`: `n + low32`, `n - low32` (saturating), the raw value (any `usize`,
/// up to `usize::MAX`), or exactly `n`.
#[must_use]
pub fn split_file_len(data: &[u8]) -> (&[u8], usize) {
    let Some((head, prefix)) = data.split_first_chunk::<FILE_LEN_HEADER>() else {
        return (data, data.len());
    };
    let v = u64::from_be_bytes(*head);
    let low = (v & 0xFFFF_FFFF) as usize;
    let n = prefix.len();
    let file_len = match v >> 62 {
        0 => n.saturating_add(low),
        1 => n.saturating_sub(low),
        2 => usize::try_from(v & !(3 << 62)).unwrap_or(usize::MAX),
        _ => n,
    };
    (prefix, file_len)
}

/// The header that makes [`split_file_len`] return `prefix.len() + extra`.
#[must_use]
pub fn file_len_header(extra: u32) -> [u8; FILE_LEN_HEADER] {
    u64::from(extra).to_be_bytes()
}

/// The image the decoder must produce for `e`, built from the crate's public stage
/// functions (inverse wavelet, inverse Star-Tetrix, transfer curve) applied to the
/// coefficients the encoder wrote. A mismatch therefore points at the entropy decoder or
/// at the slice framing in `decode_ticoraw`.
#[must_use]
pub fn expected_image(e: &Encoded) -> CfaImage {
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
    let curve = TransferCurve::from_extension(&NIKON_EXT).expect("Nikon curve parses");
    CfaImage {
        width: 2 * w,
        height: 2 * h,
        pixels: mct::assemble_rggb(&planes, w, h, &curve),
    }
}

/// Writes `raw` as an uncompressed DNG, reads it back and panics unless the mosaic and
/// its levels survive unchanged.
pub fn assert_dng_round_trip(raw: &DecodedRaw) {
    let dng = dng::encode_uncompressed_dng(raw).expect("a well-formed DecodedRaw encodes");
    let back = dng::decode_dng_cfa(&dng).expect("our own DNG decodes");
    assert_eq!(back.image, raw.image, "DNG round trip changed the mosaic");
    assert_eq!(back.cfa_pattern, raw.color.cfa_pattern);
    assert_eq!(back.black_level, raw.color.black_level);
    assert_eq!(back.white_level, raw.color.white_level);
}
