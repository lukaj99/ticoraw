//! WebAssembly bindings for [`ticoraw`] and [`ticoraw_dng`]: decode Nikon HE/HE\* NEFs and
//! convert them to DNG from JavaScript, in browsers (ideally inside a Web Worker) and in
//! Node.js.
//!
//! The JS API, as `wasm-bindgen` exports it:
//!
//! | Function | Returns |
//! |---|---|
//! | `version()` | the crate version |
//! | `inspect(bytes)` | container and codestream facts, no pixel decoding |
//! | `inspectPrefix(prefix, fileLength)` | the same from the first megabyte or two of a file |
//! | `decode(bytes)` | the CFA mosaic as a `Uint16Array` plus colour metadata |
//! | `toDng(bytes, options?)` | a complete DNG as a `Uint8Array` |
//! | `decodeDng(bytes)` | the CFA mosaic of a DNG (to check a conversion) |
//!
//! Every failure is thrown as an `Error` whose `name` is `"TicoError"`, with `kind` set to
//! the Rust error variant (`"Truncated"`, `"Container"`, `"Unsupported"`, ...) and
//! `message` to its text; `Truncated` errors also carry `what` and `offset`. A malformed
//! `toDng` options object throws a `TypeError`.
//!
//! The decoder never panics on any input (it is fuzzed). Should a bug panic anyway, the
//! panic message goes to `console.error` and the instance traps with a
//! `WebAssembly.RuntimeError`; the instance must then be discarded (`js/worker.js` replaces
//! its worker). wasm32 builds abort on panic, so no unwinding ever crosses into JS.
//!
//! The module is single-threaded and does no I/O.

#![warn(missing_docs)]

pub mod model;

use js_sys::{Array, Object, Reflect, Uint8Array, Uint16Array};
use ticoraw::{CodestreamInfo, NefInspection, RawCodec};
use wasm_bindgen::prelude::*;

use crate::model::{ErrorInfo, OptValue};

#[wasm_bindgen(typescript_custom_section)]
const TS_TYPES: &'static str = r#"
/** A rectangle inside the CFA mosaic, in photosites. */
export interface Crop { x: number; y: number; width: number; height: number; }

/** The result of `decode`. */
export interface DecodedRaw {
  /** TIFF `Make`, e.g. `"NIKON CORPORATION"`. */
  make: string;
  /** TIFF `Model`, e.g. `"NIKON Z6_3"`. */
  model: string;
  /** Adobe's camera name, e.g. `"Nikon Z 6 3"` (the DNG `UniqueCameraModel`). */
  uniqueCameraModel: string;
  /** Mosaic width in photosites. */
  width: number;
  /** Mosaic height in photosites. */
  height: number;
  /** `width * height` 14-bit samples, row-major, no black subtraction. */
  pixels: Uint16Array;
  /** 2x2 CFA pattern, row-major, e.g. `"RGGB"`. */
  cfaPattern: string;
  /** Black level per 2x2 CFA position, row-major. */
  blackLevel: [number, number, number, number];
  /** Saturation level. */
  whiteLevel: number;
  /** XYZ(D65) -> camera matrix, row-major; `null` for bodies without a built-in profile. */
  colorMatrix: number[] | null;
  /** As-shot white balance as camera-neutral values (R, G, B). */
  asShotNeutral: [number, number, number] | null;
  /** TIFF orientation (1 = normal, 6 / 8 = portrait). */
  orientation: number;
  /** The camera's recommended crop. */
  crop: Crop | null;
  /** DNG `BaselineExposure` in EV. */
  baselineExposure: number | null;
}

/** The result of `decodeDng`. */
export interface DngCfa {
  width: number;
  height: number;
  pixels: Uint16Array;
  cfaPattern: string;
  blackLevel: [number, number, number, number];
  whiteLevel: number;
}

/** A summary of a TicoRAW codestream's main header. */
export interface CodestreamInfo {
  components: number;
  componentWidth: number;
  componentHeight: number;
  bitDepths: number[];
  nlx: number;
  nly: number;
  slices: number;
  precincts: number;
  bands: number;
  transferCurve: { black: number; gain: number; knee: number };
  /** Why the codestream cannot be decoded, or `null` when it can. */
  unsupported: string | null;
}

