//! The functional core of the bindings: option parsing, error classification and value
//! conversions. Nothing here touches a JS type, so it is tested natively; `lib.rs` is the
//! shell that moves values across the wasm-bindgen boundary.

use ticoraw::TicoError;
use ticoraw_dng::{Compression, Options, Preview};

/// A JS value reduced to what option parsing needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptValue {
    /// `undefined` or `null`: the option keeps its default.
    Absent,
    /// A boolean.
    Bool(bool),
    /// A string.
    Str(String),
    /// A `Uint8Array`'s contents.
    Bytes(Vec<u8>),
    /// Anything else, described for the error message (`typeof`, or `Array` and so on).
    Other(String),
}

/// The keys `toDng` accepts, as named in JS.
pub const OPTION_KEYS: [&str; 5] = [
    "compression",
    "preview",
    "carryXmp",
    "extraXmp",
    "originalRawFileName",
];

/// Builds [`Options`] from the own enumerable properties of a JS options object.
///
/// Unknown keys and values of the wrong type are rejected rather than ignored, so a typo
/// such as `{ compresion: "uncompressed" }` fails loudly instead of being silently dropped.
///
/// # Errors
///
/// A message for a `TypeError` naming the offending key.
pub fn parse_options(entries: Vec<(String, OptValue)>) -> Result<Options, String> {
    let mut o = Options::default();
    for (key, value) in entries {
        if value == OptValue::Absent {
            if OPTION_KEYS.contains(&key.as_str()) {
                continue;
            }
            return Err(unknown_key(&key));
        }
        match (key.as_str(), value) {
            ("compression", OptValue::Str(s)) => {
                o.compression = match s.as_str() {
                    "lossless-jpeg" => Compression::LosslessJpeg,
                    "uncompressed" => Compression::Uncompressed,
                    _ => return Err(bad_value(&key, &s, "\"lossless-jpeg\" or \"uncompressed\"")),
                };
            }
            ("preview", OptValue::Str(s)) => {
                o.preview = match s.as_str() {
                    "auto" => Preview::Auto,
                    "none" => Preview::None,
                    _ => return Err(bad_value(&key, &s, "\"auto\" or \"none\"")),
                };
            }
            ("carryXmp", OptValue::Bool(b)) => o.carry_xmp = b,
            // Empty means no packet, as in the C API: writing it would replace the NEF's XMP
            // with a zero-length XMLPacket entry.
            ("extraXmp", OptValue::Bytes(b)) => o.extra_xmp = Some(b).filter(|b| !b.is_empty()),
            ("extraXmp", OptValue::Str(s)) => {
                o.extra_xmp = Some(s.into_bytes()).filter(|b| !b.is_empty());
            }
            ("originalRawFileName", OptValue::Str(s)) => o.original_raw_file_name = Some(s),
            (k, v) if OPTION_KEYS.contains(&k) => {
                let expected = match k {
                    "carryXmp" => "a boolean",
                    "extraXmp" => "a Uint8Array or a string",
                    _ => "a string",
                };
                return Err(format!(
                    "toDng option `{k}` must be {expected}, not {}",
                    describe(&v)
                ));
            }
            (k, _) => return Err(unknown_key(k)),
        }
    }
    Ok(o)
}

fn unknown_key(key: &str) -> String {
    format!(
        "unknown toDng option `{key}` (expected one of: {})",
        OPTION_KEYS.join(", ")
    )
}

fn bad_value(key: &str, got: &str, expected: &str) -> String {
    format!("toDng option `{key}` must be {expected}, not \"{got}\"")
}

fn describe(v: &OptValue) -> String {
    match v {
        OptValue::Absent => "undefined".into(),
        OptValue::Bool(_) => "a boolean".into(),
        OptValue::Str(_) => "a string".into(),
        OptValue::Bytes(_) => "a Uint8Array".into(),
        OptValue::Other(t) => t.clone(),
    }
}

/// Parses a JS number used as a byte length: a non-negative integer that fits `usize`.
#[must_use]
pub fn parse_length(x: f64) -> Option<usize> {
    if !x.is_finite() || x < 0.0 || x.fract() != 0.0 || x > usize::MAX as f64 {
        return None;
    }
    Some(x as usize)
}

