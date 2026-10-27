//! Clean-room decoder for Nikon High Efficiency (HE / HE*) NEF raw files.
//!
//! Nikon's HE and HE* modes store the Bayer mosaic as an intoPIX TicoRAW codestream: JPEG XS
//! (ISO/IEC 21122-1) entropy coding and 5/3 wavelets over four Star-Tetrix components, with
//! a vendor output transfer curve. This crate decodes it bit-exactly to the 14-bit CFA values
//! Adobe's converters produce.
//!
//! The crate operates purely on byte slices with no filesystem access, so it embeds anywhere
//! (including `wasm32-unknown-unknown` with `default-features = false`).
//!
//! ```no_run
//! let nef = std::fs::read("DSC_0001.NEF")?;
//! let raw = ticoraw::decode_nef(&nef)?;
//! println!("{}x{} {}", raw.image.width, raw.image.height, raw.model);
//! let dng = ticoraw::dng::encode_uncompressed_dng(&raw)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # API stability
//!
//! The stable surface is the crate root ([`inspect_nef`], [`inspect_nef_prefix`],
//! [`decode_nef`], [`decode_ticoraw`] and the data types they return, plus [`TicoError`]
//! and [`Result`]) and [`dng::decode_dng_cfa`] / [`dng::encode_uncompressed_dng`].
//!
//! The stage modules [`bitstream`], [`container`], [`entropy`], [`header`], [`mct`],
//! [`tiff`] and [`wavelet`] are public for testing and for tools that want to look inside a
//! codestream. Their API may change in minor releases.
//!
//! # Errors and hostile input
//!
//! Every expected failure is returned as a [`TicoError`] value; no input makes the decoder
//! panic. Headers are checked against the supported profile before any pixel work, and
//! frames are bounded by the codestream size before anything large is allocated. A crafted
//! stream that passes those checks can still decode to garbage pixels (the inverse
//! transforms use wrapping arithmetic), but it never panics.
//!
//! # Features
//!
//! `parallel` (default) decodes slices and runs the wavelet and colour stages on all cores
//! with rayon. Without it the crate is single-threaded and has no dependencies; the output
//! is identical either way.

#![warn(missing_docs)]

pub mod bitstream;
pub mod container;
pub mod dng;
pub mod entropy;
pub mod error;
pub mod header;
pub mod mct;
pub mod tiff;
pub mod wavelet;

pub use container::RawCodec;
pub use error::{Result, TicoError};

use entropy::Layout;
use header::{MainHeader, TransferCurve};

/// A single-plane CFA (Bayer) raster, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfaImage {
    /// Width in photosites (columns).
    pub width: usize,
    /// Height in photosites (rows).
    pub height: usize,
    /// `width * height` samples in row-major order (14-bit values for a TicoRAW decode).
    pub pixels: Vec<u16>,
}

/// Colour metadata needed to interpret a CFA raster.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorInfo {
    /// 2x2 CFA pattern codes (0 = R, 1 = G, 2 = B), row-major.
    pub cfa_pattern: [u8; 4],
    /// Black level per 2x2 CFA position, row-major like `cfa_pattern`: MakerNote 0x003D,
    /// or the transfer curve's black offset `B` when that tag is absent.
    pub black_level: [u16; 4],
    /// Saturation level: from the built-in per-model profile, or 16383 (the 14-bit
    /// maximum) for models without one.
    pub white_level: u16,
    /// XYZ(D65) -> camera matrix, row-major, as signed rationals; `None` for models without
    /// a built-in colour profile (see [`decode_nef`]).
    pub color_matrix: Option<[(i32, i32); 9]>,
    /// As-shot white balance as camera-neutral rationals (R, G, B), from MakerNote 0x000C.
    pub as_shot_neutral: Option<[(u32, u32); 3]>,
}

