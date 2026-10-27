//! Structure-aware codestream target: the synthetic encoder builds a valid TicoRAW
//! codestream from fuzzed options (frame size, slicing, coding modes, quantisation, raw
//! GCLIs, padding), optionally wrapped in a NEF.
//!
//! Without edits the decoder must reproduce exactly what was encoded, and the NEF must
//! decode and survive the DNG round trip. With edits (byte XORs at fuzzed positions, which
//! reach deep into valid precinct data far faster than byte-level mutation does) only
//! "no panic" is required.
#![no_main]

use libfuzzer_sys::arbitrary::{self, Arbitrary};
use libfuzzer_sys::fuzz_target;
use ticoraw_fuzz::synth::encoder::{self, Options};
use ticoraw_fuzz::synth::tiff::{self, NefSpec, Order};

const MODELS: [&str; 9] = [
    "NIKON Z6_3",
    "NIKON Z 8",
    "NIKON Z 9",
    "NIKON Z f",
    "NIKON Z5_2",
    "NIKON Z50_2",
    "NIKON ZR",
    "NIKON Z 99",
    "",
];

#[derive(Debug, Arbitrary)]
struct Input {
    /// CFA width `16 + 2 * (width % 64)`.
    width: u8,
    /// CFA height `4 * (1 + height % 16)`.
    height: u8,
    /// Precincts per slice `1 + slice_precincts % 16`.
    slice_precincts: u8,
    /// Bit mask over the coding modes D = 0..=3 (empty means D = 0 only).
    modes: u8,
    raw_percent: u8,
    q_max: u8,
    deadzone: bool,
    run_mode: bool,
    padding: bool,
    gcli_max: u8,
    planes_max: u8,
    seed: u64,
    nef: Option<Nef>,
    /// `(position, xor)` pairs applied to the codestream after encoding.
    edits: Vec<(u16, u8)>,
}

#[derive(Debug, Arbitrary)]
struct Nef {
    little_endian: bool,
    model: u8,
    high_efficiency: bool,
    black_level: Option<[u16; 4]>,
    wb: Option<[(u32, u32); 2]>,
    cfa_pattern: Option<[u8; 4]>,
    maker_note: bool,
}

impl Input {
    fn options(&self) -> Options {
        let mut o = Options::new(
            16 + 2 * (self.width as usize % 64),
            4 * (1 + self.height as usize % 16),
            1 + self.slice_precincts as usize % 16,
        );
        o.modes = (0..4u8).filter(|d| self.modes & (1 << d) != 0).collect();
        if o.modes.is_empty() {
            o.modes = vec![0];
        }
        o.raw_percent = u64::from(self.raw_percent % 101);
        o.q_max = self.q_max % 25;
        o.deadzone = self.deadzone;
        o.run_mode = self.run_mode;
        o.padding = self.padding;
        // Inside the range where the encoder's expectation is exact (see Options).
        o.gcli_max = 1 + i32::from(self.gcli_max % 16);
        o.planes_max = 1 + i32::from(self.planes_max % 15);
        o
    }
}

impl Nef {
    fn spec(&self, width: usize, height: usize) -> NefSpec {
        let mut s = NefSpec::new(width, height);
        s.order = if self.little_endian {
            Order::Little
        } else {
            Order::Big
        };
        s.model = MODELS[self.model as usize % MODELS.len()].into();
        s.compression_code = Some(if self.high_efficiency { 13 } else { 14 });
        s.black_level = self.black_level;
        s.wb = self.wb;
        s.cfa_pattern = self.cfa_pattern;
        s.maker_note = self.maker_note;
        s
    }
}

fuzz_target!(|input: Input| {
    let opts = input.options();
    let encoded = encoder::encode_random(&opts, input.seed);
    let mut cs = encoded.codestream.clone();

    if input.edits.is_empty() {
        let expected = ticoraw_fuzz::expected_image(&encoded);
        let img = ticoraw::decode_ticoraw(&cs).expect("a valid codestream decodes");
        assert!(
            img == expected,
            "decoded image differs from the encoded one"
        );
        if let Some(nef) = &input.nef {
            let file = tiff::build_nef(&nef.spec(opts.width, opts.height), &cs);
            ticoraw::inspect_nef(&file).expect("a valid NEF inspects");
            let raw = ticoraw::decode_nef(&file).expect("a valid NEF decodes");
            assert!(
                raw.image == expected,
                "NEF image differs from the encoded one"
            );
            ticoraw_fuzz::assert_dng_round_trip(&raw);
        }
        return;
    }

    for &(at, xor) in &input.edits {
        let i = at as usize % cs.len();
        cs[i] ^= xor;
    }
    let _ = ticoraw::decode_ticoraw(&cs);
    if let Some(nef) = &input.nef {
        let file = tiff::build_nef(&nef.spec(opts.width, opts.height), &cs);
        let _ = ticoraw::inspect_nef(&file);
        let _ = ticoraw::decode_nef(&file);
    }
});
