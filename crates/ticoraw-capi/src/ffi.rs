//! The exported `extern "C"` functions: the only module of the crate allowed to use
//! `unsafe`. Each function checks its pointers, turns them into references or slices, and
//! hands them to the safe core in `lib.rs` inside `guard` or `guard_value`, so no Rust
//! panic unwinds into the caller.
//!
//! # Pointer contract (all functions)
//!
//! - An input buffer `(data, len)` must be readable for `len` bytes and must not be written
//!   during the call. `data` may be `NULL` only when `len` is 0.
//! - A handle argument must be `NULL` or a live handle returned by this library and not yet
//!   freed. A handle must not be freed while another thread is using it.
//! - An output pointer must be writable for the stated number of elements. On failure the
//!   output is left unchanged, except that `*out` handles are set to `NULL`.

#![allow(unsafe_code)]

use std::ffi::{CStr, c_char};
use std::ptr;

use crate::{
    DNG_OPTIONS_V1_SIZE, Failure, OptionRefs, TR_ABI_VERSION, TrBuffer, TrCrop, TrDngOptions,
    TrRaw, TrSRational, TrStatus, TrURational, guard, guard_value,
};

/// The library version, for example `"0.1.0"`: a static NUL-terminated string.
#[unsafe(no_mangle)]
pub extern "C" fn tr_version() -> *const c_char {
    const VERSION: &CStr =
        match CStr::from_bytes_with_nul(concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes()) {
            Ok(v) => v,
            Err(_) => c"unknown",
        };
    VERSION.as_ptr()
}

/// The ABI version the library implements (`TR_ABI_VERSION` when it was built). A caller
/// compiled against a header with a higher `TR_ABI_VERSION` may use only what this version
/// provides.
#[unsafe(no_mangle)]
pub extern "C" fn tr_abi_version() -> u32 {
    TR_ABI_VERSION
}

/// The message of the last failed call on the calling thread, as a NUL-terminated UTF-8
/// string; an empty string before any failure. The pointer stays valid until the next call
/// into this library on the same thread. Never `NULL`.
#[unsafe(no_mangle)]
pub extern "C" fn tr_last_error_message() -> *const c_char {
    guard_value(c"".as_ptr(), crate::last_error_ptr)
}

/// Reads an input buffer as a slice.
///
/// # Safety
///
/// `data` must be `NULL` or readable for `len` bytes, unchanged for `'a`.
unsafe fn input<'a>(data: *const u8, len: usize) -> Result<&'a [u8], Failure> {
    if len == 0 {
        return Ok(&[]);
    }
    if data.is_null() {
        return Err(Failure::null("data"));
    }
    if isize::try_from(len).is_err() {
        return Err(Failure::invalid("len exceeds PTRDIFF_MAX"));
    }
    // SAFETY: `data` is non-null, readable for `len` bytes (caller contract) and `len` fits
    // in `isize`; `u8` has no alignment or validity requirements.
    Ok(unsafe { std::slice::from_raw_parts(data, len) })
}

/// A caller's `T **out`, already set to `NULL`. Only ever written, never read: the caller
/// may pass the address of an uninitialised pointer.
struct Out<T>(*mut *mut T);

impl<T> Out<T> {
    /// Checks `out` and clears `*out`.
    ///
    /// # Safety
    ///
    /// `out` must be `NULL` or valid for writing one pointer.
    unsafe fn new(out: *mut *mut T) -> Result<Self, Failure> {
        if out.is_null() {
            return Err(Failure::null("out"));
        }
        // SAFETY: `out` is non-null and writable for one pointer (caller contract); `write`
        // does not read the old, possibly uninitialised, value.
        unsafe { out.write(ptr::null_mut()) };
        Ok(Self(out))
    }

    /// Hands `value` to the caller through `*out`.
    fn set(self, value: T) {
        // SAFETY: `new` checked that `self.0` is writable for one pointer; the caller now
        // owns the box and releases it with the matching `tr_*_free`.
        unsafe { self.0.write(Box::into_raw(Box::new(value))) };
    }
}

/// Borrows a `TrRaw` handle.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle from `tr_decode_nef()`.
unsafe fn raw_ref<'a>(raw: *const TrRaw) -> Result<&'a TrRaw, Failure> {
    // SAFETY: `raw` is `NULL` or points to a live `TrRaw` that is only read while borrowed.
    unsafe { raw.as_ref() }.ok_or_else(|| Failure::null("raw"))
}