/// What a JS caller learns about a failure: the fields of the thrown `TicoError`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorInfo {
    /// The variant name of [`TicoError`] or [`ticoraw_dng::Error`], e.g. `"Truncated"`.
    pub kind: &'static str,
    /// The error's `Display` text, identical to what the Rust API and the CLI print.
    pub message: String,
    /// For `Truncated`: the structure that was cut short.
    pub what: Option<&'static str>,
    /// For `Truncated`: the byte offset where it starts or where reading stopped.
    pub offset: Option<usize>,
}

impl From<&TicoError> for ErrorInfo {
    fn from(e: &TicoError) -> Self {
        let (kind, what, offset) = match e {
            TicoError::Truncated { what, offset } => ("Truncated", Some(*what), Some(*offset)),
            TicoError::Container(_) => ("Container", None, None),
            TicoError::Header(_) => ("Header", None, None),
            TicoError::Unsupported(_) => ("Unsupported", None, None),
            TicoError::Entropy(_) => ("Entropy", None, None),
            TicoError::Jpeg(_) => ("Jpeg", None, None),
        };
        Self {
            kind,
            message: e.to_string(),
            what,
            offset,
        }
    }
}

impl From<&ticoraw_dng::Error> for ErrorInfo {
    fn from(e: &ticoraw_dng::Error) -> Self {
        use ticoraw_dng::Error as E;
        let kind = match e {
            E::Decode(t) => return t.into(),
            E::InvalidImage(_) => "InvalidImage",
            E::InvalidMetadata(_) => "InvalidMetadata",
            E::TooLarge(_) => "TooLarge",
            E::Preview(_) => "Preview",
        };
        Self {
            kind,
            message: e.to_string(),
            what: None,
            offset: None,
        }
    }
}

/// A 2x2 CFA pattern as letters, row-major (`[0, 1, 1, 2]` is `"RGGB"`), using the TIFF/EP
/// colour codes; unknown codes read as `?`.
#[must_use]
pub fn cfa_pattern_name(p: [u8; 4]) -> String {
    p.iter()
        .map(|c| match c {
            0 => 'R',
            1 => 'G',
            2 => 'B',
            3 => 'C',
            4 => 'M',
            5 => 'Y',
            6 => 'W',
            _ => '?',
        })
        .collect()
}

