# Fuzzing ticoraw

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets for the decoder. This crate
has its own `[workspace]`, so the main workspace's stable builds never see it.

| Target | Input | Checks |
|---|---|---|
| `decode_nef` | bytes | no panic; an accepted NEF survives the uncompressed-DNG round trip |
| `inspect_nef` | bytes | no panic |
| `inspect_nef_prefix` | 8-byte `file_len` selector + prefix | no panic, for any `file_len` up to `usize::MAX` |
| `decode_ticoraw` | bytes | no panic; output size matches the claimed frame |
| `decode_dng_cfa` | bytes | no panic; output size matches the claimed frame |
| `decode_lossless_jpeg` | bytes | no panic; output size matches the claimed frame |
| `codestream_structured` | encoder options (+ NEF wrapper, + byte edits) | exact decode of every valid synthetic codestream and NEF; no panic once edited |
| `dng_roundtrip` | arbitrary `DecodedRaw` | DNG writer output reads back unchanged |
| `ljpeg_roundtrip` | lossless JPEG parameters (+ DNG layout) | exact decode of every valid SOF3 stream and LJPEG DNG |

Hangs and memory blow-ups are caught by libFuzzer's `-timeout` and `-rss_limit_mb`.

## Running

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz
# Seed corpora, built from the synthetic encoders in src/synth (no camera files needed):
cargo run --release --manifest-path fuzz/Cargo.toml --example gen_seeds -- fuzz/corpus
cargo +nightly fuzz run -a decode_nef -- -dict=fuzz/dict/nef.dict \
    -max_total_time=600 -timeout=10 -rss_limit_mb=2048
```

`-a` turns on debug assertions, so integer overflow panics too. Dictionaries:
`nef.dict` for the three NEF targets, `codestream.dict` for `decode_ticoraw`, `dng.dict`
for `decode_dng_cfa` and `ljpeg.dict` for `decode_lossless_jpeg`. The structure-aware
targets need none.

## Crashes

Minimise a crash with `cargo +nightly fuzz tmin -a <target> <artifact>` and keep the
small reproducer under `artifacts-regressions/<target>/`. The nightly workflow
(`.github/workflows/fuzz.yml`) replays those before fuzzing, caches each target's corpus
between runs, and uploads new crash artifacts. A reproducer stays there after its fix as a
regression test; until then the nightly job for that target fails.

Fixed reproducers, kept as regressions (`crates/ticoraw/tests/synthetic.rs` replays every
file in `artifacts-regressions/` through all public entry points):

- `decode_dng_cfa/tile-byte-count-sum-overflow`: a 150-byte hand-written DNG whose two
  LONG8 `TileByteCounts` of `u64::MAX` overflowed the byte-count sum in
  `dng::decode_ljpeg_layout`; the sum now saturates.

`src/synth` is a copy of the test support in `crates/ticoraw/tests/common/`; update it
when those encoders change.