/// Writes the value `f` computes to `out[0..N]`. A `NULL` `out` fails before `f` runs, so
/// misuse is reported even when the value is absent.
///
/// # Safety
///
/// `out` must be `NULL` or valid for writing `N` elements of `T`.
unsafe fn write_array<T: Copy, const N: usize>(
    out: *mut T,
    f: impl FnOnce() -> Result<[T; N], Failure>,
) -> Result<(), Failure> {
    if out.is_null() {
        return Err(Failure::null("out"));
    }
    let value = f()?;
    // SAFETY: `out` is non-null and writable for `N` elements of `T` (caller contract); the
    // caller's array may be unaligned when it comes from a packed C struct.
    unsafe { ptr::write_unaligned(out.cast::<[T; N]>(), value) };
    Ok(())
}

/// Decodes a HE/HE\* NEF held in memory.
///
/// On success `*out` receives a handle to release with `tr_raw_free()`; on failure it is set
/// to `NULL` and the status says why. Classic (non-HE) NEFs fail with
/// `TR_STATUS_UNSUPPORTED`.
///
/// # Safety
///
/// `nef` must be readable for `len` bytes (it may be `NULL` when `len` is 0) and `out`
/// writable for one pointer. The library does not keep `nef` after the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_decode_nef(
    nef: *const u8,
    len: usize,
    out: *mut *mut TrRaw,
) -> TrStatus {
    guard(|| {
        // SAFETY: forwarded caller contract for `out`.
        let out = unsafe { Out::new(out) }?;
        // SAFETY: forwarded caller contract for `nef` and `len`.
        let nef = unsafe { input(nef, len) }?;
        out.set(crate::decode(nef)?);
        Ok(())
    })
}

/// Releases a handle from `tr_decode_nef()`. `NULL` is ignored.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle, which must not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_free(raw: *mut TrRaw) {
    guard_value((), || {
        if !raw.is_null() {
            // SAFETY: `raw` came from `Box::into_raw` in `tr_decode_nef` and is freed once.
            drop(unsafe { Box::from_raw(raw) });
        }
    });
}

/// Mosaic width in photosites; 0 for a `NULL` handle.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_width(raw: *const TrRaw) -> usize {
    guard_value(0, || {
        // SAFETY: forwarded caller contract for `raw`.
        unsafe { raw_ref(raw) }.map_or(0, |r| r.raw.image.width)
    })
}

/// Mosaic height in photosites; 0 for a `NULL` handle.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_height(raw: *const TrRaw) -> usize {
    guard_value(0, || {
        // SAFETY: forwarded caller contract for `raw`.
        unsafe { raw_ref(raw) }.map_or(0, |r| r.raw.image.height)
    })
}

/// The CFA mosaic: `width * height` 16-bit samples (14-bit values) in row-major order,
/// owned by the handle and valid until `tr_raw_free()`. `NULL` for a `NULL` handle.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_pixels(raw: *const TrRaw) -> *const u16 {
    guard_value(ptr::null(), || {
        // SAFETY: forwarded caller contract for `raw`.
        unsafe { raw_ref(raw) }.map_or(ptr::null(), |r| r.raw.image.pixels.as_ptr())
    })
}

/// TIFF `Make`, for example `"NIKON CORPORATION"`: a NUL-terminated string owned by the
/// handle. `NULL` for a `NULL` handle.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_make(raw: *const TrRaw) -> *const c_char {
    guard_value(ptr::null(), || {
        // SAFETY: forwarded caller contract for `raw`.
        unsafe { raw_ref(raw) }.map_or(ptr::null(), |r| r.make.as_ptr())
    })
}

/// TIFF `Model`, for example `"NIKON Z6_3"`: a NUL-terminated string owned by the handle.
/// `NULL` for a `NULL` handle.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_model(raw: *const TrRaw) -> *const c_char {
    guard_value(ptr::null(), || {
        // SAFETY: forwarded caller contract for `raw`.
        unsafe { raw_ref(raw) }.map_or(ptr::null(), |r| r.model.as_ptr())
    })
}

/// TIFF orientation (1 = normal, 6 / 8 = portrait); 0 for a `NULL` handle.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_orientation(raw: *const TrRaw) -> u16 {
    guard_value(0, || {
        // SAFETY: forwarded caller contract for `raw`.
        unsafe { raw_ref(raw) }.map_or(0, |r| r.raw.orientation)
    })
}

/// Saturation level of the mosaic; 0 for a `NULL` handle.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_white_level(raw: *const TrRaw) -> u16 {
    guard_value(0, || {
        // SAFETY: forwarded caller contract for `raw`.
        unsafe { raw_ref(raw) }.map_or(0, |r| r.raw.color.white_level)
    })
}

/// Writes the 2x2 CFA pattern (0 = red, 1 = green, 2 = blue, row-major) to `out[0..4]`.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle; `out` must be writable for 4 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_cfa_pattern(raw: *const TrRaw, out: *mut u8) -> TrStatus {
    guard(|| {
        // SAFETY: forwarded caller contract for `raw`.
        let r = unsafe { raw_ref(raw) }?;
        // SAFETY: forwarded caller contract for `out`.
        unsafe { write_array(out, || Ok(r.raw.color.cfa_pattern)) }
    })
}

