//! Complete DNG writer for Nikon High Efficiency (HE / HE\*) NEF files decoded by
//! [`ticoraw`].
//!
//! [`convert`] turns NEF bytes into DNG bytes that can stand in for an Adobe-converted DNG:
//!
//! - **IFD0**: a 256-pixel 8-bit RGB thumbnail, `Make`, `Model`, `Software`, the NEF's
//!   `DateTime` / `Artist` / `Copyright`, `Orientation`, DNG 1.4 version tags, Adobe's
//!   `UniqueCameraModel`, `ColorMatrix1` (D65), `AsShotNeutral`, `BaselineExposure`,
//!   `LensInfo` and `CameraSerialNumber` (from EXIF), `OriginalRawFileName` (when given),
//!   XMP, and the Nikon MakerNote in `DNGPrivateData`.
//! - **EXIF IFD** with every NEF EXIF field except the MakerNote (plus its
//!   Interoperability IFD), and the **GPS IFD**, both copied verbatim. `ExifVersion`
//!   (mandatory, but missing from Nikon Z files) is added as `0231` when absent.
//! - **SubIFD 0**: the full-resolution CFA image, 16 bits per sample, as 256x256 lossless
//!   JPEG tiles ([`Compression::LosslessJpeg`], the default) or one uncompressed strip,
//!   with CFA tags, black and white levels, default crop and active area.
//! - **SubIFD 1** (unless [`Preview::None`]): a JPEG preview. For SDR frames it is the
//!   camera's largest embedded JPEG, copied unchanged; HLG frames carry only HEVC previews,
//!   so for them (and whenever no usable JPEG is embedded) a half-size preview is rendered
//!   from the raw data by 2x2 binning.
//!
//! Carried metadata is bounded (see the size limits of [`CarriedMetadata`]), so a modified
//! NEF cannot make the DNG, or the memory used to write it, much larger than a normal one.
//!
//! The crate does no I/O: bytes in, bytes out.
//!
//! ```no_run
//! let nef = std::fs::read("DSC_0001.NEF")?;
//! let opts = ticoraw_dng::Options {
//!     original_raw_file_name: Some("DSC_0001.NEF".into()),
//!     ..ticoraw_dng::Options::default()
//! };
//! std::fs::write("DSC_0001.dng", ticoraw_dng::convert(&nef, &opts)?)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # What is not written
//!
//! No Adobe camera profile data (`ForwardMatrix`, `ProfileHueSatMap`, `ProfileLookTable`,
//! `ProfileToneCurve`, ...): those tables are Adobe's. Adobe software applies its own
//! profile anyway, because `UniqueCameraModel` carries Adobe's camera name; other raw
//! processors use their own. `NoiseProfile`, opcode lists and the raw-data digests are not
//! written either.
//!
//! # Features
//!
//! `parallel` (default) encodes tiles, renders previews and decodes on all cores with rayon.
//! The output is identical either way.

#![warn(missing_docs)]

mod ljpeg;
mod metadata;
mod render;
mod tiff_out;

use std::fmt;

use ticoraw::tiff::ByteOrder;
use ticoraw::{DecodedRaw, TicoError};

pub use metadata::{
    CarriedMakerNote, CarriedMetadata, CarriedPreview, MAX_CARRIED_FIELD_BYTES, MAX_CARRIED_FIELDS,
    MAX_CARRIED_MAKER_NOTE, MAX_CARRIED_XMP, PreviewColorSpace, RawField,
};

use metadata::{jpeg_info, within_limits};
use render::Rgb8;
use tiff_out::{Ifd, Tiff, Value};

/// Long side of the IFD0 thumbnail, in pixels.
pub const THUMBNAIL_SIZE: usize = 256;
/// JPEG quality of rendered previews.
pub const PREVIEW_QUALITY: u8 = 90;

/// How the full-resolution CFA image is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// Lossless JPEG (Compression 7) in 256x256 tiles: about the size of Adobe's DNGs.
    #[default]
    LosslessJpeg,
    /// One uncompressed 16-bit strip (Compression 1): about 1.7 to 2.6 times larger,
    /// simplest to read.
    Uncompressed,
}

/// Whether to embed a full preview (SubIFD 1). The IFD0 thumbnail is always written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preview {
    /// The camera's embedded JPEG ([`CarriedMetadata::preview`]) when there is one,
    /// otherwise a half-size rendering of the raw data.
    #[default]
    Auto,
    /// No preview.
    None,
}

/// Conversion options. `Options::default()` gives lossless compression, an automatic
/// preview and the NEF's XMP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Raw image storage.
    pub compression: Compression,
    /// Preview policy.
    pub preview: Preview,
    /// Copy the NEF's XMP packet (default `true`).
    pub carry_xmp: bool,
    /// An XMP packet to write instead of the NEF's, for example a sidecar `.xmp` the caller
    /// has merged. It **replaces** the carried packet, and is written even when `carry_xmp`
    /// is `false`. The bytes are written as given.
    pub extra_xmp: Option<Vec<u8>>,
    /// Written as `OriginalRawFileName` (usually the NEF's file name, without directories).
    pub original_raw_file_name: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            compression: Compression::default(),
            preview: Preview::default(),
            carry_xmp: true,
            extra_xmp: None,
            original_raw_file_name: None,
        }
    }
}

