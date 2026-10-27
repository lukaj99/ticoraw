//! Reversible LeGall 5/3 inverse wavelet transform (ISO/IEC 21122-1 Annex E) with
//! whole-sample symmetric extension.
//!
//! This stage module is public for testing and advanced use; its API may change in minor
//! releases.

/// Inverse 1-D 5/3 lifting: `low` (ceil(n/2) samples) and `high` (floor(n/2)) into `out`.
pub fn synthesize_1d(low: &[i32], high: &[i32], out: &mut [i32]) {
    let n = out.len();
    debug_assert_eq!(low.len(), n - n / 2);
    debug_assert_eq!(high.len(), n / 2);
    if n == 1 {
        out[0] = low[0];
        return;
    }
    let h = |k: isize| -> i32 {
        // H[-1] = H[0]; H[len] = H[len - 1] (symmetric extension around the even samples)
        let k = k.clamp(0, high.len() as isize - 1);
        high[k as usize]
    };
    for (k, &l) in low.iter().enumerate() {
        let k = k as isize;
        out[2 * k as usize] = update(l, h(k - 1), h(k));
    }
    for k in 0..high.len() {
        let left = out[2 * k];
        let right = if 2 * k + 2 < n { out[2 * k + 2] } else { left };
        out[2 * k + 1] = predict(high[k], left, right);
    }
}

/// Even-sample step `l - ((a + b + 2) >> 2)`. Arithmetic wraps: valid streams never come
/// close to `i32` limits, and crafted ones must not panic.
#[inline]
fn update(l: i32, a: i32, b: i32) -> i32 {
    l.wrapping_sub(a.wrapping_add(b).wrapping_add(2) >> 2)
}

/// Odd-sample step `h + ((a + b) >> 1)`, wrapping like [`update`].
#[inline]
fn predict(h: i32, a: i32, b: i32) -> i32 {
    h.wrapping_add(a.wrapping_add(b) >> 1)
}

/// Horizontal synthesis of one plane row laid out as `[L_n, H_n, H_(n-1), ..., H_1]`
/// (`levels` horizontal levels). `scratch` must be at least `row.len()` long.
pub fn synthesize_row(row: &mut [i32], levels: u8, scratch: &mut Vec<i32>) {
    let w = row.len();
    let mut widths = Vec::with_capacity(levels as usize + 1);
    widths.push(w);
    for _ in 0..levels {
        let last = *widths.last().unwrap_or(&w);
        widths.push(last - last / 2);
    }
    scratch.resize(w, 0);
    for lvl in (0..levels as usize).rev() {
        let n = widths[lvl];
        let nl = widths[lvl + 1];
        let out = &mut scratch[..n];
        synthesize_1d(&row[..nl], &row[nl..n], out);
        row[..n].copy_from_slice(out);
    }
}

/// Vertical synthesis of a plane whose rows alternate low (even) and high (odd) lines, one
/// pair per precinct. Writes the reconstructed plane to `out` (same shape).
pub fn synthesize_columns(coef: &[i32], width: usize, out: &mut [i32]) {
    let height = coef.len() / width;
    let pairs = height / 2;
    let row = |r: usize| &coef[r * width..(r + 1) * width];
    // Vertical-low line of pair p is row 2p, its high line row 2p + 1 (clamped at the edges).
    let high = |p: usize| row(2 * p.min(pairs - 1) + 1);
    crate::par_chunks_mut(out, 2 * width, |p, pair| {
        let (e, o) = pair.split_at_mut(width);
        let (l0, hm, h0) = (row(2 * p), high(p.saturating_sub(1)), high(p));
        for x in 0..width {
            e[x] = update(l0[x], hm[x], h0[x]);
        }
        if p + 1 < pairs {
            let (l1, h1) = (row(2 * p + 2), high(p + 1));
            for x in 0..width {
                o[x] = predict(h0[x], e[x], update(l1[x], h0[x], h1[x]));
            }
        } else {
            for x in 0..width {
                o[x] = predict(h0[x], e[x], e[x]);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Forward 5/3 lifting, used only to check perfect reconstruction.
    fn analyze_1d(x: &[i32]) -> (Vec<i32>, Vec<i32>) {
        let n = x.len();
        let xe = |i: isize| -> i32 {
            let i = if i < 0 {
                -i
            } else if i >= n as isize {
                2 * (n as isize - 1) - i
            } else {
                i
            };
            x[i as usize]
        };
        let high: Vec<i32> = (0..n / 2)
            .map(|k| xe(2 * k as isize + 1) - ((xe(2 * k as isize) + xe(2 * k as isize + 2)) >> 1))
            .collect();
        let h = |k: isize| high[k.clamp(0, high.len() as isize - 1) as usize];
        let low: Vec<i32> = (0..n - n / 2)
            .map(|k| x[2 * k] + ((h(k as isize - 1) + h(k as isize) + 2) >> 2))
            .collect();
        (low, high)
    }

    #[test]
    fn perfect_reconstruction_all_lengths() {
        for n in 2..40 {
            let x: Vec<i32> = (0..n)
                .map(|i| ((i * 7919 + 13) % 251) as i32 - 120)
                .collect();
            let (l, h) = analyze_1d(&x);
            let mut out = vec![0; n];
            synthesize_1d(&l, &h, &mut out);
            assert_eq!(out, x, "n = {n}");
        }
    }

    #[test]
    fn multilevel_row_reconstruction() {
        let n = 95 * 32;
        let x: Vec<i32> = (0..n).map(|i| (i * 31 + 7) % 1009 - 500).collect();
        // Build [L5, H5, H4, H3, H2, H1] by repeated analysis.
        let mut highs = Vec::new();
        let mut cur = x.clone();
        for _ in 0..5 {
            let (l, h) = analyze_1d(&cur);
            highs.push(h);
            cur = l;
        }
        let mut row = cur;
        for h in highs.iter().rev() {
            row.extend_from_slice(h);
        }
        let mut scratch = Vec::new();
        synthesize_row(&mut row, 5, &mut scratch);
        assert_eq!(row, x);
    }

    #[test]
    fn vertical_reconstruction() {
        let (w, hgt) = (3, 10);
        let img: Vec<i32> = (0..w * hgt).map(|i| ((i * 37 + 5) % 97) as i32).collect();
        let mut coef = vec![0; w * hgt];
        for x in 0..w {
            let col: Vec<i32> = (0..hgt).map(|y| img[y * w + x]).collect();
            let (l, h) = analyze_1d(&col);
            for p in 0..hgt / 2 {
                coef[2 * p * w + x] = l[p];
                coef[(2 * p + 1) * w + x] = h[p];
            }
        }
        let mut out = vec![0; w * hgt];
        synthesize_columns(&coef, w, &mut out);
        assert_eq!(out, img);
    }
}
