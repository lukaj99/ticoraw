//! Stable C ABI for [`ticoraw`] and [`ticoraw_dng`]: decode Nikon High Efficiency (HE /
//! HE\*) NEFs and convert them to DNG from C or any language with a C FFI.
//!
//! The C declarations are in `include/ticoraw.h`, generated from this crate with cbindgen
//! (see the crate README). Every exported function starts with `tr_`, takes and returns
//! only plain C types and opaque handles, catches Rust panics before they reach the caller,
//! and reports failure through a [`TrStatus`] code plus a thread-local message
//! ([`ffi::tr_last_error_message`]).
//!
//! This module is the safe core: the handle types, option parsing, status mapping, error
//! bookkeeping and the inspect JSON. The `extern "C"` functions, the only `unsafe` code in
//! the crate, live in [`ffi`].
//!
//! # ABI rules
//!
//! - Handles ([`TrRaw`], [`TrBuffer`]) are created by the library and freed only by their
//!   `tr_*_free` function. Freeing `NULL` is a no-op.
//! - Status values, option constants and the layout of the `#[repr(C)]` structs never
//!   change meaning; new ones are only added. [`TR_ABI_VERSION`] increases when something
//!   is added.
//! - [`TrDngOptions`] starts with `struct_size`, so fields can be appended later without
//!   breaking callers compiled against an older header.

#![warn(missing_docs)]

pub mod ffi;

use std::any::Any;
use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::fmt::Write as _;
use std::panic::{self, AssertUnwindSafe};

use ticoraw::{DecodedRaw, NefInspection, RawCodec, TicoError};

/// Version of the C ABI described by `ticoraw.h`. It increases whenever a function, status,
/// constant or struct field is added; nothing is ever removed or changed.
pub const TR_ABI_VERSION: u32 = 1;

/// Result of every fallible `tr_*` function. `TR_STATUS_OK` is 0; every other value is a
/// failure whose message `tr_last_error_message()` returns.
///
/// The codes 10..=15 mirror the variants of `TicoError` and 20..=23 those of
/// `ticoraw_dng::Error`. New codes may be added; treat unknown ones as generic failures.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrStatus {
    /// Success.
    Ok = 0,
    /// A pointer argument that must not be `NULL` was `NULL`.
    NullPointer = 1,
    /// An argument is out of range: an unknown option value, a `struct_size` that is too
    /// small, a string that is not UTF-8, or a length above `PTRDIFF_MAX`.
    InvalidArgument = 2,
    /// An optional value is absent (for example no colour matrix for an unknown camera).
    /// The output was not written. This is not an error: no message is recorded.
    NotPresent = 3,
    /// The input ended before a structure was complete (`TicoError::Truncated`).
    Truncated = 10,
    /// The TIFF / NEF / DNG container is malformed (`TicoError::Container`).
    Container = 11,
    /// A JPEG XS marker segment is malformed (`TicoError::Header`).
    Header = 12,
    /// Valid input outside what the decoder supports, such as a classic (non-HE) NEF
    /// (`TicoError::Unsupported`).
    Unsupported = 13,
    /// Entropy-coded data is inconsistent (`TicoError::Entropy`).
    Entropy = 14,
    /// A lossless JPEG stream is malformed (`TicoError::Jpeg`).
    Jpeg = 15,
    /// The decoded image is inconsistent (`ticoraw_dng::Error::InvalidImage`).
    InvalidImage = 20,
    /// Carried metadata is malformed (`ticoraw_dng::Error::InvalidMetadata`).
    InvalidMetadata = 21,
    /// The DNG would exceed the 4 GiB TIFF limit (`ticoraw_dng::Error::TooLarge`).
    TooLarge = 22,
    /// The preview JPEG encoder failed (`ticoraw_dng::Error::Preview`).
    Preview = 23,
    /// A bug in the library: a Rust panic was caught at the C boundary. The message says
    /// where; please report it.
    Panic = 100,
}

impl From<&TicoError> for TrStatus {
    fn from(e: &TicoError) -> Self {
        match e {
            TicoError::Truncated { .. } => Self::Truncated,
            TicoError::Container(_) => Self::Container,
            TicoError::Header(_) => Self::Header,
            TicoError::Unsupported(_) => Self::Unsupported,
            TicoError::Entropy(_) => Self::Entropy,
            TicoError::Jpeg(_) => Self::Jpeg,
        }
    }
}

