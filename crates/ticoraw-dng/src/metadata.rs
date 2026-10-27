//! Metadata carried from a NEF into its DNG: EXIF, Interoperability and GPS fields, IFD0
//! descriptive tags, XMP, the Nikon MakerNote and the camera's embedded JPEG preview.

use ticoraw::container;
use ticoraw::tiff::{self, ByteOrder, Ifd, Tiff};

use crate::Error;

/// One TIFF field copied verbatim from the source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawField {
    /// Tag number.
    pub tag: u16,
    /// TIFF field type (1 = BYTE, 2 = ASCII, 3 = SHORT, 4 = LONG, 5 = RATIONAL, ...; 1 to
    /// 12 are carried).
    pub typ: u16,
    /// Number of values.
    pub count: u32,
    /// Value bytes in [`CarriedMetadata::byte_order`], `count` times the type's size.
    pub bytes: Vec<u8>,
}

/// The source file's MakerNote, carried into `DNGPrivateData`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarriedMakerNote {
    /// The MakerNote value as stored (for Nikon: `Nikon\0`, a version and an embedded TIFF
    /// whose offsets are relative to that TIFF, so it survives being moved).
    pub bytes: Vec<u8>,
    /// Where the MakerNote was in the source file, recorded for readers that need it.
    pub original_offset: u32,
}

/// Colour space of a preview image, as DNG `PreviewColorSpace` codes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewColorSpace {
    /// sRGB (code 2).
    #[default]
    Srgb,
    /// Adobe RGB (1998) (code 3).
    AdobeRgb,
}

impl PreviewColorSpace {
    /// The `PreviewColorSpace` tag value.
    #[must_use]
    pub fn code(self) -> u32 {
        match self {
            Self::Srgb => 2,
            Self::AdobeRgb => 3,
        }
    }
}

/// A camera-rendered JPEG preview to embed as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarriedPreview {
    /// A baseline JPEG stream (SOI .. EOI) with three components.
    pub jpeg: Vec<u8>,
    /// Its colour space.
    pub color_space: PreviewColorSpace,
}

/// Metadata to carry into the DNG next to the raw image.
///
/// [`CarriedMetadata::from_nef`] fills it from a NEF; [`CarriedMetadata::default`] carries
/// nothing (a little-endian DNG with only the tags derived from [`ticoraw::DecodedRaw`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CarriedMetadata {
    /// Byte order of every [`RawField`] and of the MakerNote. The DNG is written in this
    /// order, so values whose layout depends on it (inside UNDEFINED fields, for example)
    /// stay valid.
    pub byte_order: ByteOrder,
    /// IFD0 descriptive fields: `DateTime` (0x0132), `Artist` (0x013B) and `Copyright`
    /// (0x8298), when not blank.
    pub ifd0: Vec<RawField>,
    /// The EXIF IFD, without the MakerNote (0x927C) and IFD pointers.
    pub exif: Vec<RawField>,
    /// The EXIF Interoperability IFD (EXIF tag 0xA005), if any.
    pub interop: Vec<RawField>,
    /// The GPS IFD (IFD0 tag 0x8825), if any.
    pub gps: Vec<RawField>,
    /// The MakerNote, written to `DNGPrivateData` (0xC634).
    pub maker_note: Option<CarriedMakerNote>,
    /// The XMP packet (0x02BC).
    pub xmp: Option<Vec<u8>>,
    /// A camera-rendered preview to embed instead of rendering one; see
    /// [`Preview::Auto`](crate::Preview::Auto).
    pub preview: Option<CarriedPreview>,
}

impl Default for CarriedMetadata {
    fn default() -> Self {
        Self {
            byte_order: ByteOrder::Little,
            ifd0: Vec::new(),
            exif: Vec::new(),
            interop: Vec::new(),
            gps: Vec::new(),
            maker_note: None,
            xmp: None,
            preview: None,
        }
    }
}

/// Shortest long side, in pixels, of an embedded JPEG worth carrying as the preview.
pub(crate) const MIN_CARRIED_PREVIEW: usize = 1024;

/// Pointer tags whose values are offsets into the source file and cannot be copied.
const POINTER_TAGS: [u16; 6] = [0x0111, 0x014A, 0x0201, 0x8769, 0x8825, 0xA005];

