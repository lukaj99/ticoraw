//! Band/packet layout and precinct entropy decoding (ISO/IEC 21122-1 Annex C) for the
//! TicoRAW profile: GCLI prediction, significance, bit-plane data, separate signs and
//! uniform/deadzone dequantisation.
//!
//! Output coefficients are written into one `i32` plane per component (`Hc x Wc`): row
//! `2p` holds the vertical-low line of precinct `p` (bands laid out left to right in
//! horizontal-synthesis order) and row `2p + 1` the vertical-high line. A component
//! without wavelet decomposition stores its two precinct lines in rows `2p` and `2p + 1`.
//!
//! This stage module is public for testing and advanced use; its API may change in minor
//! releases.

use crate::bitstream::BitReader;
use crate::error::{Result, TicoError};
use crate::header::MainHeader;

/// Coding-mode bit: significance sub-packet present.
const D_SIGNIFICANCE: u8 = 2;
/// Coding-mode bit: GCLIs predicted from the line above.
const D_VERTICAL: u8 = 1;
/// Maximum GTLI / bit-plane count.
const TRUNCATION_MAX: i32 = 15;

/// One wavelet band of one component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Band {
    /// Component (0..4) the band belongs to.
    pub component: usize,
    /// Number of coefficients per line.
    pub width: usize,
    /// Lines of this band in each precinct.
    pub lines: usize,
    /// Row offset inside a precinct's two plane rows (0 = vertical low, 1 = vertical high).
    pub row: usize,
    /// First column of the band inside its plane row.
    pub x: usize,
    /// WGT gain `G`.
    pub gain: u8,
    /// WGT priority `P`.
    pub priority: u8,
}

impl Band {
    fn gcli_width(&self, ng: usize) -> usize {
        self.width.div_ceil(ng)
    }
}

/// Static decoding layout derived once from the main header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Component-plane width `W` (`Wf / 2`).
    pub comp_width: usize,
    /// Component-plane height `H` (`Hf / 2`).
    pub comp_height: usize,
    /// Number of components (always 4).
    pub components: usize,
    /// Horizontal decomposition levels per component (vertical levels are 0 or 1).
    pub nlx: Vec<u8>,
    /// Vertical decomposition levels per component (0 or 1).
    pub nly: Vec<u8>,
    /// Bands in component-major order (the order of the precinct-header mode bits).
    pub bands: Vec<Band>,
    /// Packets: (band index, line within the band's precinct lines).
    pub packets: Vec<Vec<(usize, usize)>>,
    /// Precinct rows in the frame (`H / 2`).
    pub precincts: usize,
    /// Precinct rows per slice (`Hsl`, at least 1).
    pub precincts_per_slice: usize,
    /// Coefficients per code group (`Ng`).
    pub ng: usize,
    /// Code groups per significance group (`Ss`).
    pub ss: usize,
    /// Fractional bits (`Fq`): coefficients are stored shifted left by this much.
    pub fq: u32,
    /// Raw-mode GCLI width in bits.
    pub br: u32,
    /// Largest admissible GCLI (nominal coefficient bit depth).
    pub max_gcli: i32,
    /// Uniform (`Qpih = 1`) rather than deadzone dequantisation.
    pub uniform: bool,
    /// `Rm = 1`: an insignificant code group takes GTLI instead of the predicted GCLI.
    pub run_mode: bool,
}

