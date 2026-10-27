//! `ticoraw inspect`: container and codestream summary without decoding pixels.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ticoraw::{CodestreamInfo, NefInspection, NikonCompression, RawCodec};

use crate::CliError;
use crate::args::{Arg, Args};
use crate::json::Json;

pub const HELP: &str = "\
Describe NEF containers and TicoRAW codestreams without decoding pixels

Usage: ticoraw inspect [options] <file.nef>...

Prints the camera model, raw dimensions, codec (TicoRAW HE/HE* or classic Nikon), the
MakerNote compression mode, the raw strip location, black levels and, for TicoRAW strips,
the JPEG XS codestream layout and output transfer curve.

Options:
  --json       Emit a JSON array with one object per file
  -h, --help   Show this help

Exit status: 0 when every file was inspected, 1 if any failed, 2 on usage errors.
";

#[derive(Debug)]
pub struct Options {
    pub files: Vec<PathBuf>,
    pub json: bool,
}

/// Returns `None` when help was requested.
pub fn parse(args: &mut Args) -> Result<Option<Options>, String> {
    let mut o = Options {
        files: Vec::new(),
        json: false,
    };
    while let Some(arg) = args.next_arg()? {
        match arg {
            Arg::Opt(s) if s == "-h" || s == "--help" => return Ok(None),
            Arg::Opt(s) if s == "--json" => o.json = true,
            Arg::Opt(s) => return Err(format!("unknown option {s}")),
            Arg::Pos(p) => o.files.push(p.into()),
        }
    }
    if o.files.is_empty() {
        return Err("no input files".into());
    }
    Ok(Some(o))
}

pub fn run(o: &Options) -> Result<ExitCode, CliError> {
    let mut out = io::stdout().lock();
    let mut failed = false;
    let mut entries = Vec::with_capacity(o.files.len());
    for (i, path) in o.files.iter().enumerate() {
        let result = inspect_file(path);
        failed |= result.is_err();
        if o.json {
            entries.push(match &result {
                Ok(info) => to_json(path, info),
                Err(e) => {
                    eprintln!("ticoraw: error: {}: {e}", path.display());
                    Json::Object(vec![
                        ("path", Json::str(path.to_string_lossy())),
                        ("error", Json::str(e)),
                    ])
                }
            });
            continue;
        }
        match result {
            Ok(info) => {
                if i > 0 {
                    writeln!(out)?;
                }
                out.write_all(to_text(path, &info).as_bytes())?;
            }
            Err(e) => eprintln!("ticoraw: error: {}: {e}", path.display()),
        }
    }
    if o.json {
        writeln!(out, "{}", Json::Array(entries))?;
    }
    Ok(if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn inspect_file(path: &Path) -> Result<NefInspection, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("reading: {e}"))?;
    ticoraw::inspect_nef(&bytes).map_err(|e| e.to_string())
}

/// Short label for scan listings: `HE`, `HE*`, `lossless`, `code N` or `-`.
pub fn compression_short(c: Option<NikonCompression>) -> String {
    match c {
        Some(NikonCompression::HighEfficiency) => "HE".into(),
        Some(NikonCompression::HighEfficiencyStar) => "HE*".into(),
        Some(NikonCompression::Lossless) => "lossless".into(),
        Some(NikonCompression::Other(n)) => format!("code {n}"),
        None => "-".into(),
    }
}

fn codec_name(c: RawCodec) -> &'static str {
    match c {
        RawCodec::TicoRaw => "ticoraw",
        RawCodec::Classic => "classic",
    }
}

fn to_text(path: &Path, i: &NefInspection) -> String {
    let mut s = String::new();
    // Writing to a String cannot fail.
    let _ = write_text(&mut s, path, i);
    s
}

fn write_text(s: &mut String, path: &Path, i: &NefInspection) -> std::fmt::Result {
    writeln!(s, "{}", path.display())?;
    writeln!(s, "  camera       {} {}", i.make, i.model)?;
    writeln!(
        s,
        "  dimensions   {} x {}, {} bits/sample",
        i.width, i.height, i.bits_per_sample
    )?;
    let codec = match i.codec {
        RawCodec::TicoRaw => "TicoRAW (JPEG XS)",
        RawCodec::Classic => "classic Nikon (not TicoRAW)",
    };
    writeln!(s, "  codec        {codec}")?;
    match i.compression {
        Some(c) => writeln!(s, "  compression  {c} [code {}]", c.code())?,
        None => writeln!(s, "  compression  unknown (no MakerNote 0x0051)")?,
    }
    let pixels = (i.width * i.height).max(1);
    writeln!(
        s,
        "  raw strip    offset {}, {} bytes ({:.2} bits/pixel)",
        i.strip_offset,
        i.strip_len,
        i.strip_len as f64 * 8.0 / pixels as f64
    )?;
    match i.black_level {
        Some([a, b, c, d]) => writeln!(s, "  black level  {a} {b} {c} {d}")?,
        None => writeln!(s, "  black level  unknown")?,
    }
    if let Some(cs) = &i.codestream {
        write_codestream(s, cs)?;
    }
    Ok(())
}