impl From<&ticoraw_dng::Error> for TrStatus {
    fn from(e: &ticoraw_dng::Error) -> Self {
        use ticoraw_dng::Error;
        match e {
            Error::Decode(e) => e.into(),
            Error::InvalidImage(_) => Self::InvalidImage,
            Error::InvalidMetadata(_) => Self::InvalidMetadata,
            Error::TooLarge(_) => Self::TooLarge,
            Error::Preview(_) => Self::Preview,
        }
    }
}

/// Values of `TrDngOptions::compression`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrCompression {
    /// Lossless JPEG in 256x256 tiles, about the size of Adobe's DNGs (the default).
    LosslessJpeg = 0,
    /// One uncompressed 16-bit strip: about three times larger, simplest to read.
    Uncompressed = 1,
}

/// Values of `TrDngOptions::preview`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrPreview {
    /// The camera's embedded JPEG when there is a usable one, otherwise a half-size
    /// rendering of the raw data (the default).
    Auto = 0,
    /// No full-size preview; the IFD0 thumbnail is still written.
    None = 1,
}

/// Options for `tr_nef_to_dng()`. A zero-initialised struct with `struct_size` set
/// gives the defaults, as does passing `NULL`:
///
/// ```c
/// TrDngOptions opts = {0};
/// opts.struct_size = sizeof opts;
/// opts.original_raw_file_name = "DSC_0001.NEF";
/// ```
///
/// Fields are only ever appended, and a new field's zero value always means the earlier
/// behaviour. A library reads the fields it knows and ignores any beyond them.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TrDngOptions {
    /// `sizeof(TrDngOptions)` as the caller was compiled. Must be at least the size of the
    /// version-1 struct, or the call fails with `TR_STATUS_INVALID_ARGUMENT`.
    pub struct_size: u32,
    /// A `TrCompression` value.
    pub compression: u32,
    /// A `TrPreview` value.
    pub preview: u32,
    /// Nonzero: do not copy the NEF's XMP packet (`extra_xmp` is still written).
    pub omit_xmp: u32,
    /// Written as `OriginalRawFileName` (usually the NEF's file name without directories):
    /// a NUL-terminated UTF-8 string, or `NULL` for none. Only read during the call.
    pub original_raw_file_name: *const std::ffi::c_char,
    /// An XMP packet to write instead of the NEF's, `extra_xmp_len` bytes long, or `NULL`
    /// for none. A length of 0 also means none. Only read during the call.
    pub extra_xmp: *const u8,
    /// Length of `extra_xmp` in bytes.
    pub extra_xmp_len: usize,
}

/// Smallest `struct_size` accepted for `TrDngOptions`.
pub(crate) const DNG_OPTIONS_V1_SIZE: usize = size_of::<TrDngOptions>();

/// A signed rational, as TIFF `SRATIONAL`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrSRational {
    /// Numerator.
    pub num: i32,
    /// Denominator.
    pub den: i32,
}

/// An unsigned rational, as TIFF `RATIONAL`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrURational {
    /// Numerator.
    pub num: u32,
    /// Denominator.
    pub den: u32,
}

/// A rectangle inside the CFA mosaic, in photosites.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrCrop {
    /// First column.
    pub x: usize,
    /// First row.
    pub y: usize,
    /// Width in columns.
    pub width: usize,
    /// Height in rows.
    pub height: usize,
}

/// A decoded raw image (opaque). Created by `tr_decode_nef()`, read with the
/// `tr_raw_*` accessors, released with `tr_raw_free()`. It is immutable, so several
/// threads may read one handle at once.
#[derive(Debug)]
pub struct TrRaw {
    raw: DecodedRaw,
    make: CString,
    model: CString,
}

impl TrRaw {
    fn new(raw: DecodedRaw) -> Self {
        Self {
            make: c_string(&raw.make),
            model: c_string(&raw.model),
            raw,
        }
    }
}