/// Writes the black level of each 2x2 CFA position (row-major, like the pattern) to
/// `out[0..4]`.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle; `out` must be writable for 4 `uint16_t`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_black_level(raw: *const TrRaw, out: *mut u16) -> TrStatus {
    guard(|| {
        // SAFETY: forwarded caller contract for `raw`.
        let r = unsafe { raw_ref(raw) }?;
        // SAFETY: forwarded caller contract for `out`.
        unsafe { write_array(out, || Ok(r.raw.color.black_level)) }
    })
}

/// Writes the row-major 3x3 XYZ(D65) -> camera matrix to `out[0..9]`.
/// `TR_STATUS_NOT_PRESENT` for cameras without a built-in colour profile.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle; `out` must be writable for 9 `TrSRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_color_matrix(raw: *const TrRaw, out: *mut TrSRational) -> TrStatus {
    guard(|| {
        // SAFETY: forwarded caller contract for `raw`.
        let r = unsafe { raw_ref(raw) }?;
        let m = r.raw.color.color_matrix.ok_or_else(Failure::not_present);
        // SAFETY: forwarded caller contract for `out`.
        unsafe { write_array(out, || Ok(m?.map(|(num, den)| TrSRational { num, den }))) }
    })
}

/// Writes the as-shot white balance as camera-neutral R, G, B rationals to `out[0..3]`.
/// `TR_STATUS_NOT_PRESENT` when the NEF does not record it.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle; `out` must be writable for 3 `TrURational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_as_shot_neutral(
    raw: *const TrRaw,
    out: *mut TrURational,
) -> TrStatus {
    guard(|| {
        // SAFETY: forwarded caller contract for `raw`.
        let r = unsafe { raw_ref(raw) }?;
        let n = r.raw.color.as_shot_neutral.ok_or_else(Failure::not_present);
        // SAFETY: forwarded caller contract for `out`.
        unsafe { write_array(out, || Ok(n?.map(|(num, den)| TrURational { num, den }))) }
    })
}

/// Writes the recommended crop (the camera's active image area) to `*out`.
/// `TR_STATUS_NOT_PRESENT` when the NEF has none that fits the mosaic.
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle; `out` must be writable for one `TrCrop`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_crop(raw: *const TrRaw, out: *mut TrCrop) -> TrStatus {
    guard(|| {
        // SAFETY: forwarded caller contract for `raw`.
        let r = unsafe { raw_ref(raw) }?;
        let crop = r.raw.crop.map(|c| TrCrop {
            x: c.x,
            y: c.y,
            width: c.width,
            height: c.height,
        });
        // SAFETY: forwarded caller contract for `out`.
        unsafe { write_array(out, || Ok([crop.ok_or_else(Failure::not_present)?])) }
    })
}

/// Writes the baseline exposure in EV (the DNG `BaselineExposure`) to `*out`.
/// `TR_STATUS_NOT_PRESENT` when unknown (DNG readers then assume 0).
///
/// # Safety
///
/// `raw` must be `NULL` or a live handle; `out` must be writable for one `TrSRational`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_raw_baseline_exposure(
    raw: *const TrRaw,
    out: *mut TrSRational,
) -> TrStatus {
    guard(|| {
        // SAFETY: forwarded caller contract for `raw`.
        let r = unsafe { raw_ref(raw) }?;
        let be = r.raw.baseline_exposure.ok_or_else(Failure::not_present);
        // SAFETY: forwarded caller contract for `out`.
        unsafe {
            write_array(out, || {
                let (num, den) = be?;
                Ok([TrSRational { num, den }])
            })
        }
    })
}

/// Reads a caller's `TrDngOptions` and the data its pointers name.
///
/// # Safety
///
/// `opts` must be `NULL` or point to at least `struct_size` readable bytes that start with
/// a `TrDngOptions` prefix, whose pointer fields follow the documented contract.
unsafe fn read_options<'a>(
    opts: *const TrDngOptions,
) -> Result<Option<(TrDngOptions, OptionRefs<'a>)>, Failure> {
    if opts.is_null() {
        return Ok(None);
    }
    // SAFETY: a non-null `opts` points to a struct that starts with `uint32_t struct_size`.
    let size = unsafe { ptr::read_unaligned(opts.cast::<u32>()) } as usize;
    if size < DNG_OPTIONS_V1_SIZE {
        return Err(Failure::invalid(format!(
            "options struct_size {size} is smaller than {DNG_OPTIONS_V1_SIZE}"
        )));
    }
    // SAFETY: `struct_size` says at least a full version-1 struct is readable; any fields
    // beyond it are ignored.
    let o = unsafe { ptr::read_unaligned(opts) };
    let original_raw_file_name = if o.original_raw_file_name.is_null() {
        None
    } else {
        // SAFETY: a non-null name is a NUL-terminated string valid for the call.
        Some(unsafe { CStr::from_ptr(o.original_raw_file_name) })
    };
    let extra_xmp = if o.extra_xmp.is_null() {
        if o.extra_xmp_len != 0 {
            return Err(Failure::null("options extra_xmp"));
        }
        None
    } else {
        // SAFETY: a non-null `extra_xmp` is readable for `extra_xmp_len` bytes.
        Some(unsafe { input(o.extra_xmp, o.extra_xmp_len) }?)
    };
    Ok(Some((
        o,
        OptionRefs {
            original_raw_file_name,
            extra_xmp,
        },
    )))
}

