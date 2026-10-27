# ticoraw design

Date: 2026-10-27. Status: implemented; the decoder is bit-exact against Adobe for Nikon
Z6III HE\* files (15 reference DNGs of 11 distinct NEFs), and decodes the public HE/HE\*
samples of every other body cleanly ([compat.md](../../compat.md)). The format itself is
specified in [FORMAT.md](../../FORMAT.md); this note records how the code is organised and
why, and where the original plan was wrong.

## Goals

- Decode Nikon HE/HE\* NEF raw strips to exactly the CFA values Adobe produces.
- A library that embeds anywhere: pure functions over byte slices, no I/O, no `unsafe`,
  optional parallelism, builds for `wasm32-unknown-unknown`.
- A CLI for inspection, conversion to DNG, verification against a reference DNG and bulk
  scanning.
- Clean-room: behaviour learned from public descriptions and confirmed against Adobe
  output, no code copied from LGPL decoders.

Non-goals: classic NEF compressions, demosaicing, colour rendering, an encoder for real use.

## Architecture: functional core, imperative shell

```
             imperative shell                         functional core (crates/ticoraw)
 ┌──────────────────────────────┐       ┌──────────────────────────────────────────────┐
 │ ticoraw-cli                  │ &[u8] │ container ── tiff          (NEF → strip)      │
 │  read files, walk dirs,      │──────▶│ header                     (markers, PIH, ... │
 │  parse flags, write DNG/PGM, │       │ entropy ── bitstream       (slices → coeffs)  │
 │  print text/JSON, exit codes │◀──────│ wavelet                    (5/3 IDWT)         │
 │                              │ value │ mct                        (Star-Tetrix, RGGB)│
 └──────────────────────────────┘       │ dng                        (DNG read/write)   │
                                        └──────────────────────────────────────────────┘
```

The library never opens a file, reads the clock or spawns a process. Parallelism is the
only effect it controls, and it is invisible in the results: the `parallel` feature swaps
three small helpers in `lib.rs` (`par_map_owned`, `par_chunks_mut`,
`par_chunks_mut_collect`) between rayon and their sequential equivalents, and the output
is identical either way.

### Module map

| Module | Role |
|---|---|
| `tiff` | Minimal TIFF/IFD reader over a byte slice, either byte order, with a base offset for embedded TIFFs |
| `container` | `parse_nef` / `parse_nef_prefix`: finds the full-resolution CFA IFD, borrows the raw strip (or the part of it a file prefix holds), classifies it (`RawCodec::TicoRaw` vs `Classic`) and reads IFD0's orientation and the MakerNote fields needed (black levels, WB, crop area, compression code) |
| `header` | JPEG XS main header to `MainHeader` (CAP, PIH incl. vendor bytes, CDT, WGT, others kept verbatim); `TransferCurve` from the PIH vendor bytes |
| `bitstream` | MSB-first `BitReader`: fixed-width reads, unary codes, alignment, skips, all bounds-checked |
| `entropy` | `Layout::new`: validates the supported profile and builds bands, packets and the WGT mapping once; `decode_slice`: precinct headers, GTLIs, packets, sub-packets, dequantisation, coefficient placement |
| `wavelet` | `synthesize_1d`, `synthesize_row` (horizontal cascade), `synthesize_columns` (frame-wide vertical) |
| `mct` | `inverse_star_tetrix` (four lifting steps), `assemble_rggb` (interleave through the curve) |
| `dng` | `decode_dng_cfa` (lossless JPEG SOF3 or uncompressed CFA from a DNG, for ground truth) and `decode_lossless_jpeg` (one SOF3 stream to a `LosslessJpeg` with its dimensions); `encode_uncompressed_dng`, fallible, writing `Orientation`, `DefaultCropOrigin` / `DefaultCropSize`, `BaselineExposure` and Adobe's `UniqueCameraModel` name |
| `lib` | Public entry points `inspect_nef`, `inspect_nef_prefix`, `decode_nef`, `decode_ticoraw`; `NikonCompression::{from_code, code}`; slice framing; per-model colour profile, Adobe camera name and baseline exposure for the Z6III, Z8, Z9, Zf, Z5II, Z50II and ZR, all matching Adobe DNG Converter 18.7 |
| `error` | `TicoError` |

