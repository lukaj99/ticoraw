//! Independent forward transforms (the encoder side of the crate's inverses): reversible
//! LeGall 5/3 analysis and the forward Star-Tetrix lifting steps.

/// Forward 1-D 5/3 lifting with whole-sample symmetric extension: `(low, high)` with
/// `ceil(n/2)` and `floor(n/2)` samples.
#[must_use]
pub fn analyze_1d(x: &[i32]) -> (Vec<i32>, Vec<i32>) {
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
pub fn analyze_row(row: &mut [i32], levels: u8) {
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

/// One vertical analysis level: rows `2p` receive the low lines and `2p + 1` the high
/// lines (`height` even).
#[must_use]
pub fn analyze_columns(plane: &[i32], width: usize) -> Vec<i32> {
    let height = plane.len() / width;
    let mut out = vec![0; plane.len()];
    for x in 0..width {
        let col: Vec<i32> = (0..height).map(|y| plane[y * width + x]).collect();
        let (l, h) = analyze_1d(&col);
        for p in 0..height / 2 {
            out[2 * p * width + x] = l[p];
            out[(2 * p + 1) * width + x] = h[p];
        }
    }
    out
}

/// Full forward wavelet of one component plane as the TicoRAW decoder expects it: one
/// vertical level, then `levels` horizontal levels on low rows and one on high rows.
#[must_use]
pub fn analyze_plane(plane: &[i32], width: usize, levels: u8) -> Vec<i32> {
    if levels == 0 {
        return plane.to_vec();
    }
    let mut out = analyze_columns(plane, width);
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

/// Forward Star-Tetrix on `[Y (G at (0,1)), Cr (R), Delta (G at (1,0)), Cb (B)]` planes:
/// the inverse's four lifting steps undone in reverse order.
pub fn forward_star_tetrix(planes: &mut [Vec<i32>; 4], w: usize, h: usize) {
    let [yp, rp, dp, bp] = planes;
    // 4. Chroma: R and B minus the average of their four green neighbours.
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
    // 3. Y update on both green sites from the four chroma neighbours.
    lift(yp, w, |y, x| {
        (at(rp, w, h, y, x) + at(rp, w, h, y, x + 1) + at(bp, w, h, y - 1, x) + at(bp, w, h, y, x))
            >> 3
    });
    lift(dp, w, |y, x| {
        (at(bp, w, h, y, x - 1) + at(bp, w, h, y, x) + at(rp, w, h, y, x) + at(rp, w, h, y + 1, x))
            >> 3
    });
    // 2. Delta: the second green minus its diagonal Y neighbours.
    lift(dp, w, |y, x| {
        -((at(yp, w, h, y, x - 1)
            + at(yp, w, h, y, x)
            + at(yp, w, h, y + 1, x - 1)
            + at(yp, w, h, y + 1, x))
            >> 2)
    });
    // 1. Average: Y plus a quarter of the diagonal Delta neighbours.
    lift(yp, w, |y, x| {
        (at(dp, w, h, y - 1, x)
            + at(dp, w, h, y - 1, x + 1)
            + at(dp, w, h, y, x)
            + at(dp, w, h, y, x + 1))
            >> 3
    });
}
