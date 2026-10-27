//! A small classic-TIFF writer: a list of IFDs that may point at each other (SubIFDs, EXIF,
//! GPS, Interoperability) plus image data chunks, laid out in one pass once every size is
//! known.
//!
//! Layout: header, then every IFD followed by its out-of-line values, then the data chunks.
//! Values and chunks start on even offsets, as TIFF requires.

use ticoraw::tiff::ByteOrder;

/// One IFD entry value.
#[derive(Debug, Clone)]
pub(crate) enum Value {
    Byte(Vec<u8>),
    Ascii(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<(u32, u32)>),
    SRational(Vec<(i32, i32)>),
    /// A value copied from another TIFF of the same byte order: field type, count and
    /// value bytes exactly as stored.
    Raw {
        typ: u16,
        count: u32,
        bytes: Vec<u8>,
    },
    /// LONG offsets of these IFDs (by index in [`Tiff::ifds`]).
    Ifds(Vec<usize>),
    /// LONG offsets of these data chunks (by index in [`Tiff::chunks`]).
    Chunks(Vec<usize>),
}

/// An IFD under construction. When the file is laid out, tags are sorted and only one
/// entry per tag is kept: the first [`Ifd::set`] one, else the first [`Ifd::carry`] one.
#[derive(Debug, Clone, Default)]
pub(crate) struct Ifd {
    /// (tag, carried, value)
    entries: Vec<(u16, bool, Value)>,
}

impl Ifd {
    /// Sets a tag the writer derives itself; it beats any carried value of the same tag.
    pub(crate) fn set(&mut self, tag: u16, value: Value) {
        self.entries.push((tag, false, value));
    }

    /// Adds a tag carried over from another file, used only when nothing set it.
    pub(crate) fn carry(&mut self, tag: u16, value: Value) {
        self.entries.push((tag, true, value));
    }

    fn sorted(&self) -> Vec<(u16, &Value)> {
        let mut e: Vec<_> = self.entries.iter().collect();
        e.sort_by_key(|(tag, carried, _)| (*tag, *carried));
        e.dedup_by_key(|(tag, _, _)| *tag);
        e.into_iter().map(|(tag, _, v)| (*tag, v)).collect()
    }
}

/// A TIFF file under construction. IFD 0 is the first IFD of the file.
#[derive(Debug)]
pub(crate) struct Tiff {
    pub(crate) order: ByteOrder,
    pub(crate) ifds: Vec<Ifd>,
    pub(crate) chunks: Vec<Vec<u8>>,
}

/// The file would not fit the 32-bit offsets of classic TIFF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TooLarge;

impl Tiff {
    pub(crate) fn new(order: ByteOrder) -> Self {
        Self {
            order,
            ifds: Vec::new(),
            chunks: Vec::new(),
        }
    }

    /// Adds an IFD and returns its index.
    pub(crate) fn add_ifd(&mut self, ifd: Ifd) -> usize {
        self.ifds.push(ifd);
        self.ifds.len() - 1
    }

    /// Adds a data chunk and returns its index.
    pub(crate) fn add_chunk(&mut self, data: Vec<u8>) -> usize {
        self.chunks.push(data);
        self.chunks.len() - 1
    }

    fn u16b(&self, v: u16) -> [u8; 2] {
        match self.order {
            ByteOrder::Little => v.to_le_bytes(),
            ByteOrder::Big => v.to_be_bytes(),
        }
    }

    fn u32b(&self, v: u32) -> [u8; 4] {
        match self.order {
            ByteOrder::Little => v.to_le_bytes(),
            ByteOrder::Big => v.to_be_bytes(),
        }
    }

    /// Field type, count and value bytes; pointer values get their final offsets.
    fn encode(&self, v: &Value, ifd_at: &[u32], chunk_at: &[u32]) -> (u16, u32, Vec<u8>) {
        let n = |len: usize| len as u32;
        match v {
            Value::Byte(b) => (1, n(b.len()), b.clone()),
            Value::Ascii(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                (2, n(b.len()), b)
            }
            Value::Short(v) => (
                3,
                n(v.len()),
                v.iter().flat_map(|&x| self.u16b(x)).collect(),
            ),
            Value::Long(v) => (
                4,
                n(v.len()),
                v.iter().flat_map(|&x| self.u32b(x)).collect(),
            ),
            Value::Rational(v) => (
                5,
                n(v.len()),
                v.iter()
                    .flat_map(|&(a, b)| [self.u32b(a), self.u32b(b)].concat())
                    .collect(),
            ),
            Value::SRational(v) => (
                10,
                n(v.len()),
                v.iter()
                    .flat_map(|&(a, b)| [self.u32b(a as u32), self.u32b(b as u32)].concat())
                    .collect(),
            ),
            Value::Raw { typ, count, bytes } => (*typ, *count, bytes.clone()),
            Value::Ifds(ids) => (
                4,
                n(ids.len()),
                ids.iter().flat_map(|&i| self.u32b(ifd_at[i])).collect(),
            ),
            Value::Chunks(ids) => (
                4,
                n(ids.len()),
                ids.iter().flat_map(|&i| self.u32b(chunk_at[i])).collect(),
            ),
        }
    }

    /// Value size in bytes, known before any offset is.
    fn value_len(v: &Value) -> usize {
        match v {
            Value::Byte(b) => b.len(),
            Value::Ascii(s) => s.len() + 1,
            Value::Short(v) => 2 * v.len(),
            Value::Long(v) => 4 * v.len(),
            Value::Rational(v) => 8 * v.len(),
            Value::SRational(v) => 8 * v.len(),
            Value::Raw { bytes, .. } => bytes.len(),
            Value::Ifds(v) | Value::Chunks(v) => 4 * v.len(),
        }
    }