impl Layout {
    /// Validates the header against the supported TicoRAW profile and builds the layout.
    ///
    /// # Errors
    ///
    /// [`TicoError::Unsupported`] when the header is outside the profile;
    /// [`TicoError::Header`] when the WGT length differs from the band count.
    pub fn new(h: &MainHeader) -> Result<Self> {
        let p = &h.pih;
        let unsupported = |what: String| Err(TicoError::Unsupported(what));
        if p.nc != 4 || h.components.len() != 4 {
            return unsupported(format!("{} components (TicoRAW Bayer needs 4)", p.nc));
        }
        if p.cpih != 0 || p.ppoc != 1 {
            return unsupported(format!(
                "Cpih {} / Ppoc {} (expected TicoRAW 0 / 1)",
                p.cpih, p.ppoc
            ));
        }
        if p.cw != 0 {
            return unsupported("column-split precincts (Cw != 0)".into());
        }
        if p.ng != 4 || p.ss == 0 {
            return unsupported(format!("Ng {} / Ss {}", p.ng, p.ss));
        }
        // Bound coefficient magnitudes (GCLIs are capped at Bw); the inverse transforms also
        // wrap rather than panic should a crafted stream still reach the i32 limits.
        if p.fq > 8 || p.bw > 20 || !(1..=8).contains(&p.br) {
            return unsupported(format!("Fq {} / Bw {} / Br {}", p.fq, p.bw, p.br));
        }
        if p.fs != 1 {
            return unsupported(format!(
                "Fs {} (only separate sign sub-packets are supported)",
                p.fs
            ));
        }
        if p.qpih > 1 || p.rm > 1 {
            return unsupported(format!("Qpih {} / Rm {}", p.qpih, p.rm));
        }
        let (mut nlx, mut nly) = (Vec::new(), Vec::new());
        for c in &h.components {
            if c.sx != 1 || c.sy != 1 {
                return unsupported(format!("component sampling {}x{}", c.sx, c.sy));
            }
            let (lx, ly) = match c.extension.first() {
                Some(&b) => (b >> 4, b & 15),
                None => (p.nlx, p.nly),
            };
            if ly > 1 || (ly == 0 && lx != 0) || lx > 8 {
                return unsupported(format!("decomposition NLx {lx} NLy {ly}"));
            }
            nlx.push(lx);
            nly.push(ly);
        }
        let comp_width = p.wf as usize / 2;
        let comp_height = p.hf as usize / 2;
        if comp_width < 8 || comp_height < 2 || p.wf % 2 != 0 || p.hf % 4 != 0 {
            return unsupported(format!("frame {}x{}", p.wf, p.hf));
        }

        // Bands in component-major order with their plane placement.
        let mut bands = Vec::new();
        let mut first_band = Vec::new();
        for c in 0..4 {
            first_band.push(bands.len());
            let band = |width, lines, row, x| Band {
                component: c,
                width,
                lines,
                row,
                x,
                gain: 0,
                priority: 0,
            };
            if nlx[c] == 0 {
                bands.push(band(comp_width, 2, 0, 0));
                continue;
            }
            // Widths of the horizontal pyramid of the vertical-low line.
            let mut highs = Vec::new();
            let mut w = comp_width;
            for _ in 0..nlx[c] {
                highs.push(w / 2);
                w -= w / 2;
            }
            let mut x = 0;
            bands.push(band(w, 1, 0, x));
            x += w;
            for &hw in highs.iter().rev() {
                bands.push(band(hw, 1, 0, x));
                x += hw;
            }
            let low1 = comp_width - comp_width / 2;
            bands.push(band(low1, 1, 1, 0));
            bands.push(band(comp_width / 2, 1, 1, low1));
        }
        if bands.len() != h.weights.len() {
            return Err(TicoError::Header(format!(
                "{} WGT entries for {} bands",
                h.weights.len(),
                bands.len()
            )));
        }

        // Ppoc = 1: for each precinct line, one packet per component in index order.
        let mut packets = Vec::new();
        for line in 0..2 {
            for c in 0..4 {
                let start = first_band[c];
                let end = first_band.get(c + 1).copied().unwrap_or(bands.len());
                let members: Vec<(usize, usize)> = if nlx[c] == 0 {
                    vec![(start, line)]
                } else {
                    (start..end)
                        .filter(|&b| bands[b].row == line)
                        .map(|b| (b, 0))
                        .collect()
                };
                packets.push(members);
            }
        }
        // WGT entries are listed in order of first appearance in the packet sequence.
        let mut weighted = vec![false; bands.len()];
        let mut weights = h.weights.iter();
        for &(b, _) in packets.iter().flatten() {
            if !std::mem::replace(&mut weighted[b], true) {
                let Some(&(gain, priority)) = weights.next() else {
                    return Err(TicoError::Header("WGT has fewer entries than bands".into()));
                };
                bands[b].gain = gain;
                bands[b].priority = priority;
            }
        }

        let precincts = comp_height / 2;
        Ok(Self {
            comp_width,
            comp_height,
            components: 4,
            nlx,
            nly,
            bands,
            packets,
            precincts,
            precincts_per_slice: p.hsl.max(1) as usize,
            ng: p.ng as usize,
            ss: p.ss as usize,
            fq: u32::from(p.fq),
            br: u32::from(p.br),
            max_gcli: i32::from(p.bw),
            uniform: p.qpih == 1,
            run_mode: p.rm == 1,
        })
    }

    /// Number of slices in the frame.
    #[must_use]
    pub fn slices(&self) -> usize {
        self.precincts.div_ceil(self.precincts_per_slice)
    }
}

/// Per-band GCLI state carried from one precinct to the next within a slice.
#[derive(Debug, Clone)]
struct BandState {
    gtli: i32,
    /// GCLIs of every line of this band in the current precinct.
    gclis: Vec<Vec<i32>>,
}

