//! The C ABI called through its `extern "C"` functions, exactly as a C caller would: results
//! must equal the Rust API's, failures must give the mirrored status and message, every
//! pointer misuse the contract allows (`NULL`, short option structs, bad values) must be
//! rejected, and mutated input must never reach the panic path.

// Calling the exported functions needs `unsafe`, as it does from C. Runs of accessor calls
// on one live handle share a block and a SAFETY comment.
#![allow(unsafe_code, clippy::multiple_unsafe_ops_per_block)]

#[path = "../../ticoraw/tests/common/mod.rs"]
mod common;

use std::ffi::{CStr, c_char};
use std::ptr;

use common::Rng;
use common::encoder::{self, Options as CsOptions};
use common::tiff::{NefSpec, build_nef};
use ticoraw_capi::ffi::{
    tr_abi_version, tr_buffer_data, tr_buffer_free, tr_buffer_len, tr_decode_nef, tr_inspect_json,
    tr_last_error_message, tr_nef_to_dng, tr_raw_as_shot_neutral, tr_raw_baseline_exposure,
    tr_raw_black_level, tr_raw_cfa_pattern, tr_raw_color_matrix, tr_raw_crop, tr_raw_free,
    tr_raw_height, tr_raw_make, tr_raw_model, tr_raw_orientation, tr_raw_pixels,
    tr_raw_white_level, tr_raw_width, tr_version,
};
use ticoraw_capi::{
    TR_ABI_VERSION, TrBuffer, TrCompression, TrCrop, TrDngOptions, TrPreview, TrRaw, TrSRational,
    TrStatus, TrURational,
};

const W: usize = 64;
const H: usize = 48;

fn nef() -> Vec<u8> {
    let cs = encoder::encode_random(&CsOptions::new(W, H, 2), 7).codestream;
    build_nef(&NefSpec::new(W, H), &cs)
}

/// A non-null pointer that must be overwritten with `NULL` on failure.
fn dangling<T>() -> *mut T {
    ptr::NonNull::dangling().as_ptr()
}

fn last_error() -> String {
    // SAFETY: `tr_last_error_message` never returns NULL and its string is valid until the
    // next call into the library on this thread; it is copied before any such call.
    unsafe { CStr::from_ptr(tr_last_error_message()) }
        .to_string_lossy()
        .into_owned()
}