/// Rationals as numbers; `None` when any denominator is zero.
#[must_use]
pub fn ratios<T: Into<f64> + Copy, const N: usize>(r: [(T, T); N]) -> Option<[f64; N]> {
    let mut out = [0.0; N];
    for (o, (n, d)) in out.iter_mut().zip(r) {
        let d: f64 = d.into();
        if d == 0.0 {
            return None;
        }
        *o = n.into() / d;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ticoraw_dng::Error as E;

    fn s(v: &str) -> OptValue {
        OptValue::Str(v.into())
    }

    #[test]
    fn options_default_when_empty_or_absent() {
        assert_eq!(parse_options(vec![]), Ok(Options::default()));
        let absent = OPTION_KEYS
            .iter()
            .map(|k| ((*k).to_owned(), OptValue::Absent))
            .collect();
        assert_eq!(parse_options(absent), Ok(Options::default()));
    }

    #[test]
    fn options_parse_every_key() {
        let o = parse_options(vec![
            ("compression".into(), s("uncompressed")),
            ("preview".into(), s("none")),
            ("carryXmp".into(), OptValue::Bool(false)),
            ("extraXmp".into(), s("<x:xmpmeta/>")),
            ("originalRawFileName".into(), s("DSC_0001.NEF")),
        ])
        .unwrap();
        assert_eq!(
            o,
            Options {
                compression: Compression::Uncompressed,
                preview: Preview::None,
                carry_xmp: false,
                extra_xmp: Some(b"<x:xmpmeta/>".to_vec()),
                original_raw_file_name: Some("DSC_0001.NEF".into()),
            }
        );
        let o = parse_options(vec![("extraXmp".into(), OptValue::Bytes(vec![1, 2]))]).unwrap();
        assert_eq!(o.extra_xmp, Some(vec![1, 2]));
        let o = parse_options(vec![("compression".into(), s("lossless-jpeg"))]).unwrap();
        assert_eq!(o.compression, Compression::LosslessJpeg);
    }

    #[test]
    fn empty_extra_xmp_is_no_packet() {
        // As in the C API: an empty packet keeps the NEF's XMP instead of replacing it with
        // a zero-length XMLPacket entry.
        for v in [s(""), OptValue::Bytes(vec![])] {
            let o = parse_options(vec![("extraXmp".into(), v)]).unwrap();
            assert_eq!(o, Options::default());
        }
    }

    #[test]
    fn options_reject_typos_and_wrong_types() {
        let e = parse_options(vec![("compresion".into(), s("uncompressed"))]).unwrap_err();
        assert!(e.contains("unknown toDng option `compresion`"), "{e}");
        let e = parse_options(vec![("compresion".into(), OptValue::Absent)]).unwrap_err();
        assert!(e.contains("unknown"), "{e}");
        let e = parse_options(vec![("compression".into(), s("zip"))]).unwrap_err();
        assert!(e.contains("\"lossless-jpeg\" or \"uncompressed\""), "{e}");
        let e = parse_options(vec![("preview".into(), s("full"))]).unwrap_err();
        assert!(e.contains("\"auto\" or \"none\""), "{e}");
        let e = parse_options(vec![("carryXmp".into(), s("yes"))]).unwrap_err();
        assert_eq!(e, "toDng option `carryXmp` must be a boolean, not a string");
        let e = parse_options(vec![(
            "extraXmp".into(),
            OptValue::Other("a number".into()),
        )])
        .unwrap_err();
        assert!(e.contains("a Uint8Array or a string, not a number"), "{e}");
        let e =
            parse_options(vec![("originalRawFileName".into(), OptValue::Bool(true))]).unwrap_err();
        assert!(e.contains("must be a string, not a boolean"), "{e}");
    }

    #[test]
    fn lengths() {
        assert_eq!(parse_length(0.0), Some(0));
        assert_eq!(parse_length(15_804_416.0), Some(15_804_416));
        for bad in [-1.0, 0.5, f64::NAN, f64::INFINITY, 1e300] {
            assert_eq!(parse_length(bad), None, "{bad}");
        }
    }

    #[test]
    fn errors_keep_kind_and_message() {
        let e = TicoError::Truncated {
            what: "precinct",
            offset: 42,
        };
        assert_eq!(
            ErrorInfo::from(&e),
            ErrorInfo {
                kind: "Truncated",
                message: "truncated precinct at byte 42".into(),
                what: Some("precinct"),
                offset: Some(42),
            }
        );
        let kinds = [
            (TicoError::Container("x".into()), "Container"),
            (TicoError::Header("x".into()), "Header"),
            (TicoError::Unsupported("x".into()), "Unsupported"),
            (TicoError::Entropy("x".into()), "Entropy"),
            (TicoError::Jpeg("x".into()), "Jpeg"),
        ];
        for (e, kind) in kinds {
            let i = ErrorInfo::from(&e);
            assert_eq!((i.kind, i.what, i.offset), (kind, None, None));
            assert_eq!(i.message, e.to_string());
            // A decode failure inside the DNG writer reports the decoder's kind.
            assert_eq!(ErrorInfo::from(&E::Decode(e)).kind, kind);
        }
        for (e, kind) in [
            (E::InvalidImage("x".into()), "InvalidImage"),
            (E::InvalidMetadata("x".into()), "InvalidMetadata"),
            (E::TooLarge("x".into()), "TooLarge"),
            (E::Preview("x".into()), "Preview"),
        ] {
            let i = ErrorInfo::from(&e);
            assert_eq!((i.kind, i.message), (kind, e.to_string()));
        }
    }

    #[test]
    fn cfa_names() {
        assert_eq!(cfa_pattern_name([0, 1, 1, 2]), "RGGB");
        assert_eq!(cfa_pattern_name([1, 2, 0, 9]), "GBR?");
    }

    #[test]
    fn rationals() {
        assert_eq!(ratios([(1i32, 4i32), (-3, 2)]), Some([0.25, -1.5]));
        assert_eq!(ratios([(1u32, 1u32), (2, 0)]), None);
    }
}
