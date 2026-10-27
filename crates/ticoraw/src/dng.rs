//! DNG support: a self-contained ITU-T T.81 lossless JPEG (SOF3) reader for the CFA image
//! of Adobe-written DNGs, and a minimal uncompressed CFA DNG writer.
//!
//! [`decode_dng_cfa`], [`DngCfa`] and [`encode_uncompressed_dng`] are part of the stable
//! API. [`decode_lossless_jpeg`] and [`LosslessJpeg`] are exposed for testing and advanced
//! use; they may change in minor releases.

use crate::error::{Result, TicoError};
use crate::tiff::{self, Ifd, Tiff};
use crate::{CfaImage, DecodedRaw};

/// CFA raster plus the levels a DNG declares for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DngCfa {
    /// The full-resolution CFA image.
    pub image: CfaImage,
    /// 2x2 CFA pattern codes (0 = R, 1 = G, 2 = B), row-major.
    pub cfa_pattern: [u8; 4],
    /// Black level per 2x2 CFA position (rounded; Adobe writes integral values).
    pub black_level: [u16; 4],
    /// `WhiteLevel` (0xC61D), or 65535 when absent.
    pub white_level: u16,
}

/// Decodes the full-resolution CFA image of a DNG (lossless JPEG or uncompressed).
///
/// Black levels default to 0 when the DNG declares none.
///
/// # Errors
///
/// - [`TicoError::Container`] for a malformed TIFF, no full-resolution CFA IFD, missing or
///   inconsistent strip/tile tags, or a frame larger than the file could encode.
/// - [`TicoError::Unsupported`] for a compression other than lossless JPEG (7) or
///   uncompressed 16-bit (1), or a CFA pattern other than 2x2.
/// - [`TicoError::Truncated`] when strips or tiles run past the input.
/// - [`TicoError::Jpeg`] for a malformed lossless JPEG tile.
pub fn decode_dng_cfa(dng: &[u8]) -> Result<DngCfa> {
    let t = Tiff::parse(dng, 0)?;
    let ifds = tiff::all_ifds(&t)?;
    let raw = ifds
        .iter()
        .find(|ifd| {
            tiff::first_uint(&t, ifd, 0xFE).ok().flatten() == Some(0)
                && tiff::first_uint(&t, ifd, 0x106).ok().flatten() == Some(32803)
        })
        .ok_or_else(|| TicoError::Container("no full-resolution CFA IFD in DNG".into()))?;
    let width = tiff::require_uint(&t, raw, 0x100)? as usize;
    let height = tiff::require_uint(&t, raw, 0x101)? as usize;
    let bps = tiff::require_uint(&t, raw, 0x102)?;
    if width == 0 || height == 0 {
        return Err(TicoError::Container(format!(
            "empty {width}x{height} CFA image"
        )));
    }
    let compression = tiff::require_uint(&t, raw, 0x103)?;
    let pixels = match compression {
        7 => decode_ljpeg_layout(&t, raw, width, height)?,
        1 if bps == 16 => decode_uncompressed(&t, raw, width, height)?,
        c => {
            return Err(TicoError::Unsupported(format!(
                "DNG compression {c} with {bps} bps"
            )));
        }
    };
    Ok(DngCfa {
        image: CfaImage {
            width,
            height,
            pixels,
        },
        cfa_pattern: cfa_pattern(&t, raw)?,
        black_level: black_levels(&t, raw)?,
        white_level: tiff::first_uint(&t, raw, 0xC61D)?.unwrap_or(65535) as u16,
    })
}

fn cfa_pattern(t: &Tiff<'_>, ifd: &Ifd) -> Result<[u8; 4]> {
    match ifd.get(0x828E) {
        Some(e) if e.count == 4 => {
            let b = t.bytes(e);
            Ok([b[0], b[1], b[2], b[3]])
        }
        _ => Err(TicoError::Unsupported("CFA pattern other than 2x2".into())),
    }
}

