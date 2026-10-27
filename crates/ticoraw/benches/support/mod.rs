//! Bench-only TicoRAW encoder for realistic synthetic frames.
//!
//! The suite in `tests/common/encoder.rs` writes random coefficients to exercise every
//! coding path. A benchmark needs the opposite: data whose statistics look like a camera
//! frame, so the decoder's hot loops see camera-like GCLIs, bit-plane counts and stream
//! sizes. This encoder therefore
//!
//! 1. renders a deterministic scene (gradients, flat objects with edges, fine texture and
//!    signal-dependent noise) as a linear 14-bit RGGB mosaic and maps it through the inverse
//!    of the Nikon transfer curve;
//! 2. applies the forward Star-Tetrix and the forward 5/3 wavelet in the TicoRAW layout;
//! 3. entropy-codes every precinct the way the cameras do (coding mode 3 in every band,
//!    camera WGT, uniform quantisation), choosing Q and R per precinct so the stream hits a
//!    constant bit rate, and pads each precinct body with zeros to that rate.
//!
//! The transforms and the packet writer are adapted from the test-suite encoder (this
//! repository's own code); the band layout and coding rules follow ISO/IEC 21122-1 Annex C
//! as `docs/FORMAT.md` documents them.

#![allow(dead_code)] // each benchmark binary uses a different part

use std::ops::Range;

/// The intoPIX vendor bytes after the standard PIH in Nikon Z6III files (transfer curve).
const NIKON_EXT: [u8; 13] = [
    0x50, 0x88, 0x70, 0x83, 0xF0, 0x15, 0x23, 0xD1, 0x49, 0xCD, 0x3F, 0x7F, 0x07,
];
/// Camera WGT `(gain, priority)` in packet order (docs/FORMAT.md §2).
const CAMERA_WGT: [(u8, u8); 25] = [
    (3, 0x02),
    (3, 0x13),
    (2, 0x09),
    (2, 0x16),
    (1, 0x0A),
    (1, 0x15),
    (2, 0x05),
    (1, 0x03),
    (1, 0x11),
    (0, 0x01),
    (0, 0x07),
    (0, 0x0D),
    (0, 0x0B),
    (2, 0x06),
    (1, 0x04),
    (1, 0x10),
    (0, 0x00),
    (0, 0x08),
    (0, 0x0E),
    (1, 0x14),
    (0, 0x12),
    (0, 0x0F),
    (0, 0x17),
    (0, 0x0C),
    (0, 0x18),
];
/// Horizontal decomposition levels per component (component 2, Delta, is not transformed).
const LEVELS: [u8; 4] = [5, 5, 0, 5];
/// Fractional bits: the decoder shifts dequantised values left by this.
const FQ: u32 = 4;
/// PIH `Bw`: the largest GCLI the decoder accepts.
const BW: i32 = 18;
const NG: usize = 4;
const SS: usize = 8;
/// Precinct header: Lprc 24, Q 8, R 8 and 2 bits of coding mode per band, byte-aligned.
const PRECINCT_HEADER: usize = 5 + (2 * CAMERA_WGT.len()).div_ceil(8);
/// Every camera file codes every band with vertical prediction plus significance.
const MODE: u8 = 3;

/// A synthetic camera-like frame.
pub struct Frame {
    /// The codestream, SOC..EOC.
    pub codestream: Vec<u8>,
    /// Each slice's precinct data (after its SLH), as `decode_slice` takes it.
    pub slices: Vec<Range<usize>>,
    /// CFA width and height.
    pub width: usize,
    pub height: usize,
    /// Smallest and largest precinct Q the rate control chose.
    pub q_range: (u8, u8),
    /// Mean absolute difference, in 14-bit codes, between the decoded mosaic and the scene:
    /// a check that the stream carries the scene rather than noise.
    pub mean_abs_error: f64,
}

impl Frame {
    /// Photosites in the frame.
    pub fn photosites(&self) -> u64 {
        (self.width * self.height) as u64
    }