impl CarriedMetadata {
    /// Collects the metadata of a NEF: IFD0 descriptive tags, the EXIF IFD (with its
    /// Interoperability IFD), the GPS IFD, the XMP packet, the MakerNote and, for frames not
    /// shot in HLG tone mode, the largest embedded JPEG preview with a long side of at least
    /// 1024 pixels. HLG frames only carry HEVC previews (which DNG cannot hold) and a tiny
    /// JPEG thumbnail, so their preview is rendered from the raw data instead.
    ///
    /// # Errors
    ///
    /// [`Error::Decode`] when the container, an IFD reachable from IFD0, the EXIF IFD, the
    /// Interoperability IFD or the GPS IFD is malformed.
    pub fn from_nef(nef: &[u8]) -> Result<Self, Error> {
        let c = container::parse_nef(nef)?;
        let meta = container::parse_metadata(nef)?;
        let t = Tiff::parse(nef, 0)?;
        let ifd0 = t.ifd(t.first_ifd)?;
        let descriptive = copy_fields(&t, &ifd0)
            .into_iter()
            .filter(|f| matches!(f.tag, 0x0132 | 0x013B | 0x8298) && f.typ == 2)
            .filter(|f| f.bytes.iter().any(|&b| b != 0 && b != b' '))
            .collect();
        let (mut exif, mut interop) = (Vec::new(), Vec::new());
        if let Some(off) = meta.exif_ifd {
            let ifd = t.ifd(off)?;
            exif = copy_fields(&t, &ifd);
            exif.retain(|f| f.tag != 0x927C);
            if let Some(off) = tiff::first_uint(&t, &ifd, 0xA005)? {
                interop = copy_fields(&t, &t.ifd(off as u32)?);
            }
        }
        let gps = match meta.gps_ifd {
            Some(off) => copy_fields(&t, &t.ifd(off)?),
            None => Vec::new(),
        };
        let maker_note = meta.maker_note.map(|r| CarriedMakerNote {
            original_offset: u32::try_from(r.start).unwrap_or(u32::MAX),
            bytes: nef[r].to_vec(),
        });
        let preview = if c.maker_note.is_hlg() {
            None
        } else {
            meta.jpeg_previews
                .iter()
                .filter_map(|r| jpeg_info(&nef[r.clone()]).map(|i| (r, i)))
                .filter(|(_, i)| i.width.max(i.height) >= MIN_CARRIED_PREVIEW)
                .max_by_key(|(_, i)| i.width * i.height)
                .map(|(r, _)| CarriedPreview {
                    jpeg: nef[r.clone()].to_vec(),
                    color_space: if c.maker_note.color_space == Some(2) {
                        PreviewColorSpace::AdobeRgb
                    } else {
                        PreviewColorSpace::Srgb
                    },
                })
        };
        Ok(Self {
            byte_order: meta.byte_order,
            ifd0: descriptive,
            exif,
            interop,
            gps,
            maker_note,
            xmp: meta.xmp.map(|r| nef[r].to_vec()),
            preview,
        })
    }
}

/// Every field of `ifd` with a classic TIFF type (1..=12) that is not an offset pointer.
fn copy_fields(t: &Tiff<'_>, ifd: &Ifd) -> Vec<RawField> {
    ifd.entries
        .iter()
        .filter(|e| (1..=12).contains(&e.typ) && !POINTER_TAGS.contains(&e.tag))
        .map(|e| RawField {
            tag: e.tag,
            typ: e.typ,
            count: e.count,
            bytes: t.bytes(e).to_vec(),
        })
        .collect()
}

/// Frame facts of a baseline/extended/progressive JPEG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JpegInfo {
    pub(crate) width: usize,
    pub(crate) height: usize,
    /// Luma sampling relative to chroma: (horizontal, vertical), e.g. (2, 1) for 4:2:2.
    pub(crate) subsampling: (u16, u16),
}

/// Reads the SOF of a three-component JPEG; `None` for anything else.
pub(crate) fn jpeg_info(j: &[u8]) -> Option<JpegInfo> {
    if !j.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut pos = 2;
    loop {
        while j.get(pos) == Some(&0xFF) && j.get(pos + 1) == Some(&0xFF) {
            pos += 1; // fill bytes
        }
        if *j.get(pos)? != 0xFF {
            return None;
        }
        let marker = *j.get(pos + 1)?;
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            pos += 2;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*j.get(pos + 2)?, *j.get(pos + 3)?]));
        let seg = j.get(pos + 4..pos + 2 + len)?;
        match marker {
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                let height = usize::from(u16::from_be_bytes([*seg.get(1)?, *seg.get(2)?]));
                let width = usize::from(u16::from_be_bytes([*seg.get(3)?, *seg.get(4)?]));
                if *seg.get(5)? != 3 || width == 0 || height == 0 {
                    return None;
                }
                let (y, cb) = (*seg.get(7)?, *seg.get(10)?);
                let (yh, yv, ch, cv) = (y >> 4, y & 15, cb >> 4, cb & 15);
                if ch == 0 || cv == 0 || yh % ch != 0 || yv % cv != 0 {
                    return None;
                }
                return Some(JpegInfo {
                    width,
                    height,
                    subsampling: (u16::from(yh / ch), u16::from(yv / cv)),
                });
            }
            0xD9 | 0xDA => return None,
            _ => pos += 2 + len,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_sof_of_a_real_encoder_stream() {
        let mut j = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut j, 80);
        enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_1);
        enc.encode(&[128u8; 30 * 20 * 3], 30, 20, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        let info = jpeg_info(&j).unwrap();
        assert_eq!(
            (info.width, info.height, info.subsampling),
            (30, 20, (2, 1))
        );
        assert_eq!(jpeg_info(&j[..10]), None);
        assert_eq!(jpeg_info(b"\xFF\xD8\xFF\xD9"), None);
        assert_eq!(jpeg_info(b"nope"), None);
    }

    #[test]
    fn preview_color_space_codes() {
        assert_eq!(PreviewColorSpace::Srgb.code(), 2);
        assert_eq!(PreviewColorSpace::AdobeRgb.code(), 3);
    }
}