/// A fully decoded raw image with the metadata needed to write a DNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedRaw {
    /// TIFF `Make` of IFD0 (always starts with `NIKON`).
    pub make: String,
    /// TIFF `Model` of IFD0, for example `NIKON Z6_3`; selects the built-in colour profile.
    pub model: String,
    /// The decoded CFA mosaic.
    pub image: CfaImage,
    /// CFA pattern, levels and colour data for `image`.
    pub color: ColorInfo,
    /// TIFF Orientation of IFD0 (1 = normal; 6/8 = portrait). A missing or out-of-range
    /// value reads as 1. Written to the DNG as `Orientation`.
    pub orientation: u16,
    /// Recommended crop inside the mosaic (the camera's active image area) from MakerNote
    /// 0x0045. `None` when the tag is absent, empty or does not fit inside the mosaic.
    /// Written to the DNG as `DefaultCropOrigin` / `DefaultCropSize`.
    pub crop: Option<Crop>,
}

/// A rectangle inside the CFA mosaic, in photosites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crop {
    /// First column.
    pub x: usize,
    /// First row.
    pub y: usize,
    /// Width in columns.
    pub width: usize,
    /// Height in rows.
    pub height: usize,
}

/// Nikon's name for the raw compression mode: the 16-bit little-endian code at byte 10 of
/// MakerNote tag 0x0051.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NikonCompression {
    /// Code 13: High Efficiency (HE), a TicoRAW codestream.
    HighEfficiency,
    /// Code 14: High Efficiency\* (HE\*), a TicoRAW codestream at a higher rate.
    HighEfficiencyStar,
    /// Code 3: classic lossless compression (not decoded by this crate).
    Lossless,
    /// Any other code, kept verbatim.
    Other(u16),
}

impl NikonCompression {
    /// Maps the code stored in MakerNote 0x0051.
    #[must_use]
    pub fn from_code(code: u16) -> Self {
        match code {
            13 => Self::HighEfficiency,
            14 => Self::HighEfficiencyStar,
            3 => Self::Lossless,
            n => Self::Other(n),
        }
    }

    /// The MakerNote 0x0051 code for this mode.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            Self::HighEfficiency => 13,
            Self::HighEfficiencyStar => 14,
            Self::Lossless => 3,
            Self::Other(n) => n,
        }
    }
}

impl std::fmt::Display for NikonCompression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HighEfficiency => f.write_str("High Efficiency (HE)"),
            Self::HighEfficiencyStar => f.write_str("High Efficiency* (HE*)"),
            Self::Lossless => f.write_str("Lossless compressed"),
            Self::Other(c) => write!(f, "Nikon compression code {c}"),
        }
    }
}

/// Everything known about a NEF without decoding pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NefInspection {
    /// TIFF `Make` of IFD0.
    pub make: String,
    /// TIFF `Model` of IFD0.
    pub model: String,
    /// Width of the full-resolution raw IFD, in photosites.
    pub width: usize,
    /// Height of the full-resolution raw IFD, in photosites.
    pub height: usize,
    /// `BitsPerSample` of the raw IFD (14 on every HE/HE\* file seen).
    pub bits_per_sample: u16,
    /// Whether the raw strip is a TicoRAW codestream or a classic Nikon encoding.
    pub codec: RawCodec,
    /// Compression mode from MakerNote 0x0051; `None` when the tag is absent or too short.
    pub compression: Option<NikonCompression>,
    /// Byte offset of the raw strip in the file.
    pub strip_offset: usize,
    /// Length of the raw strip in bytes (`StripByteCounts`).
    pub strip_len: usize,
    /// Per-channel black levels from MakerNote 0x003D, if present.
    pub black_level: Option<[u16; 4]>,
    /// Present for TicoRAW strips.
    pub codestream: Option<CodestreamInfo>,
}

