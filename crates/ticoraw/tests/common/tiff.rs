//! Test-only TIFF writer plus NEF and DNG builders on top of it.

/// TIFF byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Little,
    Big,
}

/// A typed IFD value.
#[derive(Debug, Clone)]
pub enum Value {
    Byte(Vec<u8>),
    Ascii(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<(u32, u32)>),
    Undefined(Vec<u8>),
}

impl Value {
    fn encode(&self, order: Order) -> (u16, u32, Vec<u8>) {
        let u16b = |v: u16| match order {
            Order::Little => v.to_le_bytes(),
            Order::Big => v.to_be_bytes(),
        };
        let u32b = |v: u32| match order {
            Order::Little => v.to_le_bytes(),
            Order::Big => v.to_be_bytes(),
        };
        match self {
            Self::Byte(b) => (1, b.len() as u32, b.clone()),
            Self::Ascii(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                (2, b.len() as u32, b)
            }
            Self::Short(v) => (3, v.len() as u32, v.iter().flat_map(|&x| u16b(x)).collect()),
            Self::Long(v) => (4, v.len() as u32, v.iter().flat_map(|&x| u32b(x)).collect()),
            Self::Rational(v) => (
                5,
                v.len() as u32,
                v.iter()
                    .flat_map(|&(n, d)| [u32b(n), u32b(d)].concat())
                    .collect(),
            ),
            Self::Undefined(b) => (7, b.len() as u32, b.clone()),
        }
    }
}

/// Appends IFDs and blobs to a TIFF buffer; offsets are relative to the TIFF header.
#[derive(Debug, Clone)]
pub struct TiffWriter {
    pub out: Vec<u8>,
    order: Order,
}

impl TiffWriter {
    #[must_use]
    pub fn new(order: Order) -> Self {
        let mut w = Self {
            out: match order {
                Order::Little => b"II*\0".to_vec(),
                Order::Big => b"MM\0*".to_vec(),
            },
            order,
        };
        w.out.extend_from_slice(&[0; 4]);
        w
    }

    fn u32b(&self, v: u32) -> [u8; 4] {
        match self.order {
            Order::Little => v.to_le_bytes(),
            Order::Big => v.to_be_bytes(),
        }
    }

    fn align(&mut self) {
        if self.out.len() % 2 == 1 {
            self.out.push(0);
        }
    }

    /// Appends raw bytes (word-aligned) and returns their offset.
    pub fn blob(&mut self, bytes: &[u8]) -> u32 {
        self.align();
        let at = self.out.len() as u32;
        self.out.extend_from_slice(bytes);
        at
    }

    /// Appends an IFD whose out-of-line values follow it; returns its offset.
    pub fn ifd(&mut self, entries: &[(u16, Value)], next: u32) -> u32 {
        self.align();
        let at = self.out.len();
        let mut sorted: Vec<_> = entries.iter().collect();
        sorted.sort_by_key(|e| e.0);
        let mut blob_at = at + 2 + 12 * sorted.len() + 4;
        let mut ifd = match self.order {
            Order::Little => (sorted.len() as u16).to_le_bytes(),
            Order::Big => (sorted.len() as u16).to_be_bytes(),
        }
        .to_vec();
        let mut blobs = Vec::new();
        for (tag, value) in sorted {
            let (typ, count, mut bytes) = value.encode(self.order);
            let (t, ty) = match self.order {
                Order::Little => (tag.to_le_bytes(), typ.to_le_bytes()),
                Order::Big => (tag.to_be_bytes(), typ.to_be_bytes()),
            };
            ifd.extend_from_slice(&t);
            ifd.extend_from_slice(&ty);
            ifd.extend_from_slice(&self.u32b(count));
            if bytes.len() <= 4 {
                bytes.resize(4, 0);
                ifd.extend_from_slice(&bytes);
            } else {
                ifd.extend_from_slice(&self.u32b(blob_at as u32));
                blob_at += bytes.len() + bytes.len() % 2;
                if bytes.len() % 2 == 1 {
                    bytes.push(0);
                }
                blobs.extend_from_slice(&bytes);
            }
        }
        ifd.extend_from_slice(&self.u32b(next));
        self.out.extend_from_slice(&ifd);
        self.out.extend_from_slice(&blobs);
        at as u32
    }