fn write_codestream(s: &mut String, cs: &CodestreamInfo) -> std::fmt::Result {
    let depths: Vec<String> = cs
        .header
        .components
        .iter()
        .map(|c| c.bit_depth.to_string())
        .collect();
    writeln!(
        s,
        "  codestream   {} components of {} x {}, bit depth {}",
        cs.header.pih.nc,
        cs.component_width,
        cs.component_height,
        depths.join("/")
    )?;
    writeln!(
        s,
        "               wavelet NLx={} NLy={}; {} slices, {} precincts, {} bands",
        cs.header.pih.nlx, cs.header.pih.nly, cs.slices, cs.precincts, cs.bands
    )?;
    writeln!(
        s,
        "               transfer curve B={} A={} C={}",
        cs.curve.black, cs.curve.gain, cs.curve.knee
    )?;
    match &cs.unsupported {
        None => writeln!(s, "               profile supported"),
        Some(why) => writeln!(s, "               profile UNSUPPORTED: {why}"),
    }
}

fn to_json(path: &Path, i: &NefInspection) -> Json {
    let codestream = Json::opt(i.codestream.as_ref(), |cs| {
        Json::Object(vec![
            ("components", Json::uint(cs.header.pih.nc)),
            ("component_width", Json::uint(cs.component_width as u64)),
            ("component_height", Json::uint(cs.component_height as u64)),
            (
                "bit_depths",
                Json::Array(
                    cs.header
                        .components
                        .iter()
                        .map(|c| Json::uint(c.bit_depth))
                        .collect(),
                ),
            ),
            ("nlx", Json::uint(cs.header.pih.nlx)),
            ("nly", Json::uint(cs.header.pih.nly)),
            ("slices", Json::uint(cs.slices as u64)),
            ("precincts", Json::uint(cs.precincts as u64)),
            ("bands", Json::uint(cs.bands as u64)),
            (
                "transfer_curve",
                Json::Object(vec![
                    ("black", Json::Int(cs.curve.black)),
                    ("gain", Json::Int(cs.curve.gain)),
                    ("knee", Json::Int(cs.curve.knee)),
                ]),
            ),
            ("unsupported", Json::opt(cs.unsupported.clone(), Json::Str)),
        ])
    });
    Json::Object(vec![
        ("path", Json::str(path.to_string_lossy())),
        ("make", Json::str(&i.make)),
        ("model", Json::str(&i.model)),
        ("width", Json::uint(i.width as u64)),
        ("height", Json::uint(i.height as u64)),
        ("bits_per_sample", Json::uint(i.bits_per_sample)),
        ("codec", Json::str(codec_name(i.codec))),
        (
            "compression",
            Json::opt(i.compression, |c| {
                Json::Object(vec![
                    ("code", Json::uint(c.code())),
                    ("name", Json::str(c.to_string())),
                ])
            }),
        ),
        (
            "strip",
            Json::Object(vec![
                ("offset", Json::uint(i.strip_offset as u64)),
                ("length", Json::uint(i.strip_len as u64)),
            ]),
        ),
        (
            "black_level",
            Json::opt(i.black_level, |b| {
                Json::Array(b.iter().map(|&v| Json::uint(v)).collect())
            }),
        ),
        ("codestream", codestream),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classic() -> NefInspection {
        NefInspection {
            make: "NIKON CORPORATION".into(),
            model: "NIKON Z 6".into(),
            width: 6048,
            height: 4024,
            bits_per_sample: 14,
            codec: RawCodec::Classic,
            compression: Some(NikonCompression::Lossless),
            strip_offset: 4096,
            strip_len: 1000,
            black_level: Some([1008; 4]),
            codestream: None,
        }
    }

    #[test]
    fn text_summary() {
        let t = to_text(Path::new("a.nef"), &classic());
        assert!(t.starts_with("a.nef\n"));
        assert!(t.contains("classic Nikon"));
        assert!(t.contains("Lossless compressed [code 3]"));
        assert!(t.contains("black level  1008 1008 1008 1008"));
        assert!(!t.contains("codestream"));
    }

    #[test]
    fn json_summary() {
        let j = to_json(Path::new("a.nef"), &classic()).to_string();
        assert!(j.contains("\"codec\": \"classic\""));
        assert!(j.contains("\"code\": 3"));
        assert!(j.contains("\"black_level\": [1008, 1008, 1008, 1008]"));
        assert!(j.contains("\"codestream\": null"));
    }

    #[test]
    fn compression_labels() {
        assert_eq!(
            compression_short(Some(NikonCompression::HighEfficiencyStar)),
            "HE*"
        );
        assert_eq!(
            compression_short(Some(NikonCompression::Other(9))),
            "code 9"
        );
        assert_eq!(compression_short(None), "-");
        assert_eq!(NikonCompression::HighEfficiency.code(), 13);
    }
}
