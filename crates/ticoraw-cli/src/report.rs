//! `ticoraw report`: a privacy-safe JSON bundle for bug reports.
//!
//! The report is built only from structural facts: versions, camera make/model/firmware,
//! compression mode, dimensions, the codestream's main-header segments and the decode
//! outcome. Nothing is copied from the EXIF, GPS, XMP or MakerNote, so serial numbers,
//! positions, names and dates cannot leak; file names and paths are never seen by
//! [`build`].

use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use ticoraw::tiff::Tiff;
use ticoraw::{CodestreamInfo, NefInspection, RawCodec};

use crate::args::{Arg, Args};
use crate::json::Json;
use crate::{CliError, read_input, unexpected};

pub const HELP: &str = "\
Print a privacy-safe JSON bug report for one NEF

Usage: ticoraw report <file.nef>

Collects what a bug report about a file needs: the ticoraw version, operating system and
architecture, the camera's make, model and firmware, the Nikon compression mode, the raw
dimensions and codec, the codestream's main header (CAP, PIH, CDT and WGT payloads in hex),
why a codestream is unsupported, and the decode outcome (its time, or the error and the
failing slice).

Nothing personal is included: no serial numbers, GPS position, lens serial, owner, artist or
copyright, no dates, no file name or path, and no pixels. Paste the output into the
bug-report form.

Options:
  -h, --help   Show this help

Exit status: 0 when the report was printed (also when decoding failed), 1 when the file
cannot be read, 2 on usage errors.
";

/// Where reports go.
const FORM: &str = "https://github.com/lukaj99/ticoraw/issues/new?template=bug_report.yml";

/// Longest payload shown in hex per main-header segment.
const MAX_HEX: usize = 1024;

/// Longest string copied from a TIFF field (make, model, firmware).
const MAX_TEXT: usize = 64;

#[derive(Debug)]
pub struct Options {
    pub file: PathBuf,
}

/// Returns `None` when help was requested.
pub fn parse(args: &mut Args) -> Result<Option<Options>, String> {
    let mut file = None;
    while let Some(arg) = args.next_arg()? {
        match arg {
            Arg::Opt(s) if s == "-h" || s == "--help" => return Ok(None),
            Arg::Opt(s) => return Err(format!("unknown option {s}")),
            Arg::Pos(p) if file.is_none() => file = Some(PathBuf::from(p)),
            Arg::Pos(p) => return Err(unexpected(&p)),
        }
    }
    Ok(Some(Options {
        file: file.ok_or("no input file")?,
    }))
}

pub fn run(o: &Options) -> Result<ExitCode, CliError> {
    let nef = read_input(&o.file)?;
    let report = build(&nef);
    writeln!(io::stdout().lock(), "{report}")?;
    eprintln!("Paste the JSON above into the bug-report form: {FORM}");
    Ok(ExitCode::SUCCESS)
}

/// The report for a file's bytes.
pub fn build(nef: &[u8]) -> Json {
    let inspection = ticoraw::inspect_nef(nef);
    let ifd0 = Ifd0Text::read(nef);
    let mut fields = vec![
        ("ticoraw_version", Json::str(env!("CARGO_PKG_VERSION"))),
        ("os", Json::str(std::env::consts::OS)),
        ("arch", Json::str(std::env::consts::ARCH)),
        ("file_bytes", Json::uint(nef.len() as u64)),
        ("make", Json::opt(ifd0.make, Json::Str)),
        ("model", Json::opt(ifd0.model, Json::Str)),
        ("firmware", Json::opt(ifd0.software, Json::Str)),
    ];
    match &inspection {
        Ok(i) => fields.extend(inspection_fields(nef, i)),
        Err(e) => fields.push(("inspect_error", Json::str(e.to_string()))),
    }
    let t = Instant::now();
    let decode = match ticoraw::decode_nef(nef) {
        Ok(raw) => Json::Object(vec![
            ("ok", Json::Bool(true)),
            ("milliseconds", Json::Float(t.elapsed().as_secs_f64() * 1e3)),
            ("width", Json::uint(raw.image.width as u64)),
            ("height", Json::uint(raw.image.height as u64)),
        ]),
        Err(e) => {
            let text = e.to_string();
            Json::Object(vec![
                ("ok", Json::Bool(false)),
                ("failing_slice", Json::opt(failing_slice(&text), Json::uint)),
                ("error", Json::Str(text)),
            ])
        }
    };
    fields.push(("decode", decode));
    Json::Object(fields)
}