/// Summary of a TicoRAW codestream's main header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodestreamInfo {
    /// The parsed JPEG XS main header (SOC up to the first SLH).
    pub header: MainHeader,
    /// The output transfer curve carried in the PIH vendor bytes.
    pub curve: TransferCurve,
    /// Width of each of the four component planes (`Wf / 2`).
    pub component_width: usize,
    /// Height of each of the four component planes (`Hf / 2`).
    pub component_height: usize,
    /// Number of slices; 0 when the profile check failed.
    pub slices: usize,
    /// Number of precinct rows; 0 when the profile check failed.
    pub precincts: usize,
    /// Number of wavelet bands (25 on camera files); the number of WGT entries when the
    /// profile check failed.
    pub bands: usize,
    /// Why the codestream is outside the supported profile, or `None` when it can be decoded.
    pub unsupported: Option<String>,
}

/// Parses a NEF's container and codestream headers without decoding pixels.
///
/// A codestream outside the supported profile is not an error here: the reason is reported
/// in [`CodestreamInfo::unsupported`].
///
/// # Errors
///
/// - [`TicoError::Truncated`] when the input ends inside a TIFF structure, or is shorter
///   than the raw strip the raw IFD declares.
/// - [`TicoError::Container`] when the input is not a TIFF, its `Make` does not start with
///   `NIKON`, it has no full-resolution CFA IFD, or a required tag is missing or mistyped.
/// - For TicoRAW strips, [`TicoError::Header`] or [`TicoError::Truncated`] when the main
///   header is malformed, and [`TicoError::Unsupported`] when the PIH vendor extension is
///   too short to hold the transfer curve.
pub fn inspect_nef(nef: &[u8]) -> Result<NefInspection> {
    inspect_nef_prefix(nef, nef.len())
}

/// Like [`inspect_nef`], from just the start of a file of `file_len` bytes. A prefix that
/// covers the TIFF structures, the MakerNote and the codestream's main header (in practice
/// the first megabyte or two) is enough; a shorter one fails with [`TicoError::Truncated`].
///
/// `file_len` must be the real length of the file: it is used to check that the raw strip
/// fits inside it.
///
/// # Errors
///
/// As for [`inspect_nef`]. [`TicoError::Truncated`] also means the prefix was too short;
/// its `offset` says how far the parser needed to read.
pub fn inspect_nef_prefix(prefix: &[u8], file_len: usize) -> Result<NefInspection> {
    let c = container::parse_nef_prefix(prefix, file_len)?;
    let compression = c
        .maker_note
        .opaque
        .iter()
        .find(|(t, _)| *t == 0x51)
        .and_then(|(_, b)| {
            let code = u16::from_le_bytes([*b.get(10)?, *b.get(11)?]);
            Some(NikonCompression::from_code(code))
        });
    let codestream = match c.codec {
        RawCodec::TicoRaw => Some(inspect_codestream(c.strip)?),
        RawCodec::Classic => None,
    };
    Ok(NefInspection {
        make: c.make,
        model: c.model,
        width: c.width,
        height: c.height,
        bits_per_sample: c.bits_per_sample,
        codec: c.codec,
        compression,
        strip_offset: c.strip_offset,
        strip_len: c.strip_len,
        black_level: c.maker_note.black_level,
        codestream,
    })
}

fn inspect_codestream(cs: &[u8]) -> Result<CodestreamInfo> {
    let header = header::parse_main_header(cs)?;
    let curve = TransferCurve::from_extension(&header.pih.extension)?;
    let (layout, unsupported) = match Layout::new(&header) {
        Ok(l) => (Some(l), None),
        Err(e) => (None, Some(e.to_string())),
    };
    Ok(CodestreamInfo {
        component_width: header.pih.wf as usize / 2,
        component_height: header.pih.hf as usize / 2,
        slices: layout.as_ref().map_or(0, Layout::slices),
        precincts: layout.as_ref().map_or(0, |l| l.precincts),
        bands: layout
            .as_ref()
            .map_or(header.weights.len(), |l| l.bands.len()),
        header,
        curve,
        unsupported,
    })
}