fn black_levels(t: &Tiff<'_>, ifd: &Ifd) -> Result<[u16; 4]> {
    let Some(e) = ifd.get(0xC61A) else {
        return Ok([0; 4]);
    };
    let vals: Vec<i64> = match e.typ {
        5 | 10 => t
            .rationals(e)?
            .into_iter()
            .map(|(n, d)| if d == 0 { 0 } else { (n + d / 2) / d })
            .collect(),
        _ => t.uints(e)?.into_iter().map(|v| v as i64).collect(),
    };
    let pick = |i: usize| vals.get(i % vals.len().max(1)).copied().unwrap_or(0) as u16;
    Ok([pick(0), pick(1), pick(2), pick(3)])
}

fn decode_uncompressed(t: &Tiff<'_>, ifd: &Ifd, width: usize, height: usize) -> Result<Vec<u16>> {
    let offs = t.uints(
        ifd.get(0x111)
            .ok_or_else(|| TicoError::Container("no StripOffsets".into()))?,
    )?;
    let rows_per_strip = tiff::first_uint(t, ifd, 0x116)?.unwrap_or(height as u64) as usize;
    if rows_per_strip == 0 || offs.len() != height.div_ceil(rows_per_strip.max(1)) {
        return Err(TicoError::Container(
            "strip count does not match RowsPerStrip".into(),
        ));
    }
    // Each sample is 2 bytes, so a frame larger than the file cannot be valid.
    if width.saturating_mul(height).saturating_mul(2) > t.data.len() {
        return Err(TicoError::Truncated {
            what: "DNG strips",
            offset: t.data.len(),
        });
    }
    let mut out = Vec::with_capacity(width * height);
    for (s, &off) in offs.iter().enumerate() {
        let rows = rows_per_strip.min(height - s * rows_per_strip);
        let n = rows * width;
        let at = off as usize;
        let bytes = t.data.get(at..at + 2 * n).ok_or(TicoError::Truncated {
            what: "DNG strip",
            offset: at,
        })?;
        out.extend(bytes.chunks_exact(2).map(|c| match t.order {
            tiff::ByteOrder::Little => u16::from_le_bytes([c[0], c[1]]),
            tiff::ByteOrder::Big => u16::from_be_bytes([c[0], c[1]]),
        }));
    }
    if out.len() != width * height {
        return Err(TicoError::Container("strip sample count mismatch".into()));
    }
    Ok(out)
}

