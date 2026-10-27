//! NEF (TIFF) container: locates the raw strip and reads the Nikon MakerNote fields a
//! decoder needs, and locates the metadata a DNG writer carries over ([`parse_metadata`]).
//!
//! This stage module is public for testing and advanced use; its API may change in minor
//! releases. [`RawCodec`] is re-exported at the crate root as part of the stable surface.

use crate::error::{Result, TicoError};
use crate::tiff::{self, Ifd, Tiff};

/// How the raw strip of a NEF is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawCodec {
    /// JPEG XS / intoPIX TicoRAW codestream ("High Efficiency" / "High Efficiency*").
    TicoRaw,
    /// Any classic Nikon encoding (lossless/lossy Huffman, uncompressed); not decoded here.
    Classic,
}

/// The parts of a NEF a decoder needs, borrowed from the input bytes.
#[derive(Debug, Clone)]
pub struct NefContainer<'a> {
    /// TIFF `Make` of IFD0 (always starts with `NIKON`).
    pub make: String,
    /// TIFF `Model` of IFD0.
    pub model: String,
    /// Width of the raw IFD, in photosites.
    pub width: usize,
    /// Height of the raw IFD, in photosites.
    pub height: usize,
    /// `BitsPerSample` of the raw IFD.
    pub bits_per_sample: u16,
    /// How the raw strip is encoded, judged from its first bytes (`FF 10 FF 50`).
    pub codec: RawCodec,
    /// 2x2 CFA pattern codes (0 = R, 1 = G, 2 = B) from tag 0x828E; RGGB when absent.
    pub cfa_pattern: [u8; 4],
    /// TIFF Orientation of IFD0 (1 = normal, 8 = rotated 90 degrees counter-clockwise, ...).
    pub orientation: u16,
    /// Raw strip bytes (for `TicoRaw` this is the whole codestream).
    pub strip: &'a [u8],
    /// Full length of the raw strip in the file (`strip` may be shorter for a prefix).
    pub strip_len: usize,
    /// Byte offset of the raw strip in the file.
    pub strip_offset: usize,
    /// Selected Nikon MakerNote fields (all empty when the file has no Nikon MakerNote).
    pub maker_note: MakerNote,
}

/// Selected Nikon MakerNote fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MakerNote {
    /// Tag 0x003D: per-channel black level.
    pub black_level: Option<[u16; 4]>,
    /// Tag 0x000C: white-balance multipliers (red, blue) as rationals.
    pub wb_rb_levels: Option<[(u32, u32); 2]>,
    /// Tag 0x0045: the recommended crop (x, y, width, height) inside the raw frame.
    pub crop_area: Option<[u16; 4]>,
    /// Tag 0x001E: colour space (1 = sRGB, 2 = Adobe RGB, 4 = BT.2100, i.e. HLG tone mode).
    pub color_space: Option<u16>,
    /// Whether tag 0x0023 (Picture Control data) names an HLG Picture Control.
    pub hlg_picture_control: bool,
    /// Raw payloads of tags whose layout is camera-generation specific (0x0051, 0x0058, ...).
    pub opaque: Vec<(u16, Vec<u8>)>,
}

impl MakerNote {
    /// Whether the frame was shot in Nikon's HLG tone mode.
    #[must_use]
    pub fn is_hlg(&self) -> bool {
        self.color_space == Some(4) || self.hlg_picture_control
    }
}

const OPAQUE_TAGS: [u16; 4] = [0x0051, 0x0058, 0x0059, 0x005B];

/// Parses a NEF file far enough to locate and describe its full-resolution raw strip.
///
/// # Errors
///
/// [`TicoError::Container`] for a non-TIFF or non-Nikon file, a missing CFA raw IFD or a
/// missing required tag; [`TicoError::Truncated`] when a structure or the raw strip runs
/// past the end of `nef`.
pub fn parse_nef(nef: &[u8]) -> Result<NefContainer<'_>> {
    parse_nef_prefix(nef, nef.len()).and_then(|c| {
        if c.strip.len() == c.strip_len {
            Ok(c)
        } else {
            Err(TicoError::Truncated {
                what: "raw strip",
                offset: c.strip_offset + c.strip.len(),
            })
        }
    })
}

