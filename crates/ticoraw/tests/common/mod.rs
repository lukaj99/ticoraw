//! Test-only support for the corpus-independent suite: a deterministic PRNG, an MSB-first
//! bit writer, a TicoRAW codestream encoder, TIFF/NEF/DNG builders, a lossless-JPEG
//! encoder and forward transforms.
#![allow(dead_code)]

pub mod encoder;
pub mod ljpeg;
pub mod tiff;
pub mod transforms;

/// xorshift64* — small, fast and fully deterministic across platforms.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut r = Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        r.next_u64();
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }

    /// Uniform in `lo..=hi`.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        debug_assert!(lo <= hi);
        lo + self.below((hi - lo + 1) as u64) as i64
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn byte(&mut self) -> u8 {
        (self.next_u64() >> 56) as u8
    }

    pub fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.byte()).collect()
    }
}

/// MSB-first bit writer; `align` pads with zero bits.
#[derive(Debug, Clone, Default)]
pub struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    pub fn put(&mut self, value: u64, n: u32) {
        for i in (0..n).rev() {
            self.bit((value >> i) & 1 == 1);
        }
    }

    pub fn bit(&mut self, b: bool) {
        if self.bits % 8 == 0 {
            self.bytes.push(0);
        }
        if b {
            let last = self.bytes.len() - 1;
            self.bytes[last] |= 0x80 >> (self.bits % 8);
        }
        self.bits += 1;
    }

    /// `x` one bits followed by a terminating zero.
    pub fn unary(&mut self, x: u32) {
        for _ in 0..x {
            self.bit(true);
        }
        self.bit(false);
    }

    pub fn align(&mut self) {
        self.bits = self.bytes.len() * 8;
    }

    /// Pads to a byte boundary with one bits (JPEG entropy-segment convention).
    pub fn align_ones(&mut self) {
        while self.bits % 8 != 0 {
            self.bit(true);
        }
    }

    #[must_use]
    pub fn len_bytes(&self) -> usize {
        self.bytes.len()
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}
