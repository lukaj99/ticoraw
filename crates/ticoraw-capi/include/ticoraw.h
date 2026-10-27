/*
 * ticoraw C API: decode Nikon High Efficiency (HE / HE*) NEF raw files and convert them
 * to DNG. https://github.com/lukaj99/ticoraw
 *
 * SPDX-License-Identifier: MIT OR Apache-2.0
 *
 * Generated from crates/ticoraw-capi by cbindgen (scripts/gen-header.sh). Do not edit.
 *
 * Every fallible function returns a TrStatus; TR_STATUS_OK (0) is success, and
 * tr_last_error_message() describes any other value. Handles are released only with
 * their tr_*_free function; freeing NULL is a no-op.
 */

#ifndef TICORAW_H
#define TICORAW_H

#include <stddef.h>
#include <stdint.h>

// Version of the C ABI described by `ticoraw.h`. It increases whenever a function, status,
// constant or struct field is added; nothing is ever removed or changed.
#define TR_ABI_VERSION 1

// Result of every fallible `tr_*` function. `TR_STATUS_OK` is 0; every other value is a
// failure whose message `tr_last_error_message()` returns.
//
// The codes 10..=15 mirror the variants of `TicoError` and 20..=23 those of
// `ticoraw_dng::Error`. New codes may be added; treat unknown ones as generic failures.
enum TrStatus
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : int32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  // Success.
  TR_STATUS_OK = 0,
  // A pointer argument that must not be `NULL` was `NULL`.
  TR_STATUS_NULL_POINTER = 1,
  // An argument is out of range: an unknown option value, a `struct_size` that is too
  // small, a string that is not UTF-8, or a length above `PTRDIFF_MAX`.
  TR_STATUS_INVALID_ARGUMENT = 2,
  // An optional value is absent (for example no colour matrix for an unknown camera).
  // The output was not written. This is not an error: no message is recorded.
  TR_STATUS_NOT_PRESENT = 3,
  // The input ended before a structure was complete (`TicoError::Truncated`).
  TR_STATUS_TRUNCATED = 10,
  // The TIFF / NEF / DNG container is malformed (`TicoError::Container`).
  TR_STATUS_CONTAINER = 11,
  // A JPEG XS marker segment is malformed (`TicoError::Header`).
  TR_STATUS_HEADER = 12,
  // Valid input outside what the decoder supports, such as a classic (non-HE) NEF
  // (`TicoError::Unsupported`).
  TR_STATUS_UNSUPPORTED = 13,
  // Entropy-coded data is inconsistent (`TicoError::Entropy`).
  TR_STATUS_ENTROPY = 14,
  // A lossless JPEG stream is malformed (`TicoError::Jpeg`).
  TR_STATUS_JPEG = 15,
  // The decoded image is inconsistent (`ticoraw_dng::Error::InvalidImage`).
  TR_STATUS_INVALID_IMAGE = 20,
  // Carried metadata is malformed (`ticoraw_dng::Error::InvalidMetadata`).
  TR_STATUS_INVALID_METADATA = 21,
  // The DNG would exceed the 4 GiB TIFF limit (`ticoraw_dng::Error::TooLarge`).
  TR_STATUS_TOO_LARGE = 22,
  // The preview JPEG encoder failed (`ticoraw_dng::Error::Preview`).
  TR_STATUS_PREVIEW = 23,
  // A bug in the library: a Rust panic was caught at the C boundary. The message says
  // where; please report it.
  TR_STATUS_PANIC = 100,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum TrStatus TrStatus;
#else
typedef int32_t TrStatus;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

// Values of `TrDngOptions::compression`.
enum TrCompression
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  // Lossless JPEG in 256x256 tiles, about the size of Adobe's DNGs (the default).
  TR_COMPRESSION_LOSSLESS_JPEG = 0,
  // One uncompressed 16-bit strip: about three times larger, simplest to read.
  TR_COMPRESSION_UNCOMPRESSED = 1,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum TrCompression TrCompression;
#else
typedef uint32_t TrCompression;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

// Values of `TrDngOptions::preview`.
enum TrPreview
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  // The camera's embedded JPEG when there is a usable one, otherwise a half-size
  // rendering of the raw data (the default).
  TR_PREVIEW_AUTO = 0,
  // No full-size preview; the IFD0 thumbnail is still written.
  TR_PREVIEW_NONE = 1,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum TrPreview TrPreview;
#else
typedef uint32_t TrPreview;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

// A byte buffer owned by the library (opaque): a DNG file or a JSON document. Read with
// `tr_buffer_data()` and `tr_buffer_len()`, released with
// `tr_buffer_free()`.
typedef struct TrBuffer TrBuffer;

// A decoded raw image (opaque). Created by `tr_decode_nef()`, read with the
// `tr_raw_*` accessors, released with `tr_raw_free()`. It is immutable, so several
// threads may read one handle at once.
typedef struct TrRaw TrRaw;