/// Decodes every precinct of one slice into its rows of the component planes.
///
/// `data` starts right after the slice's SLH marker; `planes[c]` covers exactly the rows of
/// this slice. Returns the number of bytes consumed.
///
/// # Errors
///
/// [`TicoError::Truncated`] or [`TicoError::Entropy`] when the precinct data is shorter
/// than declared or inconsistent with it.
pub fn decode_slice(
    layout: &Layout,
    data: &[u8],
    first_precinct: usize,
    planes: &mut [&mut [i32]],
) -> Result<usize> {
    let n_prec = layout
        .precincts_per_slice
        .min(layout.precincts - first_precinct);
    let mut pos = 0;
    let mut top: Option<Vec<BandState>> = None;
    for p in 0..n_prec {
        let (state, used) = decode_precinct(layout, &data[pos..], top.as_deref(), p, planes)?;
        pos += used;
        top = Some(state);
    }
    Ok(pos)
}

fn decode_precinct(
    layout: &Layout,
    data: &[u8],
    top: Option<&[BandState]>,
    p: usize,
    planes: &mut [&mut [i32]],
) -> Result<(Vec<BandState>, usize)> {
    let nb = layout.bands.len();
    let header_len = 5 + (2 * nb).div_ceil(8);
    if data.len() < header_len {
        return Err(TicoError::Truncated {
            what: "precinct header",
            offset: 0,
        });
    }
    let mut r = BitReader::new(&data[..header_len]);
    let lprc = r.read(24)? as usize;
    let q = r.read(8)? as i32;
    let rf = r.read(8)? as i32;
    let mut modes = Vec::with_capacity(nb);
    for _ in 0..nb {
        modes.push(r.read(2)? as u8);
    }
    let body = data
        .get(header_len..header_len + lprc)
        .ok_or(TicoError::Truncated {
            what: "precinct body",
            offset: header_len,
        })?;

    let mut state: Vec<BandState> = layout
        .bands
        .iter()
        .map(|b| {
            let pump = i32::from(i32::from(b.priority) < rf);
            let g = i32::from(b.gain) + pump;
            let gtli = if q < g {
                0
            } else {
                (q - g).min(TRUNCATION_MAX)
            };
            BandState {
                gtli,
                gclis: vec![Vec::new(); b.lines],
            }
        })
        .collect();

    let mut r = BitReader::new(body);
    let mut sig = Vec::new();
    let mut coeffs = Vec::new();
    for packet in &layout.packets {
        // TicoRAW always writes long 7-byte packet headers: Dr 1, Ldat 20, Lcnt 20, Lsgn 15.
        let ph = r.read(8)? as u64;
        let rest = u64::from(r.read(24)?) << 24 | u64::from(r.read(24)?);
        let hdr = ph << 48 | rest;
        let raw = hdr >> 55 & 1 == 1;
        let ldat = (hdr >> 35 & 0xF_FFFF) as usize;
        let lcnt = (hdr >> 15 & 0xF_FFFF) as usize;
        let lsgn = (hdr & 0x7FFF) as usize;

        // Significance sub-packet.
        let mut sig_flags: Vec<Vec<bool>> = vec![Vec::new(); packet.len()];
        if !raw {
            for (i, &(b, _)) in packet.iter().enumerate() {
                if modes[b] & D_SIGNIFICANCE != 0 {
                    let n = layout.bands[b].gcli_width(layout.ng).div_ceil(layout.ss);
                    sig.clear();
                    for _ in 0..n {
                        sig.push(r.bit()?);
                    }
                    sig_flags[i].clone_from(&sig);
                }
            }
            r.align();
        }

        // GCLI sub-packet.
        let start = r.byte_pos();
        for (i, &(b, line)) in packet.iter().enumerate() {
            let band = &layout.bands[b];
            let gw = band.gcli_width(layout.ng);
            let gtli = state[b].gtli;
            let gclis = if raw {
                (0..gw)
                    .map(|_| Ok(r.read(layout.br)? as i32))
                    .collect::<Result<Vec<_>>>()?
            } else {
                // Reference line: previous line of this precinct, else last line of the
                // previous precinct in the slice; none at a slice start (zero prediction).
                let reference = if line > 0 {
                    Some((&state[b].gclis[line - 1][..], gtli))
                } else {
                    top.map(|t| (&t[b].gclis[band.lines - 1][..], t[b].gtli))
                };
                let vertical = modes[b] & D_VERTICAL != 0;
                let flags = if modes[b] & D_SIGNIFICANCE != 0 {
                    Some(&sig_flags[i][..])
                } else {
                    None
                };
                decode_gclis(
                    &mut r,
                    gw,
                    gtli,
                    layout,
                    flags,
                    if vertical { reference } else { None },
                )?
            };
            state[b].gclis[line] = gclis;
        }
        r.align();
        skip_to(&mut r, start, lcnt, "GCLI")?;

        // Data sub-packet: magnitudes, dequantised in place.
        let start = r.byte_pos();
        let mut leftovers = vec![0usize; packet.len()];
        coeffs.clear();
        for (i, &(b, line)) in packet.iter().enumerate() {
            let band = &layout.bands[b];
            let gtli = state[b].gtli;
            let mut out = vec![0i32; band.width];
            leftovers[i] = decode_data(&mut r, &state[b].gclis[line], gtli, layout, &mut out)?;
            coeffs.push(out);
        }
        r.align();
        skip_to(&mut r, start, ldat, "data")?;

        // Sign sub-packet (Fs = 1).
        let start = r.byte_pos();
        for (i, out) in coeffs.iter_mut().enumerate() {
            for v in out.iter_mut().filter(|v| **v != 0) {
                if r.bit()? {
                    *v = -*v;
                }
            }
            r.skip(leftovers[i])?;
        }
        r.align();
        skip_to(&mut r, start, lsgn, "sign")?;

        for (&(b, line), out) in packet.iter().zip(&coeffs) {
            let band = &layout.bands[b];
            let row = 2 * p + band.row + line;
            let plane = &mut planes[band.component];
            let dst = &mut plane[row * layout.comp_width + band.x..][..band.width];
            for (d, &v) in dst.iter_mut().zip(out) {
                *d = v << layout.fq;
            }
        }
    }
    Ok((state, header_len + lprc))
}