    /// Achieved rate in bits per photosite.
    pub fn bpp(&self) -> f64 {
        self.codestream.len() as f64 * 8.0 / self.photosites() as f64
    }
}

/// Encodes the deterministic scene at `width x height` (width even, height a multiple of
/// 4) at a constant `bpp` bits per photosite, with Nikon's `Hsl = 16` precincts per slice.
pub fn encode(width: usize, height: usize, bpp: f64) -> Frame {
    assert!(width % 2 == 0 && height % 4 == 0 && width >= 64);
    let (cw, ch) = (width / 2, height / 2);
    let slice_precincts = 16;

    let mut planes = scene_planes(width, height);
    let curve = ticoraw::header::TransferCurve::from_extension(&NIKON_EXT).expect("curve");
    let scene = ticoraw::mct::assemble_rggb(&planes, cw, ch, &curve);
    forward_star_tetrix(&mut planes, cw, ch);
    // Transmitted values carry FQ fractional bits fewer than the transform domain.
    let coefficients: [Vec<i32>; 4] = std::array::from_fn(|c| {
        analyze_plane(&planes[c], cw, LEVELS[c])
            .into_iter()
            .map(|v| (v + (1 << (FQ - 1))) >> FQ)
            .collect()
    });

    let (bands, packets) = band_layout(cw);
    let mut weights = vec![(0, 0); bands.len()];
    let mut seen = vec![false; bands.len()];
    let mut next = CAMERA_WGT.iter();
    for &(b, _) in packets.iter().flatten() {
        if !std::mem::replace(&mut seen[b], true) {
            weights[b] = *next.next().expect("25 bands");
        }
    }
    let enc = PrecinctEncoder {
        bands: &bands,
        packets: &packets,
        weights: &weights,
        coefficients: &coefficients,
        cw,
    };

    let mut cs = main_header(width, height, slice_precincts);
    let precincts = ch / 2;
    let precinct_bytes = bpp * (4 * width) as f64 / 8.0;
    let start = cs.len();
    let mut slices = Vec::new();
    let mut q_range = (u8::MAX, 0);
    for (s, first) in (0..precincts).step_by(slice_precincts).enumerate() {
        cs.extend_from_slice(&[0xFF, 0x20, 0x00, 0x04]);
        cs.extend_from_slice(&(s as u16).to_be_bytes());
        let slice_start = cs.len();
        let mut top: Option<Vec<BandState>> = None;
        for p in first..(first + slice_precincts).min(precincts) {
            // Constant bit rate: aim at the cumulative target, as a camera's buffer would.
            let target = start + ((p + 1) as f64 * precinct_bytes) as usize;
            let budget = target.saturating_sub(cs.len() + PRECINCT_HEADER);
            let (q, r, mut body, state) = enc.fit(top.as_deref(), p, budget);
            q_range = (q_range.0.min(q), q_range.1.max(q));
            if body.len() < budget {
                body.resize(budget, 0);
            }
            let mut w = BitWriter::default();
            w.put(body.len() as u64, 24);
            w.put(u64::from(q), 8);
            w.put(u64::from(r), 8);
            for _ in &bands {
                w.put(u64::from(MODE), 2);
            }
            cs.extend_from_slice(&w.into_bytes());
            cs.extend_from_slice(&body);
            top = Some(state);
        }
        slices.push(slice_start..cs.len());
    }
    cs.extend_from_slice(&[0xFF, 0x11]);
    let lcod = cs.len() as u32;
    cs[PIH_LCOD..PIH_LCOD + 4].copy_from_slice(&lcod.to_be_bytes());
    let decoded = ticoraw::decode_ticoraw(&cs).expect("synthetic frame decodes");
    let error: u64 = decoded
        .pixels
        .iter()
        .zip(&scene)
        .map(|(&a, &b)| u64::from(a.abs_diff(b)))
        .sum();
    Frame {
        codestream: cs,
        slices,
        width,
        height,
        q_range,
        mean_abs_error: error as f64 / scene.len() as f64,
    }
}

// ---------------------------------------------------------------------------------------
// Scene

