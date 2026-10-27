//! Preview rendering without demosaicing: each 2x2 CFA cell becomes one sRGB pixel.
//!
//! Camera RGB (black-subtracted, scaled to the white level) is white balanced with
//! `AsShotNeutral`, taken to XYZ with the inverse of the D65 `ColorMatrix` and on to linear
//! sRGB, scaled by `BaselineExposure`, and encoded with the sRGB transfer curve to 8 bits.
//! Editors re-render from the raw data anyway; this only has to look right in file browsers.

use ticoraw::DecodedRaw;

/// An 8-bit interleaved RGB image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rgb8 {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) data: Vec<u8>,
}

/// D65 white point in XYZ (Y = 1).
const D65: [f64; 3] = [0.950_47, 1.0, 1.088_83];
/// Linear sRGB from XYZ (D65), IEC 61966-2-1.
const XYZ_TO_SRGB: [[f64; 3]; 3] = [
    [3.240_454_2, -1.537_138_5, -0.498_531_4],
    [-0.969_266_0, 1.876_010_8, 0.041_556_0],
    [0.055_643_4, -0.204_025_9, 1.057_225_2],
];
/// Entries of the linear-to-sRGB lookup table.
const LUT_SIZE: usize = 4096;

type Mat3 = [[f64; 3]; 3];

fn mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut m = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    m
}

fn invert(m: &Mat3) -> Option<Mat3> {
    let c = |r: usize, s: usize| {
        let (r1, r2) = ((r + 1) % 3, (r + 2) % 3);
        let (s1, s2) = ((s + 1) % 3, (s + 2) % 3);
        m[r1][s1] * m[r2][s2] - m[r1][s2] * m[r2][s1]
    };
    let det: f64 = (0..3).map(|j| m[0][j] * c(0, j)).sum();
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let mut inv = [[0.0; 3]; 3];
    for (i, row) in inv.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = c(j, i) / det;
        }
    }
    Some(inv)
}

fn ratio<T: Into<f64>>((n, d): (T, T)) -> f64 {
    let d = d.into();
    if d == 0.0 { 0.0 } else { n.into() / d }
}

/// Matrix from black-subtracted camera RGB (in units of the white level) to linear sRGB.
fn camera_to_srgb(raw: &DecodedRaw) -> Mat3 {
    let c = &raw.color;
    // White balance: divide by the camera neutral, normalised to green = 1.
    let neutral = c
        .as_shot_neutral
        .map_or([1.0; 3], |n| n.map(ratio))
        .map(|v| if v.is_finite() && v > 0.0 { v } else { 1.0 });
    let wb = [1.0 / neutral[0], 1.0 / neutral[1], 1.0 / neutral[2]].map(|v| v * neutral[1]);
    let balance = [[wb[0], 0.0, 0.0], [0.0, wb[1], 0.0], [0.0, 0.0, wb[2]]];
    let ev = raw.baseline_exposure.map_or(0.0, ratio).clamp(-8.0, 8.0);
    let gain = 2f64.powf(ev);
    let to_srgb = c.color_matrix.and_then(|cm| {
        // XYZ -> camera, with rows scaled so D65 white maps to camera (1, 1, 1): balanced
        // camera white then lands on D65 white.
        let mut m = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] = ratio(cm[3 * i + j]);
            }
            let w: f64 = (0..3).map(|j| m[i][j] * D65[j]).sum();
            if !w.is_finite() || w.abs() < 1e-9 {
                return None;
            }
            for v in &mut m[i] {
                *v /= w;
            }
        }
        invert(&m).map(|cam_to_xyz| mul(&XYZ_TO_SRGB, &cam_to_xyz))
    });
    // Without a usable matrix, treat camera RGB as sRGB primaries.
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let m = mul(&to_srgb.unwrap_or(identity), &balance);
    m.map(|row| row.map(|v| v * gain))
}