/// Every way conversion can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The NEF could not be read or decoded.
    Decode(TicoError),
    /// The [`DecodedRaw`] passed to [`write()`] is inconsistent: no pixels, a pixel count that
    /// differs from `width * height`, or a crop outside the mosaic.
    InvalidImage(String),
    /// A [`RawField`] of the [`CarriedMetadata`] passed to [`write()`] has a type outside
    /// 1..=12 or a byte length that does not match its type and count.
    InvalidMetadata(String),
    /// The DNG would exceed the 4 GiB limit of classic TIFF, or the preview the JPEG
    /// encoder's 65535-pixel limit.
    TooLarge(String),
    /// The preview JPEG encoder failed.
    Preview(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "{e}"),
            Self::InvalidImage(m) => write!(f, "invalid image: {m}"),
            Self::InvalidMetadata(m) => write!(f, "invalid carried metadata: {m}"),
            Self::TooLarge(m) => write!(f, "too large: {m}"),
            Self::Preview(m) => write!(f, "preview: {m}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Decode(e) => Some(e),
            _ => None,
        }
    }
}

impl From<TicoError> for Error {
    fn from(e: TicoError) -> Self {
        Self::Decode(e)
    }
}

/// Decodes a HE/HE\* NEF and writes it as a complete DNG; see the [crate docs](crate).
///
/// # Errors
///
/// [`Error::Decode`] with any error of [`ticoraw::decode_nef`] or
/// [`CarriedMetadata::from_nef`], and the errors of [`write()`].
pub fn convert(nef: &[u8], opts: &Options) -> Result<Vec<u8>, Error> {
    let raw = ticoraw::decode_nef(nef)?;
    let carried = CarriedMetadata::from_nef(nef)?;
    write(&raw, &carried, opts)
}

