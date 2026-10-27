//! Minimal read-only TIFF/IFD reader shared by the NEF and DNG parsers.
//!
//! This stage module is public for testing and advanced use; its API may change in minor
//! releases.

use crate::error::{Result, TicoError};

/// Byte order of a TIFF structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    /// `II`: little-endian.
    Little,
    /// `MM`: big-endian.
    Big,
}

/// A TIFF structure embedded in `data`; all offsets inside it are relative to `base`.
#[derive(Debug, Clone, Copy)]
pub struct Tiff<'a> {
    /// The whole buffer the TIFF lives in.
    pub data: &'a [u8],
    /// Position of the TIFF header in `data`.
    pub base: usize,
    /// Byte order from the header.
    pub order: ByteOrder,
    /// Offset of IFD0, relative to `base`.
    pub first_ifd: u32,
}

/// One IFD entry.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    /// Tag number.
    pub tag: u16,
    /// TIFF field type (1 = BYTE, 2 = ASCII, 3 = SHORT, 4 = LONG, 5 = RATIONAL, ...).
    pub typ: u16,
    /// Number of values.
    pub count: u32,
    /// Absolute position of the value bytes in `Tiff::data`.
    pub pos: usize,
}

/// One image file directory.
#[derive(Debug, Clone)]
pub struct Ifd {
    /// Entries in file order.
    pub entries: Vec<Entry>,
    /// Offset of the next IFD in the chain, relative to `base` (0 = none).
    pub next: u32,
}

fn type_size(typ: u16) -> usize {
    match typ {
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 | 16..=18 => 8,
        _ => 1,
    }
}

impl<'a> Tiff<'a> {
    /// Parses a TIFF header (`II*\0` / `MM\0*`) located at `base`.
    ///
    /// # Errors
    ///
    /// [`TicoError::Truncated`] when fewer than 8 bytes follow `base`;
    /// [`TicoError::Container`] without a byte-order mark.
    pub fn parse(data: &'a [u8], base: usize) -> Result<Self> {
        let head = data.get(base..base + 8).ok_or(TicoError::Truncated {
            what: "TIFF header",
            offset: base,
        })?;
        let order = match &head[..4] {
            b"II*\0" => ByteOrder::Little,
            b"MM\0*" => ByteOrder::Big,
            _ => return Err(TicoError::Container("missing TIFF byte-order mark".into())),
        };
        let mut t = Self {
            data,
            base,
            order,
            first_ifd: 0,
        };
        t.first_ifd = t.u32_at(base + 4)?;
        Ok(t)
    }

    /// Reads a `u16` at absolute position `pos` in the TIFF's byte order.
    ///
    /// # Errors
    ///
    /// [`TicoError::Truncated`] past the end of `data`.
    pub fn u16_at(&self, pos: usize) -> Result<u16> {
        let b = self.data.get(pos..pos + 2).ok_or(TicoError::Truncated {
            what: "TIFF u16",
            offset: pos,
        })?;
        Ok(match self.order {
            ByteOrder::Little => u16::from_le_bytes([b[0], b[1]]),
            ByteOrder::Big => u16::from_be_bytes([b[0], b[1]]),
        })
    }

    /// Reads a `u32` at absolute position `pos` in the TIFF's byte order.
    ///
    /// # Errors
    ///
    /// [`TicoError::Truncated`] past the end of `data`.
    pub fn u32_at(&self, pos: usize) -> Result<u32> {
        let b = self.data.get(pos..pos + 4).ok_or(TicoError::Truncated {
            what: "TIFF u32",
            offset: pos,
        })?;
        let a = [b[0], b[1], b[2], b[3]];
        Ok(match self.order {
            ByteOrder::Little => u32::from_le_bytes(a),
            ByteOrder::Big => u32::from_be_bytes(a),
        })
    }

