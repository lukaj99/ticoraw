//! `dng::decode_dng_cfa` on arbitrary bytes. A decoded image must have the size it claims.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(cfa) = ticoraw::dng::decode_dng_cfa(data) {
        assert_eq!(cfa.image.pixels.len(), cfa.image.width * cfa.image.height);
    }
});