/// A byte buffer owned by the library (opaque): a DNG file or a JSON document. Read with
/// `tr_buffer_data()` and `tr_buffer_len()`, released with
/// `tr_buffer_free()`.
#[derive(Debug)]
pub struct TrBuffer {
    /// The contents; for text buffers followed by one NUL byte that `len` excludes.
    bytes: Box<[u8]>,
    len: usize,
}

impl TrBuffer {
    fn binary(bytes: Vec<u8>) -> Self {
        let len = bytes.len();
        Self {
            bytes: bytes.into_boxed_slice(),
            len,
        }
    }

    fn text(s: String) -> Self {
        let mut bytes = s.into_bytes();
        let len = bytes.len();
        bytes.push(0);
        Self {
            bytes: bytes.into_boxed_slice(),
            len,
        }
    }
}

/// A failed call: the status to return and the message to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failure {
    status: TrStatus,
    message: String,
}

impl Failure {
    fn new(status: TrStatus, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub(crate) fn null(what: &str) -> Self {
        Self::new(TrStatus::NullPointer, format!("{what} is NULL"))
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(TrStatus::InvalidArgument, message)
    }

    /// An optional value that is absent; recorded as no error at all.
    pub(crate) fn not_present() -> Self {
        Self::new(TrStatus::NotPresent, String::new())
    }
}

impl From<TicoError> for Failure {
    fn from(e: TicoError) -> Self {
        Self::new((&e).into(), e.to_string())
    }
}

impl From<ticoraw_dng::Error> for Failure {
    fn from(e: ticoraw_dng::Error) -> Self {
        Self::new((&e).into(), e.to_string())
    }
}

thread_local! {
    /// The message of the last failed call on this thread.
    static LAST_ERROR: RefCell<CString> = RefCell::new(CString::default());
}

fn set_last_error(message: &str) {
    let message = c_string(message);
    // `try_with` fails only while the thread is being torn down; the message is then lost.
    let _ = LAST_ERROR.try_with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = message;
        }
    });
}

/// The last recorded message as a pointer into thread-local storage; it stays valid until
/// this thread records another message or exits.
pub(crate) fn last_error_ptr() -> *const std::ffi::c_char {
    LAST_ERROR
        .try_with(|cell| cell.try_borrow().map(|s| s.as_ptr()).ok())
        .ok()
        .flatten()
        .unwrap_or(c"".as_ptr())
}

/// Runs `f`, turning a `Failure` or a panic into a status and a recorded message. Every
/// fallible exported function goes through this, so no panic unwinds into C.
pub(crate) fn guard(f: impl FnOnce() -> Result<(), Failure>) -> TrStatus {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(())) => TrStatus::Ok,
        Ok(Err(failure)) => {
            if failure.status != TrStatus::NotPresent {
                set_last_error(&failure.message);
            }
            failure.status
        }
        Err(payload) => {
            set_last_error(&format!(
                "internal error (Rust panic): {}",
                panic_message(payload.as_ref())
            ));
            TrStatus::Panic
        }
    }
}

/// Runs an infallible accessor, returning `fallback` if it panics.
pub(crate) fn guard_value<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|payload| {
        set_last_error(&format!(
            "internal error (Rust panic): {}",
            panic_message(payload.as_ref())
        ));
        fallback
    })
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown payload")
}

/// `s` up to its first NUL, as a C string.
fn c_string(s: &str) -> CString {
    let end = s.find('\0').unwrap_or(s.len());
    CString::new(&s[..end]).unwrap_or_default()
}

/// Decodes a NEF into a handle.
pub(crate) fn decode(nef: &[u8]) -> Result<TrRaw, Failure> {
    Ok(TrRaw::new(ticoraw::decode_nef(nef)?))
}

/// The pieces of a `TrDngOptions` that live behind pointers, already read by the caller.
#[derive(Debug, Default)]
pub(crate) struct OptionRefs<'a> {
    pub(crate) original_raw_file_name: Option<&'a CStr>,
    pub(crate) extra_xmp: Option<&'a [u8]>,
}

