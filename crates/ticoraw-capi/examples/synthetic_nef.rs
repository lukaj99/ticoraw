//! Test helper for the C tests: writes a synthetic HE\* NEF (a TicoRAW codestream from the
//! `ticoraw` test encoder in a minimal NEF container) and prints what decoding it must give.
//!
//! ```text
//! cargo run -p ticoraw-capi --example synthetic_nef -- out.NEF
//! 64 48 <fnv1a-64 of the pixels, hex>
//! ```
//!
//! The hash is FNV-1a (64-bit) over the decoded samples as little-endian 16-bit words;
//! `tests/c/test_capi.c` computes the same over `tr_raw_pixels`.

#[path = "../../ticoraw/tests/common/mod.rs"]
mod common;

use std::process::ExitCode;

use common::encoder::{self, Options};
use common::tiff::{NefSpec, build_nef};

/// Frame size of the synthetic NEF.
const W: usize = 64;
const H: usize = 48;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: synthetic_nef <out.NEF>");
        return ExitCode::from(2);
    };
    let codestream = encoder::encode_random(&Options::new(W, H, 2), 7).codestream;
    let nef = build_nef(&NefSpec::new(W, H), &codestream);
    let raw = match ticoraw::decode_nef(&nef) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("synthetic_nef: the synthetic NEF does not decode: {e}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::write(&path, &nef) {
        eprintln!("synthetic_nef: writing {}: {e}", path.to_string_lossy());
        return ExitCode::FAILURE;
    }
    let hash = raw
        .image
        .pixels
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, p| {
            p.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        });
    println!("{} {} {hash:016x}", raw.image.width, raw.image.height);
    ExitCode::SUCCESS
}
