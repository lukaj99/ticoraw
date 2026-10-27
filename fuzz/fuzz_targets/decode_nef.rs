//! `decode_nef` on arbitrary bytes. Any frame it accepts must also survive the
//! uncompressed-DNG round trip.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(raw) = ticoraw::decode_nef(data) {
        ticoraw_fuzz::assert_dng_round_trip(&raw);
    }
});