/// Like [`parse_nef`], but `nef` may be only the first bytes of a file of `file_len` bytes;
/// `strip` then holds whatever part of the raw strip the prefix contains (at least 4 bytes).
///
/// # Errors
///
/// As for [`parse_nef`], except that a strip extending past the prefix is accepted as long
/// as it fits in `file_len` and its first 4 bytes are present.
pub fn parse_nef_prefix(nef: &[u8], file_len: usize) -> Result<NefContainer<'_>> {
    let t = Tiff::parse(nef, 0)?;
    let ifd0 = t.ifd(t.first_ifd)?;
    let make = ifd0.get(0x10F).map(|e| t.ascii(e)).unwrap_or_default();
    let model = ifd0.get(0x110).map(|e| t.ascii(e)).unwrap_or_default();
    let orientation = tiff::first_uint(&t, &ifd0, 0x112)?
        .filter(|o| (1..=8).contains(o))
        .unwrap_or(1) as u16;
    if !make.to_ascii_uppercase().starts_with("NIKON") {
        return Err(TicoError::Container(format!(
            "not a Nikon file (Make = {make:?})"
        )));
    }
    let ifds = tiff::all_ifds(&t)?;
    let raw = ifds
        .iter()
        .filter(|ifd| tiff::first_uint(&t, ifd, 0xFE).ok().flatten() == Some(0))
        .filter(|ifd| tiff::first_uint(&t, ifd, 0x106).ok().flatten() == Some(32803))
        .max_by_key(|ifd| tiff::first_uint(&t, ifd, 0x100).ok().flatten().unwrap_or(0))
        .ok_or_else(|| TicoError::Container("no CFA raw IFD".into()))?;
    let width = tiff::require_uint(&t, raw, 0x100)? as usize;
    let height = tiff::require_uint(&t, raw, 0x101)? as usize;
    let bits_per_sample = tiff::require_uint(&t, raw, 0x102)? as u16;
    let strip_offset = tiff::require_uint(&t, raw, 0x111)? as usize;
    let strip_len = tiff::require_uint(&t, raw, 0x117)? as usize;
    let strip_end = strip_offset.saturating_add(strip_len);
    if strip_end > file_len || strip_offset.saturating_add(4) > nef.len() {
        return Err(TicoError::Truncated {
            what: "raw strip",
            offset: strip_offset,
        });
    }
    let strip = &nef[strip_offset..strip_end.min(nef.len())];
    let codec = if strip.starts_with(&[0xFF, 0x10, 0xFF, 0x50]) {
        RawCodec::TicoRaw
    } else {
        RawCodec::Classic
    };
    let cfa_pattern = match raw.get(0x828E) {
        Some(e) if e.count == 4 => {
            let b = t.bytes(e);
            [b[0], b[1], b[2], b[3]]
        }
        _ => [0, 1, 1, 2],
    };
    let maker_note = find_maker_note(&t, &ifds)?.unwrap_or_default();
    Ok(NefContainer {
        make,
        model,
        width,
        height,
        bits_per_sample,
        codec,
        cfa_pattern,
        orientation,
        strip,
        strip_len,
        strip_offset,
        maker_note,
    })
}

/// Where a NEF keeps the metadata a DNG writer carries over, as positions in the file.
///
/// Every offset and range is absolute in the input buffer (a NEF's TIFF header sits at byte
/// 0, so TIFF offsets and file positions coincide). Ranges are checked to lie inside the
/// input; IFD offsets are reported as stored and still have to be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NefMetadata {
    /// Byte order of the TIFF structure, and so of every value in the EXIF, GPS and
    /// MakerNote data.
    pub byte_order: tiff::ByteOrder,
    /// Offset of the EXIF IFD (IFD0 tag 0x8769), if present.
    pub exif_ifd: Option<u32>,
    /// Offset of the GPS IFD (IFD0 tag 0x8825), if present.
    pub gps_ifd: Option<u32>,
    /// The XMP packet (IFD0 tag 0x02BC), if present.
    pub xmp: Option<std::ops::Range<usize>>,
    /// The MakerNote value (EXIF tag 0x927C) as stored, Nikon header included, if present.
    pub maker_note: Option<std::ops::Range<usize>>,
    /// Embedded JPEG previews: every IFD with `JPEGInterchangeFormat` (0x0201) and
    /// `JPEGInterchangeFormatLength` (0x0202) whose stream lies inside the input and starts
    /// with an SOI marker, in IFD walk order (IFD0 first). A pointer that does not satisfy
    /// that is not a usable preview and is left out.
    pub jpeg_previews: Vec<std::ops::Range<usize>>,
}