### API stability

The crate root (the entry points above and the data types they return, plus `TicoError`)
and `dng::{decode_dng_cfa, encode_uncompressed_dng}` are the stable API. The stage modules
(`bitstream`, `container`, `entropy`, `header`, `mct`, `tiff`, `wavelet`) are public for
testing and advanced use, and their API may change in minor releases. The crate is built
with `#![warn(missing_docs)]`.

### Data flow

```
NEF bytes
  └─ container::parse_nef ─▶ strip (&[u8]) + metadata
       └─ header::parse_main_header ─▶ MainHeader ─▶ TransferCurve, entropy::Layout
            └─ frame_slices (SLH + precinct lengths) ─▶ 64 byte ranges
                 └─ entropy::decode_slice  ×64, in parallel ─▶ 4 coefficient planes (i32, << Fq)
                      └─ wavelet::synthesize_row / synthesize_columns  (components 0, 1, 3)
                           └─ mct::inverse_star_tetrix ─▶ mct::assemble_rggb(curve)
                                └─ CfaImage (6064 × 4040 u16) ─▶ DecodedRaw ─▶ dng::encode_uncompressed_dng
```

Slices are framed from the precinct length fields before any entropy decoding, so each
slice gets its own disjoint input range and its own disjoint rows of the four planes. The
only cross-slice dependency, the vertical wavelet, runs afterwards over whole planes.

## Error handling

Every expected failure is a `TicoError` value, never a panic:

| Variant | Meaning |
|---|---|
| `Truncated { what, offset }` | input ended inside a structure |
| `Container(String)` | malformed TIFF/NEF/DNG |
| `Header(String)` | malformed or misordered marker segment, WGT length different from the band count, codestream too small for its frame, wrong SLH index, missing EOC |
| `Unsupported(String)` | well-formed, but outside the TicoRAW profile this decoder implements (FORMAT.md §20) |
| `Entropy(String)` | a sub-packet overruns its declared length, a GCLI exceeds `Bw`, or a code group implies impossible bit planes |
| `Jpeg(String)` | malformed lossless JPEG inside a DNG |