/// Decodes a HE/HE\* NEF and writes it as a complete DNG (lossless JPEG tiles, EXIF / GPS /
/// XMP / MakerNote carried over, previews). `opts` may be `NULL` for the defaults.
///
/// On success `*out` receives a buffer to release with `tr_buffer_free()`; on failure it is
/// set to `NULL`.
///
/// # Safety
///
/// `nef` must be readable for `len` bytes (it may be `NULL` when `len` is 0), `opts` must be
/// `NULL` or a valid `TrDngOptions` (see its documentation), and `out` writable for one
/// pointer. Nothing passed in is kept after the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_nef_to_dng(
    nef: *const u8,
    len: usize,
    opts: *const TrDngOptions,
    out: *mut *mut TrBuffer,
) -> TrStatus {
    guard(|| {
        // SAFETY: forwarded caller contract for `out`.
        let out = unsafe { Out::new(out) }?;
        // SAFETY: forwarded caller contract for `nef` and `len`.
        let nef = unsafe { input(nef, len) }?;
        // SAFETY: forwarded caller contract for `opts`.
        let opts = unsafe { read_options(opts) }?;
        let opts = crate::dng_options(opts)?;
        out.set(crate::convert(nef, &opts)?);
        Ok(())
    })
}

/// Describes a NEF without decoding pixels, as a compact JSON object (UTF-8): `make`,
/// `model`, `width`, `height`, `bits_per_sample`, `codec` (`"ticoraw"` or `"classic"`),
/// `compression` (`{code, name}` or `null`), `strip` (`{offset, length}`), `black_level`
/// (4 numbers or `null`) and `codestream` (the JPEG XS layout for TicoRAW strips, else
/// `null`) — the same fields as `ticoraw inspect --json`, without `path`.
///
/// On success `*out` receives a buffer whose data is also NUL-terminated (the NUL is not
/// counted by `tr_buffer_len()`); release it with `tr_buffer_free()`.
///
/// # Safety
///
/// `nef` must be readable for `len` bytes (it may be `NULL` when `len` is 0) and `out`
/// writable for one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_inspect_json(
    nef: *const u8,
    len: usize,
    out: *mut *mut TrBuffer,
) -> TrStatus {
    guard(|| {
        // SAFETY: forwarded caller contract for `out`.
        let out = unsafe { Out::new(out) }?;
        // SAFETY: forwarded caller contract for `nef` and `len`.
        let nef = unsafe { input(nef, len) }?;
        out.set(crate::inspect_json(nef)?);
        Ok(())
    })
}

/// The buffer's bytes, valid until `tr_buffer_free()`; `NULL` for a `NULL` buffer.
///
/// # Safety
///
/// `buf` must be `NULL` or a live buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_buffer_data(buf: *const TrBuffer) -> *const u8 {
    guard_value(ptr::null(), || {
        // SAFETY: `buf` is `NULL` or a live `TrBuffer` (caller contract).
        unsafe { buf.as_ref() }.map_or(ptr::null(), |b| b.bytes.as_ptr())
    })
}

/// The buffer's length in bytes (without the NUL of a JSON buffer); 0 for `NULL`.
///
/// # Safety
///
/// `buf` must be `NULL` or a live buffer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_buffer_len(buf: *const TrBuffer) -> usize {
    // SAFETY: `buf` is `NULL` or a live `TrBuffer` (caller contract).
    guard_value(0, || unsafe { buf.as_ref() }.map_or(0, |b| b.len))
}

/// Releases a buffer. `NULL` is ignored.
///
/// # Safety
///
/// `buf` must be `NULL` or a live buffer, which must not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tr_buffer_free(buf: *mut TrBuffer) {
    guard_value((), || {
        if !buf.is_null() {
            // SAFETY: `buf` came from `Box::into_raw` in this module and is freed once.
            drop(unsafe { Box::from_raw(buf) });
        }
    });
}