// A signed rational, as TIFF `SRATIONAL`.
typedef struct TrSRational {
  // Numerator.
  int32_t num;
  // Denominator.
  int32_t den;
} TrSRational;

// An unsigned rational, as TIFF `RATIONAL`.
typedef struct TrURational {
  // Numerator.
  uint32_t num;
  // Denominator.
  uint32_t den;
} TrURational;

// A rectangle inside the CFA mosaic, in photosites.
typedef struct TrCrop {
  // First column.
  size_t x;
  // First row.
  size_t y;
  // Width in columns.
  size_t width;
  // Height in rows.
  size_t height;
} TrCrop;

// Options for `tr_nef_to_dng()`. A zero-initialised struct with `struct_size` set
// gives the defaults, as does passing `NULL`:
//
// ```c
// TrDngOptions opts = {0};
// opts.struct_size = sizeof opts;
// opts.original_raw_file_name = "DSC_0001.NEF";
// ```
//
// Fields are only ever appended, and a new field's zero value always means the earlier
// behaviour. A library reads the fields it knows and ignores any beyond them.
typedef struct TrDngOptions {
  // `sizeof(TrDngOptions)` as the caller was compiled. Must be at least the size of the
  // version-1 struct, or the call fails with `TR_STATUS_INVALID_ARGUMENT`.
  uint32_t struct_size;
  // A `TrCompression` value.
  uint32_t compression;
  // A `TrPreview` value.
  uint32_t preview;
  // Nonzero: do not copy the NEF's XMP packet (`extra_xmp` is still written).
  uint32_t omit_xmp;
  // Written as `OriginalRawFileName` (usually the NEF's file name without directories):
  // a NUL-terminated UTF-8 string, or `NULL` for none. Only read during the call.
  const char *original_raw_file_name;
  // An XMP packet to write instead of the NEF's, `extra_xmp_len` bytes long, or `NULL`
  // for none. A length of 0 also means none. Only read during the call.
  const uint8_t *extra_xmp;
  // Length of `extra_xmp` in bytes.
  size_t extra_xmp_len;
} TrDngOptions;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

// The library version, for example `"0.1.0"`: a static NUL-terminated string.
const char *tr_version(void);

// The ABI version the library implements (`TR_ABI_VERSION` when it was built). A caller
// compiled against a header with a higher `TR_ABI_VERSION` may use only what this version
// provides.
uint32_t tr_abi_version(void);

// The message of the last failed call on the calling thread, as a NUL-terminated UTF-8
// string; an empty string before any failure. The pointer stays valid until the next call
// into this library on the same thread. Never `NULL`.
const char *tr_last_error_message(void);

// Decodes a HE/HE\* NEF held in memory.
//
// On success `*out` receives a handle to release with `tr_raw_free()`; on failure it is set
// to `NULL` and the status says why. Classic (non-HE) NEFs fail with
// `TR_STATUS_UNSUPPORTED`.
//
// # Safety
//
// `nef` must be readable for `len` bytes (it may be `NULL` when `len` is 0) and `out`
// writable for one pointer. The library does not keep `nef` after the call.
TrStatus tr_decode_nef(const uint8_t *nef, size_t len, struct TrRaw **out);

// Releases a handle from `tr_decode_nef()`. `NULL` is ignored.
//
// # Safety
//
// `raw` must be `NULL` or a live handle, which must not be used afterwards.
void tr_raw_free(struct TrRaw *raw);

// Mosaic width in photosites; 0 for a `NULL` handle.
//
// # Safety
//
// `raw` must be `NULL` or a live handle.
size_t tr_raw_width(const struct TrRaw *raw);

// Mosaic height in photosites; 0 for a `NULL` handle.
//
// # Safety
//
// `raw` must be `NULL` or a live handle.
size_t tr_raw_height(const struct TrRaw *raw);

// The CFA mosaic: `width * height` 16-bit samples (14-bit values) in row-major order,
// owned by the handle and valid until `tr_raw_free()`. `NULL` for a `NULL` handle.
//
// # Safety
//
// `raw` must be `NULL` or a live handle.
const uint16_t *tr_raw_pixels(const struct TrRaw *raw);

// TIFF `Make`, for example `"NIKON CORPORATION"`: a NUL-terminated string owned by the
// handle. `NULL` for a `NULL` handle.
//
// # Safety
//
// `raw` must be `NULL` or a live handle.
const char *tr_raw_make(const struct TrRaw *raw);

// TIFF `Model`, for example `"NIKON Z6_3"`: a NUL-terminated string owned by the handle.
// `NULL` for a `NULL` handle.
//
// # Safety
//
// `raw` must be `NULL` or a live handle.
const char *tr_raw_model(const struct TrRaw *raw);

// TIFF orientation (1 = normal, 6 / 8 = portrait); 0 for a `NULL` handle.
//
// # Safety
//
// `raw` must be `NULL` or a live handle.
uint16_t tr_raw_orientation(const struct TrRaw *raw);