fn decode_ljpeg_layout(t: &Tiff<'_>, ifd: &Ifd, width: usize, height: usize) -> Result<Vec<u16>> {
    let (offs, counts, tw, th) = if let Some(e) = ifd.get(0x144) {
        let counts = t.uints(
            ifd.get(0x145)
                .ok_or_else(|| TicoError::Container("no TileByteCounts".into()))?,
        )?;
        let tw = tiff::require_uint(t, ifd, 0x142)? as usize;
        let th = tiff::require_uint(t, ifd, 0x143)? as usize;
        (t.uints(e)?, counts, tw, th)
    } else {
        let offs = t.uints(
            ifd.get(0x111)
                .ok_or_else(|| TicoError::Container("no StripOffsets".into()))?,
        )?;
        let counts = t.uints(
            ifd.get(0x117)
                .ok_or_else(|| TicoError::Container("no StripByteCounts".into()))?,
        )?;
        let rps = tiff::first_uint(t, ifd, 0x116)?.unwrap_or(height as u64) as usize;
        (offs, counts, width, rps)
    };
    if tw == 0 || th == 0 {
        return Err(TicoError::Container("zero tile or strip size".into()));
    }
    // Lossless JPEG spends at least one bit per sample.
    let total = counts.iter().fold(0u64, |a, &c| a.saturating_add(c));
    if (width as u64).saturating_mul(height as u64) > total.saturating_mul(8) {
        return Err(TicoError::Truncated {
            what: "DNG tiles",
            offset: t.data.len(),
        });
    }
    // Tiles may overlap, so also bound the frame by the real input length.
    let samples = width
        .checked_mul(height)
        .filter(|&n| n <= t.data.len().saturating_mul(8));
    let tile_samples = tw
        .checked_mul(th)
        .filter(|&n| n <= t.data.len().saturating_mul(8));
    let (Some(samples), Some(tile_samples)) = (samples, tile_samples) else {
        return Err(TicoError::Container(
            "tile or frame larger than the file can encode".into(),
        ));
    };
    let across = width.div_ceil(tw);
    let down = height.div_ceil(th);
    if offs.len() < across * down || counts.len() < offs.len() {
        return Err(TicoError::Container("tile table too short".into()));
    }
    let tiles: Vec<&[u8]> = (0..across * down)
        .map(|i| {
            let (o, n) = (offs[i] as usize, counts[i] as usize);
            o.checked_add(n)
                .and_then(|end| t.data.get(o..end))
                .ok_or(TicoError::Truncated {
                    what: "DNG tile",
                    offset: o,
                })
        })
        .collect::<Result<_>>()?;
    // Decode one row of tiles per task straight into the output, so at most one decoded
    // tile per thread is alive at a time.
    let mut out = vec![0u16; samples];
    let results = crate::par_chunks_mut_collect(&mut out, th * width, |row, band| -> Result<()> {
        let rows = band.len() / width;
        for col in 0..across {
            let jpeg = decode_lossless_jpeg(tiles[row * across + col])?;
            if jpeg.samples.len() != tile_samples {
                return Err(TicoError::Jpeg(format!(
                    "tile holds {} samples, expected {tile_samples}",
                    jpeg.samples.len()
                )));
            }
            let x0 = col * tw;
            let w = tw.min(width - x0);
            for r in 0..rows {
                band[r * width + x0..][..w].copy_from_slice(&jpeg.samples[r * tw..][..w]);
            }
        }
        Ok(())
    });
    results.into_iter().collect::<Result<Vec<()>>>()?;
    Ok(out)
}

/// Canonical Huffman table expanded to a 16-bit direct lookup: entry = (code length, symbol).
struct Huffman {
    lut: Vec<(u8, u8)>,
}

impl Huffman {
    fn new(counts: &[u8; 16], symbols: &[u8]) -> Result<Self> {
        let mut lut = vec![(0u8, 0u8); 1 << 16];
        let mut code: u32 = 0;
        let mut k = 0usize;
        for (len_minus_1, &n) in counts.iter().enumerate() {
            let len = len_minus_1 as u32 + 1;
            for _ in 0..n {
                let sym = *symbols
                    .get(k)
                    .ok_or_else(|| TicoError::Jpeg("DHT symbol overflow".into()))?;
                if sym > 16 {
                    return Err(TicoError::Jpeg(format!("lossless DHT symbol {sym}")));
                }
                k += 1;
                if code >= 1 << len {
                    return Err(TicoError::Jpeg("over-subscribed Huffman table".into()));
                }
                let shift = 16 - len;
                let start = (code << shift) as usize;
                for e in &mut lut[start..start + (1 << shift)] {
                    *e = (len as u8, sym);
                }
                code += 1;
            }
            code <<= 1;
        }
        Ok(Self { lut })
    }
}

/// MSB-first bit reader over JPEG entropy data with `FF00` unstuffing; yields zeros past a marker.
struct JpegBits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    nbits: u32,
}