    pub fn set_first_ifd(&mut self, offset: u32) {
        let b = self.u32b(offset);
        self.out[4..8].copy_from_slice(&b);
    }
}

/// What a synthetic NEF carries around its raw strip.
#[derive(Debug, Clone)]
pub struct NefSpec {
    pub order: Order,
    pub make: String,
    pub model: String,
    pub width: u32,
    pub height: u32,
    /// MakerNote 0x0051 compression code (14 = HE*, 13 = HE).
    pub compression_code: Option<u16>,
    pub black_level: Option<[u16; 4]>,
    /// MakerNote 0x000C red and blue white-balance multipliers.
    pub wb: Option<[(u32, u32); 2]>,
    pub cfa_pattern: Option<[u8; 4]>,
    /// Include an EXIF IFD with a Nikon MakerNote at all.
    pub maker_note: bool,
}

impl NefSpec {
    #[must_use]
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            order: Order::Big,
            make: "NIKON CORPORATION".into(),
            model: "NIKON Z6_3".into(),
            width: width as u32,
            height: height as u32,
            compression_code: Some(14),
            black_level: Some([1008, 1009, 1010, 1011]),
            wb: Some([(924, 512), (717, 512)]),
            cfa_pattern: Some([0, 1, 1, 2]),
            maker_note: true,
        }
    }
}

/// Nikon type-3 MakerNote: "Nikon\0", version, then an embedded big-endian TIFF.
fn maker_note(spec: &NefSpec) -> Vec<u8> {
    let mut entries = Vec::new();
    if let Some([(rn, rd), (bn, bd)]) = spec.wb {
        entries.push((
            0x000C,
            Value::Rational(vec![(rn, rd), (bn, bd), (1, 1), (1, 1)]),
        ));
    }
    if let Some(b) = spec.black_level {
        entries.push((0x003D, Value::Short(b.to_vec())));
    }
    if let Some(code) = spec.compression_code {
        let mut v = vec![0u8; 16];
        v[..4].copy_from_slice(b"0100");
        v[10..12].copy_from_slice(&code.to_le_bytes());
        entries.push((0x0051, Value::Undefined(v)));
    }
    let mut t = TiffWriter::new(Order::Big);
    let ifd = t.ifd(&entries, 0);
    t.set_first_ifd(ifd);
    let mut mn = b"Nikon\0\x02\x11\0\0".to_vec();
    mn.extend_from_slice(&t.out);
    mn
}

/// Builds a minimal NEF: a thumbnail IFD0 (Make, Model, SubIFDs, ExifIFD), a CFA SubIFD
/// whose single strip is `strip`, and an EXIF IFD holding a Nikon MakerNote.
#[must_use]
pub fn build_nef(spec: &NefSpec, strip: &[u8]) -> Vec<u8> {
    let mut t = TiffWriter::new(spec.order);
    let strip_at = t.blob(strip);
    let mut raw = vec![
        (0x00FE, Value::Long(vec![0])),
        (0x0100, Value::Long(vec![spec.width])),
        (0x0101, Value::Long(vec![spec.height])),
        (0x0102, Value::Short(vec![14])),
        (0x0103, Value::Short(vec![34713])),
        (0x0106, Value::Short(vec![32803])),
        (0x0111, Value::Long(vec![strip_at])),
        (0x0115, Value::Short(vec![1])),
        (0x0116, Value::Long(vec![spec.height])),
        (0x0117, Value::Long(vec![strip.len() as u32])),
        (0x828D, Value::Short(vec![2, 2])),
    ];
    if let Some(p) = spec.cfa_pattern {
        raw.push((0x828E, Value::Byte(p.to_vec())));
    }
    let raw_ifd = t.ifd(&raw, 0);
    let mut ifd0 = vec![
        (0x00FE, Value::Long(vec![1])),
        (0x010F, Value::Ascii(spec.make.clone())),
        (0x0110, Value::Ascii(spec.model.clone())),
        (0x014A, Value::Long(vec![raw_ifd])),
    ];
    if spec.maker_note {
        let exif = t.ifd(&[(0x927C, Value::Undefined(maker_note(spec)))], 0);
        ifd0.push((0x8769, Value::Long(vec![exif])));
    }
    let first = t.ifd(&ifd0, 0);
    t.set_first_ifd(first);
    t.out
}

