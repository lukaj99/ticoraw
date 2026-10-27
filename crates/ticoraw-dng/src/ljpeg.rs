//! Lossless JPEG (ITU-T T.81 process 14, SOF3) encoder for DNG CFA tiles.
//!
//! Each tile is coded as two interleaved components (even and odd columns), so predictor 1
//! (the sample to the left) predicts every photosite from the nearest one of the same CFA
//! colour. Huffman tables are built per tile and per component from the tile's own
//! difference statistics (T.81 Annex K.2, limited to 16-bit codes).

/// Side length of the square tiles the raw image is cut into.
pub(crate) const TILE: usize = 256;

/// Number of difference categories (SSSS 0..=16).
const CATEGORIES: usize = 17;

/// Copies tile (`tx`, `ty`) of a `width` x `height` raster into a `TILE` x `TILE` buffer.
///
/// Padding (readers ignore it) is chosen so its differences are zero: columns past the
/// right edge repeat the last sample of the same CFA parity, and rows past the bottom edge
/// repeat the first two samples of the last row of the same parity. Zeros then dominate an
/// edge tile's histogram and cost a bit or two each.
pub(crate) fn extract_tile(
    pixels: &[u16],
    width: usize,
    height: usize,
    tx: usize,
    ty: usize,
) -> Vec<u16> {
    let clamp = |v: usize, n: usize| {
        if v < n {
            v
        } else if n >= 2 {
            n - 2 + ((v - n) & 1)
        } else {
            0
        }
    };
    let x0 = tx * TILE;
    let inside = width.saturating_sub(x0).min(TILE);
    let mut tile = vec![0u16; TILE * TILE];
    for (r, out) in tile.chunks_exact_mut(TILE).enumerate() {
        let y = ty * TILE + r;
        let row = &pixels[clamp(y, height) * width..][..width];
        if y >= height {
            let pair = [row[x0], row[clamp(x0 + 1, width)]];
            for (c, o) in out.iter_mut().enumerate() {
                *o = pair[c & 1];
            }
            continue;
        }
        out[..inside].copy_from_slice(&row[x0..x0 + inside]);
        for (c, o) in out.iter_mut().enumerate().skip(inside) {
            *o = row[clamp(x0 + c, width)];
        }
    }
    tile
}