/// Make, model and firmware (`Software`) of IFD0, read directly so they are there even when
/// the NEF does not inspect.
struct Ifd0Text {
    make: Option<String>,
    model: Option<String>,
    software: Option<String>,
}

impl Ifd0Text {
    fn read(nef: &[u8]) -> Self {
        let field = |tag: u16| -> Option<String> {
            let t = Tiff::parse(nef, 0).ok()?;
            let ifd0 = t.ifd(t.first_ifd).ok()?;
            Some(printable(&t.ascii(ifd0.get(tag)?))).filter(|s| !s.is_empty())
        };
        Self {
            make: field(0x010F),
            model: field(0x0110),
            software: field(0x0131),
        }
    }
}

/// `s` restricted to printable ASCII and [`MAX_TEXT`] characters.
fn printable(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_graphic() || *c == ' ')
        .take(MAX_TEXT)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn inspection_fields(nef: &[u8], i: &NefInspection) -> Vec<(&'static str, Json)> {
    let codec = match i.codec {
        RawCodec::TicoRaw => "ticoraw",
        RawCodec::Classic => "classic",
    };
    let strip = nef
        .get(i.strip_offset..i.strip_offset.saturating_add(i.strip_len).min(nef.len()))
        .unwrap_or_default();
    vec![
        (
            "compression",
            Json::opt(i.compression, |c| {
                Json::Object(vec![
                    ("code", Json::uint(c.code())),
                    ("name", Json::str(c.to_string())),
                ])
            }),
        ),
        ("codec", Json::str(codec)),
        ("width", Json::uint(i.width as u64)),
        ("height", Json::uint(i.height as u64)),
        ("bits_per_sample", Json::uint(i.bits_per_sample)),
        (
            "strip",
            Json::Object(vec![
                ("offset", Json::uint(i.strip_offset as u64)),
                ("length", Json::uint(i.strip_len as u64)),
            ]),
        ),
        (
            "codestream",
            Json::opt(i.codestream.as_ref(), |cs| codestream(strip, cs)),
        ),
    ]
}