/// How a synthetic DNG stores its CFA image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DngLayout {
    /// Lossless JPEG tiles of `(tile_width, tile_height)` (tile width even).
    Tiles(usize, usize),
    /// Lossless JPEG strips of this many rows, one stream per strip.
    Strips(usize),
}

/// Builds a compression-7 (lossless JPEG) CFA DNG with Adobe's two-component tiles.
#[must_use]
pub fn build_ljpeg_dng(
    pixels: &[u16],
    width: usize,
    height: usize,
    layout: DngLayout,
    black_level: [u32; 4],
    white_level: u32,
) -> Vec<u8> {
    let (tw, th) = match layout {
        DngLayout::Tiles(tw, th) => (tw, th),
        DngLayout::Strips(rows) => (width, rows),
    };
    assert_eq!(tw % 2, 0);
    let mut t = TiffWriter::new(Order::Little);
    let (mut offsets, mut counts) = (Vec::new(), Vec::new());
    for ty in 0..height.div_ceil(th) {
        for tx in 0..width.div_ceil(tw) {
            let mut tile = vec![0u16; tw * th];
            for r in 0..th {
                for c in 0..tw {
                    // Edge tiles replicate the last valid sample, as Adobe does.
                    let (y, x) = ((ty * th + r).min(height - 1), (tx * tw + c).min(width - 1));
                    tile[r * tw + c] = pixels[y * width + x];
                }
            }
            let stream = super::ljpeg::encode(&tile, th, tw / 2, 2, 16, 1);
            offsets.push(t.blob(&stream));
            counts.push(stream.len() as u32);
        }
    }
    let mut entries = vec![
        (0x00FE, Value::Long(vec![0])),
        (0x0100, Value::Long(vec![width as u32])),
        (0x0101, Value::Long(vec![height as u32])),
        (0x0102, Value::Short(vec![16])),
        (0x0103, Value::Short(vec![7])),
        (0x0106, Value::Short(vec![32803])),
        (0x010F, Value::Ascii("NIKON CORPORATION".into())),
        (0x828D, Value::Short(vec![2, 2])),
        (0x828E, Value::Byte(vec![0, 1, 1, 2])),
        (0xC612, Value::Byte(vec![1, 4, 0, 0])),
        (0xC619, Value::Short(vec![2, 2])),
        (
            0xC61A,
            Value::Rational(black_level.iter().map(|&b| (4 * b + 1, 4)).collect()),
        ),
        (0xC61D, Value::Long(vec![white_level])),
    ];
    match layout {
        DngLayout::Tiles(..) => entries.extend([
            (0x0142, Value::Long(vec![tw as u32])),
            (0x0143, Value::Long(vec![th as u32])),
            (0x0144, Value::Long(offsets)),
            (0x0145, Value::Long(counts)),
        ]),
        DngLayout::Strips(rows) => entries.extend([
            (0x0111, Value::Long(offsets)),
            (0x0116, Value::Long(vec![rows as u32])),
            (0x0117, Value::Long(counts)),
        ]),
    }
    let ifd = t.ifd(&entries, 0);
    t.set_first_ifd(ifd);
    t.out
}