impl<'a> JpegBits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            acc: 0,
            nbits: 0,
        }
    }

    fn refill(&mut self) {
        while self.nbits <= 56 {
            let mut byte = 0u8;
            if let Some(&b) = self.data.get(self.pos) {
                if b == 0xFF {
                    // FF00 is a stuffed 0xFF; any other FFxx is a marker: feed zeros.
                    if self.data.get(self.pos + 1) == Some(&0x00) {
                        byte = 0xFF;
                        self.pos += 2;
                    }
                } else {
                    byte = b;
                    self.pos += 1;
                }
            }
            self.acc |= u64::from(byte) << (56 - self.nbits);
            self.nbits += 8;
        }
    }

    fn peek16(&mut self) -> u32 {
        self.refill();
        (self.acc >> 48) as u32
    }

    fn consume(&mut self, n: u32) {
        self.acc <<= n;
        self.nbits -= n;
    }

    fn bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        self.refill();
        let v = (self.acc >> (64 - n)) as u32;
        self.consume(n);
        v
    }

    /// Discards buffered bits and skips an `RSTn` marker if one is next.
    fn restart(&mut self) {
        self.acc = 0;
        self.nbits = 0;
        while self.pos + 1 < self.data.len() {
            if self.data[self.pos] == 0xFF && (0xD0..=0xD7).contains(&self.data[self.pos + 1]) {
                self.pos += 2;
                return;
            }
            self.pos += 1;
        }
    }
}

/// A decoded lossless JPEG frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LosslessJpeg {
    /// Samples per row: frame width times interleaved components.
    pub width: usize,
    /// Frame height in rows.
    pub height: usize,
    /// Samples in raster order with components interleaved, which is the CFA raster order
    /// DNG uses.
    pub samples: Vec<u16>,
}

/// Decodes one lossless JPEG (SOF3) stream.
///
/// # Errors
///
/// [`TicoError::Jpeg`] for a malformed or unsupported stream; [`TicoError::Truncated`]
/// when it ends early.
pub fn decode_lossless_jpeg(data: &[u8]) -> Result<LosslessJpeg> {
    if data.get(..2) != Some(&[0xFF, 0xD8]) {
        return Err(TicoError::Jpeg("missing SOI".into()));
    }
    let mut tables: [Option<Huffman>; 4] = [None, None, None, None];
    let mut frame: Option<(u32, usize, usize, usize)> = None; // precision, Y, X, Nf
    let mut restart_interval = 0usize;
    let mut pos = 2;
    loop {
        if data.get(pos) != Some(&0xFF) {
            return Err(TicoError::Jpeg(format!("expected marker at {pos}")));
        }
        let marker = *data.get(pos + 1).ok_or(TicoError::Truncated {
            what: "marker",
            offset: pos,
        })?;
        pos += 2;
        if marker == 0xD9 {
            return Err(TicoError::Jpeg("EOI before SOS".into()));
        }
        let len = u16::from_be_bytes([
            *data.get(pos).ok_or(TicoError::Truncated {
                what: "segment",
                offset: pos,
            })?,
            *data.get(pos + 1).ok_or(TicoError::Truncated {
                what: "segment",
                offset: pos,
            })?,
        ]) as usize;
        let seg = data.get(pos + 2..pos + len).ok_or(TicoError::Truncated {
            what: "segment",
            offset: pos,
        })?;
        pos += len;
        match marker {
            0xC4 => {
                let mut s = seg;
                while !s.is_empty() {
                    let id = (s[0] & 3) as usize;
                    let counts: [u8; 16] = s
                        .get(1..17)
                        .ok_or_else(|| TicoError::Jpeg("short DHT".into()))?
                        .try_into()
                        .map_err(|_| TicoError::Jpeg("short DHT".into()))?;
                    let n: usize = counts.iter().map(|&c| c as usize).sum();
                    let syms = s
                        .get(17..17 + n)
                        .ok_or_else(|| TicoError::Jpeg("short DHT".into()))?;
                    tables[id] = Some(Huffman::new(&counts, syms)?);
                    s = &s[17 + n..];
                }
            }
            0xC3 => {
                if seg.len() < 6 {
                    return Err(TicoError::Jpeg("short SOF3".into()));
                }
                let p = u32::from(seg[0]);
                if !(2..=16).contains(&p) {
                    return Err(TicoError::Jpeg(format!("SOF3 precision {p}")));
                }
                let y = u16::from_be_bytes([seg[1], seg[2]]) as usize;
                let x = u16::from_be_bytes([seg[3], seg[4]]) as usize;
                frame = Some((p, y, x, seg[5] as usize));
            }
            0xC0..=0xC2 | 0xC5..=0xCF if marker != 0xC8 && marker != 0xCC => {
                return Err(TicoError::Unsupported(format!(
                    "JPEG SOF{:X}",
                    marker - 0xC0
                )));
            }
            0xDD => {
                let b = seg
                    .get(..2)
                    .ok_or_else(|| TicoError::Jpeg("short DRI".into()))?;
                restart_interval = u16::from_be_bytes([b[0], b[1]]) as usize;
            }
            0xDA => {
                let (precision, rows, cols, nf) =
                    frame.ok_or_else(|| TicoError::Jpeg("SOS before SOF3".into()))?;
                let ns = seg.first().map_or(0, |&n| n as usize);
                if ns == 0 || ns != nf || seg.len() < 1 + 2 * ns + 3 {
                    return Err(TicoError::Jpeg(
                        "non-interleaved scans are not supported".into(),
                    ));
                }
                let comp_tables: Vec<&Huffman> = (0..ns)
                    .map(|c| {
                        tables[(seg[2 + 2 * c] >> 4) as usize & 3]
                            .as_ref()
                            .ok_or_else(|| TicoError::Jpeg("missing Huffman table".into()))
                    })
                    .collect::<Result<_>>()?;
                let predictor = seg[1 + 2 * ns];
                let point_transform = u32::from(seg[3 + 2 * ns] & 15);
                let samples = decode_scan(
                    &data[pos..],
                    &comp_tables,
                    (rows, cols, nf),
                    precision,
                    predictor,
                    point_transform,
                    restart_interval,
                )?;
                return Ok(LosslessJpeg {
                    width: cols * nf,
                    height: rows,
                    samples,
                });
            }
            _ => {}
        }
    }
}