/// xorshift64*: small, fast and deterministic across platforms.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Approximately standard normal (Irwin-Hall with four terms).
    fn normal(&mut self) -> f64 {
        (self.unit() + self.unit() + self.unit() + self.unit() - 2.0) * 3f64.sqrt()
    }
}

/// Renders the scene into the four component planes `[Y (G at (0,1)), Cr (R), Delta (G at
/// (1,0)), Cb (B)]` in the decoder's pre-curve sample domain.
fn scene_planes(width: usize, height: usize) -> [Vec<i32>; 4] {
    let (cw, ch) = (width / 2, height / 2);
    let curve = Curve::from_extension(&NIKON_EXT);
    let mut rng = Rng::new(0x5EED);
    // Flat objects: (x0, y0, x1, y1) in frame fractions, reflectance and an RGB tint.
    let objects: Vec<([f64; 4], f64, [f64; 3])> = (0..24)
        .map(|_| {
            let (x, y) = (rng.unit(), rng.unit());
            let (w, h) = (0.05 + 0.25 * rng.unit(), 0.05 + 0.25 * rng.unit());
            let tint = [0.4 + rng.unit(), 0.6 + 0.6 * rng.unit(), 0.4 + rng.unit()];
            ([x, y, x + w, y + h], 0.05 + 0.6 * rng.unit(), tint)
        })
        .collect();
    // White-balance-like channel gains of a daylight raw: green strongest.
    let channel_gain = [0.45, 1.0, 0.6];
    // (component, CFA dy, CFA dx, colour index)
    let sites = [(0, 0, 1, 1), (1, 0, 0, 0), (2, 1, 0, 1), (3, 1, 1, 2)];
    let mut planes: [Vec<i32>; 4] = std::array::from_fn(|_| vec![0; cw * ch]);
    for y in 0..ch {
        for x in 0..cw {
            for &(c, dy, dx, colour) in &sites {
                let (fy, fx) = (
                    (2 * y + dy) as f64 / height as f64,
                    (2 * x + dx) as f64 / width as f64,
                );
                // Sky-to-ground gradient, then the topmost object under this site.
                let mut s = 0.35 * (1.0 - fy) + 0.08;
                let mut tint = [0.8, 1.0, 1.3];
                for (r, refl, t) in &objects {
                    if fx >= r[0] && fx < r[2] && fy >= r[1] && fy < r[3] {
                        s = *refl;
                        tint = *t;
                    }
                }
                // Foliage-like texture of several frequencies in the lower half.
                if fy > 0.5 {
                    let t = (fx * 900.0).sin() * (fy * 700.0).cos() * 0.5
                        + (fx * 3100.0 + fy * 1700.0).sin() * 0.3
                        + (fx * 170.0 - fy * 230.0).sin() * 0.2;
                    s *= 1.0 + 0.35 * t;
                }
                let linear = (s * tint[colour] * channel_gain[colour]).clamp(0.0, 1.0);
                // Shot noise plus read noise, in 14-bit units above black.
                let signal = linear * f64::from(16383 - curve.black as u16);
                let noisy = signal + rng.normal() * (2.5 * signal.sqrt() + 3.0);
                planes[c][y * cw + x] = curve.inverse(noisy);
            }
        }
    }
    planes
}

/// The Nikon output transfer curve (see `TransferCurve` in the crate) and its inverse.
struct Curve {
    black: f64,
    gain: f64,
    knee: f64,
}

impl Curve {
    fn from_extension(t: &[u8]) -> Self {
        Self {
            black: f64::from(u16::from(t[3] & 0x0F) << 8 | u16::from(t[4])),
            gain: f64::from(u32::from(t[7] & 0x03) << 16 | u32::from(t[8]) << 8 | u32::from(t[9])),
            knee: f64::from(u16::from(t[10]) << 8 | u16::from(t[11])),
        }
    }

    /// The pre-curve sample whose 14-bit output is `black + above_black`.
    fn inverse(&self, above_black: f64) -> i32 {
        let g = (above_black.max(0.0) * f64::from(1 << 18)).sqrt();
        let i = self.knee + g * 65536.0 / self.gain;
        (i - 32768.0).round().clamp(-32768.0, 32767.0) as i32
    }
}

