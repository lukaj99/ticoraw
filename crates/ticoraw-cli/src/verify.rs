//! `ticoraw verify`: sample-exact comparison of a decoded NEF against a reference DNG.

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use ticoraw::CfaImage;
use ticoraw::tiff::{self, Tiff};

use crate::args::{Arg, Args};
use crate::{CliError, cfa_color, ms, read_input, unexpected};

pub const HELP: &str = "\
Compare a decoded NEF sample-for-sample against a reference DNG

Usage: ticoraw verify [options] <file.nef> <reference.dng>

Decodes the NEF and the reference DNG's full-resolution CFA image (lossless JPEG or
uncompressed) and compares every sample. Prints the mismatch count, the largest absolute
difference and mismatches per 2x2 CFA site.

Options:
  -h, --help   Show this help

Exit status: 0 only when the images are bit-exact, 1 otherwise, 2 on usage errors.
";

#[derive(Debug)]
pub struct Options {
    pub nef: PathBuf,
    pub reference: PathBuf,
}

/// Returns `None` when help was requested.
pub fn parse(args: &mut Args) -> Result<Option<Options>, String> {
    let mut paths = Vec::new();
    while let Some(arg) = args.next_arg()? {
        match arg {
            Arg::Opt(s) if s == "-h" || s == "--help" => return Ok(None),
            Arg::Opt(s) => return Err(format!("unknown option {s}")),
            Arg::Pos(p) if paths.len() < 2 => paths.push(PathBuf::from(p)),
            Arg::Pos(p) => return Err(unexpected(&p)),
        }
    }
    let mut paths = paths.into_iter();
    match (paths.next(), paths.next()) {
        (Some(nef), Some(reference)) => Ok(Some(Options { nef, reference })),
        (Some(_), None) => Err("missing reference DNG".into()),
        _ => Err("missing NEF and reference DNG".into()),
    }
}

/// Where two images first differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mismatch {
    pub x: usize,
    pub y: usize,
    pub ours: u16,
    pub reference: u16,
}

/// Result of comparing two equally sized CFA images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub samples: usize,
    pub mismatches: usize,
    pub max_abs_diff: u16,
    /// Mismatches per 2x2 CFA site, indexed `(y % 2) * 2 + x % 2`.
    pub per_site: [usize; 4],
    pub first: Option<Mismatch>,
}

/// The two images have different (width, height).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeMismatch {
    pub ours: (usize, usize),
    pub reference: (usize, usize),
}

/// Compares two CFA images sample by sample.
pub fn compare(ours: &CfaImage, reference: &CfaImage) -> Result<Comparison, SizeMismatch> {
    let (w, h) = (ours.width, ours.height);
    if (w, h) != (reference.width, reference.height) {
        return Err(SizeMismatch {
            ours: (w, h),
            reference: (reference.width, reference.height),
        });
    }
    let mut c = Comparison {
        samples: w * h,
        mismatches: 0,
        max_abs_diff: 0,
        per_site: [0; 4],
        first: None,
    };
    let rows = ours
        .pixels
        .chunks_exact(w.max(1))
        .zip(reference.pixels.chunks_exact(w.max(1)));
    for (y, (a, b)) in rows.enumerate() {
        if a == b {
            continue;
        }
        for (x, (&p, &q)) in a.iter().zip(b).enumerate() {
            if p != q {
                c.mismatches += 1;
                c.per_site[(y % 2) * 2 + x % 2] += 1;
                c.max_abs_diff = c.max_abs_diff.max(p.abs_diff(q));
                c.first.get_or_insert(Mismatch {
                    x,
                    y,
                    ours: p,
                    reference: q,
                });
            }
        }
    }
    Ok(c)
}