/// Decodes a NEF whose raw strip is a TicoRAW (HE / HE*) codestream.
///
/// Besides the mosaic, the result carries the CFA pattern, black levels, as-shot white
/// balance, orientation and crop from the container. White level and colour matrix come
/// from a built-in per-model profile: the Z6III's is verified against Adobe Lightroom
/// DNGs; those of the Z8, Z9, Zf, Z5II, Z50II and ZR come from the public LibRaw / dnglab
/// tables and are unverified. Other models get white level 16383 and no colour matrix.
///
/// # Errors
///
/// - Everything [`inspect_nef`] reports, and [`TicoError::Truncated`] when the file is
///   shorter than the raw strip.
/// - [`TicoError::Unsupported`] when the strip is not TicoRAW encoded or the codestream is
///   outside the supported profile.
/// - [`TicoError::Container`] when the codestream and the raw IFD disagree on the frame
///   size.
/// - Any error of [`decode_ticoraw`] for a malformed codestream.
pub fn decode_nef(nef: &[u8]) -> Result<DecodedRaw> {
    let c = container::parse_nef(nef)?;
    if c.codec != RawCodec::TicoRaw {
        return Err(TicoError::Unsupported(
            "raw strip is not High Efficiency (TicoRAW) encoded".into(),
        ));
    }
    let header = header::parse_main_header(c.strip)?;
    let (wf, hf) = (header.pih.wf as usize, header.pih.hf as usize);
    if (wf, hf) != (c.width, c.height) {
        return Err(TicoError::Container(format!(
            "codestream is {wf}x{hf} but the raw IFD says {}x{}",
            c.width, c.height
        )));
    }
    let image = decode_ticoraw(c.strip)?;
    let curve_black = TransferCurve::from_extension(&header.pih.extension)?.black as u16;
    let as_shot_neutral = c
        .maker_note
        .wb_rb_levels
        .map(|[(rn, rd), (bn, bd)]| [(rd, rn), (1, 1), (bd, bn)]);
    let (white_level, color_matrix) = camera_profile(&c.model);
    let crop = c
        .maker_note
        .crop_area
        .map(|[x, y, w, h]| Crop {
            x: x.into(),
            y: y.into(),
            width: w.into(),
            height: h.into(),
        })
        .filter(|r| r.width > 0 && r.height > 0 && r.x + r.width <= wf && r.y + r.height <= hf);
    Ok(DecodedRaw {
        make: c.make,
        model: c.model,
        orientation: c.orientation,
        crop,
        image,
        color: ColorInfo {
            cfa_pattern: c.cfa_pattern,
            black_level: c.maker_note.black_level.unwrap_or([curve_black; 4]),
            white_level,
            color_matrix,
            as_shot_neutral,
        },
    })
}

/// Per-model white level and XYZ(D65)->camera matrix (Adobe's ColorMatrix for D65).
///
/// The Z6III entry is confirmed against Adobe Lightroom's HE* DNGs; the others come from
/// the public LibRaw / dnglab tables and are not yet verified against Adobe HE output.
fn camera_profile(model: &str) -> (u16, Option<[(i32, i32); 9]>) {
    let matrix: [i32; 9] = match model {
        "NIKON Z6_3" | "NIKON ZR" => [11206, -4286, -941, -4879, 12847, 2251, -745, 1654, 7374],
        "NIKON Z 8" => [11423, -4564, -1123, -4816, 12895, 2119, -210, 1061, 7282],
        "NIKON Z 9" => [13389, -6049, -1441, -4544, 12757, 1969, 229, 498, 7390],
        "NIKON Z f" => [11607, -4491, -977, -4522, 12460, 2304, -458, 1519, 7616],
        "NIKON Z5_2" => [13133, -5542, -1280, -4325, 12543, 1972, 34, 792, 7662],
        "NIKON Z50_2" => [11640, -4829, -1079, -5107, 13006, 2325, -972, 1711, 7380],
        _ => return (16383, None),
    };
    let white = if model == "NIKON Z 9" { 15520 } else { 15892 };
    (white, Some(matrix.map(|n| (n, 10000))))
}