// ---------------------------------------------------------------------------------------
// Forward transforms (from tests/common/transforms.rs)

/// Forward 1-D 5/3 lifting with whole-sample symmetric extension.
fn analyze_1d(x: &[i32]) -> (Vec<i32>, Vec<i32>) {
    let n = x.len();
    if n == 1 {
        return (x.to_vec(), Vec::new());
    }
    let high: Vec<i32> = (0..n / 2)
        .map(|k| {
            let right = if 2 * k + 2 < n {
                x[2 * k + 2]
            } else {
                x[2 * k]
            };
            x[2 * k + 1] - ((x[2 * k] + right) >> 1)
        })
        .collect();
    let h = |k: isize| high[k.clamp(0, high.len() as isize - 1) as usize];
    let low = (0..n - n / 2)
        .map(|k| x[2 * k] + ((h(k as isize - 1) + h(k as isize) + 2) >> 2))
        .collect();
    (low, high)
}

/// Multi-level horizontal analysis of one row into `[L_n, H_n, H_(n-1), ..., H_1]`.
fn analyze_row(row: &mut [i32], levels: u8) {
    let mut cur = row.to_vec();
    let mut highs = Vec::new();
    for _ in 0..levels {
        let (l, h) = analyze_1d(&cur);
        highs.push(h);
        cur = l;
    }
    for h in highs.iter().rev() {
        cur.extend_from_slice(h);
    }
    row.copy_from_slice(&cur);
}

/// One vertical level (rows `2p` low, `2p + 1` high), then `levels` horizontal levels on
/// low rows and one on high rows.
fn analyze_plane(plane: &[i32], width: usize, levels: u8) -> Vec<i32> {
    if levels == 0 {
        return plane.to_vec();
    }
    let height = plane.len() / width;
    let mut out = vec![0; plane.len()];
    let mut col = vec![0; height];
    for x in 0..width {
        for (y, v) in col.iter_mut().enumerate() {
            *v = plane[y * width + x];
        }
        let (l, h) = analyze_1d(&col);
        for p in 0..height / 2 {
            out[2 * p * width + x] = l[p];
            out[(2 * p + 1) * width + x] = h[p];
        }
    }
    for (y, row) in out.chunks_mut(width).enumerate() {
        analyze_row(row, if y % 2 == 0 { levels } else { 1 });
    }
    out
}

fn at(p: &[i32], w: usize, h: usize, y: isize, x: isize) -> i32 {
    let y = y.clamp(0, h as isize - 1) as usize;
    let x = x.clamp(0, w as isize - 1) as usize;
    p[y * w + x]
}

fn lift(target: &mut [i32], w: usize, f: impl Fn(isize, isize) -> i32) {
    for (i, v) in target.iter_mut().enumerate() {
        *v += f((i / w) as isize, (i % w) as isize);
    }
}

/// Forward Star-Tetrix: the decoder's four lifting steps undone in reverse order.
fn forward_star_tetrix(planes: &mut [Vec<i32>; 4], w: usize, h: usize) {
    let [yp, rp, dp, bp] = planes;
    lift(rp, w, |y, x| {
        -((at(yp, w, h, y, x - 1)
            + at(yp, w, h, y, x)
            + at(dp, w, h, y - 1, x)
            + at(dp, w, h, y, x))
            >> 2)
    });
    lift(bp, w, |y, x| {
        -((at(dp, w, h, y, x)
            + at(dp, w, h, y, x + 1)
            + at(yp, w, h, y, x)
            + at(yp, w, h, y + 1, x))
            >> 2)
    });
    lift(yp, w, |y, x| {
        (at(rp, w, h, y, x) + at(rp, w, h, y, x + 1) + at(bp, w, h, y - 1, x) + at(bp, w, h, y, x))
            >> 3
    });
    lift(dp, w, |y, x| {
        (at(bp, w, h, y, x - 1) + at(bp, w, h, y, x) + at(rp, w, h, y, x) + at(rp, w, h, y + 1, x))
            >> 3
    });
    lift(dp, w, |y, x| {
        -((at(yp, w, h, y, x - 1)
            + at(yp, w, h, y, x)
            + at(yp, w, h, y + 1, x - 1)
            + at(yp, w, h, y + 1, x))
            >> 2)
    });
    lift(yp, w, |y, x| {
        (at(dp, w, h, y - 1, x)
            + at(dp, w, h, y - 1, x + 1)
            + at(dp, w, h, y, x)
            + at(dp, w, h, y, x + 1))
            >> 3
    });
}

