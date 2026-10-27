//! Inverse Star-Tetrix multi-component transform (ISO/IEC 21122-1 Annex F lifting steps,
//! with the fixed parameters TicoRAW uses: e1 = e2 = 0, full vertical extent, no chroma
//! gains) and assembly of the RGGB mosaic through the vendor transfer curve.
//!
//! Component roles in codestream order: 0 = Y (green at quad (0,1)), 1 = Cr (red at
//! (0,0)), 2 = Delta (green at (1,0)), 3 = Cb (blue at (1,1)). Neighbour lookups reflect at
//! the CFA grid edges, which in plane coordinates is index clamping.
//!
//! This stage module is public for testing and advanced use; its API may change in minor
//! releases.

use crate::header::TransferCurve;

/// Plane accessor with edge clamping.
#[derive(Clone, Copy)]
struct Plane<'a> {
    data: &'a [i32],
    w: usize,
    h: usize,
}

impl Plane<'_> {
    #[inline]
    fn at(&self, y: isize, x: isize) -> i32 {
        let y = y.clamp(0, self.h as isize - 1) as usize;
        let x = x.clamp(0, self.w as isize - 1) as usize;
        self.data[y * self.w + x]
    }
}

/// Sum of four taps. Arithmetic wraps: valid streams stay far from `i32` limits, and
/// crafted ones must not panic.
#[inline]
fn sum4(a: i32, b: i32, c: i32, d: i32) -> i32 {
    a.wrapping_add(b).wrapping_add(c).wrapping_add(d)
}

/// Runs one lifting step: `target[y][x] += sign * (f(y, x) >> shift)` for every sample.
fn lift(target: &mut [i32], w: usize, f: impl Fn(isize, isize) -> i32 + Sync + Send) {
    crate::par_chunks_mut(target, w, |y, row| {
        for (x, v) in row.iter_mut().enumerate() {
            *v = v.wrapping_add(f(y as isize, x as isize));
        }
    });
}

/// Inverts Star-Tetrix in place on the four component planes `[Y, Cr, Delta, Cb]`.
pub fn inverse_star_tetrix(planes: &mut [Vec<i32>; 4], w: usize, h: usize) {
    let [yp, rp, dp, bp] = planes;
    // 1. Undo the average step on Y sites (diagonal Delta neighbours).
    {
        let d = Plane { data: dp, w, h };
        lift(yp, w, |y, x| {
            -(sum4(
                d.at(y - 1, x),
                d.at(y - 1, x + 1),
                d.at(y, x),
                d.at(y, x + 1),
            ) >> 3)
        });
    }
    // 2. Undo the delta step on Delta sites (diagonal Y neighbours, already restored).
    {
        let g = Plane { data: yp, w, h };
        lift(dp, w, |y, x| {
            sum4(
                g.at(y, x - 1),
                g.at(y, x),
                g.at(y + 1, x - 1),
                g.at(y + 1, x),
            ) >> 2
        });
    }
    // 3. Undo the Y update on both green sites (4-neighbour chroma).
    {
        let r = Plane { data: rp, w, h };
        let b = Plane { data: bp, w, h };
        lift(yp, w, |y, x| {
            -(sum4(r.at(y, x), r.at(y, x + 1), b.at(y - 1, x), b.at(y, x)) >> 3)
        });
        lift(dp, w, |y, x| {
            -(sum4(b.at(y, x - 1), b.at(y, x), r.at(y, x), r.at(y + 1, x)) >> 3)
        });
    }
    // 4. Undo the chroma step on R and B sites (4-neighbour greens).
    {
        let g = Plane { data: yp, w, h };
        let d = Plane { data: dp, w, h };
        lift(rp, w, |y, x| {
            sum4(g.at(y, x - 1), g.at(y, x), d.at(y - 1, x), d.at(y, x)) >> 2
        });
        lift(bp, w, |y, x| {
            sum4(d.at(y, x), d.at(y, x + 1), g.at(y, x), g.at(y + 1, x)) >> 2
        });
    }
}

/// Interleaves the restored planes into an RGGB mosaic of `2w x 2h` through `curve`.
#[must_use]
pub fn assemble_rggb(
    planes: &[Vec<i32>; 4],
    w: usize,
    h: usize,
    curve: &TransferCurve,
) -> Vec<u16> {
    let mut out = vec![0u16; 4 * w * h];
    let [yp, rp, dp, bp] = planes;
    crate::par_chunks_mut(&mut out, 4 * w, |y, pair| {
        let (top, bottom) = pair.split_at_mut(2 * w);
        let at = y * w;
        for x in 0..w {
            top[2 * x] = curve.apply(rp[at + x]);
            top[2 * x + 1] = curve.apply(yp[at + x]);
            bottom[2 * x] = curve.apply(dp[at + x]);
            bottom[2 * x + 1] = curve.apply(bp[at + x]);
        }
    });
    debug_assert_eq!(out.len(), 4 * w * h);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_field_is_invariant() {
        // With all chroma and delta at zero and Y constant, the inverse leaves greens at Y
        // and restores R/B to the green average.
        let (w, h) = (5, 4);
        let mut planes = [
            vec![100; w * h],
            vec![0; w * h],
            vec![0; w * h],
            vec![0; w * h],
        ];
        inverse_star_tetrix(&mut planes, w, h);
        assert!(planes[0].iter().all(|&v| v == 100));
        assert!(planes[2].iter().all(|&v| v == 100));
        assert!(planes[1].iter().all(|&v| v == 100));
        assert!(planes[3].iter().all(|&v| v == 100));
    }
}
