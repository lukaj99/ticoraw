//! TicoRAW encoder: writes random quantised coefficient planes as a codestream in
//! exactly the profile Nikon HE* files use (and the decoder expects), and returns the
//! dequantised coefficients the decoder must reproduce.
//!
//! The band layout, WGT order, GTLI rule, GCLI prediction and dequantisation are derived
//! here independently of `ticoraw::entropy` from the JPEG XS rules (ISO/IEC 21122-1
//! Annex C) as the TicoRAW profile applies them.

use super::{BitWriter, Rng};

/// The intoPIX vendor bytes after the standard PIH in Nikon Z6III files (transfer curve).
pub const NIKON_EXT: [u8; 13] = [
    0x50, 0x88, 0x70, 0x83, 0xF0, 0x15, 0x23, 0xD1, 0x49, 0xCD, 0x3F, 0x7F, 0x07,
];
/// Horizontal decomposition levels per component (component 2, Delta, is not transformed).
pub const LEVELS: [u8; 4] = [5, 5, 0, 5];
/// Fractional bits: decoded coefficients are shifted left by this.
pub const FQ: u32 = 4;
const NG: usize = 4;
const SS: usize = 8;

/// One wavelet band of one component, placed in the component plane the decoder fills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BandSpec {
    pub component: usize,
    pub width: usize,
    /// Lines per precinct (2 for the untransformed component, else 1).
    pub lines: usize,
    /// Row inside the precinct's two plane rows (0 = vertical low, 1 = vertical high).
    pub row: usize,
    pub x: usize,
}