pub fn run(o: &Options) -> Result<ExitCode, CliError> {
    let nef = read_input(&o.nef)?;
    let dng = read_input(&o.reference)?;
    let t = Instant::now();
    let raw = ticoraw::decode_nef(&nef)
        .map_err(|e| CliError::Failure(format!("{}: {e}", o.nef.display())))?;
    let t_nef = t.elapsed();
    let t = Instant::now();
    let truth = ticoraw::dng::decode_dng_cfa(&dng)
        .map_err(|e| CliError::Failure(format!("{}: {e}", o.reference.display())))?;
    let t_dng = t.elapsed();

    let mut out = io::stdout().lock();
    writeln!(out, "nef        {}", o.nef.display())?;
    writeln!(out, "reference  {}", o.reference.display())?;
    if let Some(name) = original_raw_file_name(&dng) {
        writeln!(out, "           (OriginalRawFileName {name})")?;
        let ours = o.nef.file_name().map(|n| n.to_string_lossy().into_owned());
        if ours.is_some_and(|n| !n.eq_ignore_ascii_case(&name)) {
            writeln!(
                out,
                "note: the reference was converted from a different file name"
            )?;
        }
    }
    writeln!(out, "decode     NEF {}, DNG {}", ms(t_nef), ms(t_dng))?;
    let pattern = raw.color.cfa_pattern;
    if pattern != truth.cfa_pattern {
        writeln!(
            out,
            "note: CFA patterns differ (NEF {}, DNG {})",
            pattern.map(cfa_color).iter().collect::<String>(),
            truth.cfa_pattern.map(cfa_color).iter().collect::<String>()
        )?;
    }
    if raw.color.black_level != truth.black_level || raw.color.white_level != truth.white_level {
        writeln!(
            out,
            "note: levels differ (NEF black {:?} white {}, DNG black {:?} white {})",
            raw.color.black_level, raw.color.white_level, truth.black_level, truth.white_level
        )?;
    }

    let c = match compare(&raw.image, &truth.image) {
        Ok(c) => c,
        Err(SizeMismatch {
            ours: (w, h),
            reference: (rw, rh),
        }) => {
            writeln!(out, "FAIL: size differs (NEF {w} x {h}, DNG {rw} x {rh})")?;
            return Ok(ExitCode::FAILURE);
        }
    };
    writeln!(
        out,
        "samples    {} ({} x {})",
        c.samples, raw.image.width, raw.image.height
    )?;
    writeln!(out, "mismatches {}", c.mismatches)?;
    writeln!(out, "max |diff| {}", c.max_abs_diff)?;
    let sites: Vec<String> = (0..4)
        .map(|i| {
            format!(
                "{}({},{})={}",
                cfa_color(pattern[i]),
                i / 2,
                i % 2,
                c.per_site[i]
            )
        })
        .collect();
    writeln!(out, "per site   {}", sites.join(" "))?;
    if let Some(m) = c.first {
        writeln!(
            out,
            "first      x={} y={} ours={} reference={}",
            m.x, m.y, m.ours, m.reference
        )?;
    }
    if c.mismatches == 0 {
        writeln!(out, "OK: bit-exact")?;
        Ok(ExitCode::SUCCESS)
    } else {
        writeln!(
            out,
            "FAIL: {} of {} samples differ",
            c.mismatches, c.samples
        )?;
        Ok(ExitCode::FAILURE)
    }
}

/// DNG tag 0xC68B (OriginalRawFileName) from IFD0, if present.
fn original_raw_file_name(dng: &[u8]) -> Option<String> {
    let t = Tiff::parse(dng, 0).ok()?;
    let ifds = tiff::all_ifds(&t).ok()?;
    let e = ifds.first()?.get(0xC68B)?;
    Some(t.ascii(e)).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(width: usize, pixels: Vec<u16>) -> CfaImage {
        CfaImage {
            width,
            height: pixels.len() / width,
            pixels,
        }
    }

    #[test]
    fn identical_images() {
        let a = img(2, vec![1, 2, 3, 4]);
        let c = compare(&a, &a.clone()).unwrap();
        assert_eq!(c.mismatches, 0);
        assert_eq!(c.samples, 4);
        assert_eq!(c.first, None);
    }

    #[test]
    fn counts_per_site() {
        let a = img(4, vec![0, 0, 0, 0, 0, 0, 0, 0]);
        let b = img(4, vec![0, 5, 0, 1, 0, 0, 0, 9]);
        let c = compare(&a, &b).unwrap();
        assert_eq!(c.mismatches, 3);
        assert_eq!(c.max_abs_diff, 9);
        assert_eq!(c.per_site, [0, 2, 0, 1]);
        assert_eq!(
            c.first,
            Some(Mismatch {
                x: 1,
                y: 0,
                ours: 0,
                reference: 5
            })
        );
    }

    #[test]
    fn size_mismatch() {
        let a = img(2, vec![0; 4]);
        let b = img(4, vec![0; 4]);
        assert_eq!(
            compare(&a, &b),
            Err(SizeMismatch {
                ours: (2, 2),
                reference: (4, 1)
            })
        );
    }
}