/// Decodes a bare TicoRAW codestream (SOC..EOC) to its CFA mosaic.
///
/// The result is `Wf x Hf` photosites in RGGB order, mapped through the codestream's
/// transfer curve to 14-bit values.
///
/// # Errors
///
/// - [`TicoError::Unsupported`] when the header is outside the supported profile (see
///   `docs/FORMAT.md` §20 for the exact list).
/// - [`TicoError::Header`] for a missing or misordered marker segment, a WGT whose length
///   differs from the band count, a codestream with fewer than `Wf * Hf / 16` bytes, an
///   SLH with the wrong index, or no EOC after the last precinct.
/// - [`TicoError::Truncated`] when a segment, slice or precinct runs past the input.
/// - [`TicoError::Entropy`], naming the slice, for any failure inside a slice's precinct
///   data: a sub-packet that overruns its declared length or the precinct, a unary code
///   that is too long, or a GCLI that exceeds `Bw` or implies more than 15 bit planes.
pub fn decode_ticoraw(cs: &[u8]) -> Result<CfaImage> {
    let header = header::parse_main_header(cs)?;
    let curve = TransferCurve::from_extension(&header.pih.extension)?;
    let layout = Layout::new(&header)?;
    let (w, h) = (layout.comp_width, layout.comp_height);
    // Nikon writes constant-rate streams of 2.5-5 bits per photosite. Refusing streams far
    // below 0.5 bits keeps a tiny crafted input from making us allocate gigabytes.
    if cs.len().saturating_mul(16) < 4 * w * h {
        return Err(TicoError::Header(format!(
            "{} codestream bytes cannot hold a {}x{} frame",
            cs.len(),
            2 * w,
            2 * h
        )));
    }

    // Frame the slices from precinct lengths so they can be decoded independently.
    let slices = frame_slices(cs, header.first_slice, &layout)?;
    let rows_per_slice = 2 * layout.precincts_per_slice;
    let mut planes: [Vec<i32>; 4] = std::array::from_fn(|_| vec![0i32; w * h]);
    {
        let mut iters: Vec<_> = planes
            .iter_mut()
            .map(|p| p.chunks_mut(rows_per_slice * w))
            .collect();
        let mut jobs = Vec::with_capacity(slices.len());
        for (s, range) in slices.iter().enumerate() {
            let chunks: Vec<&mut [i32]> = iters.iter_mut().filter_map(Iterator::next).collect();
            jobs.push((s, range.clone(), chunks));
        }
        let results = par_map_owned(jobs, |(s, range, mut chunks)| {
            entropy::decode_slice(
                &layout,
                &cs[range],
                s * layout.precincts_per_slice,
                &mut chunks,
            )
            .map(|_| ())
            // Entropy errors carry offsets relative to the slice; name the slice.
            .map_err(|e| TicoError::Entropy(format!("slice {s}: {e}")))
        });
        results.into_iter().collect::<Result<Vec<()>>>()?;
    }

    // Inverse wavelet per component.
    for (c, plane) in planes.iter_mut().enumerate() {
        let levels = layout.nlx[c];
        if levels == 0 {
            continue;
        }
        par_chunks_mut(plane, w, |y, row| {
            let mut scratch = Vec::new();
            wavelet::synthesize_row(row, if y % 2 == 0 { levels } else { 1 }, &mut scratch);
        });
        let mut out = vec![0i32; w * h];
        wavelet::synthesize_columns(plane, w, &mut out);
        *plane = out;
    }

    mct::inverse_star_tetrix(&mut planes, w, h);
    let pixels = mct::assemble_rggb(&planes, w, h, &curve);
    Ok(CfaImage {
        width: 2 * w,
        height: 2 * h,
        pixels,
    })
}

