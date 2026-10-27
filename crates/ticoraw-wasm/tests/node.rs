//! The JS-facing API on a synthetic NEF, run inside a real wasm instance in Node.js:
//!
//! ```text
//! wasm-pack test --node crates/ticoraw-wasm
//! ```
//!
//! The NEF comes from the `ticoraw` test encoder (`crates/ticoraw/tests/common`), so this
//! needs no sample files. Natively the file compiles to nothing; `src/model.rs` has the
//! native tests.
#![cfg(target_arch = "wasm32")]

#[path = "../../ticoraw/tests/common/mod.rs"]
mod common;

use common::encoder::{self, Options as CsOptions};
use common::tiff::{NefSpec, build_nef};
use js_sys::{Object, Reflect, Uint8Array, Uint16Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::wasm_bindgen_test;

const W: usize = 64;
const H: usize = 48;

fn nef() -> Vec<u8> {
    let cs = encoder::encode_random(&CsOptions::new(W, H, 2), 7).codestream;
    build_nef(&NefSpec::new(W, H), &cs)
}

fn get(v: &JsValue, key: &str) -> JsValue {
    Reflect::get(v, &key.into()).unwrap()
}

fn num(v: &JsValue, key: &str) -> f64 {
    get(v, key)
        .as_f64()
        .unwrap_or_else(|| panic!("{key} is not a number"))
}

fn string(v: &JsValue, key: &str) -> String {
    get(v, key)
        .as_string()
        .unwrap_or_else(|| panic!("{key} is not a string"))
}

fn numbers(v: &JsValue, key: &str) -> Vec<f64> {
    js_sys::Array::from(&get(v, key))
        .iter()
        .map(|x| x.as_f64().unwrap())
        .collect()
}

fn options(fields: &[(&str, JsValue)]) -> Object {
    let o = Object::new();
    for (k, v) in fields {
        Reflect::set(&o, &(*k).into(), v).unwrap();
    }
    o
}

/// The thrown value is an `Error` named `TicoError`; returns its `kind`.
fn tico_kind(e: &JsValue) -> String {
    let err = e.dyn_ref::<js_sys::Error>().expect("an Error instance");
    assert_eq!(String::from(err.name()), "TicoError");
    string(e, "kind")
}

fn is_type_error(e: &JsValue) -> bool {
    e.is_instance_of::<js_sys::TypeError>()
}

#[wasm_bindgen_test]
fn version_is_the_crate_version() {
    assert_eq!(ticoraw_wasm::version(), env!("CARGO_PKG_VERSION"));
}

#[wasm_bindgen_test]
fn inspect_reports_container_and_codestream() {
    let nef = nef();
    let i = ticoraw_wasm::inspect(&nef).unwrap();
    assert_eq!(string(&i, "model"), "NIKON Z6_3");
    assert_eq!(string(&i, "codec"), "ticoraw");
    assert_eq!(num(&i, "width"), W as f64);
    assert_eq!(num(&i, "bitsPerSample"), 14.0);
    assert_eq!(num(&get(&i, "compression"), "code"), 14.0);
    assert_eq!(numbers(&i, "blackLevel"), [1008.0, 1009.0, 1010.0, 1011.0]);
    let cs = get(&i, "codestream");
    assert_eq!(num(&cs, "componentWidth"), (W / 2) as f64);
    assert_eq!(num(&cs, "bands"), 25.0);
    assert!(get(&cs, "unsupported").is_null());

    // A prefix that holds every structure gives the same answer.
    let p = ticoraw_wasm::inspect_prefix(&nef, nef.len() as f64).unwrap();
    assert_eq!(js_sys::JSON::stringify(&p), js_sys::JSON::stringify(&i));
    let e = ticoraw_wasm::inspect_prefix(&nef, -1.0).unwrap_err();
    assert!(is_type_error(&e));
    let e = ticoraw_wasm::inspect_prefix(&nef[..16], nef.len() as f64).unwrap_err();
    assert_eq!(tico_kind(&e), "Truncated");
}

#[wasm_bindgen_test]
fn decode_matches_the_rust_api() {
    let nef = nef();
    let want = ticoraw::decode_nef(&nef).unwrap();
    let d = ticoraw_wasm::decode(&nef).unwrap();
    assert_eq!(num(&d, "width"), W as f64);
    assert_eq!(num(&d, "height"), H as f64);
    let px = get(&d, "pixels").dyn_into::<Uint16Array>().unwrap();
    assert_eq!(px.to_vec(), want.image.pixels);
    assert_eq!(string(&d, "make"), "NIKON CORPORATION");
    assert_eq!(string(&d, "uniqueCameraModel"), "Nikon Z 6 3");
    assert_eq!(string(&d, "cfaPattern"), "RGGB");
    assert_eq!(numbers(&d, "blackLevel"), [1008.0, 1009.0, 1010.0, 1011.0]);
    assert_eq!(num(&d, "whiteLevel"), 15892.0);
    assert_eq!(num(&d, "orientation"), 1.0);
    assert_eq!(num(&d, "baselineExposure"), 0.25);
    let m = numbers(&d, "colorMatrix");
    assert_eq!(m.len(), 9);
    assert!((m[0] - 1.1206).abs() < 1e-12);
    let n = numbers(&d, "asShotNeutral");
    assert!((n[0] - 512.0 / 924.0).abs() < 1e-12);
    assert_eq!(n[1], 1.0);
    assert!(
        get(&d, "crop").is_null(),
        "the synthetic MakerNote has no crop"
    );
}

#[wasm_bindgen_test]
fn to_dng_round_trips_with_every_compression() {
    let nef = nef();
    let want = ticoraw::decode_nef(&nef).unwrap().image.pixels;
    let mut sizes = Vec::new();
    for opts in [
        None,
        Some(options(&[("compression", "lossless-jpeg".into())])),
        Some(options(&[("compression", "uncompressed".into())])),
        Some(options(&[
            ("compression", "uncompressed".into()),
            ("preview", "none".into()),
            ("carryXmp", false.into()),
            ("originalRawFileName", "SYN_0001.NEF".into()),
            ("extraXmp", Uint8Array::from(&b"<x:xmpmeta/>"[..]).into()),
        ])),
        Some(options(&[
            ("extraXmp", "<x:xmpmeta/>".into()),
            ("preview", JsValue::NULL),
        ])),
    ] {
        let dng = ticoraw_wasm::to_dng(&nef, opts).unwrap();
        let back = ticoraw_wasm::decode_dng(&dng).unwrap();
        let px = get(&back, "pixels").dyn_into::<Uint16Array>().unwrap();
        assert_eq!(px.to_vec(), want);
        assert_eq!(string(&back, "cfaPattern"), "RGGB");
        assert_eq!(
            numbers(&back, "blackLevel"),
            [1008.0, 1009.0, 1010.0, 1011.0]
        );
        sizes.push(dng.len());
    }
    assert_eq!(sizes[0], sizes[1], "lossless JPEG is the default");
    assert_ne!(sizes[2], sizes[0], "uncompressed differs: {sizes:?}");
    assert!(
        sizes[3] < sizes[2],
        "preview \"none\" drops the preview: {sizes:?}"
    );
}

#[wasm_bindgen_test]
fn malformed_options_throw_type_errors() {
    let nef = nef();
    for o in [
        options(&[("compresion", "uncompressed".into())]),
        options(&[("compression", "zip".into())]),
        options(&[("carryXmp", "no".into())]),
        options(&[("extraXmp", 3.into())]),
        js_sys::Array::new().unchecked_into(),
    ] {
        let e = ticoraw_wasm::to_dng(&nef, Some(o)).unwrap_err();
        assert!(is_type_error(&e), "{e:?}");
    }
}

#[wasm_bindgen_test]
fn failures_are_tico_errors_with_the_rust_kind_and_message() {
    let e = ticoraw_wasm::decode(b"not a raw file").unwrap_err();
    let want = ticoraw::decode_nef(b"not a raw file").unwrap_err();
    assert_eq!(tico_kind(&e), "Container");
    assert_eq!(string(&e, "message"), want.to_string());

    let nef = nef();
    let cut = &nef[..nef.len() / 2];
    let e = ticoraw_wasm::to_dng(cut, None).unwrap_err();
    assert_eq!(tico_kind(&e), "Truncated");
    assert!(get(&e, "what").is_string());
    assert!(num(&e, "offset") >= 0.0);

    // A NEF's raw IFD is a CFA image, but not one a DNG reader decodes.
    let e = ticoraw_wasm::decode_dng(&nef).unwrap_err();
    assert_eq!(tico_kind(&e), "Unsupported");
}

#[wasm_bindgen_test]
fn hostile_input_never_traps() {
    // Every truncation and a sweep of single-byte corruptions must come back as values.
    let nef = nef();
    let mut rng = common::Rng::new(3);
    for cut in (0..nef.len()).step_by(97) {
        assert!(ticoraw_wasm::decode(&nef[..cut]).is_err());
    }
    for _ in 0..200 {
        let mut m = nef.clone();
        let at = rng.below(m.len() as u64) as usize;
        m[at] ^= rng.byte() | 1;
        let _ = ticoraw_wasm::inspect(&m);
        let _ = ticoraw_wasm::to_dng(&m, None);
    }
}