// ---------------------------------------------------------------------------------------
// Codestream syntax

/// MSB-first bit writer; `align` pads with zero bits.
#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    fn put(&mut self, value: u64, n: u32) {
        for i in (0..n).rev() {
            self.bit((value >> i) & 1 == 1);
        }
    }

    fn bit(&mut self, b: bool) {
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
    fn unary(&mut self, x: u32) {
        for _ in 0..x {
            self.bit(true);
        }
        self.bit(false);
    }

    fn align(&mut self) {
        self.bits = self.bytes.len() * 8;
    }

    fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// One wavelet band of one component, placed in the component plane the decoder fills.
struct BandSpec {
    component: usize,
    width: usize,
    /// Lines per precinct (2 for the untransformed component, else 1).
    lines: usize,
    /// Row inside the precinct's two plane rows (0 = vertical low, 1 = vertical high).
    row: usize,
    x: usize,
}

/// Bands in component-major order and the `Ppoc = 1` packet sequence of (band, line).
fn band_layout(comp_width: usize) -> (Vec<BandSpec>, Vec<Vec<(usize, usize)>>) {
    let mut bands = Vec::new();
    let mut per_component = Vec::new();
    for (c, &levels) in LEVELS.iter().enumerate() {
        let first = bands.len();
        let mut push = |width, row, x: &mut usize, lines| {
            bands.push(BandSpec {
                component: c,
                width,
                lines,
                row,
                x: *x,
            });
            *x += width;
        };
        if levels == 0 {
            push(comp_width, 0, &mut 0, 2);
        } else {
            let mut widths = vec![comp_width];
            for _ in 0..levels {
                let w: usize = *widths.last().expect("non-empty");
                widths.push(w.div_ceil(2));
            }
            let mut x = 0;
            push(widths[levels as usize], 0, &mut x, 1);
            for lvl in (0..levels as usize).rev() {
                push(widths[lvl] / 2, 0, &mut x, 1);
            }
            let mut x = 0;
            push(comp_width.div_ceil(2), 1, &mut x, 1);
            push(comp_width / 2, 1, &mut x, 1);
        }
        per_component.push(first..bands.len());
    }
    let mut packets = Vec::new();
    for line in 0..2 {
        for (c, range) in per_component.iter().enumerate() {
            packets.push(if LEVELS[c] == 0 {
                vec![(range.start, line)]
            } else {
                range
                    .clone()
                    .filter(|&b| bands[b].row == line)
                    .map(|b| (b, 0))
                    .collect()
            });
        }
    }
    (bands, packets)
}

/// Byte offset of the PIH Lcod field (SOC 2 + CAP 36 + PIH marker/length 4).
const PIH_LCOD: usize = 42;

fn main_header(width: usize, height: usize, slice_precincts: usize) -> Vec<u8> {
    let mut cs = vec![0xFF, 0x10, 0xFF, 0x50, 0x00, 0x22];
    cs.extend_from_slice(b"CONTACT_INTOPIX_");
    cs.extend_from_slice(&[0xEF, 0xC0]);
    cs.extend_from_slice(&[0; 14]);
    cs.extend_from_slice(&[0xFF, 0x12, 0x00, 0x27]);
    debug_assert_eq!(cs.len(), PIH_LCOD);
    cs.extend_from_slice(&[0; 4]); // Lcod, patched once the length is known
    cs.extend_from_slice(&[0, 0, 0, 0]); // Ppih, Plev
    cs.extend_from_slice(&(width as u16).to_be_bytes());
    cs.extend_from_slice(&(height as u16).to_be_bytes());
    cs.extend_from_slice(&[0, 0]); // Cw
    cs.extend_from_slice(&(slice_precincts as u16).to_be_bytes());
    // Nc, Ng, Ss, Bw, Fq|Br, Fslc|Ppoc|Cpih, NLx|NLy, Lh|Rl|Qpih|Fs|Rm: the camera values.
    cs.extend_from_slice(&[
        4,
        NG as u8,
        SS as u8,
        BW as u8,
        (FQ as u8) << 4 | 4,
        0x10,
        0x51,
        0x14,
    ]);
    cs.extend_from_slice(&NIKON_EXT);
    cs.extend_from_slice(&[0xFF, 0x13, 0x00, 0x0E]);
    for levels in LEVELS {
        cs.extend_from_slice(&[14, 0x11, if levels == 0 { 0 } else { levels << 4 | 1 }]);
    }
    cs.extend_from_slice(&[0xFF, 0x14]);
    cs.extend_from_slice(&(2 + 2 * CAMERA_WGT.len() as u16).to_be_bytes());
    for (g, p) in CAMERA_WGT {
        cs.extend_from_slice(&[g, p]);
    }
    cs
}

/// Per-band GCLI state, mirrored from what the decoder will hold.
#[derive(Clone)]
struct BandState {
    gtli: i32,
    gclis: Vec<Vec<i32>>,
}

/// One coded band line: GCLIs, significance flags, prediction and magnitudes.
struct LineChoice {
    gclis: Vec<i32>,
    insignificant: Vec<bool>,
    m_top: Option<Vec<i32>>,
    mags: Vec<u32>,
    negative: Vec<bool>,
}

struct PrecinctEncoder<'a> {
    bands: &'a [BandSpec],
    packets: &'a [Vec<(usize, usize)>],
    weights: &'a [(u8, u8)],
    coefficients: &'a [Vec<i32>; 4],
    cw: usize,
}