/// Byte ranges of each slice's precinct data (after its SLH), validated against EOC.
fn frame_slices(cs: &[u8], first: usize, layout: &Layout) -> Result<Vec<std::ops::Range<usize>>> {
    let header_len = 5 + (2 * layout.bands.len()).div_ceil(8);
    let mut pos = first;
    let mut out = Vec::with_capacity(layout.slices());
    for s in 0..layout.slices() {
        if header::be16(cs, pos)? != header::SLH || header::be16(cs, pos + 2)? != 4 {
            return Err(TicoError::Header(format!(
                "expected SLH for slice {s} at byte {pos}"
            )));
        }
        let y = header::be16(cs, pos + 4)? as usize;
        if y != s {
            return Err(TicoError::Header(format!("slice {s} carries index {y}")));
        }
        pos += 6;
        let start = pos;
        let n = layout
            .precincts_per_slice
            .min(layout.precincts - s * layout.precincts_per_slice);
        for _ in 0..n {
            let b = cs.get(pos..pos + 3).ok_or(TicoError::Truncated {
                what: "precinct",
                offset: pos,
            })?;
            let lprc = (b[0] as usize) << 16 | (b[1] as usize) << 8 | b[2] as usize;
            pos += header_len + lprc;
        }
        if pos > cs.len() {
            return Err(TicoError::Truncated {
                what: "slice",
                offset: start,
            });
        }
        out.push(start..pos);
    }
    if header::be16(cs, pos)? != header::EOC {
        return Err(TicoError::Header(format!("expected EOC at byte {pos}")));
    }
    Ok(out)
}

#[cfg(feature = "parallel")]
pub(crate) fn par_map_owned<T: Send, R: Send>(
    items: Vec<T>,
    f: impl Fn(T) -> R + Sync + Send,
) -> Vec<R> {
    use rayon::prelude::*;
    items.into_par_iter().map(f).collect()
}

#[cfg(not(feature = "parallel"))]
pub(crate) fn par_map_owned<T: Send, R: Send>(
    items: Vec<T>,
    f: impl Fn(T) -> R + Sync + Send,
) -> Vec<R> {
    items.into_iter().map(f).collect()
}

/// Like [`par_chunks_mut`], collecting each call's result in chunk order.
#[cfg(feature = "parallel")]
pub(crate) fn par_chunks_mut_collect<T: Send, R: Send>(
    data: &mut [T],
    size: usize,
    f: impl Fn(usize, &mut [T]) -> R + Sync + Send,
) -> Vec<R> {
    use rayon::prelude::*;
    data.par_chunks_mut(size)
        .enumerate()
        .map(|(i, c)| f(i, c))
        .collect()
}

#[cfg(not(feature = "parallel"))]
pub(crate) fn par_chunks_mut_collect<T: Send, R: Send>(
    data: &mut [T],
    size: usize,
    f: impl Fn(usize, &mut [T]) -> R + Sync + Send,
) -> Vec<R> {
    data.chunks_mut(size)
        .enumerate()
        .map(|(i, c)| f(i, c))
        .collect()
}

/// Calls `f(index, chunk)` for each `size`-element chunk of `data`, in parallel when enabled.
#[cfg(feature = "parallel")]
pub(crate) fn par_chunks_mut<T: Send>(
    data: &mut [T],
    size: usize,
    f: impl Fn(usize, &mut [T]) + Sync + Send,
) {
    use rayon::prelude::*;
    data.par_chunks_mut(size)
        .enumerate()
        .for_each(|(i, c)| f(i, c));
}

#[cfg(not(feature = "parallel"))]
pub(crate) fn par_chunks_mut<T: Send>(
    data: &mut [T],
    size: usize,
    f: impl Fn(usize, &mut [T]) + Sync + Send,
) {
    data.chunks_mut(size).enumerate().for_each(|(i, c)| f(i, c));
}
