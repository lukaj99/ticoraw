# Cross-camera compatibility

Status as of 2026-10-27. The decoder is **bit-exact against Adobe on every Nikon body that writes HE/HE\***. Two references back this:

- **Local Z6III HE\* corpus:** 15 DNGs made from 11 distinct NEFs (14 written by Adobe Lightroom, 1 by Topaz Photo AI with an identical CFA image). All match at every photosite.
- **Public CC0 samples, below:** all 13 converted with Adobe DNG Converter 18.7 (`-u -p0`, uncompressed). Running the ground-truth suite on them (`TICORAW_CORPUS` pointing at the samples plus Adobe's DNGs) gives 13 of 13 pairs with 0 mismatching photosites. The CFA pattern, black and white levels, `BaselineExposure`, `Orientation` and default crop also agree with Adobe's for every pair.

## Adobe DNG Converter 18.7 results

| Body | Mode | Frame | Photosites | Mismatches | Adobe `UniqueCameraModel` | Adobe `BaselineExposure` |
|---|---|---|---|---|---|---|
| Z 9 | HE, HE\* | FX 8280x5520 | 45,705,600 | 0, 0 | Nikon Z 9 | +0.20 |
| Z 8 | HE, HE\* | FX 8280x5520 | 45,705,600 | 0, 0 | Nikon Z 8 | +0.20 |
| Z f | HE, HE\* | FX 6064x4040 | 24,498,560 | 0, 0 | Nikon Z f | +0.20 |
| Z6III | HE | FX 6064x4040 | 24,498,560 | 0 | Nikon Z 6 3 | +0.25 |
| Z6III | HE\* | 16:9 6064x3416 | 20,714,624 | 0 | Nikon Z 6 3 | +0.25 |
| Z 5 II | HE | DX 4000x2672 | 10,688,000 | 0 | Nikon Z 5 2 | +0.20 |
| Z 5 II | HE\* | FX 6064x4040 | 24,498,560 | 0 | Nikon Z 5 2 | +0.20 |
| Z 50 II | HE | 1:1 3728x3728 | 13,897,984 | 0 | Nikon Z 50 2 | +0.25 |
| Z 50 II | HE\* | DX 5600x3728 | 20,876,800 | 0 | Nikon Z 50 2 | +0.25 |
| ZR | HE\* (HLG) | FX 6064x4040 | 24,498,560 | 0 | Nikon ZR | +1.25 (= +0.25 + 1 stop HLG) |

Adobe writes WhiteLevel 15892 for every one of these files, and its D65 ColorMatrix equals the built-in table for every body. Adobe's DNGs are not stored in this repository.

## Summary

- **Bodies covered:** every Nikon body known to write HE/HE\*, using 13 public files: Z 9, Z 8, Z f, Z6III, Z 5 II, Z 50 II and ZR.
  - Each body has an HE and an HE\* file, except the ZR: only HE\* is public.
  - The files cover four frame shapes: FX 3:2, DX crop, 1:1 and 16:9.
- **Decoding:** all 13 decode without error, and every result looks like a natural image (metrics below).
- **Entropy layer:** a separate walk of the codestream decodes every GCLI. For each of the 111,648 packets, the GCLI and data sub-packets end exactly at the lengths the encoder wrote in the packet header. The bit-exact Z6III reference frame behaves the same way.
- **Headers:** the codestream main header is **byte-identical** on every body and in both modes.
  - This covers CAP, all PIH fields, the 13 PIH vendor bytes (and so the transfer curve), CDT and WGT.
  - The header matches the local Z6III corpus.
  - Only Wf, Hf and Lcod change from file to file.
- **Bit-exact:** every file matches Adobe DNG Converter 18.7 at every photosite (table above), which also exercises the frame-size-dependent paths: odd band widths, partial last slices and edge clamping.
- **Colour metadata:** the D65 ColorMatrix, WhiteLevel (15892), `UniqueCameraModel` and `BaselineExposure` that ticoraw writes equal Adobe's for every body.

## Samples

All samples are CC0 1.0. Each was downloaded from the source URL and its SHA-256 matched the checksum the archive publishes. None are stored in this repository.

The archive's "Mode" column labels every one of these files "8bit compressed", which is wrong. The HE/HE\* split below comes from three sources:
- The MakerNote 0x0051 compression code: 13 = HE, 14 = HE\*.
- The constant strip rate: HE is 3.000 bpp and HE\* is 5.000 bpp.
- The RawDB labels for the same files (identical SHA-256).

| # | Body (EXIF model) | FW | Mode | Crop | Raw WxH | Strip bytes | Source | SHA-256 |
|---|---|---|---|---|---|---|---|---|
| 1 | NIKON Z 9 | 01.00 | HE (13) | FX | 8280x5520 | 17,139,200 | [raw.pixls.us 5148](https://raw.pixls.us/getfile.php/5148/nice/Nikon%20-%20Z%209%20-%208bit%208bit%20compressed%20(3:2).NEF) | `be91bac25573adf7ec6320a542105d87b4fb9c956d7b620ec90d57ac2ae6c3f0` |
| 2 | NIKON Z 9 | 01.00 | HE\* (14) | FX | 8280x5520 | 28,565,504 | [raw.pixls.us 5147](https://raw.pixls.us/getfile.php/5147/nice/Nikon%20-%20Z%209%20-%208bit%208bit%20compressed%20(3:2).NEF) | `e113deb305d84153b26251ce259d1f839be1f36b048cbe83a4548cbda806c0f5` |
| 3 | NIKON Z 8 | 01.00 | HE | FX | 8280x5520 | 17,139,200 | [raw.pixls.us 6616](https://raw.pixls.us/getfile.php/6616/nice/Nikon%20-%20Z%208%20-%208bit%208bit%20compressed%20(3:2).NEF) | `82df041542b8d328738443bb5ff3373b396e406ecc8cbe7dc3b35252790c53e7` |
| 4 | NIKON Z 8 | 01.00 | HE\* | FX | 8280x5520 | 28,565,504 | [raw.pixls.us 6618](https://raw.pixls.us/getfile.php/6618/nice/Nikon%20-%20Z%208%20-%208bit%208bit%20compressed%20(3:2).NEF) | `ea8b093cd11c946c5eeb501b775859e8e617ce1d79577abc41b233c339e4004c` |
| 5 | NIKON Z f | 01.00 | HE | FX | 6064x4040 | 9,186,816 | [raw.pixls.us 6887](https://raw.pixls.us/getfile.php/6887/nice/Nikon%20-%20Z%20f%20-%208bit%208bit%20lossy%20compressed%20(3:2).NEF) | `98d6ca8e6c98048ca7ffed68ccaeda7b2b9f03807f0d320d97d5678db21748c2` |
| 6 | NIKON Z f | 01.00 | HE\* | FX | 6064x4040 | 15,311,360 | [raw.pixls.us 6886](https://raw.pixls.us/getfile.php/6886/nice/Nikon%20-%20Z%20f%20-%208bit%208bit%20lossy%20compressed%20(3:2).NEF) | `c888f109dc420e359853a2ce768d8a6274b8ba0109b2f5cb6e0c982981d5a624` |
| 7 | NIKON Z6_3 | 01.00 | HE | FX | 6064x4040 | 9,186,816 | [raw.pixls.us 7811](https://raw.pixls.us/getfile.php/7811/nice/Nikon%20-%20Z6_3%20-%208bit%20compressed%20(3:2).NEF) | `9ae7346adb3b16f011d1d3c11b4c7ae2ab0231c64111dff3fad8e28dd55353c8` |
| 8 | NIKON Z6_3 | 01.00 | HE\* | 16:9 | 6064x3416 | 12,946,432 | [raw.pixls.us 7813](https://raw.pixls.us/getfile.php/7813/nice/Nikon%20-%20Z6_3%20-%208bit%20compressed%20(16:9).NEF) | `b84499212126ef8bab31aa2b150f6dbf8cb6e0e2aab7ca669d85c3259784ecc5` |
| 9 | NIKON Z5_2 | 01.00 | HE | DX | 4000x2672 | 4,007,936 | [raw.pixls.us 7737](https://raw.pixls.us/getfile.php/7737/nice/Nikon%20-%20Z5_2%20-%208bit%20compressed%20(3:2).NEF) | `196971ea960cfa6f5c0a19cebe515ac118e079c57ceeef850ab3a4995dc7d20f` |
| 10 | NIKON Z5_2 | 01.00 | HE\* | FX | 6064x4040 | 15,311,360 | [raw.pixls.us 7743](https://raw.pixls.us/getfile.php/7743/nice/Nikon%20-%20Z5_2%20-%208bit%20compressed%20(3:2).NEF) | `b44bb31787098631a396b553454a8ebf960766d9b538521ce9852896bd6df3a8` |
| 11 | NIKON Z50_2 | 01.00 | HE | 1:1 | 3728x3728 | 5,211,648 | [raw.pixls.us 7767](https://raw.pixls.us/getfile.php/7767/nice/Nikon%20-%20Z50_2%20-%208bit%20compressed%20(1:1).NEF) | `99b2a569356fb084d6946fa6e0b501b014e6e004be0a8e8d564a7b9dab91857e` |
| 12 | NIKON Z50_2 | 01.00 | HE\* | DX | 5600x3728 | 13,047,808 | [raw.pixls.us 7763](https://raw.pixls.us/getfile.php/7763/nice/Nikon%20-%20Z50_2%20-%208bit%20compressed%20(3:2).NEF) | `0add1dad5776669a4896640de31b128032f893e13418f6569da7a4744a05ed42` |
| 13 | NIKON ZR | 01.01 | HE\* | FX | 6064x4040 | 15,311,360 | [RawDB Nikon/ZR](https://rawdb.dnglab.org/sets/Nikon/ZR), `he_modes/NIKON ZR_ISO_800_14bits_High Efficiency*.NRW` | `cfb4e5daa71e4590dba01b3753c9043a864726b0994ab216c1401bf80ed85322` |

Notes:
- The archive listings are https://raw.pixls.us/ and https://rawdb.dnglab.org/. RawDB mirrors the Z 5 II and Z 50 II sets under mode-labelled names (`he_mode/Z5_II_High_Efficiency_DX.NEF` and so on).
- The ZR sample has a `.NRW` extension but is an ordinary NEF/TIFF container. Its picture control is HLG: MakerNote 0x0051 byte 8 = 0x11 and 0x0059 = `0303`.
- All strips are 14 bps with TIFF Compression 34713, CFA RGGB, and MakerNote BlackLevel (0x003D) = 1008 x 4.
- On every file, Lcod equals the strip length (constant bitrate). Lcod is also repeated as a u32 LE in MakerNote 0x0058.
- The Z 9 at firmware 1.00 has no MakerNote 0x0059 or 0x005B, and its 0x0051 version string is `0101`; every other file has `0102`. Neither difference affects decoding.

## Method

A scratch crate outside the repository depends on `ticoraw` by path and runs the following on each file:

1. **`inspect_nef`:** prints model, raw dimensions, NikonCompression, every PIH field, the PIH vendor bytes, the decoded curve (B, A, C), CDT trailer bytes, WGT pairs, CAP, extra markers and the `unsupported` reason.
2. **Independent codestream walk** (header parse plus `entropy::Layout` from the crate; everything else re-implemented):
   - Frames every slice and precinct, and records Q, R and the per-band coding modes.
   - Parses all packet headers and counts Dr = 1 (raw) packets.
   - Reads the significance flags and decodes every GCLI (zero or vertical prediction, run mode).
   - Checks that each GCLI sub-packet ends exactly at its Lcnt, and that `ceil(4 * sum(GCLI - GTLI) / 8)` equals its Ldat.
   - Checks that the precinct bodies end with only zero padding before EOC.
3. **`decode_nef`:** records ok/err and a warm wall time (rayon on, release build, Apple Silicon). Then it computes:
   - Per CFA site: min, 1st and 99th percentiles, mean and max.
   - The mean |difference| between same-colour neighbours two pixels apart, horizontally and vertically, divided by the site's p99 - p1 range. This is the natural-image check: values far below 0.15 mean smooth image content, not noise or garbage.
   - Vertical same-colour differences for pairs crossing a precinct boundary (4 CFA rows) and a slice boundary (`4 * Hsl` CFA rows), relative to pairs inside a precinct. A ratio near 1.0 means no seams.
   - Edge rows and columns.
   - A white-balanced, gamma-encoded preview PNG, which was checked by eye.
   - An FNV-1a-64 hash of the CFA output (u16 LE, row-major) for future regression checks.

The method was calibrated on a local Z6III HE\* frame that is bit-exact against Adobe:
- Neighbour ratio 0.0109.
- Precinct seam ratio 1.001 and slice seam ratio 0.977.
- 8080 of 8080 GCLI and data sub-packets exact.
- DNG compare: 0 mismatches.

## Results

| # | Body / mode | Component planes | Slices / precincts (last slice) | Q range | Packets, exact sync | Dr = 1 | Neighbour ratio (worst site) | Precinct / slice seam | Warm decode | CFA FNV-1a-64 |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | Z 9 HE | 4140x2760 | 87 / 1380 (4) | 3..5 | 11040 / 11040 | 0 | 0.0210 | 1.002 / 1.011 | 64 ms | `8bf222f53a5c024c` |
| 2 | Z 9 HE\* | 4140x2760 | 87 / 1380 (4) | 1..3 | 11040 / 11040 | 0 | 0.0207 | 1.004 / 0.987 | 69 ms | `fa90c9e030b17cbf` |
| 3 | Z 8 HE | 4140x2760 | 87 / 1380 (4) | 3..6 | 11040 / 11040 | 0 | 0.0152 | 1.000 / 1.001 | 63 ms | `5fb1095b3271f93e` |
| 4 | Z 8 HE\* | 4140x2760 | 87 / 1380 (4) | 1..5 | 11040 / 11040 | 0 | 0.0173 | 1.001 / 0.998 | 70 ms | `7624ff2fa0702484` |
| 5 | Z f HE | 3032x2020 | 64 / 1010 (2) | 3..5 | 8080 / 8080 | 0 | 0.0159 | 0.997 / 1.014 | 38 ms | `2dcad6ffcdea95c6` |
| 6 | Z f HE\* | 3032x2020 | 64 / 1010 (2) | 1..4 | 8080 / 8080 | 0 | 0.0160 | 0.997 / 0.990 | 39 ms | `d15601868e6e9800` |
| 7 | Z6III HE | 3032x2020 | 64 / 1010 (2) | 3..6 | 8080 / 8080 | 0 | 0.0413 | 1.001 / 0.975 | 37 ms | `0d17923e7bddebb5` |
| 8 | Z6III HE\* 16:9 | 3032x1708 | 54 / 854 (6) | 2..5 | 6832 / 6832 | 0 | 0.0412 | 1.002 / 0.992 | 34 ms | `0ffa2544dafa9237` |
| 9 | Z 5 II HE DX | 2000x1336 | 42 / 668 (12) | 3..5 | 5344 / 5344 | 0 | 0.0324 | 1.002 / 0.991 | 19 ms | `5934e2cfa10ec6c7` |
| 10 | Z 5 II HE\* | 3032x2020 | 64 / 1010 (2) | 0..4 | 8080 / 8080 | 0 | 0.0268 | 0.999 / 0.989 | 38 ms | `ae93afe788638f80` |
| 11 | Z 50 II HE 1:1 | 1864x1864 | 59 / 932 (4) | 3..5 | 7456 / 7456 | 0 | 0.0144 | 1.005 / 0.990 | 22 ms | `17b4466979e57a79` |
| 12 | Z 50 II HE\* DX | 2800x1864 | 59 / 932 (4) | 1..3 | 7456 / 7456 | 0 | 0.0132 | 0.997 / 1.002 | 35 ms | `c350af4ef3305e17` |
| 13 | ZR HE\* (HLG) | 3032x2020 | 64 / 1010 (2) | 2..3 | 8080 / 8080 | 0 | 0.0204 | 1.000 / 1.003 | 40 ms | `ddd4f1c3e2bc2833` |

Every file:
- Decodes OK.
- Has its dimensions agree between the codestream and the raw IFD.
- Ends at EOC exactly 2 bytes before the strip end.
- Has no `unsupported` reason.
- Has a preview that shows a clean photograph, with no block, slice or edge artefacts and no colour-plane swaps.

The higher neighbour ratios on the two Z6III files (about 0.04) come from fine scene detail. Their seam ratios are flat.

Observations that settle two formerly open questions, for the bodies seen so far: does the main header vary between bodies and modes, and how is the first precinct of a slice coded? The questions that remain open are listed in [FORMAT.md §20](FORMAT.md#20-profile-checks-and-unknowns).
- **The header never varies.** CAP, all PIH fields and the vendor bytes `50887083f01523d149cd3f7f07` are the same on every file, so the curve is always B = 1008, A = 84429, C = 16255. The same holds for CDT (`0e1151 0e1151 0e1100 0e1151`) and WGT (25 pairs, same order).
  - This is true across HE versus HE\*, SDR versus HLG, FX/DX/1:1/16:9, and bodies from firmware 1.00 (2022) to 2025.
  - HE and HE\* differ only in Lcod and in the per-precinct Q and R values. HE uses Q of 3 to 6; HE\* uses Q of 0 to 5.
  - The unknown vendor bytes (A0, t[0..2], t[12]) and the CAP payload are therefore never exercised with other values.
- **No raw GCLI packets (Dr = 1)** were found in 111,648 packets.
- **Every band of every precinct has coding mode 3** (vertical prediction plus significance), including the first precinct of each slice.
  - At a slice start there is no reference line, so the decoder treats the vertical bit as zero prediction, as it does for the bit-exact Z6III frames ([FORMAT.md §12.2](FORMAT.md#122-gcli)).
  - Exact sub-packet sync on all files confirms this reading.
- **Precinct bodies carry zero padding:** 0.2 to 0.7 MB per file. This is the constant-bitrate fill.
- **Frame-size variety:** the samples include component widths whose wavelet pyramids have odd band widths (4140, 2800, 2000, 1864), and partial last slices of 2, 4, 6 and 12 precincts. No seams or edge anomalies were found.

## Known gaps

1. **Adobe reference is one release.** The verification above used Adobe DNG Converter 18.7 (build 2743). Re-run it after major Adobe or camera-firmware updates:
   - Convert with `"Adobe DNG Converter" -u -p0 -d <dir> <file>.NEF` (the converter also runs from an expanded package, `pkgutil --expand-full`, without being installed).
   - Then run `ticoraw verify <nef> <dng>`, or the ground-truth suite on the folder.
2. **One sample per body and mode.** Every frame shape and rate seen so far decodes exactly, but each body/mode pair rests on one public file (two for the Z6III).
3. **12-bit HE, N-RAW video and other untested variants.** All public HE/HE\* stills are 14-bit.
   - No 12-bit HE NEF was found. Nikon's menus suggest HE is 14-bit only, but this is unverified.
   - N-RAW (`.NEV`) video frames reportedly use the same TicoRAW codestream at video resolutions (github.com/hayamdk/nev2nef). They are untested.
   - No HE file from a Z 9 or Z 8 on later firmware was available.
4. **Raw GCLI mode is unexercised by cameras.** No file contains Dr = 1 packets. The decoder reads raw GCLIs at the PIH `Br` width (4 in every file seen); the synthetic test suite covers this path.
5. **`.NRW` extension.** The ZR HE\* samples on RawDB use the `.NRW` extension; `ticoraw scan` accepts `.nef` and `.nrw` in any case.