fn decode_scan(
    data: &[u8],
    tables: &[&Huffman],
    (rows, cols, nf): (usize, usize, usize),
    precision: u32,
    predictor: u8,
    pt: u32,
    restart_interval: usize,
) -> Result<Vec<u16>> {
    if !(1..=7).contains(&predictor) {
        return Err(TicoError::Jpeg(format!("predictor {predictor}")));
    }
    if pt >= precision {
        return Err(TicoError::Jpeg(format!(
            "point transform {pt} for precision {precision}"
        )));
    }
    let stride = cols * nf;
    // Every sample costs at least one bit, which bounds the allocation by the input size.
    if rows.saturating_mul(stride) > data.len().saturating_mul(8) {
        return Err(TicoError::Truncated {
            what: "lossless JPEG scan",
            offset: data.len(),
        });
    }
    let mut out = vec![0u16; rows * stride];
    let mut bits = JpegBits::new(data);
    let initial = 1i32 << (precision - pt - 1);
    let mut mcus_left = restart_interval;
    let mut fresh_row = true; // first row after start/restart predicts from the left only
    for y in 0..rows {
        if restart_interval > 0 && y > 0 && mcus_left == 0 {
            bits.restart();
            mcus_left = restart_interval;
            fresh_row = true;
        }
        for x in 0..cols {
            for c in 0..nf {
                let i = y * stride + x * nf + c;
                let pred = if x == 0 {
                    if fresh_row {
                        initial
                    } else {
                        i32::from(out[i - stride])
                    }
                } else if fresh_row {
                    i32::from(out[i - nf])
                } else {
                    let ra = i32::from(out[i - nf]);
                    let rb = i32::from(out[i - stride]);
                    let rc = i32::from(out[i - stride - nf]);
                    match predictor {
                        1 => ra,
                        2 => rb,
                        3 => rc,
                        4 => ra + rb - rc,
                        5 => ra + ((rb - rc) >> 1),
                        6 => rb + ((ra - rc) >> 1),
                        _ => (ra + rb) >> 1,
                    }
                };
                let (len, ssss) = tables[c].lut[bits.peek16() as usize];
                if len == 0 {
                    return Err(TicoError::Jpeg("invalid Huffman code".into()));
                }
                bits.consume(u32::from(len));
                let diff = match ssss {
                    0 => 0,
                    16 => 32768,
                    s => {
                        let s = u32::from(s);
                        let v = bits.bits(s) as i32;
                        if v < 1 << (s - 1) {
                            v - (1 << s) + 1
                        } else {
                            v
                        }
                    }
                };
                out[i] = (pred + diff) as u16;
            }
        }
        fresh_row = false;
        if restart_interval > 0 {
            // DNG writers use one restart interval per row of MCUs when they use any.
            mcus_left = mcus_left.saturating_sub(cols);
        }
    }
    Ok(out)
}