    /// Lays out and serialises the file.
    pub(crate) fn finish(self) -> Result<Vec<u8>, TooLarge> {
        let sorted: Vec<Vec<(u16, &Value)>> = self.ifds.iter().map(Ifd::sorted).collect();
        let mut at = 8usize;
        let mut ifd_at = Vec::with_capacity(sorted.len());
        for entries in &sorted {
            ifd_at.push(at);
            at += 2 + 12 * entries.len() + 4;
            for (_, v) in entries {
                let len = Self::value_len(v);
                if len > 4 {
                    at += len.next_multiple_of(2);
                }
            }
        }
        let mut chunk_at = Vec::with_capacity(self.chunks.len());
        for c in &self.chunks {
            chunk_at.push(at);
            at += c.len().next_multiple_of(2);
        }
        let total = at;
        let to32 = |v: &[usize]| -> Result<Vec<u32>, TooLarge> {
            v.iter()
                .map(|&o| u32::try_from(o).map_err(|_| TooLarge))
                .collect()
        };
        if u32::try_from(total).is_err() || sorted.iter().any(|e| e.len() > usize::from(u16::MAX)) {
            return Err(TooLarge);
        }
        let (ifd_at, chunk_at) = (to32(&ifd_at)?, to32(&chunk_at)?);

        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(match self.order {
            ByteOrder::Little => b"II*\0",
            ByteOrder::Big => b"MM\0*",
        });
        out.extend_from_slice(&self.u32b(if sorted.is_empty() { 0 } else { 8 }));
        for (i, entries) in sorted.iter().enumerate() {
            debug_assert_eq!(out.len(), ifd_at[i] as usize);
            let encoded: Vec<_> = entries
                .iter()
                .map(|&(tag, v)| (tag, self.encode(v, &ifd_at, &chunk_at)))
                .collect();
            out.extend_from_slice(&self.u16b(encoded.len() as u16));
            let mut blob_at = ifd_at[i] as usize + 2 + 12 * encoded.len() + 4;
            for (tag, (typ, count, bytes)) in &encoded {
                out.extend_from_slice(&self.u16b(*tag));
                out.extend_from_slice(&self.u16b(*typ));
                out.extend_from_slice(&self.u32b(*count));
                if bytes.len() <= 4 {
                    let mut inline = [0u8; 4];
                    inline[..bytes.len()].copy_from_slice(bytes);
                    out.extend_from_slice(&inline);
                } else {
                    out.extend_from_slice(&self.u32b(blob_at as u32));
                    blob_at += bytes.len().next_multiple_of(2);
                }
            }
            out.extend_from_slice(&[0; 4]); // no next IFD: every IFD hangs off IFD0
            for (_, (_, _, bytes)) in &encoded {
                if bytes.len() > 4 {
                    out.extend_from_slice(bytes);
                    out.resize(out.len().next_multiple_of(2), 0);
                }
            }
        }
        for c in &self.chunks {
            out.extend_from_slice(c);
            out.resize(out.len().next_multiple_of(2), 0);
        }
        debug_assert_eq!(out.len(), total);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ticoraw::tiff::{self as rd, Tiff as Reader};

    #[test]
    fn nested_ifds_and_chunks_resolve() {
        for order in [ByteOrder::Little, ByteOrder::Big] {
            let mut t = Tiff::new(order);
            let chunk = t.add_chunk(vec![1, 2, 3]);
            let mut ifd0 = Ifd::default();
            ifd0.set(0x0110, Value::Ascii("MODEL".into()));
            ifd0.set(0x0100, Value::Long(vec![7]));
            ifd0.carry(0x0100, Value::Long(vec![9])); // carried: loses to any set value
            ifd0.set(0x0100, Value::Long(vec![8])); // duplicate: the first one wins
            ifd0.carry(0x0101, Value::Long(vec![3]));
            ifd0.carry(0x0101, Value::Long(vec![4]));
            ifd0.set(0x14A, Value::Ifds(vec![1]));
            ifd0.set(0x111, Value::Chunks(vec![chunk]));
            ifd0.set(0xC621, Value::SRational(vec![(-3, 2); 9]));
            t.add_ifd(ifd0);
            let mut sub = Ifd::default();
            sub.set(0x00FE, Value::Short(vec![0]));
            t.add_ifd(sub);
            let bytes = t.finish().unwrap();

            let r = Reader::parse(&bytes, 0).unwrap();
            assert_eq!(r.order, order);
            let ifds = rd::all_ifds(&r).unwrap();
            assert_eq!(ifds.len(), 2);
            let ifd0 = &ifds[0];
            let tags: Vec<u16> = ifd0.entries.iter().map(|e| e.tag).collect();
            assert_eq!(tags, vec![0x100, 0x101, 0x110, 0x111, 0x14A, 0xC621]);
            assert_eq!(rd::first_uint(&r, ifd0, 0x100).unwrap(), Some(7));
            assert_eq!(rd::first_uint(&r, ifd0, 0x101).unwrap(), Some(3));
            assert_eq!(r.ascii(ifd0.get(0x110).unwrap()), "MODEL");
            let at = rd::first_uint(&r, ifd0, 0x111).unwrap().unwrap() as usize;
            assert_eq!(&bytes[at..at + 3], &[1, 2, 3]);
            assert_eq!(at % 2, 0);
            let m = r.rationals(ifd0.get(0xC621).unwrap()).unwrap();
            assert_eq!(m, vec![(-3, 2); 9]);
            assert_eq!(rd::first_uint(&r, &ifds[1], 0xFE).unwrap(), Some(0));
        }
    }
}