// Saturation level of the mosaic; 0 for a `NULL` handle.
//
// # Safety
//
// `raw` must be `NULL` or a live handle.
uint16_t tr_raw_white_level(const struct TrRaw *raw);

// Writes the 2x2 CFA pattern (0 = red, 1 = green, 2 = blue, row-major) to `out[0..4]`.
//
// # Safety
//
// `raw` must be `NULL` or a live handle; `out` must be writable for 4 bytes.
TrStatus tr_raw_cfa_pattern(const struct TrRaw *raw, uint8_t *out);

// Writes the black level of each 2x2 CFA position (row-major, like the pattern) to
// `out[0..4]`.
//
// # Safety
//
// `raw` must be `NULL` or a live handle; `out` must be writable for 4 `uint16_t`.
TrStatus tr_raw_black_level(const struct TrRaw *raw, uint16_t *out);

// Writes the row-major 3x3 XYZ(D65) -> camera matrix to `out[0..9]`.
// `TR_STATUS_NOT_PRESENT` for cameras without a built-in colour profile.
//
// # Safety
//
// `raw` must be `NULL` or a live handle; `out` must be writable for 9 `TrSRational`.
TrStatus tr_raw_color_matrix(const struct TrRaw *raw, struct TrSRational *out);

// Writes the as-shot white balance as camera-neutral R, G, B rationals to `out[0..3]`.
// `TR_STATUS_NOT_PRESENT` when the NEF does not record it.
//
// # Safety
//
// `raw` must be `NULL` or a live handle; `out` must be writable for 3 `TrURational`.
TrStatus tr_raw_as_shot_neutral(const struct TrRaw *raw, struct TrURational *out);

// Writes the recommended crop (the camera's active image area) to `*out`.
// `TR_STATUS_NOT_PRESENT` when the NEF has none that fits the mosaic.
//
// # Safety
//
// `raw` must be `NULL` or a live handle; `out` must be writable for one `TrCrop`.
TrStatus tr_raw_crop(const struct TrRaw *raw, struct TrCrop *out);

// Writes the baseline exposure in EV (the DNG `BaselineExposure`) to `*out`.
// `TR_STATUS_NOT_PRESENT` when unknown (DNG readers then assume 0).
//
// # Safety
//
// `raw` must be `NULL` or a live handle; `out` must be writable for one `TrSRational`.
TrStatus tr_raw_baseline_exposure(const struct TrRaw *raw, struct TrSRational *out);

// Decodes a HE/HE\* NEF and writes it as a complete DNG (lossless JPEG tiles, EXIF / GPS /
// XMP / MakerNote carried over, previews). `opts` may be `NULL` for the defaults.
//
// On success `*out` receives a buffer to release with `tr_buffer_free()`; on failure it is
// set to `NULL`.
//
// # Safety
//
// `nef` must be readable for `len` bytes (it may be `NULL` when `len` is 0), `opts` must be
// `NULL` or a valid `TrDngOptions` (see its documentation), and `out` writable for one
// pointer. Nothing passed in is kept after the call.
TrStatus tr_nef_to_dng(const uint8_t *nef,
                       size_t len,
                       const struct TrDngOptions *opts,
                       struct TrBuffer **out);

// Describes a NEF without decoding pixels, as a compact JSON object (UTF-8): `make`,
// `model`, `width`, `height`, `bits_per_sample`, `codec` (`"ticoraw"` or `"classic"`),
// `compression` (`{code, name}` or `null`), `strip` (`{offset, length}`), `black_level`
// (4 numbers or `null`) and `codestream` (the JPEG XS layout for TicoRAW strips, else
// `null`) — the same fields as `ticoraw inspect --json`, without `path`.
//
// On success `*out` receives a buffer whose data is also NUL-terminated (the NUL is not
// counted by `tr_buffer_len()`); release it with `tr_buffer_free()`.
//
// # Safety
//
// `nef` must be readable for `len` bytes (it may be `NULL` when `len` is 0) and `out`
// writable for one pointer.
TrStatus tr_inspect_json(const uint8_t *nef, size_t len, struct TrBuffer **out);

// The buffer's bytes, valid until `tr_buffer_free()`; `NULL` for a `NULL` buffer.
//
// # Safety
//
// `buf` must be `NULL` or a live buffer.
const uint8_t *tr_buffer_data(const struct TrBuffer *buf);

// The buffer's length in bytes (without the NUL of a JSON buffer); 0 for `NULL`.
//
// # Safety
//
// `buf` must be `NULL` or a live buffer.
size_t tr_buffer_len(const struct TrBuffer *buf);

// Releases a buffer. `NULL` is ignored.
//
// # Safety
//
// `buf` must be `NULL` or a live buffer, which must not be used afterwards.
void tr_buffer_free(struct TrBuffer *buf);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* TICORAW_H */