fn srgb_lut() -> Vec<u8> {
    (0..LUT_SIZE)
        .map(|i| {
            let l = i as f64 / (LUT_SIZE - 1) as f64;
            let v = if l <= 0.003_130_8 {
                12.92 * l
            } else {
                1.055 * l.powf(1.0 / 2.4) - 0.055
            };
            (v * 255.0 + 0.5) as u8
        })
        .collect()
}

/// Renders the default crop (the whole mosaic without one) at half resolution.
pub(crate) fn half_size(raw: &DecodedRaw) -> Rgb8 {
    let img = &raw.image;
    let (x0, y0, cw, ch) = raw.crop.map_or((0, 0, img.width, img.height), |c| {
        (c.x, c.y, c.width, c.height)
    });
    let (width, height) = ((cw / 2).max(1), (ch / 2).max(1));
    let m = camera_to_srgb(raw).map(|r| r.map(|v| v as f32));
    let black = raw.color.black_level.map(f32::from);
    let mean_black = black.iter().sum::<f32>() / 4.0;
    let scale = 1.0 / (f32::from(raw.color.white_level) - mean_black).max(1.0);
    let lut = srgb_lut();
    let mut data = vec![0u8; width * height * 3];
    crate::par_rows(&mut data, width * 3, |j, row| {
        for (i, px) in row.chunks_exact_mut(3).enumerate() {
            let mut sum = [0f32; 3];
            let mut n = [0u32; 3];
            for (dy, dx) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                let x = (x0 + 2 * i + dx).min(img.width - 1);
                let y = (y0 + 2 * j + dy).min(img.height - 1);
                let pos = (y & 1) * 2 + (x & 1);
                let ch = usize::from(raw.color.cfa_pattern[pos]).min(2);
                sum[ch] += (f32::from(img.pixels[y * img.width + x]) - black[pos]).max(0.0);
                n[ch] += 1;
            }
            let cam: [f32; 3] = std::array::from_fn(|c| {
                if n[c] == 0 {
                    0.0
                } else {
                    sum[c] / n[c] as f32 * scale
                }
            });
            for (out, r) in px.iter_mut().zip(&m) {
                let lin = r[0] * cam[0] + r[1] * cam[1] + r[2] * cam[2];
                let idx = (lin.clamp(0.0, 1.0) * (LUT_SIZE - 1) as f32 + 0.5) as usize;
                *out = lut[idx.min(LUT_SIZE - 1)];
            }
        }
    });
    Rgb8 {
        width,
        height,
        data,
    }
}