/// Writes `raw` and `carried` as a DNG in `carried.byte_order`.
///
/// Fields of `carried` are copied verbatim, except where this function writes the same tag
/// itself (dimensions, offsets, CFA, levels, colour, crop, orientation, IFD pointers, ...):
/// its own value always wins. Fields, MakerNote and XMP past the
/// [size limits](CarriedMetadata#size-limits) are left out.
///
/// # Errors
///
/// - [`Error::InvalidImage`] when `raw.image` is empty, its pixel count is not
///   `width * height`, or `raw.crop` does not lie inside it.
/// - [`Error::InvalidMetadata`] when a carried field is malformed.
/// - [`Error::TooLarge`] when the file would exceed 4 GiB or a rendered preview the JPEG
///   size limit.
/// - [`Error::Preview`] when the preview JPEG encoder fails.
pub fn write(
    raw: &DecodedRaw,
    carried: &CarriedMetadata,
    opts: &Options,
) -> Result<Vec<u8>, Error> {
    let img = &raw.image;
    if img.width == 0
        || img.height == 0
        || img.width.checked_mul(img.height) != Some(img.pixels.len())
    {
        return Err(Error::InvalidImage(format!(
            "{} pixels for a {}x{} image",
            img.pixels.len(),
            img.width,
            img.height
        )));
    }
    if let Some(c) = raw.crop {
        let fits =
            |o: usize, n: usize, max: usize| n > 0 && o.checked_add(n).is_some_and(|e| e <= max);
        if !fits(c.x, c.width, img.width) || !fits(c.y, c.height, img.height) {
            return Err(Error::InvalidImage(format!(
                "crop {}x{} at ({}, {}) outside the {}x{} mosaic",
                c.width, c.height, c.x, c.y, img.width, img.height
            )));
        }
    }
    check_fields(carried)?;
    let too_large = || Error::TooLarge("image too large for a classic TIFF/DNG".into());
    let width = u32::try_from(img.width).map_err(|_| too_large())?;
    let height = u32::try_from(img.height).map_err(|_| too_large())?;

    let mut budget = MAX_CARRIED_FIELD_BYTES;
    let [carried_ifd0, carried_exif, carried_interop, carried_gps] =
        [&carried.ifd0, &carried.exif, &carried.interop, &carried.gps]
            .map(|list| within_limits(list, &mut budget, |f| (f.tag, f.bytes.len())));
    let maker_note = carried
        .maker_note
        .as_ref()
        .filter(|mn| mn.bytes.len() <= MAX_CARRIED_MAKER_NOTE);
    let carried_xmp = carried
        .xmp
        .as_ref()
        .filter(|x| opts.carry_xmp && x.len() <= MAX_CARRIED_XMP);

    let order = carried.byte_order;
    let mut tiff = Tiff::new(order);

    // Rendering (needed for the thumbnail, and for the preview without a camera JPEG).
    let rendered = render::half_size(raw);
    let thumb = render::downscale(&rendered, THUMBNAIL_SIZE);
    let preview = match opts.preview {
        Preview::None => None,
        Preview::Auto => Some(choose_preview(carried, &rendered)?),
    };
    drop(rendered);

    let thumb_chunk = tiff.add_chunk(thumb.data);
    let preview_chunk = preview.map(|(jpeg, info, cs)| (tiff.add_chunk(jpeg), info, cs));

    // SubIFD 0: the raw image.
    let mut rawifd = Ifd::default();
    rawifd.set(0x00FE, Value::Long(vec![0]));
    rawifd.set(0x0100, Value::Long(vec![width]));
    rawifd.set(0x0101, Value::Long(vec![height]));
    rawifd.set(0x0102, Value::Short(vec![16]));
    rawifd.set(0x0106, Value::Short(vec![32803]));
    rawifd.set(0x0115, Value::Short(vec![1]));
    rawifd.set(0x011C, Value::Short(vec![1]));
    match opts.compression {
        Compression::LosslessJpeg => {
            let tiles = encode_tiles(raw);
            let counts = tiles
                .iter()
                .map(|t| u32::try_from(t.len()).map_err(|_| too_large()))
                .collect::<Result<Vec<_>, _>>()?;
            let ids = tiles.into_iter().map(|t| tiff.add_chunk(t)).collect();
            let tile = ljpeg::TILE as u32;
            rawifd.set(0x0103, Value::Short(vec![7]));
            rawifd.set(0x0142, Value::Long(vec![tile]));
            rawifd.set(0x0143, Value::Long(vec![tile]));
            rawifd.set(0x0144, Value::Chunks(ids));
            rawifd.set(0x0145, Value::Long(counts));
        }
        Compression::Uncompressed => {
            let strip: Vec<u8> = match order {
                ByteOrder::Little => img.pixels.iter().flat_map(|p| p.to_le_bytes()).collect(),
                ByteOrder::Big => img.pixels.iter().flat_map(|p| p.to_be_bytes()).collect(),
            };
            let len = u32::try_from(strip.len()).map_err(|_| too_large())?;
            rawifd.set(0x0103, Value::Short(vec![1]));
            rawifd.set(0x0111, Value::Chunks(vec![tiff.add_chunk(strip)]));
            rawifd.set(0x0116, Value::Long(vec![height]));
            rawifd.set(0x0117, Value::Long(vec![len]));
        }
    }
    let c = &raw.color;
    rawifd.set(0x828D, Value::Short(vec![2, 2]));
    rawifd.set(0x828E, Value::Byte(c.cfa_pattern.to_vec()));
    rawifd.set(0xC616, Value::Byte(vec![0, 1, 2]));
    rawifd.set(0xC617, Value::Short(vec![1]));
    rawifd.set(0xC619, Value::Short(vec![2, 2]));
    rawifd.set(0xC61A, Value::Short(c.black_level.to_vec()));
    rawifd.set(0xC61D, Value::Short(vec![c.white_level]));
    if let Some(crop) = raw.crop {
        // Fits: the crop lies inside the mosaic, whose size fits in u32.
        let v = |n: usize| n as u32;
        rawifd.set(0xC61F, Value::Long(vec![v(crop.x), v(crop.y)]));
        rawifd.set(0xC620, Value::Long(vec![v(crop.width), v(crop.height)]));
    }
    rawifd.set(0xC68D, Value::Long(vec![0, 0, height, width]));

    // IFD0: thumbnail and global tags.
    let mut ifd0 = Ifd::default();
    ifd0.set(0x00FE, Value::Long(vec![1]));
    ifd0.set(0x0100, Value::Long(vec![thumb.width as u32]));
    ifd0.set(0x0101, Value::Long(vec![thumb.height as u32]));
    ifd0.set(0x0102, Value::Short(vec![8, 8, 8]));
    ifd0.set(0x0103, Value::Short(vec![1]));
    ifd0.set(0x0106, Value::Short(vec![2]));
    ifd0.set(0x010F, Value::Ascii(raw.make.clone()));
    ifd0.set(0x0110, Value::Ascii(raw.model.clone()));
    ifd0.set(0x0111, Value::Chunks(vec![thumb_chunk]));
    ifd0.set(0x0112, Value::Short(vec![raw.orientation]));
    ifd0.set(0x0115, Value::Short(vec![3]));
    ifd0.set(0x0116, Value::Long(vec![thumb.height as u32]));
    ifd0.set(
        0x0117,
        Value::Long(vec![(thumb.width * thumb.height * 3) as u32]),
    );
    ifd0.set(0x011C, Value::Short(vec![1]));
    ifd0.set(0x0131, Value::Ascii(SOFTWARE.into()));
    ifd0.set(0xC612, Value::Byte(vec![1, 4, 0, 0]));
    ifd0.set(0xC613, Value::Byte(vec![1, 1, 0, 0]));
    ifd0.set(
        0xC614,
        Value::Ascii(ticoraw::unique_camera_model(&raw.make, &raw.model)),
    );
    if let Some(m) = &c.color_matrix {
        ifd0.set(0xC621, Value::SRational(m.to_vec()));
        ifd0.set(0xC65A, Value::Short(vec![21])); // D65
    } else {
        // DNG requires a colour matrix; identity keeps the file valid (colours will be off).
        let identity = [
            (1, 1),
            (0, 1),
            (0, 1),
            (0, 1),
            (1, 1),
            (0, 1),
            (0, 1),
            (0, 1),
            (1, 1),
        ];
        ifd0.set(0xC621, Value::SRational(identity.to_vec()));
    }
    if let Some(n) = &c.as_shot_neutral {
        ifd0.set(0xC628, Value::Rational(n.to_vec()));
    }
    if let Some(ev) = raw.baseline_exposure {
        ifd0.set(0xC62A, Value::SRational(vec![ev]));
    }
    ifd0.set(0xC716, Value::Ascii("ticoraw".into()));
    ifd0.set(0xC717, Value::Ascii(env!("CARGO_PKG_VERSION").into()));
    ifd0.set(0xC71A, Value::Long(vec![PreviewColorSpace::Srgb.code()]));
    if let Some(name) = &opts.original_raw_file_name {
        ifd0.set(0xC68B, Value::Ascii(name.clone()));
    }
    if let Some(x) = opts.extra_xmp.as_ref().or(carried_xmp) {
        ifd0.set(0x02BC, Value::Byte(x.clone()));
    }
    if let Some(mn) = maker_note {
        ifd0.set(
            0xC634,
            Value::Byte(wrap_maker_note(mn, order).ok_or_else(too_large)?),
        );
    }
    for f in &carried_ifd0 {
        ifd0.carry(f.tag, raw_value(f));
    }
    // DNG copies of EXIF lens and body facts, like Adobe writes.
    for (exif_tag, dng_tag, typ, count) in [(0xA432, 0xC630, 5, 4), (0xA431, 0xC62F, 2, 0)] {
        if let Some(f) = carried_exif
            .iter()
            .find(|f| f.tag == exif_tag && f.typ == typ && (count == 0 || f.count == count))
        {
            ifd0.carry(dng_tag, raw_value(f));
        }
    }

    // Lay out the IFD tree: IFD0 first, its SubIFDs, then EXIF (+ Interop) and GPS.
    let ifd0_id = tiff.add_ifd(Ifd::default()); // filled in below, once its pointers exist
    let raw_id = tiff.add_ifd(rawifd);
    let mut subs = vec![raw_id];
    if let Some((chunk, info, cs)) = preview_chunk {
        subs.push(tiff.add_ifd(preview_ifd(chunk, info, cs, &tiff)?));
    }
    ifd0.set(0x014A, Value::Ifds(subs));
    if !carried_exif.is_empty() || !carried_interop.is_empty() {
        let mut exif = Ifd::default();
        for f in &carried_exif {
            exif.carry(f.tag, raw_value(f));
        }
        // ExifVersion is mandatory, but Nikon Z bodies omit it; a carried one comes first.
        exif.carry(
            0x9000,
            Value::Raw {
                typ: 7,
                count: 4,
                bytes: b"0231".to_vec(),
            },
        );
        if !carried_interop.is_empty() {
            let mut interop = Ifd::default();
            for f in &carried_interop {
                interop.carry(f.tag, raw_value(f));
            }
            exif.set(0xA005, Value::Ifds(vec![tiff.add_ifd(interop)]));
        }
        ifd0.set(0x8769, Value::Ifds(vec![tiff.add_ifd(exif)]));
    }
    if !carried_gps.is_empty() {
        let mut gps = Ifd::default();
        for f in &carried_gps {
            gps.carry(f.tag, raw_value(f));
        }
        ifd0.set(0x8825, Value::Ifds(vec![tiff.add_ifd(gps)]));
    }
    tiff.ifds[ifd0_id] = ifd0;
    tiff.finish().map_err(|_| too_large())
}

