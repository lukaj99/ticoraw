//! `decode_ticoraw` on arbitrary bytes. A decoded image must have the size it claims.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(img) = ticoraw::decode_ticoraw(data) {
        assert_eq!(img.pixels.len(), img.width * img.height);
    }
});