    /// Parses the IFD at `offset` (relative to `base`), checking that every entry's value
    /// lies inside `data`.
    ///
    /// # Errors
    ///
    /// [`TicoError::Truncated`] when the IFD or an entry value runs past `data`.
    pub fn ifd(&self, offset: u32) -> Result<Ifd> {
        let at = self
            .base
            .checked_add(offset as usize)
            .ok_or(TicoError::Truncated {
                what: "TIFF IFD",
                offset: usize::MAX,
            })?;
        let n = self.u16_at(at)? as usize;
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let p = at + 2 + 12 * i;
            let tag = self.u16_at(p)?;
            let typ = self.u16_at(p + 2)?;
            let count = self.u32_at(p + 4)?;
            let size = type_size(typ).saturating_mul(count as usize);
            let pos = if size <= 4 {
                p + 8
            } else {
                self.base
                    .checked_add(self.u32_at(p + 8)? as usize)
                    .ok_or(TicoError::Truncated {
                        what: "TIFF entry value",
                        offset: usize::MAX,
                    })?
            };
            if pos.saturating_add(size) > self.data.len() {
                return Err(TicoError::Truncated {
                    what: "TIFF entry value",
                    offset: pos,
                });
            }
            entries.push(Entry {
                tag,
                typ,
                count,
                pos,
            });
        }
        let next = self.u32_at(at + 2 + 12 * n)?;
        Ok(Ifd { entries, next })
    }

    /// Integer values of an entry (BYTE, SHORT, LONG, IFD, LONG8 are accepted).
    ///
    /// # Errors
    ///
    /// [`TicoError::Container`] for a non-integer type.
    pub fn uints(&self, e: &Entry) -> Result<Vec<u64>> {
        let n = e.count as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            out.push(match e.typ {
                1 | 7 => u64::from(self.data[e.pos + i]),
                3 => u64::from(self.u16_at(e.pos + 2 * i)?),
                4 | 13 => u64::from(self.u32_at(e.pos + 4 * i)?),
                16 | 18 => {
                    let hi = u64::from(self.u32_at(e.pos + 8 * i)?);
                    let lo = u64::from(self.u32_at(e.pos + 8 * i + 4)?);
                    match self.order {
                        ByteOrder::Little => (lo << 32) | hi,
                        ByteOrder::Big => (hi << 32) | lo,
                    }
                }
                t => {
                    return Err(TicoError::Container(format!(
                        "tag {:#06x} has non-integer type {t}",
                        e.tag
                    )));
                }
            });
        }
        Ok(out)
    }

    /// (numerator, denominator) pairs of a RATIONAL / SRATIONAL entry.
    pub fn rationals(&self, e: &Entry) -> Result<Vec<(i64, i64)>> {
        (0..e.count as usize)
            .map(|i| {
                let n = self.u32_at(e.pos + 8 * i)?;
                let d = self.u32_at(e.pos + 8 * i + 4)?;
                Ok(if e.typ == 10 {
                    (i64::from(n as i32), i64::from(d as i32))
                } else {
                    (i64::from(n), i64::from(d))
                })
            })
            .collect()
    }

    /// Raw value bytes of `e` (checked against `data` when the IFD was parsed).
    #[must_use]
    pub fn bytes(&self, e: &Entry) -> &'a [u8] {
        let len = type_size(e.typ) * e.count as usize;
        &self.data[e.pos..e.pos + len]
    }

    /// Value of an ASCII entry up to its first NUL, with trailing whitespace trimmed and
    /// invalid UTF-8 replaced.
    #[must_use]
    pub fn ascii(&self, e: &Entry) -> String {
        let b = self.bytes(e);
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).trim_end().to_owned()
    }
}

impl Ifd {
    /// The first entry with `tag`, if any.
    #[must_use]
    pub fn get(&self, tag: u16) -> Option<&Entry> {
        self.entries.iter().find(|e| e.tag == tag)
    }
}

/// Convenience: first integer value of `tag`, if present.
pub fn first_uint(t: &Tiff<'_>, ifd: &Ifd, tag: u16) -> Result<Option<u64>> {
    match ifd.get(tag) {
        Some(e) => Ok(t.uints(e)?.first().copied()),
        None => Ok(None),
    }
}

/// First integer value of `tag`.
///
/// # Errors
///
/// [`TicoError::Container`] when the tag is missing or not an integer type.
pub fn require_uint(t: &Tiff<'_>, ifd: &Ifd, tag: u16) -> Result<u64> {
    first_uint(t, ifd, tag)?.ok_or_else(|| TicoError::Container(format!("missing tag {tag:#06x}")))
}

/// Most IFDs [`all_ifds`] will visit.
pub const MAX_IFDS: usize = 64;

/// Walks IFD0, its chain, and SubIFDs (tag 0x14A) breadth-first, returning every IFD found.
///
/// At most [`MAX_IFDS`] IFDs are visited, and only the first [`MAX_IFDS`] SubIFD offsets of
/// each IFD are followed, so hostile fan-out cannot amplify memory.
pub fn all_ifds(t: &Tiff<'_>) -> Result<Vec<Ifd>> {
    let mut out = Vec::new();
    let mut queue = vec![t.first_ifd];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(off) = queue.pop() {
        if out.len() >= MAX_IFDS {
            break;
        }
        if off == 0 || !seen.insert(off) {
            continue;
        }
        let ifd = t.ifd(off)?;
        if let Some(e) = ifd.get(0x14A) {
            let mut subs = Vec::with_capacity(MAX_IFDS.min(e.count as usize));
            for i in 0..MAX_IFDS.min(e.count as usize) {
                subs.push(match e.typ {
                    4 | 13 => t.u32_at(e.pos + 4 * i)?,
                    _ => return Err(TicoError::Container("SubIFDs entry is not LONG/IFD".into())),
                });
            }
            queue.extend(subs.into_iter().rev());
        }
        if ifd.next != 0 {
            queue.insert(0, ifd.next);
        }
        out.push(ifd);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_little_endian_ifd() {
        // header, IFD with one SHORT entry (tag 0x100 = 6064), no next IFD
        let mut b = b"II*\0\x08\0\0\0".to_vec();
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&0x100u16.to_le_bytes());
        b.extend_from_slice(&3u16.to_le_bytes());
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&6064u32.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        let t = Tiff::parse(&b, 0).unwrap();
        let ifd = t.ifd(t.first_ifd).unwrap();
        assert_eq!(require_uint(&t, &ifd, 0x100).unwrap(), 6064);
        assert!(first_uint(&t, &ifd, 0x101).unwrap().is_none());
    }

    #[test]
    fn rejects_garbage() {
        assert!(Tiff::parse(b"NOTATIFF", 0).is_err());
        assert!(Tiff::parse(b"II", 0).is_err());
    }
}