/// Bytes per value of the classic TIFF field types 1..=12.
fn type_size(typ: u16) -> Option<usize> {
    Some(match typ {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => return None,
    })
}

/// Every carried field must have a classic TIFF type and exactly `count` values.
fn check_fields(carried: &CarriedMetadata) -> Result<(), Error> {
    let lists = [&carried.ifd0, &carried.exif, &carried.interop, &carried.gps];
    for f in lists.into_iter().flatten() {
        let expected = type_size(f.typ).and_then(|s| s.checked_mul(f.count as usize));
        if expected != Some(f.bytes.len()) {
            return Err(Error::InvalidMetadata(format!(
                "tag {:#06x}: type {} x {} values does not match {} bytes",
                f.tag,
                f.typ,
                f.count,
                f.bytes.len()
            )));
        }
    }
    Ok(())
}

const SOFTWARE: &str = concat!("ticoraw-dng ", env!("CARGO_PKG_VERSION"));

fn raw_value(f: &RawField) -> Value {
    Value::Raw {
        typ: f.typ,
        count: f.count,
        bytes: f.bytes.clone(),
    }
}

/// The DNG spec's `DNGPrivateData` wrapper for a MakerNote: `Adobe\0`, `MakN`, the byte
/// count of what follows (big-endian), the source byte order mark, the MakerNote's original
/// offset (big-endian) and the MakerNote itself. `None` past 4 GiB.
fn wrap_maker_note(mn: &CarriedMakerNote, order: ByteOrder) -> Option<Vec<u8>> {
    let count = u32::try_from(mn.bytes.len().checked_add(6)?).ok()?;
    let mut b = Vec::with_capacity(mn.bytes.len() + 20);
    b.extend_from_slice(b"Adobe\0MakN");
    b.extend_from_slice(&count.to_be_bytes());
    b.extend_from_slice(match order {
        ByteOrder::Little => b"II",
        ByteOrder::Big => b"MM",
    });
    b.extend_from_slice(&mn.original_offset.to_be_bytes());
    b.extend_from_slice(&mn.bytes);
    Some(b)
}

