//! `inspect_nef` on arbitrary bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = ticoraw::inspect_nef(data);
});