impl PrecinctEncoder<'_> {
    /// Finest `(Q, R)` whose body fits `budget` bytes (or the coarsest when none does), with
    /// that body and the GCLI state it leaves for the next precinct.
    ///
    /// Settings are ranked from finest to coarsest as Q ascending, then R descending: R
    /// lowers the GTLI of every band whose priority is below it, and `(q, 25)` equals
    /// `(q - 1, 0)`.
    fn fit(
        &self,
        top: Option<&[BandState]>,
        p: usize,
        budget: usize,
    ) -> (u8, u8, Vec<u8>, Vec<BandState>) {
        let setting = |k: usize| ((k / 25) as u8, (24 - k % 25) as u8);
        let (mut lo, mut hi) = (0, 16 * 25 - 1);
        let mut best = None;
        while lo <= hi {
            let mid = usize::midpoint(lo, hi);
            let (q, r) = setting(mid);
            let (body, state) = self.encode(top, p, q, r);
            if body.len() <= budget {
                best = Some((q, r, body, state));
                if mid == 0 {
                    break;
                }
                hi = mid - 1;
            } else {
                lo = mid + 1;
            }
        }
        best.unwrap_or_else(|| {
            let (body, state) = self.encode(top, p, 15, 0);
            (15, 0, body, state)
        })
    }

    fn encode(
        &self,
        top: Option<&[BandState]>,
        p: usize,
        q: u8,
        r: u8,
    ) -> (Vec<u8>, Vec<BandState>) {
        let (q, r) = (i32::from(q), i32::from(r));
        let mut state: Vec<BandState> = self
            .bands
            .iter()
            .zip(self.weights)
            .map(|(b, &(gain, priority))| {
                let g = i32::from(gain) + i32::from(i32::from(priority) < r);
                BandState {
                    gtli: if q < g { 0 } else { (q - g).min(15) },
                    gclis: vec![Vec::new(); b.lines],
                }
            })
            .collect();
        let mut body = Vec::new();
        for packet in self.packets {
            let mut lines = Vec::with_capacity(packet.len());
            for &(b, line) in packet {
                let reference = if line > 0 {
                    Some((&state[b].gclis[line - 1][..], state[b].gtli))
                } else {
                    top.map(|t| (&t[b].gclis[self.bands[b].lines - 1][..], t[b].gtli))
                };
                let choice = self.code_line(b, p, line, state[b].gtli, reference);
                state[b].gclis[line].clone_from(&choice.gclis);
                lines.push((b, choice));
            }
            body.extend_from_slice(&Self::write_packet(&state, &lines));
        }
        (body, state)
    }

    /// Codes one band line from its coefficients.
    fn code_line(
        &self,
        b: usize,
        p: usize,
        line: usize,
        gtli: i32,
        reference: Option<(&[i32], i32)>,
    ) -> LineChoice {
        let band = &self.bands[b];
        let row = 2 * p + band.row + line;
        let src = &self.coefficients[band.component][row * self.cw + band.x..][..band.width];
        let gw = band.width.div_ceil(NG);
        let m_top: Option<Vec<i32>> = reference.map(|(top, gtli_top)| {
            let t = gtli_top.max(gtli);
            top.iter().map(|&g| g.max(t)).collect()
        });
        let max_planes = 15.min(BW - gtli);
        let mut gclis = vec![gtli; gw];
        let mut mags = vec![0u32; gw * NG];
        let mut negative = vec![false; gw * NG];
        for (g, gcli) in gclis.iter_mut().enumerate() {
            let group = &src[g * NG..((g + 1) * NG).min(band.width)];
            let largest = group.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
            let planes = ((32 - largest.leading_zeros()) as i32 - gtli).clamp(0, max_planes);
            *gcli = gtli + planes;
            for (k, &v) in group.iter().enumerate() {
                let m = (v.unsigned_abs() >> gtli).min((1 << planes) - 1);
                mags[g * NG + k] = m;
                negative[g * NG + k] = v < 0 && m != 0;
            }
        }
        let implied = |g: usize| m_top.as_ref().map_or(gtli, |m| m[g]);
        let insignificant = (0..gw.div_ceil(SS))
            .map(|s| (s * SS..((s + 1) * SS).min(gw)).all(|g| gclis[g] == implied(g)))
            .collect();
        LineChoice {
            gclis,
            insignificant,
            m_top,
            mags,
            negative,
        }
    }

    fn write_packet(state: &[BandState], lines: &[(usize, LineChoice)]) -> Vec<u8> {
        let mut sig = BitWriter::default();
        for (_, c) in lines {
            for &f in &c.insignificant {
                sig.bit(f);
            }
        }
        sig.align();

        let mut gcli = BitWriter::default();
        for (b, c) in lines {
            let gtli = state[*b].gtli;
            for (g, &v) in c.gclis.iter().enumerate() {
                if c.insignificant[g / SS] {
                    continue;
                }
                let x = match &c.m_top {
                    None => v - gtli,
                    Some(m) => {
                        let (delta, thr) = (v - m[g], m[g] - gtli);
                        if delta > thr {
                            delta + thr
                        } else if delta >= 0 {
                            2 * delta
                        } else {
                            -2 * delta - 1
                        }
                    }
                };
                gcli.unary(x as u32);
            }
        }
        gcli.align();

        let mut data = BitWriter::default();
        for (b, c) in lines {
            let gtli = state[*b].gtli;
            for (g, &v) in c.gclis.iter().enumerate() {
                for plane in (0..v - gtli).rev() {
                    for k in 0..NG {
                        data.bit((c.mags[g * NG + k] >> plane) & 1 == 1);
                    }
                }
            }
        }
        data.align();

        let mut sign = BitWriter::default();
        for (_, c) in lines {
            for (&m, &neg) in c.mags.iter().zip(&c.negative) {
                if m != 0 {
                    sign.bit(neg);
                }
            }
        }
        sign.align();

        let (ldat, lcnt, lsgn) = (data.bytes.len(), gcli.bytes.len(), sign.bytes.len());
        assert!(ldat < 1 << 20 && lcnt < 1 << 20 && lsgn < 1 << 15);
        let mut out = BitWriter::default();
        out.put(0, 1); // Dr = 0: cameras never use raw GCLIs
        out.put(ldat as u64, 20);
        out.put(lcnt as u64, 20);
        out.put(lsgn as u64, 15);
        let mut bytes = out.into_bytes();
        for part in [sig, gcli, data, sign] {
            bytes.extend_from_slice(&part.into_bytes());
        }
        bytes
    }
}