/// The camera JPEG when it is usable, otherwise the rendering encoded as JPEG.
fn choose_preview(
    carried: &CarriedMetadata,
    rendered: &Rgb8,
) -> Result<(Vec<u8>, metadata::JpegInfo, PreviewColorSpace), Error> {
    if let Some(p) = &carried.preview {
        if let Some(info) = jpeg_info(&p.jpeg) {
            return Ok((p.jpeg.clone(), info, p.color_space));
        }
    }
    let dim =
        |n: usize| u16::try_from(n).map_err(|_| Error::TooLarge(format!("{n}-pixel preview side")));
    let (w, h) = (dim(rendered.width)?, dim(rendered.height)?);
    let mut jpeg = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut jpeg, PREVIEW_QUALITY);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
    enc.encode(&rendered.data, w, h, jpeg_encoder::ColorType::Rgb)
        .map_err(|e| Error::Preview(e.to_string()))?;
    let info = jpeg_info(&jpeg)
        .ok_or_else(|| Error::Preview("encoder output has no readable SOF".into()))?;
    Ok((jpeg, info, PreviewColorSpace::Srgb))
}

fn preview_ifd(
    chunk: usize,
    info: metadata::JpegInfo,
    cs: PreviewColorSpace,
    tiff: &Tiff,
) -> Result<Ifd, Error> {
    let len = u32::try_from(tiff.chunks[chunk].len())
        .map_err(|_| Error::TooLarge("preview JPEG".into()))?;
    let mut p = Ifd::default();
    p.set(0x00FE, Value::Long(vec![1]));
    p.set(0x0100, Value::Long(vec![info.width as u32]));
    p.set(0x0101, Value::Long(vec![info.height as u32]));
    p.set(0x0102, Value::Short(vec![8, 8, 8]));
    p.set(0x0103, Value::Short(vec![7]));
    p.set(0x0106, Value::Short(vec![6]));
    p.set(0x0111, Value::Chunks(vec![chunk]));
    p.set(0x0115, Value::Short(vec![3]));
    p.set(0x0116, Value::Long(vec![info.height as u32]));
    p.set(0x0117, Value::Long(vec![len]));
    p.set(0x011C, Value::Short(vec![1]));
    p.set(
        0x0211,
        Value::Rational(vec![(299, 1000), (587, 1000), (114, 1000)]),
    );
    p.set(
        0x0212,
        Value::Short(vec![info.subsampling.0, info.subsampling.1]),
    );
    p.set(0x0213, Value::Short(vec![1]));
    p.set(
        0x0214,
        Value::Rational(vec![
            (0, 1),
            (255, 1),
            (128, 1),
            (255, 1),
            (128, 1),
            (255, 1),
        ]),
    );
    p.set(0xC716, Value::Ascii("ticoraw".into()));
    p.set(0xC717, Value::Ascii(env!("CARGO_PKG_VERSION").into()));
    p.set(0xC71A, Value::Long(vec![cs.code()]));
    Ok(p)
}

/// Every 256x256 tile of the mosaic as a lossless JPEG stream, in row-major tile order.
fn encode_tiles(raw: &DecodedRaw) -> Vec<Vec<u8>> {
    let img = &raw.image;
    let across = img.width.div_ceil(ljpeg::TILE);
    let down = img.height.div_ceil(ljpeg::TILE);
    par_map(across * down, |i| {
        let tile = ljpeg::extract_tile(&img.pixels, img.width, img.height, i % across, i / across);
        ljpeg::encode_tile(&tile, ljpeg::TILE, ljpeg::TILE)
    })
}

#[cfg(feature = "parallel")]
fn par_map<R: Send>(n: usize, f: impl Fn(usize) -> R + Sync + Send) -> Vec<R> {
    use rayon::prelude::*;
    (0..n).into_par_iter().map(f).collect()
}

#[cfg(not(feature = "parallel"))]
fn par_map<R: Send>(n: usize, f: impl Fn(usize) -> R + Sync + Send) -> Vec<R> {
    (0..n).map(f).collect()
}

/// Calls `f(row_index, row)` for each `row_len`-element row of `data`.
#[cfg(feature = "parallel")]
pub(crate) fn par_rows<T: Send>(
    data: &mut [T],
    row_len: usize,
    f: impl Fn(usize, &mut [T]) + Sync + Send,
) {
    use rayon::prelude::*;
    data.par_chunks_mut(row_len)
        .enumerate()
        .for_each(|(i, r)| f(i, r));
}