fn skip_to(r: &mut BitReader<'_>, start: usize, len: usize, what: &str) -> Result<()> {
    let used = r.byte_pos() - start;
    if used > len {
        return Err(TicoError::Entropy(format!(
            "{what} sub-packet overran its {len} bytes (read {used})"
        )));
    }
    r.skip((len - used) * 8)
}

/// Decodes one band line of GCLIs (non-raw mode).
fn decode_gclis(
    r: &mut BitReader<'_>,
    gw: usize,
    gtli: i32,
    layout: &Layout,
    significance: Option<&[bool]>,
    reference: Option<(&[i32], i32)>,
) -> Result<Vec<i32>> {
    let mut out = Vec::with_capacity(gw);
    for g in 0..gw {
        let insignificant = significance.is_some_and(|s| s[g / layout.ss]);
        let v = match reference {
            None => {
                if insignificant {
                    gtli
                } else {
                    gtli + r.unary(64)? as i32
                }
            }
            Some((top, gtli_top)) => {
                let t = gtli_top.max(gtli);
                let m_top = top[g].max(t);
                if insignificant {
                    if layout.run_mode { gtli } else { m_top }
                } else {
                    let x = r.unary(64)? as i32;
                    let thr = (m_top - gtli).max(0);
                    let delta = if x > 2 * thr {
                        x - thr
                    } else if x & 1 == 1 {
                        -(x + 1) / 2
                    } else {
                        x / 2
                    };
                    m_top + delta
                }
            }
        };
        out.push(v);
    }
    Ok(out)
}

/// Decodes one band line of bit-plane data into dequantised magnitudes; returns the number
/// of non-zero coefficients that fall past the band end (their signs must be skipped).
fn decode_data(
    r: &mut BitReader<'_>,
    gclis: &[i32],
    gtli: i32,
    layout: &Layout,
    out: &mut [i32],
) -> Result<usize> {
    let ng = layout.ng;
    let mut leftover = 0;
    for (g, &gcli) in gclis.iter().enumerate() {
        let planes = gcli - gtli;
        if planes <= 0 {
            continue;
        }
        if planes > TRUNCATION_MAX || gcli > layout.max_gcli {
            return Err(TicoError::Entropy(format!(
                "{planes} bit planes in a code group"
            )));
        }
        let mut mag = [0i32; 4];
        for _ in 0..planes {
            let nib = r.read(4)? as i32;
            for (k, m) in mag.iter_mut().enumerate() {
                *m = (*m << 1) | (nib >> (3 - k) & 1);
            }
        }
        for (k, &m) in mag.iter().enumerate() {
            let idx = g * ng + k;
            let v = dequantise(m << gtli, gcli, gtli, layout.uniform);
            if idx < out.len() {
                out[idx] = v;
            } else if v != 0 {
                leftover += 1;
            }
        }
    }
    Ok(leftover)
}

#[inline]
fn dequantise(v: i32, gcli: i32, gtli: i32, uniform: bool) -> i32 {
    if gtli == 0 || v == 0 {
        return v;
    }
    if uniform {
        let s = gcli - gtli + 1;
        let (mut acc, mut x) = (0, v);
        while x > 0 {
            acc += x;
            x >>= s;
        }
        acc
    } else {
        v | 1 << (gtli - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_dequantisation_matches_reference() {
        // 1 plane above GTLI 3: v = 1 << 3 = 8, s = 2 -> 8 + 2 = 10
        assert_eq!(dequantise(8, 4, 3, true), 10);
        assert_eq!(dequantise(8, 4, 0, true), 8);
        assert_eq!(dequantise(0, 4, 3, true), 0);
        // deadzone sets the half-step bit
        assert_eq!(dequantise(8, 4, 3, false), 12);
    }
}
