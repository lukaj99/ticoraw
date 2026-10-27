# Conformance kit

This kit is a set of public Nikon HE/HE\* raw files plus the exact output a correct decoder must produce for each one. It covers every body known to write HE/HE\*: Z 9, Z 8, Z f, Z6III, Z 5 II, Z 50 II and ZR. Each body has an HE and an HE\* file, except the ZR, which has only HE\*. The files cover FX, DX, 1:1 and 16:9 frames.

Anyone can run it: ticoraw's CI runs it, and authors of other decoders can check theirs against the same expected values. The files themselves are not in this repository. [`fetch.sh`](fetch.sh) downloads them from the public archives that host them.

| File | What it is |
|---|---|
| [`manifest.toml`](manifest.toml) | The samples: where each comes from, its SHA-256, and its expected decoded output. |
| [`fetch.sh`](fetch.sh) | Downloads the samples into a cache directory and verifies their checksums. |
| [`../crates/ticoraw/tests/conformance.rs`](../crates/ticoraw/tests/conformance.rs) | Checks ticoraw against the manifest. |
| [`../.github/workflows/conformance.yml`](../.github/workflows/conformance.yml) | Runs that check in CI. |

## What "Adobe-verified" means

Each sample was converted with Adobe DNG Converter 18.7 (build 2743) using `-u -p0`, which writes an uncompressed DNG. Two things were then checked for every sample:

- **The CFA hash.** The SHA-256 of the CFA mosaic in Adobe's DNG equals `cfa_sha256`, and so does the SHA-256 of ticoraw's output. So the two decoders agree at every photosite.
- **The metadata.** Every other expected value equals the tag in Adobe's DNG: `cfa_pattern`, `black_level`, `white_level`, `baseline_exposure`, `orientation`, `crop` (`DefaultCropOrigin` and `DefaultCropSize`) and `unique_camera_model`.

Adobe's DNGs are not distributed. The hash is enough to check a decoder against them.

## Running it against ticoraw

```sh
conformance/fetch.sh ~/.cache/ticoraw-conformance        # about 250 MB
TICORAW_CONFORMANCE_DIR=~/.cache/ticoraw-conformance \
  cargo test -p ticoraw --release --test conformance -- --nocapture
```

The test prints one line per sample and fails if any expected value differs. It also encodes each decode as a DNG with `encode_uncompressed_dng` and checks the tags that DNG carries, so the DNG writer is covered too. If `TICORAW_CONFORMANCE_DIR` is unset, the test checks only the manifest and skips the samples.

`fetch.sh <cache-dir> [manifest]`:
- Needs only `curl` and one of `sha256sum`, `shasum` or `openssl`.
- Saves each sample as `<cache-dir>/<id>.nef`, whatever its name upstream. The ZR file is a `.NRW` upstream.
- Skips files that are already present with the right SHA-256, so running it again is cheap.
- Downloads over HTTPS only, to a temporary name. A file is renamed into place only after its SHA-256 matches.
- Retries failed downloads with backoff and pauses between downloads.
- Never runs anything it downloads.
- Exits non-zero if any sample is missing or does not match.

## Checking another decoder

The manifest is plain TOML, so any TOML library can read it. For each `[[sample]]`:

1. Download `url` and check that its SHA-256 equals `nef_sha256`. Or run `fetch.sh` and read `<cache-dir>/<id>.nef`.
2. Decode the full-resolution CFA mosaic: all `width` x `height` photosites of the raw IFD.
   - Do not crop, scale, subtract black or linearise.
   - The values are the 14-bit numbers Adobe stores in its DNG, which is the codestream after its transfer curve.
3. Write every sample as a little-endian unsigned 16-bit integer, in row-major order, which gives `width * height * 2` bytes. The SHA-256 of those bytes must equal `cfa_sha256`.

In Python with NumPy, for example:

```python
import hashlib, numpy as np
cfa = my_decoder(path)  # 2-D array, height x width
digest = hashlib.sha256(np.ascontiguousarray(cfa, dtype="<u2").tobytes()).hexdigest()
```

The other fields hold what a DNG written from the file should carry, if your decoder writes DNGs or reports these values. They are listed at the top of [`manifest.toml`](manifest.toml). `cfa_pattern` is a 2x2 pattern read row-major, so every current sample is `RGGB`. `crop` is `[x, y, width, height]` in photosites.

### Manifest format

The manifest uses a small subset of TOML:
- Top-level keys first, then one `[[sample]]` table per sample.
- One `key = value` per line.
- Values are plain strings without escapes, integers, decimals, or one-line integer arrays.

Any TOML parser reads this subset. It is also simple enough that `fetch.sh` reads it with `awk` and the Rust test reads it with a small parser, so neither needs a dependency. The test rejects unknown keys, missing keys, duplicate keys and any licence other than `CC0-1.0`. JSON was the other option. TOML was chosen because it allows comments and is easier to edit and review by hand.

`schema` is the format version. It changes only when the meaning of a field changes.

## Contributing a sample

New samples are welcome, especially:
- bodies and firmware versions not covered yet;
- 12-bit files;
- any file whose main header differs from the one documented in [`docs/FORMAT.md`](../docs/FORMAT.md).

Each sample becomes a CI conformance case. Requirements:

- **CC0 1.0 only.** Upload the file to a public raw archive that publishes a checksum and a stable download URL: [raw.pixls.us](https://raw.pixls.us/) or [RawDB](https://rawdb.dnglab.org/). Do not open a pull request that contains the raw file itself.
- **Only images you are happy to publish.** Raw files keep EXIF data, which can include the camera serial number, GPS position and owner name, and the file cannot be edited without changing its checksum. Take a fresh frame with location services off. A dull scene is fine.
- **Straight from the camera.** Do not re-save or convert the file.

Steps:

1. Convert the file with Adobe DNG Converter: `"Adobe DNG Converter" -u -p0 -d <outdir> <file>.NEF`. On macOS the converter also runs from an expanded package without being installed.
2. Check that ticoraw decodes it to exactly what Adobe wrote: `ticoraw verify <file>.NEF <outdir>/<file>.dng` must report 0 mismatches.
3. Add a `[[sample]]` to `manifest.toml`:
   - `id`: lower-case letters, digits and `_`, for example `z6iii_he_dx`.
   - `body`: the EXIF model.
   - `mode` and `frame`.
   - The archive's download `url`, `licence = "CC0-1.0"`, `nef_sha256` and `nef_bytes`.
   - The expected values, read from Adobe's DNG. `exiftool -a -G1 <file>.dng` shows the tags. Because step 2 passed, ticoraw's `cfa_sha256` is Adobe's: run the conformance test with a placeholder hash and copy the actual hash from the failure message.
4. Run `fetch.sh` and the conformance test. Then open a pull request that says which archive hosts the file and which Adobe DNG Converter version you used.

If ticoraw does not decode your file, or `ticoraw verify` reports mismatches, please open an issue instead. That is exactly the sample we need.

## CI

[`conformance.yml`](../.github/workflows/conformance.yml) runs:
- on pushes to `main`;
- on pull requests;
- weekly;
- on demand.

Pushes and pull requests restore the samples from the Actions cache, keyed on the manifest's hash, so they rarely download anything. The weekly run skips the cache and downloads every sample afresh. If an archive changes or removes a file, the weekly run catches it.
