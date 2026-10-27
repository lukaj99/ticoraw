# The Nikon HE / HE* TicoRAW codestream

This document describes the raw-data format that Nikon cameras write in their "High
Efficiency" (HE) and "High Efficiency\*" (HE\*) NEF modes, exactly as the `ticoraw` crate
decodes it. It is self-contained: with it and a copy of ISO/IEC 21122-1 terminology you
should be able to write an independent, bit-exact decoder.

The decoder this describes is **bit-exact against Adobe Lightroom DNGs of Nikon Z6III HE\*
files** (every photosite of every frame tested matches). Statements that rest on that
verification are unmarked. Statements that hold for this decoder but have not been
exercised by real camera files are marked **[unexercised]**; open questions are marked
**[unknown]**.

Conventions:

- All multi-byte codestream fields are big-endian. All bit fields are read MSB first.
- `>>` is an arithmetic (flooring) right shift on signed integers.
- "Component plane" means one of the four half-resolution planes the codestream carries.
- `W`, `H` are the component-plane width and height (3032 x 2020 on the Z6III).

Contents:

1. [Container](#1-container)
2. [Codestream overview](#2-codestream-overview)
3. [Markers](#3-markers)
4. [Picture header (PIH) and vendor bytes](#4-picture-header-pih-and-vendor-bytes)
5. [Component table (CDT)](#5-component-table-cdt)
6. [Slices and precincts](#6-slices-and-precincts)
7. [Bands](#7-bands)
8. [Packet order (Ppoc = 1) and WGT order](#8-packet-order-ppoc--1-and-wgt-order)
9. [Precinct header](#9-precinct-header)
10. [Truncation levels (GTLI)](#10-truncation-levels-gtli)
11. [Packet header](#11-packet-header)
12. [Sub-packets](#12-sub-packets)
13. [Dequantisation](#13-dequantisation)
14. [Coefficient placement and the Fq shift](#14-coefficient-placement-and-the-fq-shift)
15. [Inverse 5/3 wavelet](#15-inverse-53-wavelet)
16. [Inverse Star-Tetrix](#16-inverse-star-tetrix)
17. [Transfer curve](#17-transfer-curve)
18. [Mosaic assembly](#18-mosaic-assembly)
19. [Decode order](#19-decode-order)
20. [Profile checks and unknowns](#20-profile-checks-and-unknowns)
21. [Sources and acknowledgements](#21-sources-and-acknowledgements)

---

## 1. Container

A HE/HE\* NEF is an ordinary Nikon TIFF container.

- The raw IFD is the one with `NewSubfileType` (0x00FE) = 0 and `PhotometricInterpretation`
  (0x0106) = 32803 (CFA). If several match, take the widest.
- Its single strip (`StripOffsets` 0x0111, `StripByteCounts` 0x0117) is the complete JPEG XS
  codestream, from SOC to EOC. The TIFF `Compression` tag stays at Nikon's value 34713, so
  the reliable signature is the strip itself: it starts `FF 10 FF 50` (SOC then CAP).
- `CFAPattern` (0x828E) is RGGB (`0 1 1 2`).
- The Nikon type-3 MakerNote (`"Nikon\0"`, a 4-byte version, then an embedded TIFF whose
  offsets are relative to MakerNote + 10) carries:
  - tag 0x0051: a 16-bit little-endian compression code at byte offset 10: 13 = HE,
    14 = HE\*, 3 = lossless compressed;
  - tag 0x003D: the four per-channel black levels;
  - tag 0x000C: white-balance multipliers for red and blue as rationals;
  - tag 0x0045: the recommended crop (x, y, width, height) inside the raw frame, as four
    SHORTs. `ticoraw` writes it to the DNG as `DefaultCropOrigin` / `DefaultCropSize`,
    together with IFD0's `Orientation` (0x0112).

HE and HE\* differ in rate (the Q/R values per precinct and the total length), not in
structure.

## 2. Codestream overview

The codestream is JPEG XS (ISO/IEC 21122-1) with intoPIX TicoRAW deviations. Every Z6III
HE\* frame carries the same main header:

```
FF10                                   SOC
FF50 0022  "CONTACT_INTOPIX_" EF C0 00x14              CAP (vendor signature)
FF12 0027  00E9A200 0000 0000 17B0 0FC8 0000 0010
           04 04 08 12 44 10 51 14 | 50 88 70 83 F0 15 23 D1 49 CD 3F 7F 07   PIH
FF13 000E  0E 11 51 | 0E 11 51 | 0E 11 00 | 0E 11 51  CDT
FF14 0034  (03,02)(03,13)(02,09)(02,16)(01,0A)(01,15)(02,05)(01,03)(01,11)(00,01)
           (00,07)(00,0D)(00,0B)(02,06)(01,04)(01,10)(00,00)(00,08)(00,0E)(01,14)
           (00,12)(00,0F)(00,17)(00,0C)(00,18)                       WGT (G, P) x 25
FF20 0004 0000  <16 precincts>         SLH, slice 0
FF20 0004 0001  <16 precincts>         SLH, slice 1
...
FF20 0004 003F  <2 precincts>          SLH, slice 63
FF11                                   EOC
```

What is standard and what is not:

| Stage | Status |
|---|---|
| Marker syntax, SLH, EOC | standard |
| CAP payload | vendor signature, not capability bits |
| PIH | standard 24 bytes + 13 vendor bytes; `Wf x Hf` is the CFA size; `Ppoc = 1`; `Fq = 4` |
| CDT | 3 bytes per component; a per-component decomposition byte; one component without a wavelet |
| WGT | standard syntax; entries in packet order |
| Precinct header | standard fields; coding-mode bits in component-major band order |
| Packet header | always the long (7-byte) form |
| GCLI coding, significance, data, signs | standard |
| Dequantisation (uniform) | standard |
| 5/3 inverse wavelet | standard |
| Colour transform | Star-Tetrix lifting steps, but signalled nowhere (`Cpih = 0`, no CTS/CRG) and with Nikon's own geometry |
| Output | no NLT marker; a vendor quadratic curve parameterised by the PIH vendor bytes |

## 3. Markers

| Marker | Code | Length field | Content |
|---|---|---|---|
| SOC | FF10 | none | start of codestream |
| CAP | FF50 | 34 | ASCII `CONTACT_INTOPIX_`, then `EF C0` and 14 zero bytes. Skip it. |
| PIH | FF12 | 39 | 24 standard bytes + 13 vendor bytes (§4) |
| CDT | FF13 | 14 | 4 x 3 bytes (§5) |
| WGT | FF14 | 52 | 25 x (gain G, priority P), one byte each (§8) |
| SLH | FF20 | 4 | `Yslh`, the 16-bit slice index, then the slice's precincts |
| EOC | FF11 | none | end of codestream |

NLT (FF16), CWD (FF17), CTS (FF18), CRG (FF19) and COM (FF15) are absent from camera files.
The length field counts itself (2 bytes) plus the payload. The main header runs from SOC to
the first SLH.

## 4. Picture header (PIH) and vendor bytes

Payload layout (offsets within the payload):

| Offset | Field | Bits | Z6III HE\* value |
|---|---|---|---|
| 0 | `Lcod`, codestream length in bytes | 32 | 15 311 360 |
| 4 | `Ppih`, profile | 16 | 0 |
| 6 | `Plev`, level | 16 | 0 |
| 8 | `Wf`, frame width | 16 | 6064 |
| 10 | `Hf`, frame height | 16 | 4040 |
| 12 | `Cw`, precinct width (0 = full width) | 16 | 0 |
| 14 | `Hsl`, slice height **in precincts** | 16 | 16 |
| 16 | `Nc`, components | 8 | 4 |
| 17 | `Ng`, coefficients per code group | 8 | 4 |
| 18 | `Ss`, code groups per significance group | 8 | 8 |
| 19 | `Bw`, nominal bit depth | 8 | 18 |
| 20 | `Fq` (high 4) / `Br` (low 4, raw GCLI width) | 8 | 0x44: Fq = 4, Br = 4 |
| 21 | `Fslc` (1) / `Ppoc` (3) / `Cpih` (4) | 8 | 0x10: Fslc = 0, **Ppoc = 1**, Cpih = 0 |
| 22 | `NLx` (4) / `NLy` (4) | 8 | 0x51: 5 horizontal, 1 vertical level |
| 23 | `Lh` (1) / `Rl` (1) / `Qpih` (2) / `Fs` (2) / `Rm` (2) | 8 | 0x14: Qpih = 1 (uniform), Fs = 1 (separate signs), others 0 |
| 24..36 | vendor extension | 13 bytes | `50 88 70 83 F0 15 23 D1 49 CD 3F 7F 07` |

Deviations from the standard:

- `Wf x Hf` is the **CFA mosaic** size. Each component plane is `W = Wf / 2` by
  `H = Hf / 2`, although `Sx = Sy = 1`.
- `Ppoc = 1` selects a non-standard packet progression (§8). The standard defines only 0.
- `Fq = 4` with `Bw = 18`, a combination the standard's reference decoders reject. The
  arithmetic is generic (§14).
- `Lh = 0` and `Wf * Nc < 32752` would select short packet headers under the standard
  rule; Nikon writes long ones anyway (§11).

Vendor bytes `t[0..12]` carry the transfer-curve parameters (§17):

```
B = ((t[3] & 0x0F) << 8) | t[4]                     // 1008, output black offset
A = ((t[7] & 0x03) << 16) | (t[8] << 8) | t[9]      // 84429, Q16 gain (~1.2883)
C = (t[10] << 8) | t[11]                            // 16255, knee in the 16-bit domain
```

[unknown] The meaning of the remaining bits: `t[0..2]`, the high nibble of `t[3]`, the
18-bit field `((t[5] & 0x0F) << 14) | (t[6] << 6) | (t[7] >> 2)` (= 84212) directly before
`A`, and `t[12]`. None of them is needed for a bit-exact decode of the tested files.

## 5. Component table (CDT)

Three bytes per component:

| Byte | Content | Value |
|---|---|---|
| 0 | `Bc`, component bit depth | 14 |
| 1 | `Sx << 4 \| Sy` | 0x11 |
| 2 | vendor: `NLx << 4 \| NLy` for this component | 0x51; **0x00 for component 2** |

Component 2 carries no wavelet decomposition at all. Standard JPEG XS can only leave the
*trailing* components undecomposed (CWD), and only through a separate marker.

Component roles (§16): 0 = Y, 1 = Cr, 2 = Δ (green difference), 3 = Cb.

## 6. Slices and precincts

- A precinct is `2^NLy = 2` component lines tall and, with `Cw = 0`, spans the full width.
- The component height `H = Hf / 2` must be even, so `Hf` is a multiple of 4 (4040 on the
  Z6III); the decoder rejects other heights (§20). Precinct rows: `H / 2` = **1010**.
- A slice is `Hsl` = 16 precinct rows = 32 component lines.
- Slices: `ceil(1010 / 16)` = **64**; the last one holds 2 precincts.

Each slice starts with an SLH whose `Yslh` equals its index. Precincts are stored back to
back after it; each one's length is in its header (§9), so slices can be framed without
entropy decoding and decoded independently (only the inverse wavelet couples them, §15).

## 7. Bands

A decomposed component (`NLx = 5`, `NLy = 1`) has `2 * NLy + NLx + 1 = 8` bands. Local
index order is the standard one:

| Band | Kind | Width | GCLI groups `ceil(w/4)` | Significance groups `ceil(g/8)` | Precinct line |
|---|---|---|---|---|---|
| b0 | LL: horizontal low at level 5, vertical low | 95 | 24 | 3 | low |
| b1 | horizontal high, level 5 | 95 | 24 | 3 | low |
| b2 | horizontal high, level 4 | 189 | 48 | 6 | low |
| b3 | horizontal high, level 3 | 379 | 95 | 12 | low |
| b4 | horizontal high, level 2 | 758 | 190 | 24 | low |
| b5 | HL1: horizontal high, level 1 | 1516 | 379 | 48 | low |
| b6 | LH1: horizontal low, vertical high | 1516 | 379 | 48 | high |
| b7 | HH1: horizontal high, vertical high | 1516 | 379 | 48 | high |

Widths come from repeated splits of `W = 3032`: at each level `high = w / 2` and
`low = w - w / 2`. Bands b0..b5 tile the vertical-low line (sum 3032) and b6..b7 tile the
vertical-high line. Each of these bands has one line per precinct.

Component 2 has a single band of width 3032 (758 GCLI groups, 95 significance groups) with
**two** lines per precinct.

Total: 3 x 8 + 1 = **25 bands**, the number of WGT entries.

The decoder indexes bands **component-major**: c0 b0..b7, c1 b0..b7, c2 b0, c3 b0..b7. This
is the order of the precinct-header mode bits (§9).

## 8. Packet order (Ppoc = 1) and WGT order

Each precinct holds 8 packets. For each vertical half `h` in {0, 1}, for each component `c`
in index order, one packet:

- decomposed component, `h = 0`: bands b0..b5 (the vertical-low line);
- decomposed component, `h = 1`: bands b6..b7 (the vertical-high line);
- component 2: line `h` of its single band.

So the sequence is (c0, low), (c1, low), (c2, line 0), (c3, low), (c0, high), (c1, high),
(c2, line 1), (c3, high). The standard `Ppoc = 0` order (Annex B: horizontal bands of all
components band-major, then one packet per (band type, line)) does not apply.

**WGT order** is the order in which bands first appear in that packet sequence:

```
c0 b0..b5, c1 b0..b5, c2 b0, c3 b0..b5, c0 b6..b7, c1 b6..b7, c3 b6..b7
  (6 + 6 + 1 + 6 + 2 + 2 + 2 = 25 entries)
```

The 25 priorities are all distinct. A wrong WGT order does not desynchronise the bit
stream (unary GCLI codes keep their lengths), it only produces wrong values, so this is
worth testing against ground truth rather than assuming.

## 9. Precinct header

| Field | Bits |
|---|---|
| `Lprc`, bytes of packet data after this header | 24 |
| `Q[p]`, quantisation | 8 |
| `R[p]`, refinement | 8 |
| `D[p, b]`, coding mode, 25 bands in **component-major** order (§7) | 2 each, 50 total |
| padding to a byte boundary | 6 |

The header is 5 + 7 = 12 bytes; the next precinct starts `12 + Lprc` bytes later. Bit 1 of
`D` (value 2) means the band uses a significance sub-packet; bit 0 (value 1) means its
GCLIs are vertically predicted. The standard orders `D` band-major across components;
Nikon does not.

## 10. Truncation levels (GTLI)

For each band `b` with WGT gain `G[b]` and priority `P[b]`:

```
pump    = (P[b] < R[p]) ? 1 : 0
GTLI[b] = (Q[p] < G[b] + pump) ? 0 : min(Q[p] - G[b] - pump, 15)
```

This standard formula decodes HE and HE\* alike; no per-mode table is needed.

## 11. Packet header

Always the long form, 7 bytes (56 bits):

| Field | Bits |
|---|---|
| `Dr`, raw-GCLI mode for this packet | 1 |
| `Ldat`, data sub-packet bytes | 20 |
| `Lcnt`, GCLI sub-packet bytes | 20 |
| `Lsgn`, sign sub-packet bytes | 15 |

The sub-packets follow in the order significance, GCLI, data, sign. Each ends on a byte
boundary. A sub-packet that is shorter than its declared length is followed by padding up to
that length, which the decoder skips; one that overruns its length is an error.

## 12. Sub-packets

Unary codes are a run of `1` bits terminated by a `0`; the value is the number of `1`s.

### 12.1 Significance (only when `Dr = 0`)

For each band of the packet whose `D` has bit 1 set, read one bit per significance group
(`Ss = 8` code groups). `1` means the group is **insignificant**. Then byte-align. There is
no length field; the size follows from the band geometry.

### 12.2 GCLI

One continuous bit stream over the packet's bands, byte-aligned only at its end.

- **Raw (`Dr = 1`)** [unexercised by cameras]: `Br` (= 4) bits per code group, the absolute GCLI. No
  significance, no prediction.
- **Zero prediction** (`D` bit 0 clear): an insignificant group has `GCLI = GTLI`; a
  significant one has `GCLI = GTLI + unary()`.
- **Vertical prediction** (`D` bit 0 set), for each code group `i`, against a reference line
  with GCLIs `GCLI_top` and truncation `GTLI_top`:

  ```
  t     = max(GTLI_top, GTLI)
  m_top = max(GCLI_top[i], t)
  if the group is significant (or the band has no significance coding):
      x     = unary()
      thr   = max(m_top - GTLI, 0)
      delta = x > 2*thr ? x - thr
            : x odd     ? -(x + 1) / 2
            :             x / 2
      GCLI  = m_top + delta
  else:
      GCLI  = (Rm == 1) ? GTLI : m_top
  ```

- **Reference line**:
  - for component 2's second line, the first line of the same band in the same precinct
    (with the same GTLI);
  - otherwise the last line of the same band in the **previous precinct of the same
    slice**, with that precinct's GTLI;
  - at the first precinct of a slice there is no reference line. **Every camera file
    nevertheless sets `D = 3`** (significance plus vertical prediction) on every band of
    that precinct, so a decoder must accept the vertical bit there and treat it as **zero
    prediction**. Equivalently, predict from a virtual reference line with
    `GCLI_top = 0` and `GTLI_top = GTLI`: then `m_top = GTLI` and `thr = 0`, so a
    significant group decodes to `GTLI + unary()` and an insignificant one to `GTLI`,
    exactly as zero prediction. This reading makes the Z6III frames bit-exact, and keeps
    every sub-packet of the cross-body sweep in exact sync ([compat.md](compat.md)).

### 12.3 Data

For each band, for each code group of `Ng = 4` coefficients with `n = GCLI - GTLI > 0`:

- read `n` 4-bit nibbles, most significant bit plane first; bit 3 of each nibble belongs
  to coefficient 0 of the group, bit 0 to coefficient 3;
- the magnitude is the accumulated bits shifted left by `GTLI`;
- groups with `n <= 0` are zero. `n > 15` is an error.

With `Fs = 1` there is no inline sign nibble. The last group of a band whose width is not a
multiple of 4 (here 95, 189, 379 and 758) still decodes all four coefficients; non-zero values that fall past the band end are discarded,
but **counted**, because their sign bits are present (§12.4). Byte-align at the end.

### 12.4 Sign (`Fs = 1`)

For each band, one bit per non-zero in-band coefficient in coefficient order, `1` meaning
negative; then skip one bit for each discarded non-zero coefficient counted in §12.3.
Byte-align at the end.

## 13. Dequantisation

`Qpih = 1` selects uniform reconstruction. For a non-zero magnitude `v` (already shifted
`<< GTLI`) in a group with `GTLI > 0`:

```
s = GCLI - GTLI + 1
r = 0
while v > 0 { r += v; v >>= s }
magnitude = r
```

With `GTLI = 0` the magnitude is used as is. (The deadzone mode, `Qpih = 0`, would instead
set bit `GTLI - 1`; camera files do not use it.) Apply the sign afterwards to obtain a
two's-complement value.

## 14. Coefficient placement and the Fq shift

Every coefficient is shifted left by `Fq = 4` as it is stored into its component plane. The
lifting arithmetic that follows runs on these scaled values unchanged, and the result of
the colour transform is fed to the transfer curve **without** any shift back.

Plane layout before the inverse wavelet (one `i32` plane of `W x H` per component):

- row `2p` holds the vertical-low line of precinct `p`, laid out left to right as
  `[b0 | b1 | b2 | b3 | b4 | b5]` = `[L5 | H5 | H4 | H3 | H2 | H1]`;
- row `2p + 1` holds the vertical-high line, `[b6 | b7]`;
- component 2 stores its two precinct lines directly in rows `2p` and `2p + 1`.

## 15. Inverse 5/3 wavelet

The reversible LeGall 5/3 filter of ISO/IEC 21122-1 Annex E, with whole-sample symmetric
extension.

**1-D synthesis** of `n` samples from `L` (`ceil(n/2)` samples) and `H` (`floor(n/2)`),
with `H[-1] = H[0]` and `H[len] = H[len - 1]`:

```
even:  out[2k]     = L[k] - ((H[k-1] + H[k] + 2) >> 2)
odd:   out[2k + 1] = H[k] + ((out[2k] + out[2k+2]) >> 1)    // out[2k+2] := out[2k] past the end
```

**Horizontal.** Even plane rows of a decomposed component cascade five levels:
`b0 + b1 -> 190`, `+ b2 -> 379`, `+ b3 -> 758`, `+ b4 -> 1516`, `+ b5 -> 3032`. Odd rows
undergo one level: `b6 + b7 -> 3032`.

**Vertical.** One pass over the **whole plane**, column by column, with `L_p` = row `2p` and
`H_p` = row `2p + 1` after horizontal synthesis, and `H_{-1} = H_0`:

```
even line 2p     = L_p - ((H_{p-1} + H_p + 2) >> 2)
odd  line 2p + 1 = H_p + ((even_{2p} + even_{2p+2}) >> 1)   // last precinct: H_p + even_{2p}
```

The vertical transform does **not** restart at slice boundaries: the first line of a slice
uses the last high line of the previous slice. Slices are independent for entropy decoding
only.

Component 2 is not transformed.

## 16. Inverse Star-Tetrix

The four planes are decorrelated with the lifting steps of ISO/IEC 21122-1 Annex F
(Star-Tetrix), although the codestream signals no colour transform (`Cpih = 0`, no CTS or
CRG marker). The parameters are fixed: `e1 = e2 = 0`, full vertical extent across quad rows
(`Cf = 0`), no chroma gains.

Each 2x2 quad of the CFA grid at component position `(y, x)` holds:

| Quad position | CFA site | Component |
|---|---|---|
| (0, 0) | R | 1, Cr |
| (0, 1) | G | 0, Y |
| (1, 0) | G | 2, Δ |
| (1, 1) | B | 3, Cb |

This differs from the CFA-type-0 arrangement of ISO Table F.10, which places Y at (1, 0)
and Δ at (0, 1).

Writing the planes as `Y`, `R`, `D` (Δ) and `B`, run these four steps in order, each over the
whole plane before the next:

```
1. Y[y][x] -= (D[y-1][x] + D[y-1][x+1] + D[y][x]   + D[y][x+1]) >> 3
2. D[y][x] += (Y[y][x-1] + Y[y][x]     + Y[y+1][x-1] + Y[y+1][x]) >> 2    // Y after step 1
3. Y[y][x] -= (R[y][x]   + R[y][x+1]   + B[y-1][x] + B[y][x])   >> 3
   D[y][x] -= (B[y][x-1] + B[y][x]     + R[y][x]   + R[y+1][x]) >> 3
4. R[y][x] += (Y[y][x-1] + Y[y][x]     + D[y-1][x] + D[y][x])   >> 2      // final greens
   B[y][x] += (D[y][x]   + D[y][x+1]   + Y[y][x]   + Y[y+1][x]) >> 2
```

Step 1 uses each Y site's four diagonal Δ neighbours, step 2 each Δ site's four diagonal Y
neighbours, steps 3 and 4 the four direct neighbours. Neighbours outside the CFA grid are
found by reflection (CFA index `-1 -> 1`, `2W -> 2W - 2`). In plane coordinates that is
simple index clamping: `D[-1][x] -> D[0][x]`, `Y[H][x] -> Y[H-1][x]`, `R[y][W] -> R[y][W-1]`,
`B[y][-1] -> B[y][0]`, and so on. The bottom-row case of step 2 (`Y[H]`) is a known pitfall.

Sanity check: Y constant, R = D = B = 0 inverts to a flat field equal to Y in all four
planes.

## 17. Transfer curve

There is no NLT marker. Each Star-Tetrix output sample `v` (signed, still in the `<< Fq`
domain) maps to a 14-bit raw value through a vendor quadratic curve with the parameters
`B`, `A`, `C` of §4. Use 64-bit signed arithmetic:

```
i   = clamp(v + 32768, 0, 65535)
d   = i - C
q   = d < 0 ? -(d * d)                                // below the knee: no gain
            : ((d * A + 32768) >> 16)^2               // gain rounds half-up, then squares
raw = clamp(B + ((q + 163840) >> 18), 0, 16383)
```

`163840 = 32768 + 131072` folds a `(q + 2^15) >> 16` rounding stage and a `(x + 2) >> 2`
stage into one shift. The curve has an odd extension below the knee, so values below the
black level `B` are representable.

Test vectors for `B = 1008`, `A = 84429`, `C = 16255`:

| v | -32768 | -20000 | -16513 | -16000 | -10000 | 0 | 10000 | 20000 | 26000 | 30000 | 32767 |
|---|---|---|---|---|---|---|---|---|---|---|---|
| raw | 0 | 962 | 1008 | 1010 | 1277 | 2734 | 5458 | 9449 | 12451 | 14705 | 16383 |

On the Z6III the curve first reaches the DNG white level 15892 at `v = 31973` and 16383 at
`v = 32766`. Adobe writes black level 1008 (= `B`) with no linearisation table.

## 18. Mosaic assembly

The output is a `2W x 2H` (6064 x 4040) RGGB mosaic. For each component position `(y, x)`:

```
mosaic[2y    ][2x    ] = curve(R[y][x])     // component 1
mosaic[2y    ][2x + 1] = curve(Y[y][x])     // component 0
mosaic[2y + 1][2x    ] = curve(D[y][x])     // component 2
mosaic[2y + 1][2x + 1] = curve(B[y][x])     // component 3
```

## 19. Decode order

1. Locate the raw strip in the NEF (§1).
2. Parse the main header; skip CAP (§3-§5).
3. Build the band table, packet table and WGT mapping (§7-§8).
4. Frame the slices: check each SLH and its index, then walk the precinct lengths; EOC must
   follow the last precinct (§6, §9).
5. For each slice (independently, in parallel if desired), for each precinct: read the
   header, compute the GTLIs, decode the 8 packets, dequantise, apply signs and store the
   `<< Fq` coefficients (§9-§14). Vertical GCLI prediction state resets at each slice.
6. Run the horizontal then the frame-wide vertical inverse wavelet on components 0, 1 and 3
   (§15).
7. Run the inverse Star-Tetrix (§16).
8. Map each sample through the transfer curve and interleave into RGGB (§17-§18).

## 20. Profile checks and unknowns

The profile check runs once per codestream, before any entropy decoding. It rejects, as
`Unsupported`, any codestream with:

- `Nc != 4`, or a CDT that does not hold exactly 4 entries;
- `Cpih != 0` or `Ppoc != 1`;
- `Cw != 0` (column-split precincts);
- `Ng != 4` or `Ss = 0`;
- `Fq > 8`, `Bw > 20`, or `Br` outside 1..=8;
- `Fs != 1` (inline signs);
- `Qpih > 1` or `Rm > 1`;
- any component with `Sx != 1` or `Sy != 1`;
- for any component, decomposition levels (its CDT trailer byte, else the PIH `NLx` /
  `NLy`) with `NLy > 1`, `NLy = 0` but `NLx != 0`, or `NLx > 8`;
- an odd `Wf`, an `Hf` that is not a multiple of 4, or component planes narrower than 8
  or shorter than 2 (`Wf < 16` or `Hf < 4`);
- a PIH vendor extension shorter than 12 bytes (no transfer curve, §4).

Two further header checks fail with a `Header` error instead, because they mean the
codestream is inconsistent rather than merely outside the profile:

- a WGT whose entry count differs from the band count (§7);
- a codestream shorter than `Wf * Hf / 16` bytes (half a bit per photosite). Nikon
  writes 3 (HE) or 5 (HE\*) bits per photosite; the bound stops a tiny crafted input from
  making the decoder allocate a huge frame.

`inspect_nef` (and so `ticoraw inspect`) reports a failed profile check or WGT mismatch as
the `unsupported` reason instead of failing; a too-short vendor extension fails the
inspection, and the size bound is only checked when decoding.

During entropy decoding, a GCLI above `Bw`, a code group with more than 15 bit planes, a
unary code longer than 64 and a sub-packet that overruns its declared length are `Entropy`
errors. Raw GCLIs are read at `Br` bits. The inverse wavelet and Star-Tetrix use wrapping
32-bit arithmetic: valid streams never come close to the limits, and a hostile stream that
passes every check above can decode to garbage, but never makes the decoder panic.

Which bodies have been seen to use this profile, and how far each is verified, is recorded
in [compat.md](compat.md).

Open questions [unknown]:

- the CAP payload after `CONTACT_INTOPIX_`;
- the unused PIH vendor bits (§4). They are identical on every file seen so far, across
  bodies and in both modes, so their meaning cannot be inferred from variation;
- whether any camera file uses raw-GCLI packets (`Dr = 1`). None of the 111,648 packets in
  the cross-body sweep does; the path is covered only by synthetic tests.

## 21. Sources and acknowledgements

This decoder is a **clean-room implementation**. The facts above were gathered from public
descriptions and then confirmed empirically against Adobe output; no code was copied from
any of the projects below. In particular, the LGPL-licensed decoders (LibRaw, dnglab,
darktable) were read as documentation of behaviour only, and the ISO reference software
was not used as a code source.

- **ISO/IEC 21122 (JPEG XS)**, parts 1 (core coding system) and 5 (reference software):
  marker syntax, entropy coding, 5/3 wavelet and Star-Tetrix (Annex F).
  <https://jpeg.org/jpegxs/> and <https://jpeg.org/jpegxs/software.html>
- **SVT-JPEG-XS** (Intel / Open Visual Cloud, BSD-2-Clause-Patent): an open JPEG XS
  implementation, used as a readable account of the standard decoding process.
  <https://github.com/OpenVisualCloud/SVT-JPEG-XS>
- **LibRaw PR #826** (yogthos and contributors): the first public HE/HE\* decoder, and the
  source of the Star-Tetrix geometry. **RdWing**'s review of that PR contributed the exact
  transfer-curve formula and its verification against Adobe DNG Converter, plus the
  bottom-row and raw-GCLI fixes. <https://github.com/LibRaw/LibRaw/pull/826>
- **darktable PR #22743** (Wes2000ley): a port of the LibRaw work verified against Adobe,
  with format notes for the Z5II. <https://github.com/darktable-org/darktable/pull/22743>
- **dnglab PR #835** (nbuchwitz): the clearest public description of the header, band,
  packet-order, WGT-order and precinct-header deviations.
  <https://github.com/dnglab/dnglab/pull/835>
- **T. Richter, S. Fößel, A. Descampe, G. Rouvroy**, "Bayer CFA Pattern Compression With
  JPEG XS", *IEEE Transactions on Image Processing* 30 (2021) 6557-6569: background on
  Star-Tetrix and the pre-emphasis curve. <https://doi.org/10.1109/TIP.2021.3095421>

JPEG XS is subject to a patent pool, and intoPIX describes TicoRAW as patented technology;
see the patent notice in the [README](../README.md#patent-notice).