/// Bands in component-major order and the Ppoc = 1 packet sequence (band, line) lists.
#[must_use]
pub fn band_layout(comp_width: usize) -> (Vec<BandSpec>, Vec<Vec<(usize, usize)>>) {
    let mut bands = Vec::new();
    let mut per_component = Vec::new();
    for (c, &levels) in LEVELS.iter().enumerate() {
        let first = bands.len();
        if levels == 0 {
            bands.push(BandSpec {
                component: c,
                width: comp_width,
                lines: 2,
                row: 0,
                x: 0,
            });
        } else {
            // Vertical-low line: [L_n, H_n, ..., H_1] of the n-level horizontal pyramid.
            let mut widths = vec![comp_width];
            for _ in 0..levels {
                let w = *widths.last().unwrap();
                widths.push(w.div_ceil(2));
            }
            let mut x = 0;
            let mut push = |width, row, x: &mut usize| {
                bands.push(BandSpec {
                    component: c,
                    width,
                    lines: 1,
                    row,
                    x: *x,
                });
                *x += width;
            };
            push(widths[levels as usize], 0, &mut x);
            for lvl in (0..levels as usize).rev() {
                push(widths[lvl] / 2, 0, &mut x);
            }
            // Vertical-high line: one horizontal level.
            let mut x = 0;
            push(comp_width.div_ceil(2), 1, &mut x);
            push(comp_width / 2, 1, &mut x);
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

/// What the random encoder may use.
#[derive(Debug, Clone)]
pub struct Options {
    /// CFA width and height (width even, height a multiple of 4).
    pub width: usize,
    pub height: usize,
    /// Precincts per slice (PIH Hsl).
    pub slice_precincts: usize,
    /// Allowed per-band coding modes D (bit 1 = significance, bit 0 = vertical prediction).
    pub modes: Vec<u8>,
    /// Probability (percent) that a packet uses raw GCLIs (Dr = 1).
    pub raw_percent: u64,
    /// Precinct quantisation Q is drawn from `0..=q_max`.
    pub q_max: u8,
    /// Qpih = 0 (deadzone) instead of 1 (uniform).
    pub deadzone: bool,
    /// Rm = 1: insignificant groups take the GTLI even with vertical prediction.
    pub run_mode: bool,
    /// Insert spare bytes after sub-packets and precinct bodies, and a COM segment.
    pub padding: bool,
    /// Largest GCLI written outside raw mode. The default 16 keeps every reconstructed
    /// sample far from i32 overflow so the expectation can use plain arithmetic.
    pub gcli_max: i32,
    /// Largest number of bit planes in a code group (at most 15).
    pub planes_max: i32,
}

impl Options {
    #[must_use]
    pub fn new(width: usize, height: usize, slice_precincts: usize) -> Self {
        Self {
            width,
            height,
            slice_precincts,
            modes: vec![0, 1, 2, 3],
            raw_percent: 15,
            q_max: 8,
            deadzone: false,
            run_mode: false,
            padding: false,
            gcli_max: 16,
            planes_max: 10,
        }
    }
}

/// An encoded codestream and the coefficients it carries.
#[derive(Debug, Clone)]
pub struct Encoded {
    pub codestream: Vec<u8>,
    /// Dequantised signed coefficients per component plane (`comp_width x comp_height`),
    /// before the `<< FQ` the decoder applies, in the decoder's plane layout.
    pub coefficients: [Vec<i32>; 4],
    pub comp_width: usize,
    pub comp_height: usize,
    pub bands: Vec<BandSpec>,
    /// (gain, priority) per band in band order.
    pub weights: Vec<(u8, u8)>,
}

/// Decoded per-band GCLI state, mirrored from what the decoder will hold.
#[derive(Debug, Clone)]
struct BandState {
    gtli: i32,
    gclis: Vec<Vec<i32>>,
}

/// Dequantises a quantised magnitude index `m` of a group with GCLI `gcli`.
#[must_use]
pub fn dequantise(m: u32, gcli: i32, gtli: i32, deadzone: bool) -> i32 {
    let v = (m as i32) << gtli;
    if gtli == 0 || v == 0 {
        return v;
    }
    if deadzone {
        return v | (1 << (gtli - 1));
    }
    let s = gcli - gtli + 1;
    let (mut r, mut x) = (0, v);
    while x > 0 {
        r += x;
        x >>= s;
    }
    r
}

/// Encodes a random frame per `opts`, deterministically from `seed`.
#[must_use]
pub fn encode_random(opts: &Options, seed: u64) -> Encoded {
    assert!(opts.width % 2 == 0 && opts.height % 4 == 0 && opts.width >= 16);
    let mut rng = Rng::new(seed);
    let (cw, ch) = (opts.width / 2, opts.height / 2);
    let (bands, packets) = band_layout(cw);
    let weights: Vec<(u8, u8)> = bands
        .iter()
        .map(|_| (rng.range(0, 3) as u8, rng.range(0, 24) as u8))
        .collect();
    // WGT lists bands in order of first appearance in the packet sequence.
    let mut wgt = Vec::new();
    let mut seen = vec![false; bands.len()];
    for &(b, _) in packets.iter().flatten() {
        if !std::mem::replace(&mut seen[b], true) {
            wgt.push(weights[b]);
        }
    }

    let mut cs = main_header(opts, &wgt, &mut rng);
    let mut coefficients: [Vec<i32>; 4] = std::array::from_fn(|_| vec![0; cw * ch]);
    let precincts = ch / 2;
    let enc = PrecinctEncoder {
        opts,
        bands: &bands,
        packets: &packets,
        weights: &weights,
        cw,
    };
    for (s, first) in (0..precincts).step_by(opts.slice_precincts).enumerate() {
        cs.extend_from_slice(&[0xFF, 0x20, 0x00, 0x04]);
        cs.extend_from_slice(&(s as u16).to_be_bytes());
        let mut top: Option<Vec<BandState>> = None;
        for p in first..(first + opts.slice_precincts).min(precincts) {
            let (bytes, state) = enc.encode(&mut rng, top.as_deref(), p, &mut coefficients);
            cs.extend_from_slice(&bytes);
            top = Some(state);
        }
    }
    cs.extend_from_slice(&[0xFF, 0x11]);
    let lcod = cs.len() as u32;
    cs[PIH_LCOD..PIH_LCOD + 4].copy_from_slice(&lcod.to_be_bytes());
    Encoded {
        codestream: cs,
        coefficients,
        comp_width: cw,
        comp_height: ch,
        bands,
        weights,
    }
}

/// Byte offset of the PIH Lcod field (SOC 2 + CAP 36 + PIH marker/length 4).
const PIH_LCOD: usize = 42;

fn main_header(opts: &Options, wgt: &[(u8, u8)], rng: &mut Rng) -> Vec<u8> {
    let mut cs = vec![0xFF, 0x10, 0xFF, 0x50, 0x00, 0x22];
    cs.extend_from_slice(b"CONTACT_INTOPIX_");
    cs.extend_from_slice(&[0xEF, 0xC0]);
    cs.extend_from_slice(&[0; 14]);
    cs.extend_from_slice(&[0xFF, 0x12, 0x00, 0x27]);
    debug_assert_eq!(cs.len(), PIH_LCOD);
    cs.extend_from_slice(&[0; 4]); // Lcod, patched once the length is known
    cs.extend_from_slice(&[0, 0, 0, 0]); // Ppih, Plev
    cs.extend_from_slice(&(opts.width as u16).to_be_bytes());
    cs.extend_from_slice(&(opts.height as u16).to_be_bytes());
    cs.extend_from_slice(&[0, 0]); // Cw
    cs.extend_from_slice(&(opts.slice_precincts as u16).to_be_bytes());
    // Nc, Ng, Ss, Bw, Fq|Br, Fslc|Ppoc|Cpih, NLx|NLy, Lh|Rl|Qpih|Fs|Rm
    let qpih = u8::from(!opts.deadzone);
    cs.extend_from_slice(&[
        4,
        NG as u8,
        SS as u8,
        18,
        (FQ as u8) << 4 | 4,
        1 << 4,
        0x51,
        qpih << 4 | 1 << 2 | u8::from(opts.run_mode),
    ]);
    cs.extend_from_slice(&NIKON_EXT);
    cs.extend_from_slice(&[0xFF, 0x13, 0x00, 0x0E]);
    for levels in LEVELS {
        cs.extend_from_slice(&[14, 0x11, if levels == 0 { 0 } else { levels << 4 | 1 }]);
    }
    cs.extend_from_slice(&[0xFF, 0x14]);
    cs.extend_from_slice(&(2 + 2 * wgt.len() as u16).to_be_bytes());
    for &(g, p) in wgt {
        cs.extend_from_slice(&[g, p]);
    }
    if opts.padding {
        let n = rng.range(0, 9) as usize;
        let text = rng.bytes(n);
        cs.extend_from_slice(&[0xFF, 0x15]);
        cs.extend_from_slice(&(4 + text.len() as u16).to_be_bytes());
        cs.extend_from_slice(&[0, 1]); // Rcom
        cs.extend_from_slice(&text);
    }
    cs
}

struct PrecinctEncoder<'a> {
    opts: &'a Options,
    bands: &'a [BandSpec],
    packets: &'a [Vec<(usize, usize)>],
    weights: &'a [(u8, u8)],
    cw: usize,
}

/// One band line chosen for a packet: decoded GCLIs, significance flags and magnitudes.
struct LineChoice {
    gclis: Vec<i32>,
    /// Per significance group: true = insignificant (no GCLI coded).
    insignificant: Vec<bool>,
    /// Vertical reference (`m_top` per code group) when the line is predicted.
    m_top: Option<Vec<i32>>,
    /// Quantised magnitudes and signs, `gclis.len() * NG` entries.
    mags: Vec<u32>,
    negative: Vec<bool>,
}

impl PrecinctEncoder<'_> {
    fn encode(
        &self,
        rng: &mut Rng,
        top: Option<&[BandState]>,
        p: usize,
        coefficients: &mut [Vec<i32>; 4],
    ) -> (Vec<u8>, Vec<BandState>) {
        let opts = self.opts;
        let q = rng.range(0, i64::from(opts.q_max)) as i32;
        let r = rng.range(0, 30) as i32;
        let modes: Vec<u8> = self
            .bands
            .iter()
            .map(|_| opts.modes[rng.below(opts.modes.len() as u64) as usize])
            .collect();
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
            let raw = rng.chance(opts.raw_percent);
            let mut lines = Vec::with_capacity(packet.len());
            for &(b, line) in packet {
                let gtli = state[b].gtli;
                let reference = if line > 0 {
                    Some((state[b].gclis[line - 1].clone(), gtli))
                } else {
                    top.map(|t| (t[b].gclis[self.bands[b].lines - 1].clone(), t[b].gtli))
                };
                let choice = self.choose_line(rng, b, gtli, modes[b], raw, reference);
                state[b].gclis[line].clone_from(&choice.gclis);
                // Expected coefficients for the decoder's plane layout.
                let band = &self.bands[b];
                let row = 2 * p + band.row + line;
                let plane = &mut coefficients[band.component];
                for i in 0..band.width {
                    let gcli = choice.gclis[i / NG];
                    let m = if gcli > gtli { choice.mags[i] } else { 0 };
                    let v = dequantise(m, gcli, gtli, opts.deadzone);
                    plane[row * self.cw + band.x + i] = if choice.negative[i] { -v } else { v };
                }
                lines.push((b, choice));
            }
            body.extend_from_slice(&self.write_packet(rng, raw, &modes, &state, &lines));
        }
        if opts.padding && rng.chance(30) {
            let n = rng.range(1, 5) as usize;
            body.extend(rng.bytes(n));
        }

        let mut out = BitWriter::default();
        out.put(body.len() as u64, 24);
        out.put(q as u64, 8);
        out.put(r as u64, 8);
        for &m in &modes {
            out.put(u64::from(m), 2);
        }
        out.align();
        let mut bytes = out.into_bytes();
        bytes.extend_from_slice(&body);
        (bytes, state)
    }

    /// Picks the decoded GCLIs (and magnitudes that fit them) for one band line.
    fn choose_line(
        &self,
        rng: &mut Rng,
        b: usize,
        gtli: i32,
        mode: u8,
        raw: bool,
        reference: Option<(Vec<i32>, i32)>,
    ) -> LineChoice {
        let gw = self.bands[b].width.div_ceil(NG);
        let vertical = !raw && mode & 1 != 0;
        let significance = !raw && mode & 2 != 0;
        let m_top: Option<Vec<i32>> = reference
            .filter(|_| vertical)
            .map(|(top, gtli_top)| top.iter().map(|&t| t.max(gtli_top.max(gtli))).collect());
        // The GCLI the decoder assigns to a group flagged insignificant.
        let implied = |g: usize| match &m_top {
            Some(m) if !self.opts.run_mode => m[g],
            _ => gtli,
        };
        let fits =
            |g: i32| g >= gtli && g <= self.opts.gcli_max && g - gtli <= self.opts.planes_max;

        let mut gclis = vec![0; gw];
        if raw {
            for g in &mut gclis {
                let room = (15 - gtli).min(self.opts.planes_max);
                *g = if room < 1 || rng.chance(30) {
                    // Any raw value at or below the GTLI codes an all-zero group.
                    rng.range(0, i64::from(gtli)) as i32
                } else {
                    gtli + rng.range(1, i64::from(room)) as i32
                };
            }
        } else {
            for run in (0..gw).step_by(SS) {
                let run = run..(run + SS).min(gw);
                let snap = significance && rng.chance(40);
                for g in run {
                    let room = (self.opts.gcli_max - gtli).min(self.opts.planes_max);
                    gclis[g] = if snap && fits(implied(g)) {
                        implied(g)
                    } else if rng.chance(25) || room < 1 {
                        gtli
                    } else if let Some(m) = m_top.as_ref().filter(|_| rng.chance(30)) {
                        // Small deltas around the prediction, both signs.
                        (m[g] + rng.range(-2, 2) as i32).clamp(gtli, gtli + room)
                    } else {
                        gtli + rng.range(1, i64::from(room)) as i32
                    };
                }
            }
        }
        let insignificant: Vec<bool> = (0..gw.div_ceil(SS))
            .map(|s| {
                significance
                    && (s * SS..((s + 1) * SS).min(gw)).all(|g| gclis[g] == implied(g))
                    && rng.chance(85)
            })
            .collect();
        let mut mags = vec![0u32; gw * NG];
        let mut negative = vec![false; gw * NG];
        for (i, (m, neg)) in mags.iter_mut().zip(&mut negative).enumerate() {
            let planes = gclis[i / NG] - gtli;
            if planes > 0 && !rng.chance(20) {
                *m = rng.below(1 << planes) as u32;
                *neg = *m != 0 && rng.chance(50);
            }
        }
        LineChoice {
            gclis,
            insignificant,
            m_top,
            mags,
            negative,
        }
    }

    fn write_packet(
        &self,
        rng: &mut Rng,
        raw: bool,
        modes: &[u8],
        state: &[BandState],
        lines: &[(usize, LineChoice)],
    ) -> Vec<u8> {
        let pad = |w: &mut BitWriter, rng: &mut Rng| {
            w.align();
            if self.opts.padding && rng.chance(25) {
                for _ in 0..rng.range(1, 3) {
                    w.put(u64::from(rng.byte()), 8);
                }
            }
        };

        // Significance sub-packet (absent in raw mode).
        let mut sig = BitWriter::default();
        if !raw {
            for (b, c) in lines {
                if modes[*b] & 2 != 0 {
                    for &f in &c.insignificant {
                        sig.bit(f);
                    }
                }
            }
            sig.align();
        }

        // GCLI sub-packet.
        let mut gcli = BitWriter::default();
        for (b, c) in lines {
            let gtli = state[*b].gtli;
            for (g, &v) in c.gclis.iter().enumerate() {
                if raw {
                    gcli.put(v as u64, 4);
                    continue;
                }
                if c.insignificant.get(g / SS).copied().unwrap_or(false) {
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
        pad(&mut gcli, rng);

        // Data sub-packet: bit planes MSB first, one nibble (4 coefficients) per plane.
        let mut data = BitWriter::default();
        for (b, c) in lines {
            let gtli = state[*b].gtli;
            for (g, &v) in c.gclis.iter().enumerate() {
                for plane in (0..(v - gtli).max(0)).rev() {
                    for k in 0..NG {
                        data.bit((c.mags[g * NG + k] >> plane) & 1 == 1);
                    }
                }
            }
        }
        pad(&mut data, rng);

        // Sign sub-packet: in-band non-zero coefficients, then the padding ones of the last
        // partial group (the decoder skips those).
        let mut sign = BitWriter::default();
        for (b, c) in lines {
            let (width, gtli) = (self.bands[*b].width, state[*b].gtli);
            let nonzero = |i: usize| c.gclis[i / NG] > gtli && c.mags[i] != 0;
            for i in (0..width).filter(|&i| nonzero(i)) {
                sign.bit(c.negative[i]);
            }
            for _ in (width..c.mags.len()).filter(|&i| nonzero(i)) {
                sign.bit(rng.chance(50));
            }
        }
        pad(&mut sign, rng);

        let (ldat, lcnt, lsgn) = (data.len_bytes(), gcli.len_bytes(), sign.len_bytes());
        assert!(ldat < 1 << 20 && lcnt < 1 << 20 && lsgn < 1 << 15);
        let mut out = BitWriter::default();
        out.put(u64::from(raw), 1);
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