Validation happens once at the boundary: `Layout::new` checks the header against the
profile, and the decode loops then index tables they know to be consistent. All reads from
the input go through bounds-checked accessors (`BitReader`, `be16`, `slice::get`).
`Unsupported` is deliberately separate from the malformed-input variants, so that a caller
(the CLI's `scan`, say) can tell "not this decoder's format" from "corrupt file".

Hostile input is bounded before allocation: a codestream must hold at least
`Wf * Hf / 16` bytes, DNG frames and tiles are bounded by the file length with checked
products, TIFF offset arithmetic is checked, and the IFD walk visits at most 64 IFDs and
64 SubIFD offsets per IFD. The inverse 5/3 and Star-Tetrix lifting steps use wrapping
arithmetic, so a crafted stream that passes every check can decode to garbage but never
panics.

## CLI shell

`ticoraw-cli` owns all I/O. Output files are written atomically: to a temporary file in
the output's directory, then renamed into place (`--force`) or hard-linked only if the
target does not exist, so a failed write never leaves a partial file or destroys an old
one. `decode` refuses to overwrite its own input, comparing canonical paths and, on Unix,
device and inode numbers (which catches hard links), and warns on stderr when the model
has no built-in colour profile. `scan` accepts `.nef` and `.nrw` in any case, refuses
non-regular files, and without `--decode` reads only a prefix of each file, grown as far
as `inspect_nef_prefix` needs to reach the codestream header.

## Testing strategy

1. **Unit tests** beside each module, with fixed inputs and outputs and no mocks: bit reader
   semantics, header parsing of the real Nikon header bytes, transfer-curve test vectors,
   5/3 perfect reconstruction for every length 2..40 and for the 5-level pyramid, vertical
   reconstruction, Star-Tetrix flat-field invariance, dequantisation, DNG round trip,
   lossless-JPEG decoding.
2. **Synthetic encoder round trip.** A test-only encoder writes TicoRAW codestreams in the
   same profile from known planes, exercising code paths camera files rarely reach (raw
   GCLIs, run mode, significance, every band width), and the decoder must reproduce the
   input exactly.
3. **Robustness.** Truncated, bit-flipped and length-corrupted codestreams and containers
   must return `Err`, never panic or hang.
4. **Ground truth, environment-gated.** With `TICORAW_CORPUS` pointing at a folder of
   NEFs and DNGs converted from them, the suite discovers every DNG whose
   `OriginalRawFileName` tag names a TicoRAW NEF in the folder. The two CFA images must
   match at every photosite, and the CFA pattern and levels must agree. The current local
   corpus has 15 such DNGs of 11 distinct Z6III HE\* NEFs (14 from Adobe Lightroom, one
   from Topaz Photo AI with an identical CFA image), all bit-exact. Without the variable
   the suite is skipped, so CI and contributors without camera files still pass. The
   folder is only read.

## Corrections to the original plan

The first plan was written from secondary sources before any frame had been decoded. The
points below supersede it; each was settled by decoding against Adobe output.

1. **Slice and precinct count.** The plan's figure of 253 is wrong (it is
   `ceil(4040 / 16)`, which reads `Hsl` as CFA lines). `Hsl` counts *precincts*, a precinct is `2^NLy = 2`
   component lines, and the component planes are `Hf / 2 = 2020` lines tall. A frame
   therefore has **1010 precinct rows in 64 slices** of 16 (the last slice holds 2).
2. **Transfer curve.** Fitted quadratics such as `(v + 16158)^2 / 191488 + 1050` with a
   floor at 1050 are not exact. The exact curve is parameterised
   by the PIH vendor bytes (`B = 1008`, `A = 84429`, `C = 16255`), has an odd extension
   below the knee instead of a floor, applies the gain with half-up rounding before
   squaring, and folds two rounding stages into `(q + 163840) >> 18`. A floor would clip
   noise below black.
3. **Packet order.** The standard `Ppoc = 0` progression does not apply. Nikon writes
   `Ppoc = 1`: per precinct, one packet per (vertical half, component), component 2
   contributing one line per half. The WGT entries follow the order in which bands first
   appear in that sequence. A wrong order decodes without desynchronising but with wrong
   values.
4. **Coding-mode bits.** The precinct header's `D[p, b]` bits are in **component-major**
   band order (c0 b0..b7, c1 b0..b7, c2 b0, c3 b0..b7), not the standard band-major order.
5. **Star-Tetrix geometry.** ISO Table F.10 (CFA type 0: Y at quad position (1, 0), Δ at
   (0, 1)), with row-confined vertical taps and fitted chroma gains, is not what Nikon
   uses. Nikon's geometry puts **Y at (0, 1) and Δ at (1, 0)**, uses `e1 = e2 = 0`, full vertical
   extent across quad rows and no chroma gains. Neighbour reflection at the grid edges
   reduces to index clamping in plane coordinates, including the bottom row of the Δ step.

Three further details that the plan left open are now fixed by the implementation: the
`Fq = 4` scaling stays in place through the wavelet and colour transform and is consumed
by the transfer curve; the vertical inverse wavelet runs across slice boundaries; and the
first precinct of every slice is coded with `D = 3` (vertical prediction) although it has
no reference line, which the decoder treats as zero prediction (FORMAT.md §12.2).