// ---------------------------------------------------------------------------------------
// The decoder's stages, run one at a time through the public stage API, the way
// `ticoraw::decode_ticoraw` chains them.

/// Header-derived state every stage needs.
pub struct Stages {
    pub layout: ticoraw::entropy::Layout,
    pub curve: ticoraw::header::TransferCurve,
    /// Component plane width and height.
    pub w: usize,
    pub h: usize,
}

impl Stages {
    pub fn new(frame: &Frame) -> Self {
        let header = ticoraw::header::parse_main_header(&frame.codestream).expect("header");
        let curve = ticoraw::header::TransferCurve::from_extension(&header.pih.extension)
            .expect("transfer curve");
        let layout = ticoraw::entropy::Layout::new(&header).expect("supported profile");
        let (w, h) = (layout.comp_width, layout.comp_height);
        Self {
            layout,
            curve,
            w,
            h,
        }
    }

    /// Entropy-decodes every slice, one after the other, into fresh component planes.
    pub fn entropy(&self, frame: &Frame) -> [Vec<i32>; 4] {
        let rows = 2 * self.layout.precincts_per_slice * self.w;
        let mut planes: [Vec<i32>; 4] = std::array::from_fn(|_| vec![0; self.w * self.h]);
        let mut chunks: Vec<_> = planes.iter_mut().map(|p| p.chunks_mut(rows)).collect();
        for (s, range) in frame.slices.iter().enumerate() {
            let mut slice: Vec<&mut [i32]> = chunks.iter_mut().filter_map(Iterator::next).collect();
            ticoraw::entropy::decode_slice(
                &self.layout,
                &frame.codestream[range.clone()],
                s * self.layout.precincts_per_slice,
                &mut slice,
            )
            .expect("valid slice");
        }
        planes
    }

