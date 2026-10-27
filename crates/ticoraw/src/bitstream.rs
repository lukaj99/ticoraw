//! MSB-first bit reader for JPEG XS entropy data (no byte stuffing).
//!
//! This stage module is public for testing and advanced use; its API may change in minor
//! releases.

use crate::error::{Result, TicoError};

/// Reads bit fields MSB first from a byte slice; every read is bounds-checked.
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Absolute bit position.
    pos: usize,
    /// Bit position one past the last readable bit.
    end: usize,
}

impl<'a> BitReader<'a> {
    /// A reader positioned at the first bit of `data`.
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            end: data.len() * 8,
        }
    }

    /// Bits consumed so far.
    #[must_use]
    pub fn bit_pos(&self) -> usize {
        self.pos
    }

    /// Bytes consumed so far, counting a partly read byte as consumed.
    #[must_use]
    pub fn byte_pos(&self) -> usize {
        self.pos.div_ceil(8)
    }

    /// Bits left before the end of the data.
    #[must_use]
    pub fn remaining_bits(&self) -> usize {
        self.end.saturating_sub(self.pos)
    }

    /// Peeks up to 32 bits; bits past the end read as zero.
    #[inline]
    #[must_use]
    pub fn peek(&self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        if n == 0 {
            return 0;
        }
        let byte = self.pos >> 3;
        let word = match self.data.get(byte..byte + 8) {
            // Fast path: a fixed 8-byte load compiles to a single unaligned read.
            Some(b) => u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            None => {
                let mut word = [0u8; 8];
                let avail = self.data.len().saturating_sub(byte).min(8);
                word[..avail].copy_from_slice(&self.data[byte..byte + avail]);
                u64::from_be_bytes(word)
            }
        };
        let v = word << (self.pos & 7);
        (v >> (64 - n)) as u32
    }

    /// Reads an `n`-bit unsigned field (`n <= 32`).
    ///
    /// # Errors
    ///
    /// [`TicoError::Truncated`] when fewer than `n` bits remain.
    #[inline]
    pub fn read(&mut self, n: u32) -> Result<u32> {
        if self.pos + n as usize > self.end {
            return Err(TicoError::Truncated {
                what: "bitstream",
                offset: self.pos / 8,
            });
        }
        let v = self.peek(n);
        self.pos += n as usize;
        Ok(v)
    }

    /// Reads one bit.
    ///
    /// # Errors
    ///
    /// [`TicoError::Truncated`] at the end of the data.
    #[inline]
    pub fn bit(&mut self) -> Result<bool> {
        Ok(self.read(1)? == 1)
    }

    /// Counts `1` bits up to and including a terminating `0`, failing past `max`.
    ///
    /// # Errors
    ///
    /// [`TicoError::Entropy`] when the code is longer than `max`; [`TicoError::Truncated`]
    /// when the data ends before the terminating `0`.
    #[inline]
    pub fn unary(&mut self, max: u32) -> Result<u32> {
        let mut n = 0;
        loop {
            let probe = self.peek(32);
            let ones = probe.leading_ones();
            if ones < 32 {
                let take = n + ones;
                if take > max {
                    return Err(TicoError::Entropy(format!("unary code longer than {max}")));
                }
                if self.pos + ones as usize + 1 > self.end {
                    return Err(TicoError::Truncated {
                        what: "unary code",
                        offset: self.pos / 8,
                    });
                }
                self.pos += ones as usize + 1;
                return Ok(take);
            }
            n += 32;
            self.pos += 32;
            if n > max || self.pos > self.end {
                return Err(TicoError::Entropy(format!("unary code longer than {max}")));
            }
        }
    }

    /// Advances to the next byte boundary (no-op when already aligned).
    pub fn align(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }

    /// Skips `bits` bits.
    ///
    /// # Errors
    ///
    /// [`TicoError::Truncated`] when fewer than `bits` bits remain.
    pub fn skip(&mut self, bits: usize) -> Result<()> {
        if self.pos + bits > self.end {
            return Err(TicoError::Truncated {
                what: "bitstream skip",
                offset: self.pos / 8,
            });
        }
        self.pos += bits;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_msb_first() {
        let mut r = BitReader::new(&[0b1011_0010, 0xFF, 0x00]);
        assert_eq!(r.read(1).unwrap(), 1);
        assert_eq!(r.read(3).unwrap(), 0b011);
        assert_eq!(r.read(8).unwrap(), 0b0010_1111);
        r.align();
        assert_eq!(r.byte_pos(), 2);
        assert_eq!(r.read(8).unwrap(), 0);
        assert!(r.read(1).is_err());
    }

    #[test]
    fn unary_counts_ones() {
        let mut r = BitReader::new(&[0b1110_0110, 0xFF, 0xFF, 0xFF, 0xFF, 0b1000_0000]);
        assert_eq!(r.unary(40).unwrap(), 3);
        assert_eq!(r.unary(40).unwrap(), 0);
        assert_eq!(r.unary(40).unwrap(), 2);
        assert_eq!(r.unary(40).unwrap(), 33);
        assert!(BitReader::new(&[0xFF]).unary(4).is_err());
    }
}