/// Encodes a `tile_width` x `tile_height` tile (`tile_width` even) as a 16-bit, two-component,
/// predictor-1 lossless JPEG stream.
pub(crate) fn encode_tile(tile: &[u16], tile_width: usize, tile_height: usize) -> Vec<u8> {
    debug_assert!(tile_width % 2 == 0 && tile.len() == tile_width * tile_height);
    let cols = tile_width / 2;
    // Pass 1: differences (stored wrapped to 16 bits) and their category histograms.
    let mut diffs = vec![0u16; tile.len()];
    let mut hist = [[0u32; CATEGORIES]; 2];
    for y in 0..tile_height {
        let row = y * tile_width;
        for x in 0..tile_width {
            let i = row + x;
            let pred = if x >= 2 {
                tile[i - 2]
            } else if y > 0 {
                tile[i - tile_width]
            } else {
                1 << 15
            };
            let d = tile[i].wrapping_sub(pred);
            diffs[i] = d;
            hist[x & 1][category(d)] += 1;
        }
    }
    let tables = [
        Table::from_histogram(&hist[0]),
        Table::from_histogram(&hist[1]),
    ];

    let mut out = Vec::with_capacity(tile.len() + 128);
    out.extend_from_slice(&[0xFF, 0xD8]);
    for (id, t) in tables.iter().enumerate() {
        let n = t.values.len();
        out.extend_from_slice(&[0xFF, 0xC4]);
        out.extend_from_slice(&((2 + 1 + 16 + n) as u16).to_be_bytes());
        out.push(id as u8);
        out.extend_from_slice(&t.bits);
        out.extend_from_slice(&t.values);
    }
    out.extend_from_slice(&[0xFF, 0xC3, 0x00, 0x0E, 16]);
    out.extend_from_slice(&(tile_height as u16).to_be_bytes());
    out.extend_from_slice(&(cols as u16).to_be_bytes());
    out.extend_from_slice(&[2, 0, 0x11, 0, 1, 0x11, 0]);
    // SOS: two components (tables 0 and 1), predictor 1, no point transform.
    out.extend_from_slice(&[0xFF, 0xDA, 0x00, 0x0A, 2, 0, 0x00, 1, 0x10, 1, 0, 0]);

    // Pass 2: entropy-code the differences in raster (interleaved MCU) order.
    let mut bits = BitWriter { out, acc: 0, n: 0 };
    for (i, &d) in diffs.iter().enumerate() {
        let t = &tables[i & 1];
        let s = category(d);
        let (code, len) = t.codes[s];
        if s == 0 || s == 16 {
            bits.put(u32::from(code), u32::from(len));
        } else {
            // Negative differences are sent as d - 1 in s bits (T.81 F.1.2.1.1).
            let v = i32::from(d as i16);
            let extra = (if v < 0 { v - 1 } else { v }) as u32 & ((1 << s) - 1);
            bits.put((u32::from(code) << s) | extra, u32::from(len) + s as u32);
        }
    }
    let mut out = bits.finish();
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// SSSS category of a difference stored as a wrapped `u16` (T.81 Table H.2).
fn category(d: u16) -> usize {
    match d as i16 {
        0 => 0,
        i16::MIN => 16,
        v => (16 - v.unsigned_abs().leading_zeros()) as usize,
    }
}

/// A Huffman table in DHT form (`bits`, `values`) plus the code of every category.
struct Table {
    bits: [u8; 16],
    values: Vec<u8>,
    /// (code, length) per category; length 0 for categories that never occur.
    codes: [(u16, u8); CATEGORIES],
}

impl Table {
    /// Optimal code lengths limited to 16 bits (T.81 Annex K.2 and K.3), then canonical codes
    /// (Annex C).
    fn from_histogram(hist: &[u32; CATEGORIES]) -> Self {
        // One extra symbol with frequency 1 reserves the all-ones code word (K.2).
        const N: usize = CATEGORIES + 1;
        let mut freq = [0u64; N];
        for (f, &h) in freq.iter_mut().zip(hist) {
            *f = u64::from(h);
        }
        freq[CATEGORIES] = 1;
        let mut size = [0usize; N];
        let mut others = [usize::MAX; N];
        loop {
            // The two least frequent live symbols; ties go to the larger symbol (K.2).
            let mut v1 = None;
            for i in 0..N {
                if freq[i] > 0 && v1.is_none_or(|v: usize| freq[i] <= freq[v]) {
                    v1 = Some(i);
                }
            }
            let mut v2 = None;
            for i in 0..N {
                if freq[i] > 0 && Some(i) != v1 && v2.is_none_or(|v: usize| freq[i] <= freq[v]) {
                    v2 = Some(i);
                }
            }
            let (Some(mut v1), Some(mut v2)) = (v1, v2) else {
                break;
            };
            freq[v1] += freq[v2];
            freq[v2] = 0;
            size[v1] += 1;
            while others[v1] != usize::MAX {
                v1 = others[v1];
                size[v1] += 1;
            }
            others[v1] = v2;
            size[v2] += 1;
            while others[v2] != usize::MAX {
                v2 = others[v2];
                size[v2] += 1;
            }
        }
        // Count code lengths (at most N - 1 = 17 deep), then limit them to 16 (K.3).
        let mut count = [0u32; N + 1];
        for &s in &size {
            if s > 0 {
                count[s] += 1;
            }
        }
        for i in (17..=N).rev() {
            while count[i] > 0 {
                let mut j = i - 2;
                while count[j] == 0 {
                    j -= 1;
                }
                count[i] -= 2;
                count[i - 1] += 1;
                count[j + 1] += 2;
                count[j] -= 1;
            }
        }
        // Drop the reserved symbol's code: it is one of the longest.
        let mut longest = 16;
        while count[longest] == 0 {
            longest -= 1;
        }
        count[longest] -= 1;

        // Symbols by original code length, then by value (K.4); the reserved one is last.
        let mut order: Vec<usize> = (0..CATEGORIES).filter(|&s| size[s] > 0).collect();
        order.sort_by_key(|&s| (size[s], s));
        let mut bits = [0u8; 16];
        for (b, &c) in bits.iter_mut().zip(&count[1..=16]) {
            *b = c as u8;
        }
        let values: Vec<u8> = order.iter().map(|&s| s as u8).collect();
        let mut codes = [(0u16, 0u8); CATEGORIES];
        let mut code = 0u32;
        let mut k = 0;
        for len in 1..=16u8 {
            for _ in 0..bits[usize::from(len) - 1] {
                codes[order[k]] = (code as u16, len);
                k += 1;
                code += 1;
            }
            code <<= 1;
        }
        debug_assert_eq!(k, order.len());
        Self {
            bits,
            values,
            codes,
        }
    }
}

/// MSB-first bit writer with JPEG `FF00` byte stuffing.
struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    /// Appends the low `len` (at most 32) bits of `bits`.
    fn put(&mut self, bits: u32, len: u32) {
        self.acc = (self.acc << len) | u64::from(bits);
        self.n += len;
        while self.n >= 8 {
            self.n -= 8;
            let byte = (self.acc >> self.n) as u8;
            self.out.push(byte);
            if byte == 0xFF {
                self.out.push(0);
            }
        }
    }

    /// Pads the last byte with one bits (T.81 F.1.2.3) and returns the stream.
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            let pad = 8 - self.n;
            self.put((1 << pad) - 1, pad);
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ticoraw::dng::decode_lossless_jpeg;

    fn round_trip(tile: &[u16], w: usize, h: usize) -> usize {
        let j = encode_tile(tile, w, h);
        let d = decode_lossless_jpeg(&j).unwrap();
        assert_eq!((d.width, d.height), (w, h));
        assert_eq!(d.samples, tile);
        j.len()
    }

    fn lcg(seed: &mut u64) -> u64 {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        *seed >> 33
    }

    #[test]
    fn round_trips_noise_extremes_and_flat_tiles() {
        let mut seed = 1;
        let noise: Vec<u16> = (0..64 * 32).map(|_| lcg(&mut seed) as u16).collect();
        round_trip(&noise, 64, 32);
        // Alternating extremes produce every category, including 16 (difference -32768).
        let extremes: Vec<u16> = (0..16 * 8)
            .map(|i| match i % 3 {
                0 => 0,
                1 => 65535,
                _ => 32768,
            })
            .collect();
        round_trip(&extremes, 16, 8);
        let flat = vec![1008u16; 32 * 4];
        assert!(round_trip(&flat, 32, 4) < 100);
        round_trip(&[7, 9], 2, 1);
    }

    #[test]
    fn smooth_14_bit_data_compresses() {
        let (w, h) = (256, 256);
        let tile: Vec<u16> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                (1008 + (x * 7 + y * 3) % 900 + (x & 1) * 2000) as u16
            })
            .collect();
        let n = round_trip(&tile, w, h);
        assert!(n < w * h, "{n} bytes for a smooth 256x256 tile");
    }

    #[test]
    fn skewed_histograms_are_limited_to_16_bits() {
        // Fibonacci frequencies force an unconstrained Huffman tree deeper than 16 levels.
        let mut hist = [0u32; CATEGORIES];
        let (mut a, mut b) = (1u32, 1u32);
        for h in &mut hist {
            *h = a;
            (a, b) = (b, a + b);
        }
        let t = Table::from_histogram(&hist);
        assert_eq!(t.values.len(), CATEGORIES);
        assert!(t.codes.iter().all(|&(_, l)| (1..=16).contains(&l)));
        let kraft: f64 = t
            .codes
            .iter()
            .map(|&(_, l)| 0.5f64.powi(i32::from(l)))
            .sum();
        assert!(kraft < 1.0, "the all-ones code must stay unused");
        let total: u32 = t.bits.iter().map(|&b| u32::from(b)).sum();
        assert_eq!(total as usize, CATEGORIES);
    }

    #[test]
    fn single_symbol_tables_are_valid() {
        let mut hist = [0u32; CATEGORIES];
        hist[0] = 500;
        let t = Table::from_histogram(&hist);
        assert_eq!(t.values, vec![0]);
        assert_eq!(t.codes[0], (0, 1));
    }

    #[test]
    fn edge_tiles_repeat_same_parity_samples() {
        let (w, h) = (5, 3);
        let px: Vec<u16> = (0..(w * h) as u16).collect();
        let t = extract_tile(&px, w, h, 0, 0);
        assert_eq!(&t[..8], &[0, 1, 2, 3, 4, 3, 4, 3]);
        // Rows 3 and 4 repeat the first two samples of rows 1 and 2 (same parity).
        assert_eq!(
            &t[3 * TILE..3 * TILE + 4],
            &[px[w], px[w + 1], px[w], px[w + 1]]
        );
        assert_eq!(t[4 * TILE + 255], px[2 * w + 1]);
        let one = extract_tile(&[42], 1, 1, 0, 0);
        assert!(one.iter().all(|&v| v == 42));
    }
}