    /// Horizontal inverse wavelet of every row (sequential).
    pub fn horizontal(&self, planes: &mut [Vec<i32>; 4]) {
        let mut scratch = Vec::new();
        for (c, plane) in planes.iter_mut().enumerate() {
            let levels = self.layout.nlx[c];
            if levels == 0 {
                continue;
            }
            for (y, row) in plane.chunks_mut(self.w).enumerate() {
                let n = if y % 2 == 0 { levels } else { 1 };
                ticoraw::wavelet::synthesize_row(row, n, &mut scratch);
            }
        }
    }

    /// Vertical inverse wavelet of every transformed component.
    pub fn vertical(&self, planes: &mut [Vec<i32>; 4]) {
        for (c, plane) in planes.iter_mut().enumerate() {
            if self.layout.nlx[c] == 0 {
                continue;
            }
            let mut out = vec![0; plane.len()];
            ticoraw::wavelet::synthesize_columns(plane, self.w, &mut out);
            *plane = out;
        }
    }

    pub fn star_tetrix(&self, planes: &mut [Vec<i32>; 4]) {
        ticoraw::mct::inverse_star_tetrix(planes, self.w, self.h);
    }

    /// The transfer curve and RGGB interleave.
    pub fn curve(&self, planes: &[Vec<i32>; 4]) -> Vec<u16> {
        ticoraw::mct::assemble_rggb(planes, self.w, self.h, &self.curve)
    }

    /// Every stage in order; must equal `decode_ticoraw`.
    pub fn all(&self, frame: &Frame) -> Vec<u16> {
        let mut planes = self.entropy(frame);
        self.horizontal(&mut planes);
        self.vertical(&mut planes);
        self.star_tetrix(&mut planes);
        self.curve(&planes)
    }
}