/// Writes a minimal, valid uncompressed 16-bit CFA DNG (DNG 1.4) for `raw`.
///
/// Besides the mosaic and its levels, the DNG carries `Orientation`, `DefaultCropOrigin` /
/// `DefaultCropSize` (when [`DecodedRaw::crop`] is known), `ColorMatrix1` with D65 as
/// `CalibrationIlluminant1`, and `AsShotNeutral` and `BaselineExposure` (when known; see
/// [`DecodedRaw::baseline_exposure`]).
///
/// Models without a built-in colour profile get an identity `ColorMatrix1` (DNG requires
/// one), so their colours will be wrong in raw processors; see
/// [`ColorInfo::color_matrix`](crate::ColorInfo::color_matrix).
///
/// # Errors
///
/// [`TicoError::Container`] when `raw.image.pixels` does not hold `width * height`
/// samples; [`TicoError::Unsupported`] when the image or file would exceed the 4 GiB
/// limit of classic TIFF.
pub fn encode_uncompressed_dng(raw: &DecodedRaw) -> Result<Vec<u8>> {
    let img = &raw.image;
    let too_big = || TicoError::Unsupported("image too large for a classic TIFF/DNG".into());
    if img.pixels.len() != img.width * img.height {
        return Err(TicoError::Container(format!(
            "{} pixels for a {}x{} image",
            img.pixels.len(),
            img.width,
            img.height
        )));
    }
    let width = u32::try_from(img.width).map_err(|_| too_big())?;
    let height = u32::try_from(img.height).map_err(|_| too_big())?;
    let strip_bytes = u32::try_from(img.pixels.len() * 2).map_err(|_| too_big())?;
    let c = &raw.color;
    let mut w = DngWriter::default();
    w.long(0xFE, &[0]);
    w.long(0x100, &[width]);
    w.long(0x101, &[height]);
    w.short(0x102, &[16]);
    w.short(0x103, &[1]);
    w.short(0x106, &[32803]);
    w.ascii(0x10F, &raw.make);
    w.ascii(0x110, &raw.model);
    w.strip_offset(0x111);
    w.short(0x112, &[raw.orientation]);
    w.short(0x115, &[1]);
    w.long(0x116, &[height]);
    w.long(0x117, &[strip_bytes]);
    w.short(0x11C, &[1]);
    w.ascii(0x131, concat!("ticoraw ", env!("CARGO_PKG_VERSION")));
    w.short(0x828D, &[2, 2]);
    w.bytes(0x828E, &c.cfa_pattern);
    w.bytes(0xC612, &[1, 4, 0, 0]);
    w.bytes(0xC613, &[1, 1, 0, 0]);
    let unique = crate::adobe_camera_name(&raw.model)
        .map_or_else(|| format!("{} {}", raw.make, raw.model), str::to_owned);
    w.ascii(0xC614, &unique);
    w.short(0xC619, &[2, 2]);
    w.short(0xC61A, &c.black_level);
    w.long(0xC61D, &[u32::from(c.white_level)]);
    if let Some(crop) = raw.crop {
        let v = |n: usize| u32::try_from(n).map_err(|_| too_big());
        w.long(0xC61F, &[v(crop.x)?, v(crop.y)?]);
        w.long(0xC620, &[v(crop.width)?, v(crop.height)?]);
    }
    if let Some(m) = &c.color_matrix {
        w.srational(0xC621, m);
        w.short(0xC65A, &[21]); // D65
    } else {
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
        w.srational(0xC621, &identity);
    }
    if let Some(n) = &c.as_shot_neutral {
        w.rational(0xC628, n);
    }
    if let Some(ev) = raw.baseline_exposure {
        w.srational(0xC62A, &[ev]);
    }
    w.finish(&img.pixels)
}