/** The result of `inspect` / `inspectPrefix`. */
export interface Inspection {
  make: string;
  model: string;
  width: number;
  height: number;
  bitsPerSample: number;
  /** `"ticoraw"` for HE / HE* strips, `"classic"` for any other Nikon encoding. */
  codec: "ticoraw" | "classic";
  compression: { code: number; name: string } | null;
  strip: { offset: number; length: number };
  blackLevel: [number, number, number, number] | null;
  codestream: CodestreamInfo | null;
}

/** Options of `toDng`. Unknown keys throw a `TypeError`. */
export interface DngOptions {
  /** Raw image storage (default `"lossless-jpeg"`). */
  compression?: "lossless-jpeg" | "uncompressed";
  /** `"none"` skips the full preview; the IFD0 thumbnail is always written. */
  preview?: "auto" | "none";
  /** Copy the NEF's XMP packet (default `true`). */
  carryXmp?: boolean;
  /** An XMP packet that replaces the NEF's, written as given. */
  extraXmp?: Uint8Array | string;
  /** Written as `OriginalRawFileName`, usually the NEF's file name. */
  originalRawFileName?: string;
}

/** The `kind` of a thrown `TicoError`. */
export type TicoErrorKind =
  | "Truncated" | "Container" | "Header" | "Unsupported" | "Entropy" | "Jpeg"
  | "InvalidImage" | "InvalidMetadata" | "TooLarge" | "Preview";

/** Every decoding or conversion failure is thrown as one of these. */
export interface TicoError extends Error {
  name: "TicoError";
  kind: TicoErrorKind;
  /** For `Truncated`: the structure that was cut short. */
  what?: string;
  /** For `Truncated`: the byte offset where it starts or where reading stopped. */
  offset?: number;
}

/**
 * Decodes a HE/HE* NEF and writes it as a complete DNG: lossless JPEG tiles, EXIF, GPS,
 * XMP, the Nikon MakerNote and a preview.
 *
 * @throws {TypeError} for a malformed `options` object.
 * @throws {TicoError} when the file is not a HE/HE* NEF or is malformed.
 */
export function toDng(bytes: Uint8Array, options?: DngOptions | null): Uint8Array;
"#;

/// The crate version, e.g. `"0.1.0"`.
#[wasm_bindgen]
#[must_use]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Parses a NEF's container and codestream headers without decoding pixels.
///
/// @throws {TicoError} when the file is not a NEF this library can read (a classic,
/// non-HE NEF is not an error: its `codec` is `"classic"`).
#[wasm_bindgen(unchecked_return_type = "Inspection")]
pub fn inspect(bytes: &[u8]) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let i = ticoraw::inspect_nef(bytes).map_err(|e| tico_error(&(&e).into()))?;
    inspection(&i)
}

/// Like `inspect`, from just the start of a file of `fileLength` bytes, so a page can
/// classify a `File` from `file.slice()` without reading all of it. Embedded previews can
/// put the raw strip several megabytes in (up to ~8 MB on the Z9); when the prefix is too
/// short the error is `Truncated` with `offset` = the strip start, so read up to
/// `offset + 64 KiB` and retry.
///
/// @throws {TypeError} when `fileLength` is not a non-negative integer.
/// @throws {TicoError} as `inspect` does; kind `"Truncated"` when the prefix is too short.
#[wasm_bindgen(js_name = inspectPrefix, unchecked_return_type = "Inspection")]
pub fn inspect_prefix(
    prefix: &[u8],
    #[wasm_bindgen(js_name = fileLength)] file_length: f64,
) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let len = model::parse_length(file_length).ok_or_else(|| {
        JsValue::from(js_sys::TypeError::new(
            "inspectPrefix: fileLength must be a non-negative integer",
        ))
    })?;
    let i = ticoraw::inspect_nef_prefix(prefix, len).map_err(|e| tico_error(&(&e).into()))?;
    inspection(&i)
}