fn codestream(strip: &[u8], cs: &CodestreamInfo) -> Json {
    let segments = main_header_segments(strip)
        .into_iter()
        .map(|(marker, payload)| {
            let mut f = vec![
                ("marker", Json::str(marker_name(marker))),
                ("code", Json::str(format!("{marker:#06x}"))),
                ("length", Json::uint(payload.len() as u64)),
            ];
            // A COM segment is free text; its contents stay out of the report.
            if marker != COM {
                f.push((
                    "hex",
                    Json::Str(hex(&payload[..payload.len().min(MAX_HEX)])),
                ));
            }
            Json::Object(f)
        })
        .collect();
    Json::Object(vec![
        (
            "main_header_bytes",
            Json::uint(cs.header.first_slice as u64),
        ),
        ("segments", Json::Array(segments)),
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
}

const SOC: u16 = 0xFF10;
const SLH: u16 = 0xFF20;
const COM: u16 = 0xFF15;

/// The marker segments between SOC and the first SLH, as (marker, payload). Stops quietly
/// at anything malformed: `ticoraw::inspect_nef` has already reported that.
fn main_header_segments(cs: &[u8]) -> Vec<(u16, &[u8])> {
    let be16 = |at: usize| cs.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let mut out = Vec::new();
    if be16(0) != Some(SOC) {
        return out;
    }
    let mut pos = 2;
    while out.len() < 64 {
        let (Some(marker), Some(len)) = (be16(pos), be16(pos + 2)) else {
            break;
        };
        if marker == SLH || len < 2 {
            break;
        }
        let Some(payload) = cs.get(pos + 4..pos + 2 + usize::from(len)) else {
            break;
        };
        out.push((marker, payload));
        pos += 2 + usize::from(len);
    }
    out
}

fn marker_name(marker: u16) -> &'static str {
    match marker {
        0xFF50 => "CAP",
        0xFF12 => "PIH",
        0xFF13 => "CDT",
        0xFF14 => "WGT",
        COM => "COM",
        0xFF16 => "NLT",
        0xFF17 => "CWD",
        0xFF18 => "CTS",
        0xFF19 => "CRG",
        _ => "unknown",
    }
}

fn hex(b: &[u8]) -> String {
    use std::fmt::Write as _;
    b.iter()
        .fold(String::with_capacity(b.len() * 2), |mut s, x| {
            let _ = write!(s, "{x:02x}");
            s
        })
}

/// The slice index named in a decode error (`... slice 3: ...`), if any.
fn failing_slice(error: &str) -> Option<u64> {
    let (_, rest) = error.split_once("slice ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synthetic;

    #[test]
    fn report_has_the_facts_and_nothing_personal() {
        for nef in [synthetic::he_nef(5), synthetic::classic_nef()] {
            // The synthetic NEF really carries every personal string.
            for secret in synthetic::PERSONAL {
                let found = nef.windows(secret.len()).any(|w| w == secret.as_bytes());
                assert!(found, "{secret} is in the synthetic NEF");
            }
            let text = build(&nef).to_string();
            for secret in synthetic::PERSONAL {
                assert!(!text.contains(secret), "{secret} leaked into {text}");
            }
            assert!(text.contains("\"model\": \"NIKON Z6_3\""), "{text}");
            assert!(text.contains(&format!("\"firmware\": \"{}\"", synthetic::FIRMWARE)));
            assert!(text.contains(&format!(
                "\"ticoraw_version\": \"{}\"",
                env!("CARGO_PKG_VERSION")
            )));
            assert!(text.contains(&format!("\"os\": \"{}\"", std::env::consts::OS)));
        }
    }

    #[test]
    fn he_report_shows_header_segments_and_decode_time() {
        let text = build(&synthetic::he_nef(5)).to_string();
        for needle in [
            "\"codec\": \"ticoraw\"",
            "\"code\": 14",
            "\"marker\": \"CAP\"",
            "\"marker\": \"PIH\"",
            "\"marker\": \"CDT\"",
            "\"marker\": \"WGT\"",
            "\"unsupported\": null",
            "\"ok\": true",
            "\"milliseconds\": ",
        ] {
            assert!(text.contains(needle), "{needle} in {text}");
        }
        // CAP carries the vendor's tag in its payload.
        assert!(text.contains(&hex(b"CONTACT_INTOPIX_")), "{text}");
    }

    #[test]
    fn failures_are_reported_not_fatal() {
        let text = build(&synthetic::classic_nef()).to_string();
        assert!(text.contains("\"codec\": \"classic\""), "{text}");
        assert!(text.contains("\"ok\": false"), "{text}");
        let text = build(b"not a tiff").to_string();
        assert!(text.contains("\"inspect_error\": "), "{text}");
        assert!(text.contains("\"make\": null"), "{text}");
    }

    #[test]
    fn slice_index_from_error_text() {
        assert_eq!(failing_slice("entropy data: slice 12: bad"), Some(12));
        assert_eq!(
            failing_slice("codestream header: expected SLH for slice 3 at byte 9"),
            Some(3)
        );
        assert_eq!(failing_slice("container: no CFA raw IFD"), None);
    }

    #[test]
    fn segments_stop_at_slh_and_garbage() {
        let cs = [0xFF, 0x10, 0xFF, 0x50, 0, 4, 1, 2, 0xFF, 0x20, 0, 4];
        assert_eq!(main_header_segments(&cs), vec![(0xFF50, &[1u8, 2][..])]);
        let none: Vec<(u16, &[u8])> = Vec::new();
        assert_eq!(main_header_segments(&[0xFF, 0x10, 0xFF, 0x50, 0, 9]), none);
        assert_eq!(main_header_segments(b"xx"), none);
    }
}