/// Parses C options into `ticoraw_dng::Options`; `None` gives the defaults.
pub(crate) fn dng_options(
    opts: Option<(TrDngOptions, OptionRefs<'_>)>,
) -> Result<ticoraw_dng::Options, Failure> {
    let Some((o, refs)) = opts else {
        return Ok(ticoraw_dng::Options::default());
    };
    let compression = match o.compression {
        0 => ticoraw_dng::Compression::LosslessJpeg,
        1 => ticoraw_dng::Compression::Uncompressed,
        n => return Err(Failure::invalid(format!("unknown compression value {n}"))),
    };
    let preview = match o.preview {
        0 => ticoraw_dng::Preview::Auto,
        1 => ticoraw_dng::Preview::None,
        n => return Err(Failure::invalid(format!("unknown preview value {n}"))),
    };
    let original_raw_file_name = refs
        .original_raw_file_name
        .map(|s| {
            s.to_str()
                .map(str::to_owned)
                .map_err(|_| Failure::invalid("original_raw_file_name is not UTF-8"))
        })
        .transpose()?;
    Ok(ticoraw_dng::Options {
        compression,
        preview,
        carry_xmp: o.omit_xmp == 0,
        extra_xmp: refs.extra_xmp.filter(|x| !x.is_empty()).map(<[u8]>::to_vec),
        original_raw_file_name,
    })
}

/// Converts a NEF to DNG bytes.
pub(crate) fn convert(nef: &[u8], opts: &ticoraw_dng::Options) -> Result<TrBuffer, Failure> {
    Ok(TrBuffer::binary(ticoraw_dng::convert(nef, opts)?))
}

/// Inspects a NEF and renders the result as a JSON object (see `tr_inspect_json()`).
pub(crate) fn inspect_json(nef: &[u8]) -> Result<TrBuffer, Failure> {
    Ok(TrBuffer::text(inspection_json(&ticoraw::inspect_nef(nef)?)))
}

fn inspection_json(i: &NefInspection) -> String {
    let mut j = String::new();
    // Writing to a String cannot fail.
    let _ = write_inspection(&mut j, i);
    j
}

fn write_inspection(j: &mut String, i: &NefInspection) -> std::fmt::Result {
    j.push_str("{\"make\":");
    json_str(j, &i.make);
    j.push_str(",\"model\":");
    json_str(j, &i.model);
    write!(
        j,
        ",\"width\":{},\"height\":{},\"bits_per_sample\":{},\"codec\":\"{}\",\"compression\":",
        i.width,
        i.height,
        i.bits_per_sample,
        match i.codec {
            RawCodec::TicoRaw => "ticoraw",
            RawCodec::Classic => "classic",
        }
    )?;
    match i.compression {
        Some(c) => {
            write!(j, "{{\"code\":{},\"name\":", c.code())?;
            json_str(j, &c.to_string());
            j.push('}');
        }
        None => j.push_str("null"),
    }
    write!(
        j,
        ",\"strip\":{{\"offset\":{},\"length\":{}}},\"black_level\":",
        i.strip_offset, i.strip_len
    )?;
    match i.black_level {
        Some([a, b, c, d]) => write!(j, "[{a},{b},{c},{d}]")?,
        None => j.push_str("null"),
    }
    j.push_str(",\"codestream\":");
    let Some(cs) = &i.codestream else {
        j.push_str("null}");
        return Ok(());
    };
    write!(
        j,
        "{{\"components\":{},\"component_width\":{},\"component_height\":{},\"bit_depths\":[",
        cs.header.pih.nc, cs.component_width, cs.component_height
    )?;
    for (k, c) in cs.header.components.iter().enumerate() {
        if k > 0 {
            j.push(',');
        }
        write!(j, "{}", c.bit_depth)?;
    }
    write!(
        j,
        "],\"nlx\":{},\"nly\":{},\"slices\":{},\"precincts\":{},\"bands\":{},\
         \"transfer_curve\":{{\"black\":{},\"gain\":{},\"knee\":{}}},\"unsupported\":",
        cs.header.pih.nlx,
        cs.header.pih.nly,
        cs.slices,
        cs.precincts,
        cs.bands,
        cs.curve.black,
        cs.curve.gain,
        cs.curve.knee
    )?;
    match &cs.unsupported {
        Some(why) => json_str(j, why),
        None => j.push_str("null"),
    }
    j.push_str("}}");
    Ok(())
}

fn json_str(j: &mut String, s: &str) {
    j.push('"');
    for c in s.chars() {
        match c {
            '"' => j.push_str("\\\""),
            '\\' => j.push_str("\\\\"),
            '\n' => j.push_str("\\n"),
            '\r' => j.push_str("\\r"),
            '\t' => j.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(j, "\\u{:04x}", u32::from(c));
            }
            c => j.push(c),
        }
    }
    j.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use ticoraw::{CodestreamInfo, NikonCompression, header};

    fn classic() -> NefInspection {
        NefInspection {
            make: "NIKON CORPORATION".into(),
            model: "NIKON \"Z\" 6\n".into(),
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
    fn inspection_json_is_compact_and_escaped() {
        let j = inspection_json(&classic());
        assert_eq!(
            j,
            "{\"make\":\"NIKON CORPORATION\",\"model\":\"NIKON \\\"Z\\\" 6\\n\",\
             \"width\":6048,\"height\":4024,\"bits_per_sample\":14,\"codec\":\"classic\",\
             \"compression\":{\"code\":3,\"name\":\"Lossless compressed\"},\
             \"strip\":{\"offset\":4096,\"length\":1000},\"black_level\":[1008,1008,1008,1008],\
             \"codestream\":null}"
        );
        let mut s = String::new();
        json_str(&mut s, "a\u{1}b");
        assert_eq!(s, "\"a\\u0001b\"");
    }

    #[test]
    fn inspection_json_codestream() {
        let header = header::MainHeader {
            cap: Vec::new(),
            pih: header::PictureHeader {
                lcod: 0,
                ppih: 0,
                plev: 0,
                wf: 64,
                hf: 48,
                cw: 0,
                hsl: 1,
                nc: 4,
                ng: 4,
                ss: 8,
                bw: 20,
                fq: 4,
                br: 4,
                fslc: 0,
                ppoc: 0,
                cpih: 0,
                nlx: 5,
                nly: 1,
                lh: 0,
                rl: 0,
                qpih: 1,
                fs: 0,
                rm: 0,
                extension: Vec::new(),
            },
            components: vec![
                header::ComponentInfo {
                    bit_depth: 14,
                    sx: 1,
                    sy: 1,
                    extension: Vec::new(),
                };
                2
            ],
            weights: Vec::new(),
            other: Vec::new(),
            first_slice: 0,
        };
        let i = NefInspection {
            codec: RawCodec::TicoRaw,
            compression: Some(NikonCompression::HighEfficiencyStar),
            black_level: None,
            codestream: Some(CodestreamInfo {
                header,
                curve: header::TransferCurve {
                    black: 1,
                    gain: 2,
                    knee: 3,
                },
                component_width: 32,
                component_height: 24,
                slices: 2,
                precincts: 12,
                bands: 25,
                unsupported: Some("x".into()),
            }),
            ..classic()
        };
        let j = inspection_json(&i);
        assert!(j.contains("\"black_level\":null"), "{j}");
        assert!(
            j.ends_with(
                "\"codestream\":{\"components\":4,\"component_width\":32,\
                 \"component_height\":24,\"bit_depths\":[14,14],\"nlx\":5,\"nly\":1,\
                 \"slices\":2,\"precincts\":12,\"bands\":25,\
                 \"transfer_curve\":{\"black\":1,\"gain\":2,\"knee\":3},\"unsupported\":\"x\"}}"
            ),
            "{j}"
        );
    }

    #[test]
    fn statuses_mirror_errors() {
        use ticoraw_dng::Error;
        let cases = [
            (
                TicoError::Truncated {
                    what: "x",
                    offset: 0,
                },
                TrStatus::Truncated,
            ),
            (TicoError::Container(String::new()), TrStatus::Container),
            (TicoError::Header(String::new()), TrStatus::Header),
            (TicoError::Unsupported(String::new()), TrStatus::Unsupported),
            (TicoError::Entropy(String::new()), TrStatus::Entropy),
            (TicoError::Jpeg(String::new()), TrStatus::Jpeg),
        ];
        for (e, s) in cases {
            assert_eq!(TrStatus::from(&e), s);
            assert_eq!(TrStatus::from(&Error::Decode(e)), s);
        }
        assert_eq!(
            TrStatus::from(&Error::InvalidImage(String::new())),
            TrStatus::InvalidImage
        );
        assert_eq!(
            TrStatus::from(&Error::InvalidMetadata(String::new())),
            TrStatus::InvalidMetadata
        );
        assert_eq!(
            TrStatus::from(&Error::TooLarge(String::new())),
            TrStatus::TooLarge
        );
        assert_eq!(
            TrStatus::from(&Error::Preview(String::new())),
            TrStatus::Preview
        );
    }

    #[test]
    fn guard_catches_panics_and_records_messages() {
        let status = guard(|| panic!("boom"));
        assert_eq!(status, TrStatus::Panic);
        let msg = LAST_ERROR.with(|c| c.borrow().to_string_lossy().into_owned());
        assert_eq!(msg, "internal error (Rust panic): boom");

        assert_eq!(
            guard(|| Err(Failure::invalid("bad"))),
            TrStatus::InvalidArgument
        );
        let msg = LAST_ERROR.with(|c| c.borrow().to_string_lossy().into_owned());
        assert_eq!(msg, "bad");

        // NotPresent and success leave the message alone.
        assert_eq!(guard(|| Err(Failure::not_present())), TrStatus::NotPresent);
        assert_eq!(guard(|| Ok(())), TrStatus::Ok);
        let msg = LAST_ERROR.with(|c| c.borrow().to_string_lossy().into_owned());
        assert_eq!(msg, "bad");

        assert_eq!(guard_value(7, || panic!("{}", String::from("owned"))), 7);
        let msg = LAST_ERROR.with(|c| c.borrow().to_string_lossy().into_owned());
        assert_eq!(msg, "internal error (Rust panic): owned");
    }

    #[test]
    fn options_parse_and_reject() {
        let zero = TrDngOptions {
            struct_size: DNG_OPTIONS_V1_SIZE as u32,
            compression: 0,
            preview: 0,
            omit_xmp: 0,
            original_raw_file_name: std::ptr::null(),
            extra_xmp: std::ptr::null(),
            extra_xmp_len: 0,
        };
        assert_eq!(
            dng_options(Some((zero, OptionRefs::default()))).unwrap(),
            ticoraw_dng::Options::default()
        );
        assert_eq!(dng_options(None).unwrap(), ticoraw_dng::Options::default());

        let o = TrDngOptions {
            compression: TrCompression::Uncompressed as u32,
            preview: TrPreview::None as u32,
            omit_xmp: 1,
            ..zero
        };
        let refs = OptionRefs {
            original_raw_file_name: Some(c"DSC_0001.NEF"),
            extra_xmp: Some(b"<x/>"),
        };
        assert_eq!(
            dng_options(Some((o, refs))).unwrap(),
            ticoraw_dng::Options {
                compression: ticoraw_dng::Compression::Uncompressed,
                preview: ticoraw_dng::Preview::None,
                carry_xmp: false,
                extra_xmp: Some(b"<x/>".to_vec()),
                original_raw_file_name: Some("DSC_0001.NEF".into()),
            }
        );
        let empty_xmp = OptionRefs {
            extra_xmp: Some(b""),
            ..OptionRefs::default()
        };
        assert_eq!(
            dng_options(Some((zero, empty_xmp))).unwrap().extra_xmp,
            None
        );

        for bad in [
            TrDngOptions {
                compression: 2,
                ..zero
            },
            TrDngOptions { preview: 9, ..zero },
        ] {
            let e = dng_options(Some((bad, OptionRefs::default()))).unwrap_err();
            assert_eq!(e.status, TrStatus::InvalidArgument);
        }
        let not_utf8 = OptionRefs {
            original_raw_file_name: Some(c"\xff"),
            ..OptionRefs::default()
        };
        assert_eq!(
            dng_options(Some((zero, not_utf8))).unwrap_err().status,
            TrStatus::InvalidArgument
        );
    }

    #[test]
    fn strings_stop_at_nul_and_text_buffers_are_terminated() {
        assert_eq!(c_string("ab\0cd").as_bytes(), b"ab");
        let b = TrBuffer::text("{}".into());
        assert_eq!((b.len, &b.bytes[..]), (2, &b"{}\0"[..]));
        let b = TrBuffer::binary(vec![1, 2]);
        assert_eq!((b.len, &b.bytes[..]), (2, &[1u8, 2][..]));
    }
}