/// Decodes a HE/HE\* NEF to its CFA mosaic and colour metadata. The mosaic is bit-exact
/// with the CFA image of Adobe's DNG of the same file.
///
/// @throws {TicoError} when the file is not a HE/HE\* NEF or is malformed.
#[wasm_bindgen(unchecked_return_type = "DecodedRaw")]
pub fn decode(bytes: &[u8]) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let raw = ticoraw::decode_nef(bytes).map_err(|e| tico_error(&(&e).into()))?;
    let c = &raw.color;
    let unique = ticoraw::unique_camera_model(&raw.make, &raw.model);
    let crop = match raw.crop {
        Some(r) => obj(&[
            ("x", num(r.x)),
            ("y", num(r.y)),
            ("width", num(r.width)),
            ("height", num(r.height)),
        ])?,
        None => JsValue::NULL,
    };
    let floats = |v: Option<Vec<f64>>| v.map_or(JsValue::NULL, |v| array(v.into_iter()).into());
    obj(&[
        ("make", raw.make.as_str().into()),
        ("model", raw.model.as_str().into()),
        ("uniqueCameraModel", unique.into()),
        ("width", num(raw.image.width)),
        ("height", num(raw.image.height)),
        ("pixels", Uint16Array::from(&raw.image.pixels[..]).into()),
        ("cfaPattern", model::cfa_pattern_name(c.cfa_pattern).into()),
        ("blackLevel", array(c.black_level.into_iter()).into()),
        ("whiteLevel", c.white_level.into()),
        (
            "colorMatrix",
            floats(c.color_matrix.and_then(model::ratios).map(Vec::from)),
        ),
        (
            "asShotNeutral",
            floats(c.as_shot_neutral.and_then(model::ratios).map(Vec::from)),
        ),
        ("orientation", raw.orientation.into()),
        ("crop", crop),
        (
            "baselineExposure",
            raw.baseline_exposure
                .and_then(|r| model::ratios([r]))
                .map_or(JsValue::NULL, |[ev]| ev.into()),
        ),
    ])
}

/// Decodes a HE/HE\* NEF and writes it as a complete DNG: lossless JPEG tiles, EXIF, GPS,
/// XMP, the Nikon MakerNote and a preview (see the `ticoraw-dng` crate).
///
/// Its TypeScript signature is written in `TS_TYPES`, so that `options` can be omitted.
///
/// @throws {TypeError} for a malformed `options` object.
/// @throws {TicoError} when the file is not a HE/HE\* NEF or is malformed.
#[wasm_bindgen(js_name = toDng, skip_typescript)]
pub fn to_dng(bytes: &[u8], options: Option<Object>) -> Result<Vec<u8>, JsValue> {
    install_panic_hook();
    let opts = match options {
        None => ticoraw_dng::Options::default(),
        Some(o) => {
            let entries = option_entries(&o)?;
            model::parse_options(entries).map_err(|m| JsValue::from(js_sys::TypeError::new(&m)))?
        }
    };
    ticoraw_dng::convert(bytes, &opts).map_err(|e| tico_error(&(&e).into()))
}

/// Decodes the full-resolution CFA image of a DNG (lossless JPEG or uncompressed), for
/// example to check that `toDng` round-trips.
///
/// @throws {TicoError} for a DNG without a readable CFA image.
#[wasm_bindgen(js_name = decodeDng, unchecked_return_type = "DngCfa")]
pub fn decode_dng(bytes: &[u8]) -> Result<JsValue, JsValue> {
    install_panic_hook();
    let d = ticoraw::dng::decode_dng_cfa(bytes).map_err(|e| tico_error(&(&e).into()))?;
    obj(&[
        ("width", num(d.image.width)),
        ("height", num(d.image.height)),
        ("pixels", Uint16Array::from(&d.image.pixels[..]).into()),
        ("cfaPattern", model::cfa_pattern_name(d.cfa_pattern).into()),
        ("blackLevel", array(d.black_level.into_iter()).into()),
        ("whiteLevel", d.white_level.into()),
    ])
}

fn inspection(i: &NefInspection) -> Result<JsValue, JsValue> {
    let compression = match i.compression {
        Some(c) => obj(&[("code", c.code().into()), ("name", c.to_string().into())])?,
        None => JsValue::NULL,
    };
    let codestream = match &i.codestream {
        Some(cs) => codestream(cs)?,
        None => JsValue::NULL,
    };
    obj(&[
        ("make", i.make.as_str().into()),
        ("model", i.model.as_str().into()),
        ("width", num(i.width)),
        ("height", num(i.height)),
        ("bitsPerSample", i.bits_per_sample.into()),
        (
            "codec",
            match i.codec {
                RawCodec::TicoRaw => "ticoraw",
                RawCodec::Classic => "classic",
            }
            .into(),
        ),
        ("compression", compression),
        (
            "strip",
            obj(&[
                ("offset", num(i.strip_offset)),
                ("length", num(i.strip_len)),
            ])?,
        ),
        (
            "blackLevel",
            i.black_level
                .map_or(JsValue::NULL, |b| array(b.into_iter()).into()),
        ),
        ("codestream", codestream),
    ])
}