/// Tag value: inline-or-blob bytes, or the offset of the pixel strip (written last).
enum Value {
    Bytes(Vec<u8>),
    StripOffset,
}

#[derive(Default)]
struct DngWriter {
    /// (tag, type, count, value)
    entries: Vec<(u16, u16, u32, Value)>,
}

impl DngWriter {
    fn push(&mut self, tag: u16, typ: u16, count: usize, bytes: Vec<u8>) {
        self.entries
            .push((tag, typ, count as u32, Value::Bytes(bytes)));
    }
    fn short(&mut self, tag: u16, v: &[u16]) {
        self.push(
            tag,
            3,
            v.len(),
            v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        );
    }
    fn long(&mut self, tag: u16, v: &[u32]) {
        self.push(
            tag,
            4,
            v.len(),
            v.iter().flat_map(|x| x.to_le_bytes()).collect(),
        );
    }
    fn strip_offset(&mut self, tag: u16) {
        self.entries.push((tag, 4, 1, Value::StripOffset));
    }
    fn bytes(&mut self, tag: u16, v: &[u8]) {
        self.push(tag, 1, v.len(), v.to_vec());
    }
    fn ascii(&mut self, tag: u16, s: &str) {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        self.push(tag, 2, b.len(), b);
    }
    fn rational(&mut self, tag: u16, v: &[(u32, u32)]) {
        let b = v
            .iter()
            .flat_map(|&(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat())
            .collect();
        self.push(tag, 5, v.len(), b);
    }
    fn srational(&mut self, tag: u16, v: &[(i32, i32)]) {
        let b = v
            .iter()
            .flat_map(|&(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat())
            .collect();
        self.push(tag, 10, v.len(), b);
    }

    /// Lays out header, IFD, out-of-line values and finally the pixel strip in one buffer.
    fn finish(mut self, pixels: &[u16]) -> Result<Vec<u8>> {
        self.entries.sort_by_key(|e| e.0);
        let ifd_size = 2 + 12 * self.entries.len() + 4;
        let blobs_len: usize = self
            .entries
            .iter()
            .map(|(_, _, _, v)| match v {
                Value::Bytes(b) if b.len() > 4 => b.len().next_multiple_of(2),
                _ => 0,
            })
            .sum();
        let strip_at = 8 + ifd_size + blobs_len;
        let total = strip_at + 2 * pixels.len();
        if u32::try_from(total).is_err() {
            return Err(TicoError::Unsupported(
                "image too large for a classic TIFF/DNG".into(),
            ));
        }
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(b"II*\0\x08\0\0\0");
        out.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        let mut blob_at = 8 + ifd_size;
        for (tag, typ, count, val) in &self.entries {
            out.extend_from_slice(&tag.to_le_bytes());
            out.extend_from_slice(&typ.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            match val {
                Value::StripOffset => out.extend_from_slice(&(strip_at as u32).to_le_bytes()),
                Value::Bytes(b) if b.len() <= 4 => {
                    let mut inline = [0u8; 4];
                    inline[..b.len()].copy_from_slice(b);
                    out.extend_from_slice(&inline);
                }
                Value::Bytes(b) => {
                    out.extend_from_slice(&(blob_at as u32).to_le_bytes());
                    blob_at += b.len().next_multiple_of(2);
                }
            }
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        for (_, _, _, val) in &self.entries {
            match val {
                Value::Bytes(b) if b.len() > 4 => {
                    out.extend_from_slice(b);
                    out.resize(out.len().next_multiple_of(2), 0); // word-align values
                }
                _ => {}
            }
        }
        debug_assert_eq!(out.len(), strip_at);
        for p in pixels {
            out.extend_from_slice(&p.to_le_bytes());
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ColorInfo;

    fn sample_raw() -> DecodedRaw {
        let (w, h) = (6, 4);
        DecodedRaw {
            make: "NIKON CORPORATION".into(),
            model: "TEST".into(),
            image: CfaImage {
                width: w,
                height: h,
                pixels: (0..(w * h) as u16).map(|v| v * 997).collect(),
            },
            color: ColorInfo {
                cfa_pattern: [0, 1, 1, 2],
                black_level: [1008; 4],
                white_level: 15892,
                color_matrix: None,
                as_shot_neutral: Some([(512, 924), (1, 1), (512, 717)]),
            },
            orientation: 8,
            baseline_exposure: Some((125, 100)),
            crop: Some(crate::Crop {
                x: 1,
                y: 1,
                width: 4,
                height: 2,
            }),
        }
    }

    #[test]
    fn uncompressed_round_trip() {
        let raw = sample_raw();
        let bytes = encode_uncompressed_dng(&raw).unwrap();
        let back = decode_dng_cfa(&bytes).unwrap();
        let t = Tiff::parse(&bytes, 0).unwrap();
        let ifd = t.ifd(t.first_ifd).unwrap();
        assert_eq!(tiff::first_uint(&t, &ifd, 0x112).unwrap(), Some(8));
        assert_eq!(t.uints(ifd.get(0xC620).unwrap()).unwrap(), vec![4, 2]);
        assert_eq!(
            t.rationals(ifd.get(0xC62A).unwrap()).unwrap(),
            vec![(125, 100)]
        );
        let mut bad = raw.clone();
        bad.image.pixels.pop();
        assert!(encode_uncompressed_dng(&bad).is_err());
        assert_eq!(back.image, raw.image);
        assert_eq!(back.black_level, [1008; 4]);
        assert_eq!(back.white_level, 15892);
        assert_eq!(back.cfa_pattern, [0, 1, 1, 2]);
    }

    #[test]
    fn huffman_rejects_oversubscribed() {
        let mut counts = [0u8; 16];
        counts[0] = 3; // three 1-bit codes cannot exist
        assert!(Huffman::new(&counts, &[0, 1, 2]).is_err());
    }

    #[test]
    fn decodes_tiny_lossless_jpeg() {
        // Hand-built 2x2, 1 component, 8-bit, predictor 1; Huffman: one 1-bit code for SSSS=0
        // plus a 2-bit code for SSSS=1. Samples: 128 (diff 0 from 2^7), 129, 128, 129.
        // Bits: [0] [10 1] [0... first col uses above=128 => diff 0] [10 1]
        let mut j = vec![0xFF, 0xD8];
        j.extend_from_slice(&[0xFF, 0xC4, 0x00, 0x15, 0x00]);
        let mut counts = [0u8; 16];
        counts[0] = 1;
        counts[1] = 1;
        j.extend_from_slice(&counts);
        j.extend_from_slice(&[0, 1]);
        j.extend_from_slice(&[0xFF, 0xC3, 0x00, 0x0B, 8, 0, 2, 0, 2, 1, 1, 0x11, 0]);
        j.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x08, 1, 1, 0x00, 1, 0, 0]);
        // bitstring: 0 101 0 101 -> 0101 0101 = 0x55
        j.push(0x55);
        j.extend_from_slice(&[0xFF, 0xD9]);
        let d = decode_lossless_jpeg(&j).unwrap();
        assert_eq!((d.width, d.height), (2, 2));
        assert_eq!(d.samples, vec![128, 129, 128, 129]);
    }
}
