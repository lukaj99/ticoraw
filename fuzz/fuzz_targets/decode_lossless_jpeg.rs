//! `dng::decode_lossless_jpeg` on arbitrary bytes. A decoded frame must have the size it
//! claims.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(j) = ticoraw::dng::decode_lossless_jpeg(data) {
        assert_eq!(j.samples.len(), j.width * j.height);
    }
});