fn codestream(cs: &CodestreamInfo) -> Result<JsValue, JsValue> {
    let pih = &cs.header.pih;
    obj(&[
        ("components", pih.nc.into()),
        ("componentWidth", num(cs.component_width)),
        ("componentHeight", num(cs.component_height)),
        (
            "bitDepths",
            array(cs.header.components.iter().map(|c| c.bit_depth)).into(),
        ),
        ("nlx", pih.nlx.into()),
        ("nly", pih.nly.into()),
        ("slices", num(cs.slices)),
        ("precincts", num(cs.precincts)),
        ("bands", num(cs.bands)),
        (
            "transferCurve",
            obj(&[
                ("black", (cs.curve.black as f64).into()),
                ("gain", (cs.curve.gain as f64).into()),
                ("knee", (cs.curve.knee as f64).into()),
            ])?,
        ),
        (
            "unsupported",
            cs.unsupported
                .as_deref()
                .map_or(JsValue::NULL, JsValue::from),
        ),
    ])
}

/// Reads the own enumerable properties of a `toDng` options object.
fn option_entries(o: &Object) -> Result<Vec<(String, OptValue)>, JsValue> {
    let v: &JsValue = o.as_ref();
    if !v.is_object() || Array::is_array(v) {
        return Err(js_sys::TypeError::new("toDng options must be an object").into());
    }
    let mut out = Vec::new();
    for key in Object::keys(o) {
        let name = key.as_string().unwrap_or_default();
        let value = Reflect::get(o, &key)?;
        out.push((name, opt_value(&value)));
    }
    Ok(out)
}

fn opt_value(v: &JsValue) -> OptValue {
    if v.is_undefined() || v.is_null() {
        OptValue::Absent
    } else if let Some(b) = v.as_bool() {
        OptValue::Bool(b)
    } else if let Some(s) = v.as_string() {
        OptValue::Str(s)
    } else if let Some(u) = v.dyn_ref::<Uint8Array>() {
        OptValue::Bytes(u.to_vec())
    } else if Array::is_array(v) {
        OptValue::Other("an Array".into())
    } else {
        OptValue::Other(format!(
            "a {}",
            v.js_typeof().as_string().unwrap_or_default()
        ))
    }
}

/// An `Error` named `TicoError` carrying `kind`, and `what` / `offset` for truncation.
/// If a field cannot be set (impossible on a fresh `Error`), the engine's exception is
/// thrown instead.
fn tico_error(info: &ErrorInfo) -> JsValue {
    let e = js_sys::Error::new(&info.message);
    e.set_name("TicoError");
    let mut fields = vec![("kind", JsValue::from(info.kind))];
    if let Some(w) = info.what {
        fields.push(("what", w.into()));
    }
    if let Some(o) = info.offset {
        fields.push(("offset", num(o)));
    }
    fields
        .into_iter()
        .try_for_each(|(k, v)| Reflect::set(&e, &k.into(), &v).map(drop))
        .map_or_else(|thrown| thrown, |()| e.into())
}

fn obj(fields: &[(&str, JsValue)]) -> Result<JsValue, JsValue> {
    let o = Object::new();
    for (k, v) in fields {
        Reflect::set(&o, &JsValue::from_str(k), v)?;
    }
    Ok(o.into())
}

fn array<T: Into<JsValue>>(items: impl Iterator<Item = T>) -> Array {
    items.map(Into::into).collect()
}

/// Sizes and offsets as JS numbers (exact below 2^53, far beyond any wasm32 buffer).
fn num(n: usize) -> JsValue {
    (n as f64).into()
}

/// Sends panic messages to `console.error` before the instance traps.
fn install_panic_hook() {
    #[cfg(target_arch = "wasm32")]
    {
        static HOOK: std::sync::Once = std::sync::Once::new();
        HOOK.call_once(|| {
            std::panic::set_hook(Box::new(|info| console_error(&format!("ticoraw: {info}"))));
        });
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console, js_name = error)]
    fn console_error(message: &str);
}