fn c_str(p: *const c_char) -> String {
    assert!(!p.is_null());
    // SAFETY: `p` is a NUL-terminated string owned by a live handle.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// Owns a `TrRaw` handle for the duration of a test.
#[derive(Debug)]
struct Raw(*mut TrRaw);

impl Drop for Raw {
    fn drop(&mut self) {
        // SAFETY: the handle came from `tr_decode_nef` and is freed once.
        unsafe { tr_raw_free(self.0) };
    }
}

fn decode(nef: &[u8]) -> Result<Raw, (TrStatus, String)> {
    let mut out = dangling();
    // SAFETY: `nef` is readable for its length and `out` is a valid out pointer.
    let status = unsafe { tr_decode_nef(nef.as_ptr(), nef.len(), &raw mut out) };
    if status == TrStatus::Ok {
        assert!(!out.is_null());
        Ok(Raw(out))
    } else {
        assert!(out.is_null(), "*out is cleared on failure");
        Err((status, last_error()))
    }
}

/// Takes a buffer's bytes and frees it.
fn take(buf: *mut TrBuffer) -> Vec<u8> {
    // SAFETY: `buf` is a live buffer; its data is valid for `tr_buffer_len` bytes.
    let len = unsafe { tr_buffer_len(buf) };
    // SAFETY: as above.
    let data = unsafe { tr_buffer_data(buf) };
    // SAFETY: `data` is valid for `len` bytes until the buffer is freed below.
    let bytes = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
    // SAFETY: freed once, not used afterwards.
    unsafe { tr_buffer_free(buf) };
    bytes
}

fn to_dng(nef: &[u8], opts: *const TrDngOptions) -> Result<Vec<u8>, (TrStatus, String)> {
    let mut out = dangling();
    // SAFETY: `nef` is readable, `opts` is NULL or a valid options struct, `out` writable.
    let status = unsafe { tr_nef_to_dng(nef.as_ptr(), nef.len(), opts, &raw mut out) };
    if status == TrStatus::Ok {
        Ok(take(out))
    } else {
        assert!(out.is_null(), "*out is cleared on failure");
        Err((status, last_error()))
    }
}

fn inspect(nef: &[u8]) -> Result<Vec<u8>, (TrStatus, String)> {
    let mut out = dangling();
    // SAFETY: `nef` is readable and `out` writable.
    let status = unsafe { tr_inspect_json(nef.as_ptr(), nef.len(), &raw mut out) };
    if status == TrStatus::Ok {
        // SAFETY: `out` is a live JSON buffer, whose data is NUL-terminated after `len`.
        let nul = unsafe { *tr_buffer_data(out).add(tr_buffer_len(out)) };
        assert_eq!(nul, 0, "JSON buffers are NUL-terminated");
        Ok(take(out))
    } else {
        assert!(out.is_null(), "*out is cleared on failure");
        Err((status, last_error()))
    }
}

fn default_options() -> TrDngOptions {
    TrDngOptions {
        struct_size: size_of::<TrDngOptions>() as u32,
        compression: TrCompression::LosslessJpeg as u32,
        preview: TrPreview::Auto as u32,
        omit_xmp: 0,
        original_raw_file_name: ptr::null(),
        extra_xmp: ptr::null(),
        extra_xmp_len: 0,
    }
}

#[test]
fn version_and_abi() {
    assert_eq!(c_str(tr_version()), env!("CARGO_PKG_VERSION"));
    assert_eq!(tr_abi_version(), TR_ABI_VERSION);
}

#[test]
fn decode_matches_the_rust_api() {
    let nef = nef();
    let want = ticoraw::decode_nef(&nef).unwrap();
    let raw = decode(&nef).unwrap();
    let r = raw.0;
    // SAFETY: `r` is a live handle and every out pointer names a local array of the
    // documented length.
    unsafe {
        assert_eq!(tr_raw_width(r), W);
        assert_eq!(tr_raw_height(r), H);
        let px = std::slice::from_raw_parts(tr_raw_pixels(r), W * H);
        assert_eq!(px, &want.image.pixels[..]);
        assert_eq!(c_str(tr_raw_make(r)), want.make);
        assert_eq!(c_str(tr_raw_model(r)), "NIKON Z6_3");
        assert_eq!(tr_raw_orientation(r), want.orientation);
        assert_eq!(tr_raw_white_level(r), want.color.white_level);

        let mut cfa = [9u8; 4];
        assert_eq!(tr_raw_cfa_pattern(r, cfa.as_mut_ptr()), TrStatus::Ok);
        assert_eq!(cfa, want.color.cfa_pattern);
        let mut black = [0u16; 4];
        assert_eq!(tr_raw_black_level(r, black.as_mut_ptr()), TrStatus::Ok);
        assert_eq!(black, [1008, 1009, 1010, 1011]);

        let mut m = [TrSRational::default(); 9];
        assert_eq!(tr_raw_color_matrix(r, m.as_mut_ptr()), TrStatus::Ok);
        let want_m = want.color.color_matrix.unwrap();
        assert_eq!(m.map(|q| (q.num, q.den)), want_m);

        let mut n = [TrURational::default(); 3];
        assert_eq!(tr_raw_as_shot_neutral(r, n.as_mut_ptr()), TrStatus::Ok);
        assert_eq!(
            n.map(|q| (q.num, q.den)),
            want.color.as_shot_neutral.unwrap()
        );

        let mut be = TrSRational::default();
        let status = tr_raw_baseline_exposure(r, &raw mut be);
        match want.baseline_exposure {
            Some(v) => assert_eq!((status, (be.num, be.den)), (TrStatus::Ok, v)),
            None => assert_eq!(status, TrStatus::NotPresent),
        }
    }
}

#[test]
fn absent_values_report_not_present_without_a_message() {
    let nef = nef();
    // No MakerNote crop in the synthetic NEF; an unknown model has no colour matrix.
    let unknown = {
        let cs = encoder::encode_random(&CsOptions::new(W, H, 2), 7).codestream;
        let mut spec = NefSpec::new(W, H);
        spec.model = "NIKON Z 99".into();
        build_nef(&spec, &cs)
    };
    let a = decode(&nef).unwrap();
    let b = decode(&unknown).unwrap();
    assert_eq!(decode(&[]).unwrap_err().0, TrStatus::Truncated);
    let before = last_error();
    assert_ne!(before, "");
    let mut crop = TrCrop {
        x: 7,
        ..TrCrop::default()
    };
    let mut m = [TrSRational::default(); 9];
    // SAFETY: live handles and writable outputs of the documented size.
    unsafe {
        assert_eq!(tr_raw_crop(a.0, &raw mut crop), TrStatus::NotPresent);
        assert_eq!(
            tr_raw_color_matrix(b.0, m.as_mut_ptr()),
            TrStatus::NotPresent
        );
    }
    assert_eq!(crop.x, 7, "the output is untouched");
    assert_eq!(last_error(), before, "NotPresent records no message");
}

#[test]
fn dng_matches_the_rust_api() {
    let nef = nef();
    let lossless = to_dng(&nef, ptr::null()).unwrap();
    assert_eq!(
        lossless,
        ticoraw_dng::convert(&nef, &ticoraw_dng::Options::default()).unwrap()
    );
    let zeroed = default_options();
    assert_eq!(to_dng(&nef, &raw const zeroed).unwrap(), lossless);

    let xmp = b"<x:xmpmeta>merged</x:xmpmeta>";
    let opts = TrDngOptions {
        compression: TrCompression::Uncompressed as u32,
        preview: TrPreview::None as u32,
        omit_xmp: 1,
        original_raw_file_name: c"SYN_0001.NEF".as_ptr(),
        extra_xmp: xmp.as_ptr(),
        extra_xmp_len: xmp.len(),
        ..default_options()
    };
    let want = ticoraw_dng::convert(
        &nef,
        &ticoraw_dng::Options {
            compression: ticoraw_dng::Compression::Uncompressed,
            preview: ticoraw_dng::Preview::None,
            carry_xmp: false,
            extra_xmp: Some(xmp.to_vec()),
            original_raw_file_name: Some("SYN_0001.NEF".into()),
        },
    )
    .unwrap();
    let dng = to_dng(&nef, &raw const opts).unwrap();
    assert_eq!(dng, want);
    let back = ticoraw::dng::decode_dng_cfa(&dng).unwrap();
    assert_eq!(back.image, ticoraw::decode_nef(&nef).unwrap().image);
}

#[test]
fn options_struct_size_is_checked_and_newer_structs_accepted() {
    /// A caller compiled against a future header with more fields.
    #[repr(C)]
    struct Newer {
        v1: TrDngOptions,
        future: [u64; 4],
    }

    let nef = nef();
    let short = TrDngOptions {
        struct_size: 8,
        ..default_options()
    };
    let (status, msg) = to_dng(&nef, &raw const short).unwrap_err();
    assert_eq!(status, TrStatus::InvalidArgument);
    assert!(msg.contains("struct_size 8"), "{msg}");
    let newer = Newer {
        v1: TrDngOptions {
            struct_size: size_of::<Newer>() as u32,
            ..default_options()
        },
        future: [0; 4],
    };
    assert!(to_dng(&nef, (&raw const newer).cast()).is_ok());
    assert_eq!(newer.future, [0; 4]);

    for (opts, want) in [
        (
            TrDngOptions {
                compression: 7,
                ..default_options()
            },
            "unknown compression value 7",
        ),
        (
            TrDngOptions {
                preview: 2,
                ..default_options()
            },
            "unknown preview value 2",
        ),
        (
            TrDngOptions {
                extra_xmp_len: 5,
                ..default_options()
            },
            "options extra_xmp is NULL",
        ),
        (
            TrDngOptions {
                original_raw_file_name: c"\xffname".as_ptr(),
                ..default_options()
            },
            "original_raw_file_name is not UTF-8",
        ),
    ] {
        let (status, msg) = to_dng(&nef, &raw const opts).unwrap_err();
        assert!(
            matches!(status, TrStatus::InvalidArgument | TrStatus::NullPointer),
            "{status:?}"
        );
        assert_eq!(msg, want);
    }
}

#[test]
fn inspect_json_describes_the_file() {
    let nef = nef();
    let json = String::from_utf8(inspect(&nef).unwrap()).unwrap();
    assert!(
        json.starts_with(
            "{\"make\":\"NIKON CORPORATION\",\"model\":\"NIKON Z6_3\",\"width\":64,\
             \"height\":48,\"bits_per_sample\":14,\"codec\":\"ticoraw\",\
             \"compression\":{\"code\":14,\"name\":\"High Efficiency* (HE*)\"},"
        ),
        "{json}"
    );
    assert!(
        json.contains("\"black_level\":[1008,1009,1010,1011],\"codestream\":{\"components\":4,"),
        "{json}"
    );
    assert!(json.ends_with("\"unsupported\":null}}"), "{json}");
}

#[test]
fn failures_mirror_the_rust_errors() {
    let nef = nef();
    let cases: [&[u8]; 4] = [
        &[],
        &nef[..nef.len() / 2],
        b"not a tiff at all",
        &nef[..200],
    ];
    for input in cases {
        let want = ticoraw::decode_nef(input).unwrap_err();
        let (status, msg) = decode(input).unwrap_err();
        assert_eq!((status, msg), (TrStatus::from(&want), want.to_string()));
        let want = ticoraw_dng::convert(input, &ticoraw_dng::Options::default()).unwrap_err();
        let (status, msg) = to_dng(input, ptr::null()).unwrap_err();
        assert_eq!((status, msg), (TrStatus::from(&want), want.to_string()));
    }
    let want = ticoraw::inspect_nef(b"II*\0").unwrap_err();
    let (status, msg) = inspect(b"II*\0").unwrap_err();
    assert_eq!((status, msg), (TrStatus::from(&want), want.to_string()));
}

#[test]
fn null_pointers_are_rejected() {
    let nef = nef();
    // SAFETY: every pointer is NULL or valid; NULL is what is being tested.
    unsafe {
        assert_eq!(
            tr_decode_nef(nef.as_ptr(), nef.len(), ptr::null_mut()),
            TrStatus::NullPointer
        );
        assert_eq!(last_error(), "out is NULL");
        let mut out = dangling();
        assert_eq!(
            tr_decode_nef(ptr::null(), 10, &raw mut out),
            TrStatus::NullPointer
        );
        assert!(out.is_null());
        assert_eq!(last_error(), "data is NULL");
        // NULL with length 0 is an empty input.
        assert_eq!(
            tr_decode_nef(ptr::null(), 0, &raw mut out),
            TrStatus::Truncated
        );

        let mut buf = dangling();
        assert_eq!(
            tr_nef_to_dng(ptr::null(), 1, ptr::null(), &raw mut buf),
            TrStatus::NullPointer
        );
        assert!(buf.is_null());
        assert_eq!(
            tr_inspect_json(nef.as_ptr(), nef.len(), ptr::null_mut()),
            TrStatus::NullPointer
        );

        // Accessors on a NULL handle.
        let none: *const TrRaw = ptr::null();
        assert_eq!(tr_raw_width(none), 0);
        assert_eq!(tr_raw_height(none), 0);
        assert!(tr_raw_pixels(none).is_null());
        assert!(tr_raw_make(none).is_null());
        assert!(tr_raw_model(none).is_null());
        assert_eq!(tr_raw_orientation(none), 0);
        assert_eq!(tr_raw_white_level(none), 0);
        let mut cfa = [0u8; 4];
        assert_eq!(
            tr_raw_cfa_pattern(none, cfa.as_mut_ptr()),
            TrStatus::NullPointer
        );
        assert_eq!(last_error(), "raw is NULL");
        assert!(tr_buffer_data(ptr::null()).is_null());
        assert_eq!(tr_buffer_len(ptr::null()), 0);

        // NULL outputs on a live handle.
        let raw = decode(&nef).unwrap();
        assert_eq!(
            tr_raw_black_level(raw.0, ptr::null_mut()),
            TrStatus::NullPointer
        );
        assert_eq!(tr_raw_crop(raw.0, ptr::null_mut()), TrStatus::NullPointer);
        assert_eq!(last_error(), "out is NULL");

        // Freeing NULL is a no-op.
        tr_raw_free(ptr::null_mut());
        tr_buffer_free(ptr::null_mut());
    }
}

#[test]
fn last_error_is_per_thread() {
    assert_eq!(
        decode(b"garbage, long enough to be no TIFF").unwrap_err().0,
        TrStatus::Container
    );
    let mine = last_error();
    let other = std::thread::spawn(|| {
        let fresh = last_error();
        let _ = decode(&[]);
        (fresh, last_error())
    })
    .join()
    .unwrap();
    assert_eq!(other.0, "", "a new thread starts with no message");
    assert!(other.1.starts_with("truncated"), "{}", other.1);
    assert_eq!(last_error(), mine);
}

#[test]
fn mutated_input_never_reaches_the_panic_path() {
    let nef = nef();
    let mut rng = Rng::new(0x5eed);
    for round in 0..300 {
        let mut m = nef.clone();
        match round % 3 {
            0 => {
                for _ in 0..=rng.below(8) {
                    let at = rng.below(m.len() as u64) as usize;
                    m[at] ^= 1 << rng.below(8);
                }
            }
            1 => m.truncate(rng.below(m.len() as u64) as usize),
            _ => {
                let at = rng.below(m.len() as u64) as usize;
                let n = (1 + rng.below(16) as usize).min(m.len() - at);
                m[at..at + n].copy_from_slice(&rng.bytes(n));
            }
        }
        for status in [
            decode(&m).err().map(|e| e.0),
            to_dng(&m, ptr::null()).err().map(|e| e.0),
            inspect(&m).err().map(|e| e.0),
        ]
        .into_iter()
        .flatten()
        {
            assert_ne!(status, TrStatus::Panic, "round {round}: {}", last_error());
            assert_ne!(status, TrStatus::Ok);
        }
    }
}