/// Locates the metadata of a NEF (or any TIFF) that a DNG writer carries over: EXIF, GPS,
/// XMP, the MakerNote and embedded JPEG previews.
///
/// Unlike [`parse_nef`] this does not require a Nikon `Make` or a raw IFD.
///
/// # Errors
///
/// [`TicoError::Container`] or [`TicoError::Truncated`] when the TIFF header, an IFD
/// reachable from IFD0, or the EXIF IFD is malformed.
pub fn parse_metadata(nef: &[u8]) -> Result<NefMetadata> {
    let t = Tiff::parse(nef, 0)?;
    let ifds = tiff::all_ifds(&t)?;
    let ifd0 = t.ifd(t.first_ifd)?;
    let pointer =
        |tag| -> Result<Option<u32>> { Ok(tiff::first_uint(&t, &ifd0, tag)?.map(|v| v as u32)) };
    let exif_ifd = pointer(0x8769)?;
    let gps_ifd = pointer(0x8825)?;
    let range = |e: &tiff::Entry| {
        let b = t.bytes(e);
        e.pos..e.pos + b.len()
    };
    let xmp = ifd0.get(0x02BC).map(range);
    let maker_note = match exif_ifd {
        Some(off) => t.ifd(off)?.get(0x927C).map(range),
        None => None,
    };
    let jpeg_previews = ifds
        .iter()
        .filter_map(|ifd| {
            let at = tiff::first_uint(&t, ifd, 0x201).ok()??;
            let len = tiff::first_uint(&t, ifd, 0x202).ok()??;
            let start = usize::try_from(at).ok()?;
            let end = start.checked_add(usize::try_from(len).ok()?)?;
            nef.get(start..end)
                .filter(|j| j.starts_with(&[0xFF, 0xD8]))
                .map(|_| start..end)
        })
        .collect();
    Ok(NefMetadata {
        byte_order: t.order,
        exif_ifd,
        gps_ifd,
        xmp,
        maker_note,
        jpeg_previews,
    })
}

fn find_maker_note(t: &Tiff<'_>, ifds: &[Ifd]) -> Result<Option<MakerNote>> {
    let ifd0 = &ifds[0];
    let Some(exif) = tiff::first_uint(t, ifd0, 0x8769)? else {
        return Ok(None);
    };
    let exif = t.ifd(exif as u32)?;
    let Some(e) = exif.get(0x927C) else {
        return Ok(None);
    };
    let mn = t.bytes(e);
    // Nikon type-3 MakerNote: "Nikon\0" + version(4) + embedded TIFF.
    if !mn.starts_with(b"Nikon\0") || mn.len() < 18 {
        return Ok(None);
    }
    let mt = Tiff::parse(t.data, e.pos + 10)?;
    let ifd = mt.ifd(mt.first_ifd)?;
    let black_level = match ifd.get(0x3D) {
        Some(e) if e.count == 4 => {
            let v = mt.uints(e)?;
            Some([v[0] as u16, v[1] as u16, v[2] as u16, v[3] as u16])
        }
        _ => None,
    };
    let wb_rb_levels = match ifd.get(0x0C) {
        Some(e) if e.typ == 5 && e.count >= 2 => {
            let r = mt.rationals(e)?;
            Some([
                (r[0].0 as u32, r[0].1 as u32),
                (r[1].0 as u32, r[1].1 as u32),
            ])
        }
        _ => None,
    };
    let crop_area = match ifd.get(0x45) {
        Some(e) if e.typ == 3 && e.count == 4 => {
            let v = mt.uints(e)?;
            Some([v[0] as u16, v[1] as u16, v[2] as u16, v[3] as u16])
        }
        _ => None,
    };
    let color_space = match ifd.get(0x1E) {
        Some(e) if e.typ == 3 && e.count >= 1 => mt.uints(e)?.first().map(|&v| v as u16),
        _ => None,
    };
    let hlg_picture_control = ifd
        .get(0x23)
        .is_some_and(|e| mt.bytes(e).windows(5).any(|w| w == b"(HLG)"));
    let opaque = OPAQUE_TAGS
        .iter()
        .filter_map(|&tag| ifd.get(tag).map(|e| (tag, mt.bytes(e).to_vec())))
        .collect();
    Ok(Some(MakerNote {
        black_level,
        wb_rb_levels,
        crop_area,
        color_space,
        hlg_picture_control,
        opaque,
    }))
}