/// Box-filters `img` so its long side is at most `long_side` pixels, keeping the aspect
/// ratio (never upscaling).
pub(crate) fn downscale(img: &Rgb8, long_side: usize) -> Rgb8 {
    let long = img.width.max(img.height);
    if long <= long_side {
        return img.clone();
    }
    let dim = |n: usize| ((n * long_side + long / 2) / long).max(1);
    let (width, height) = (dim(img.width), dim(img.height));
    let mut data = vec![0u8; width * height * 3];
    for (j, row) in data.chunks_exact_mut(width * 3).enumerate() {
        let (sy0, sy1) = (
            j * img.height / height,
            ((j + 1) * img.height / height).max(j * img.height / height + 1),
        );
        for (i, px) in row.chunks_exact_mut(3).enumerate() {
            let (sx0, sx1) = (
                i * img.width / width,
                ((i + 1) * img.width / width).max(i * img.width / width + 1),
            );
            let mut sum = [0u32; 3];
            for y in sy0..sy1.min(img.height) {
                for x in sx0..sx1.min(img.width) {
                    let p = &img.data[(y * img.width + x) * 3..][..3];
                    for c in 0..3 {
                        sum[c] += u32::from(p[c]);
                    }
                }
            }
            let n = ((sy1.min(img.height) - sy0) * (sx1.min(img.width) - sx0)).max(1) as u32;
            for c in 0..3 {
                px[c] = ((sum[c] + n / 2) / n) as u8;
            }
        }
    }
    Rgb8 {
        width,
        height,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ticoraw::{CfaImage, ColorInfo, Crop};

    fn grey_raw(level: u16) -> DecodedRaw {
        let (w, h) = (8, 6);
        DecodedRaw {
            make: "NIKON CORPORATION".into(),
            model: "NIKON Z6_3".into(),
            image: CfaImage {
                width: w,
                height: h,
                pixels: (0..w * h)
                    .map(|i| {
                        // A neutral grey under the as-shot illuminant: camera = neutral * k.
                        let (x, y) = (i % w, i / w);
                        let k = f64::from(level - 1008);
                        let n = match (y & 1, x & 1) {
                            (0, 0) => 512.0 / 924.0,
                            (1, 1) => 512.0 / 717.0,
                            _ => 1.0,
                        };
                        (1008.0 + k * n).round() as u16
                    })
                    .collect(),
            },
            color: ColorInfo {
                cfa_pattern: [0, 1, 1, 2],
                black_level: [1008; 4],
                white_level: 15892,
                color_matrix: Some(
                    [11206, -4286, -941, -4879, 12847, 2251, -745, 1654, 7374].map(|v| (v, 10000)),
                ),
                as_shot_neutral: Some([(512, 924), (1, 1), (512, 717)]),
            },
            orientation: 1,
            crop: Some(Crop {
                x: 2,
                y: 2,
                width: 4,
                height: 4,
            }),
            baseline_exposure: Some((0, 100)),
        }
    }

    #[test]
    fn neutral_grey_renders_grey() {
        let img = half_size(&grey_raw(1008 + 3000));
        assert_eq!((img.width, img.height), (2, 2));
        for px in img.data.chunks_exact(3) {
            let (lo, hi) = (*px.iter().min().unwrap(), *px.iter().max().unwrap());
            assert!(hi - lo <= 2, "grey pixel {px:?}");
            assert!((100..200).contains(&lo), "{px:?}");
        }
        let black = half_size(&grey_raw(1008));
        assert!(black.data.iter().all(|&v| v == 0));
    }

    #[test]
    fn hostile_metadata_does_not_panic() {
        let mut raw = grey_raw(5000);
        raw.color.color_matrix = Some([(0, 0); 9]);
        raw.color.as_shot_neutral = Some([(0, 0), (1, 0), (5, 1)]);
        raw.color.white_level = 0;
        raw.color.cfa_pattern = [9, 9, 9, 9];
        raw.baseline_exposure = Some((i32::MAX, 1));
        raw.crop = None;
        let img = half_size(&raw);
        assert_eq!((img.width, img.height), (4, 3));
        raw.image = CfaImage {
            width: 1,
            height: 1,
            pixels: vec![9],
        };
        assert_eq!(half_size(&raw).data.len(), 3);
    }

    #[test]
    fn downscale_keeps_aspect_and_averages() {
        let img = Rgb8 {
            width: 600,
            height: 400,
            data: (0..600 * 400 * 3)
                .map(|i| if i % 2 == 0 { 100 } else { 200 })
                .collect(),
        };
        let small = downscale(&img, 256);
        assert_eq!((small.width, small.height), (256, 171));
        assert!(small.data.iter().all(|&v| (99..=201).contains(&v)));
        assert_eq!(downscale(&small, 256), small);
    }

    #[test]
    fn matrix_inverse_round_trips() {
        let m = [[2.0, 1.0, 0.0], [0.0, 1.0, 3.0], [1.0, 0.0, 1.0]];
        let p = mul(&m, &invert(&m).unwrap());
        for (i, row) in p.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                assert!((v - f64::from(u8::from(i == j))).abs() < 1e-12);
            }
        }
        assert!(invert(&[[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [0.0, 0.0, 1.0]]).is_none());
    }
}