#[cfg(not(feature = "parallel"))]
pub(crate) fn par_rows<T: Send>(
    data: &mut [T],
    row_len: usize,
    f: impl Fn(usize, &mut [T]) + Sync + Send,
) {
    data.chunks_mut(row_len)
        .enumerate()
        .for_each(|(i, r)| f(i, r));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ticoraw::dng::decode_dng_cfa;
    use ticoraw::tiff::{self as rd, Tiff as Reader};
    use ticoraw::{CfaImage, ColorInfo, Crop};

    fn sample_raw(w: usize, h: usize) -> DecodedRaw {
        let mut seed = 7u64;
        DecodedRaw {
            make: "NIKON CORPORATION".into(),
            model: "NIKON Z6_3".into(),
            image: CfaImage {
                width: w,
                height: h,
                pixels: (0..w * h)
                    .map(|i| {
                        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                        let (x, y) = (i % w, i / w);
                        (1008 + (x * 13 + y * 7) % 3000 + (seed >> 61) as usize) as u16
                    })
                    .collect(),
            },
            color: ColorInfo {
                cfa_pattern: [0, 1, 1, 2],
                black_level: [1008, 1009, 1010, 1011],
                white_level: 15892,
                color_matrix: Some(
                    [11206, -4286, -941, -4879, 12847, 2251, -745, 1654, 7374].map(|v| (v, 10000)),
                ),
                as_shot_neutral: Some([(512, 924), (1, 1), (512, 717)]),
            },
            orientation: 6,
            crop: Some(Crop {
                x: 2,
                y: 4,
                width: w - 4,
                height: h - 6,
            }),
            baseline_exposure: Some((25, 100)),
        }
    }

    fn carried() -> CarriedMetadata {
        let f = |tag, typ, count, bytes: &[u8]| RawField {
            tag,
            typ,
            count,
            bytes: bytes.to_vec(),
        };
        CarriedMetadata {
            byte_order: ByteOrder::Little,
            ifd0: vec![f(0x0132, 2, 20, b"2025:01:02 03:04:05\0")],
            exif: vec![
                f(0x829A, 5, 1, &[1, 0, 0, 0, 0xF4, 1, 0, 0]),
                f(0x8827, 3, 1, &[0x90, 1]),
                f(0x9003, 2, 20, b"2025:01:02 03:04:05\0"),
                f(0xA431, 2, 4, b"123\0"),
            ],
            interop: vec![f(0x0001, 2, 4, b"R98\0")],
            gps: vec![f(0x0000, 1, 4, &[2, 3, 0, 0])],
            maker_note: Some(CarriedMakerNote {
                bytes: b"Nikon\0\x02\x11\0\0II*\0\x08\0\0\0\0\0\0\0\0\0".to_vec(),
                original_offset: 36144,
            }),
            xmp: Some(b"<x:xmpmeta/>".to_vec()),
            preview: None,
        }
    }

    #[test]
    fn round_trips_both_compressions_and_odd_sizes() {
        for (w, h) in [(300, 270), (64, 40), (258, 257), (7, 9), (2, 2)] {
            let mut raw = sample_raw(w.max(8), h.max(8));
            if w < 8 {
                raw = sample_raw(8, 10);
                raw.crop = None;
                raw.image = CfaImage {
                    width: w,
                    height: h,
                    pixels: raw.image.pixels[..w * h].to_vec(),
                };
            }
            for compression in [Compression::LosslessJpeg, Compression::Uncompressed] {
                let opts = Options {
                    compression,
                    ..Options::default()
                };
                let dng = write(&raw, &carried(), &opts).unwrap();
                let back = decode_dng_cfa(&dng).unwrap();
                assert_eq!(back.image, raw.image, "{w}x{h} {compression:?}");
                assert_eq!(back.black_level, raw.color.black_level);
                assert_eq!(back.white_level, 15892);
                assert_eq!(back.cfa_pattern, [0, 1, 1, 2]);
            }
        }
    }

    #[test]
    fn layout_and_metadata() {
        let raw = sample_raw(600, 400);
        let opts = Options {
            original_raw_file_name: Some("DSC_0001.NEF".into()),
            ..Options::default()
        };
        let dng = write(&raw, &carried(), &opts).unwrap();
        let t = Reader::parse(&dng, 0).unwrap();
        let ifd0 = t.ifd(t.first_ifd).unwrap();
        let u = |ifd: &rd::Ifd, tag| rd::first_uint(&t, ifd, tag).unwrap();
        assert_eq!(u(&ifd0, 0xFE), Some(1));
        assert_eq!((u(&ifd0, 0x100), u(&ifd0, 0x101)), (Some(256), Some(169)));
        assert_eq!(u(&ifd0, 0x112), Some(6));
        assert_eq!(t.ascii(ifd0.get(0xC614).unwrap()), "Nikon Z 6 3");
        assert_eq!(t.ascii(ifd0.get(0xC68B).unwrap()), "DSC_0001.NEF");
        assert_eq!(t.ascii(ifd0.get(0x0132).unwrap()), "2025:01:02 03:04:05");
        assert_eq!(t.ascii(ifd0.get(0xC62F).unwrap()), "123");
        assert_eq!(t.bytes(ifd0.get(0x02BC).unwrap()), b"<x:xmpmeta/>");
        assert_eq!(t.bytes(ifd0.get(0xC612).unwrap()), &[1, 4, 0, 0]);
        let private = t.bytes(ifd0.get(0xC634).unwrap());
        assert_eq!(&private[..10], b"Adobe\0MakN");
        let mn = &carried().maker_note.unwrap().bytes;
        assert_eq!(&private[10..14], &((mn.len() + 6) as u32).to_be_bytes());
        assert_eq!(&private[14..16], b"II");
        assert_eq!(&private[16..20], &36144u32.to_be_bytes());
        assert_eq!(&private[20..], &mn[..]);
        // Thumbnail: 8-bit RGB strip of the right size.
        let strip = u(&ifd0, 0x111).unwrap() as usize;
        assert_eq!(u(&ifd0, 0x117), Some(256 * 169 * 3));
        assert!(dng[strip..strip + 256 * 169 * 3].iter().any(|&v| v > 0));

        let subs = t.uints(ifd0.get(0x14A).unwrap()).unwrap();
        assert_eq!(subs.len(), 2);
        let rawifd = t.ifd(subs[0] as u32).unwrap();
        assert_eq!(u(&rawifd, 0xFE), Some(0));
        assert_eq!(u(&rawifd, 0x103), Some(7));
        assert_eq!(t.uints(rawifd.get(0xC61F).unwrap()).unwrap(), vec![2, 4]);
        assert_eq!(
            t.uints(rawifd.get(0xC68D).unwrap()).unwrap(),
            vec![0, 0, 400, 600]
        );
        let prev = t.ifd(subs[1] as u32).unwrap();
        assert_eq!(u(&prev, 0xFE), Some(1));
        assert_eq!(u(&prev, 0x103), Some(7));
        assert_eq!(u(&prev, 0x106), Some(6));
        // The rendered preview covers the crop at half size.
        assert_eq!((u(&prev, 0x100), u(&prev, 0x101)), (Some(298), Some(197)));
        let at = u(&prev, 0x111).unwrap() as usize;
        let len = u(&prev, 0x117).unwrap() as usize;
        assert_eq!(&dng[at..at + 2], &[0xFF, 0xD8]);
        assert_eq!(&dng[at + len - 2..at + len], &[0xFF, 0xD9]);
        assert_eq!(u(&prev, 0xC71A), Some(2));

        let exif = t.ifd(u(&ifd0, 0x8769).unwrap() as u32).unwrap();
        assert_eq!(u(&exif, 0x8827), Some(400));
        assert_eq!(t.ascii(exif.get(0x9003).unwrap()), "2025:01:02 03:04:05");
        assert_eq!(
            t.rationals(exif.get(0x829A).unwrap()).unwrap(),
            vec![(1, 500)]
        );
        let interop = t.ifd(u(&exif, 0xA005).unwrap() as u32).unwrap();
        assert_eq!(t.ascii(interop.get(0x0001).unwrap()), "R98");
        let gps = t.ifd(u(&ifd0, 0x8825).unwrap() as u32).unwrap();
        assert_eq!(t.bytes(gps.get(0).unwrap()), &[2, 3, 0, 0]);
    }

    #[test]
    fn options_control_preview_and_xmp() {
        let raw = sample_raw(64, 48);
        let opts = Options {
            preview: Preview::None,
            carry_xmp: false,
            ..Options::default()
        };
        let dng = write(&raw, &carried(), &opts).unwrap();
        let t = Reader::parse(&dng, 0).unwrap();
        let ifd0 = t.ifd(t.first_ifd).unwrap();
        assert_eq!(t.uints(ifd0.get(0x14A).unwrap()).unwrap().len(), 1);
        assert!(ifd0.get(0x02BC).is_none());

        let opts = Options {
            carry_xmp: false,
            extra_xmp: Some(b"<sidecar/>".to_vec()),
            ..Options::default()
        };
        let dng = write(&raw, &carried(), &opts).unwrap();
        let t = Reader::parse(&dng, 0).unwrap();
        let ifd0 = t.ifd(t.first_ifd).unwrap();
        assert_eq!(t.bytes(ifd0.get(0x02BC).unwrap()), b"<sidecar/>");
    }

    #[test]
    fn carried_preview_is_copied_verbatim() {
        let raw = sample_raw(64, 48);
        let mut jpeg = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut jpeg, 70);
        enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_1);
        enc.encode(&[90u8; 40 * 30 * 3], 40, 30, jpeg_encoder::ColorType::Rgb)
            .unwrap();
        let mut c = carried();
        c.preview = Some(CarriedPreview {
            jpeg: jpeg.clone(),
            color_space: PreviewColorSpace::AdobeRgb,
        });
        c.byte_order = ByteOrder::Big;
        c.maker_note = None;
        let dng = write(&raw, &c, &Options::default()).unwrap();
        let t = Reader::parse(&dng, 0).unwrap();
        assert_eq!(t.order, ByteOrder::Big);
        let ifd0 = t.ifd(t.first_ifd).unwrap();
        let subs = t.uints(ifd0.get(0x14A).unwrap()).unwrap();
        let prev = t.ifd(subs[1] as u32).unwrap();
        let u = |tag| rd::first_uint(&t, &prev, tag).unwrap().unwrap();
        assert_eq!((u(0x100), u(0x101), u(0xC71A)), (40, 30, 3));
        assert_eq!(t.uints(prev.get(0x212).unwrap()).unwrap(), vec![2, 1]);
        let at = u(0x111) as usize;
        assert_eq!(&dng[at..at + jpeg.len()], &jpeg[..]);
        assert_eq!(decode_dng_cfa(&dng).unwrap().image, raw.image);
    }

    #[test]
    fn invalid_images_are_errors() {
        let mut raw = sample_raw(16, 16);
        raw.image.pixels.pop();
        assert!(matches!(
            write(&raw, &CarriedMetadata::default(), &Options::default()),
            Err(Error::InvalidImage(_))
        ));
        let mut raw = sample_raw(16, 16);
        raw.crop = Some(Crop {
            x: usize::MAX,
            y: 0,
            width: 2,
            height: 2,
        });
        assert!(matches!(
            write(&raw, &CarriedMetadata::default(), &Options::default()),
            Err(Error::InvalidImage(_))
        ));
        let mut raw = sample_raw(16, 16);
        raw.image = CfaImage {
            width: 0,
            height: 0,
            pixels: vec![],
        };
        raw.crop = None;
        assert!(write(&raw, &CarriedMetadata::default(), &Options::default()).is_err());
    }

    #[test]
    fn malformed_carried_fields_are_errors() {
        let raw = sample_raw(16, 16);
        let mut c = carried();
        c.gps.push(RawField {
            tag: 2,
            typ: 5,
            count: 3,
            bytes: vec![0; 8],
        });
        assert!(matches!(
            write(&raw, &c, &Options::default()),
            Err(Error::InvalidMetadata(_))
        ));
        let mut c = carried();
        c.exif[0].typ = 13; // an IFD offset cannot be carried
        assert!(write(&raw, &c, &Options::default()).is_err());
        // A carried tag the writer sets itself loses: the SubIFDs pointer stays valid.
        let mut c = carried();
        c.ifd0.push(RawField {
            tag: 0x014A,
            typ: 4,
            count: 1,
            bytes: vec![0xFF; 4],
        });
        let dng = write(&raw, &c, &Options::default()).unwrap();
        assert_eq!(decode_dng_cfa(&dng).unwrap().image, raw.image);
    }

    #[test]
    fn hand_built_metadata_past_the_limits_is_left_out() {
        let raw = sample_raw(16, 16);
        let field = |tag, len: usize| RawField {
            tag,
            typ: 7,
            count: len as u32,
            bytes: vec![0xAB; len],
        };
        let mut c = carried();
        // 64 KiB fields: 16 would fill the shared budget, and the small fields of carried()
        // already use part of it.
        c.exif
            .extend((0..3000).map(|k| field(0xB000 + k, 64 << 10)));
        // A duplicate of a carried tag, then more fields than one list may carry.
        c.gps.push(field(0x0000, 4));
        c.gps.extend((0..3000).map(|k| field(0x1000 + k, 0)));
        c.maker_note.as_mut().unwrap().bytes = vec![0; MAX_CARRIED_MAKER_NOTE + 1];
        c.xmp = Some(vec![b' '; MAX_CARRIED_XMP + 1]);
        let dng = write(&raw, &c, &Options::default()).unwrap();
        let normal = write(&raw, &carried(), &Options::default()).unwrap();
        assert!(dng.len() < normal.len() + MAX_CARRIED_FIELD_BYTES + 64 * MAX_CARRIED_FIELDS);

        let t = Reader::parse(&dng, 0).unwrap();
        let ifd0 = t.ifd(t.first_ifd).unwrap();
        assert!(ifd0.get(0xC634).is_none() && ifd0.get(0x02BC).is_none());
        let u = |ifd: &rd::Ifd, tag| rd::first_uint(&t, ifd, tag).unwrap();
        let exif = t.ifd(u(&ifd0, 0x8769).unwrap() as u32).unwrap();
        assert_eq!(exif.entries.iter().filter(|e| e.tag >= 0xB000).count(), 15);
        assert_eq!(u(&exif, 0x8827), Some(400));
        let gps = t.ifd(u(&ifd0, 0x8825).unwrap() as u32).unwrap();
        assert_eq!(gps.entries.len(), MAX_CARRIED_FIELDS);
        assert_eq!(t.bytes(gps.get(0).unwrap()), &[2, 3, 0, 0]);
        assert_eq!(decode_dng_cfa(&dng).unwrap().image, raw.image);

        // At the limits, the MakerNote and XMP are still carried.
        let mut c = carried();
        c.maker_note.as_mut().unwrap().bytes = vec![0; MAX_CARRIED_MAKER_NOTE];
        c.xmp = Some(vec![b' '; MAX_CARRIED_XMP]);
        let dng = write(&raw, &c, &Options::default()).unwrap();
        let t = Reader::parse(&dng, 0).unwrap();
        let ifd0 = t.ifd(t.first_ifd).unwrap();
        assert_eq!(t.bytes(ifd0.get(0x02BC).unwrap()).len(), MAX_CARRIED_XMP);
        let private = t.bytes(ifd0.get(0xC634).unwrap());
        assert_eq!(private.len(), 20 + MAX_CARRIED_MAKER_NOTE);
    }

    #[test]
    fn unknown_models_get_an_identity_matrix() {
        let mut raw = sample_raw(32, 32);
        raw.model = "NIKON Z 99".into();
        raw.color.color_matrix = None;
        raw.color.as_shot_neutral = None;
        raw.baseline_exposure = None;
        let dng = write(&raw, &CarriedMetadata::default(), &Options::default()).unwrap();
        let t = Reader::parse(&dng, 0).unwrap();
        let ifd0 = t.ifd(t.first_ifd).unwrap();
        assert_eq!(
            t.ascii(ifd0.get(0xC614).unwrap()),
            "NIKON CORPORATION NIKON Z 99"
        );
        assert_eq!(t.rationals(ifd0.get(0xC621).unwrap()).unwrap()[0], (1, 1));
        assert!(ifd0.get(0x8769).is_none() && ifd0.get(0xC634).is_none());
    }
}
