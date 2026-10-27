//! `inspect_nef_prefix` with a fuzzed `file_len`: the first 8 bytes choose it (see
//! `ticoraw_fuzz::split_file_len`), the rest is the prefix.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (prefix, file_len) = ticoraw_fuzz::split_file_len(data);
    let _ = ticoraw::inspect_nef_prefix(prefix, file_len);
});
