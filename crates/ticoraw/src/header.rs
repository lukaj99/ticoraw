//! JPEG XS (ISO/IEC 21122-1) main-header parsing, including the intoPIX TicoRAW
//! vendor extensions found in Nikon HE/HE* codestreams.
//!
//! This stage module is public for testing and advanced use; its API may change in minor
//! releases.
//! [`MainHeader`] and [`TransferCurve`] also appear in [`crate::CodestreamInfo`].

use crate::error::{Result, TicoError};

/// Start of codestream.
pub const SOC: u16 = 0xFF10;
/// End of codestream.
pub const EOC: u16 = 0xFF11;
/// Picture header.
pub const PIH: u16 = 0xFF12;
/// Component table.
pub const CDT: u16 = 0xFF13;
/// Weights table (gain and priority per band).
pub const WGT: u16 = 0xFF14;
/// Extension (comment) marker.
pub const COM: u16 = 0xFF15;
/// Nonlinearity marker.
pub const NLT: u16 = 0xFF16;
/// Component-dependent wavelet decomposition marker.
pub const CWD: u16 = 0xFF17;
/// Colour transformation specification marker.
pub const CTS: u16 = 0xFF18;
/// Component registration marker.
pub const CRG: u16 = 0xFF19;
/// Slice header.
pub const SLH: u16 = 0xFF20;
/// Capabilities marker (TicoRAW: a vendor signature, not capability bits).
pub const CAP: u16 = 0xFF50;

/// Picture header (PIH) fields, ISO/IEC 21122-1 Table A.7.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PictureHeader {
    /// `Lcod`: codestream length in bytes.
    pub lcod: u32,
    /// `Ppih`: profile (0 on camera files).
    pub ppih: u16,
    /// `Plev`: level and sublevel (0 on camera files).
    pub plev: u16,
    /// `Wf`: frame width; on TicoRAW the CFA mosaic width.
    pub wf: u16,
    /// `Hf`: frame height; on TicoRAW the CFA mosaic height.
    pub hf: u16,
    /// `Cw`: precinct width (0 = full frame width).
    pub cw: u16,
    /// `Hsl`: slice height, in precincts.
    pub hsl: u16,
    /// `Nc`: number of components.
    pub nc: u8,
    /// `Ng`: coefficients per code group.
    pub ng: u8,
    /// `Ss`: code groups per significance group.
    pub ss: u8,
    /// `Bw`: nominal bit depth of the wavelet coefficients (the largest admissible GCLI).
    pub bw: u8,
    /// `Fq`: fractional bits of the coefficients.
    pub fq: u8,
    /// `Br`: bits per raw-coded GCLI.
    pub br: u8,
    /// `Fslc`: slice coding mode.
    pub fslc: u8,
    /// `Ppoc`: progression order (TicoRAW uses the non-standard value 1).
    pub ppoc: u8,
    /// `Cpih`: colour transform (0 = none signalled).
    pub cpih: u8,
    /// `NLx`: default horizontal decomposition levels.
    pub nlx: u8,
    /// `NLy`: default vertical decomposition levels.
    pub nly: u8,
    /// `Lh`: long precinct headers enforced.
    pub lh: u8,
    /// `Rl`: raw-mode selection per packet.
    pub rl: u8,
    /// `Qpih`: inverse quantiser (0 = deadzone, 1 = uniform).
    pub qpih: u8,
    /// `Fs`: sign handling (0 = inline, 1 = separate sign sub-packets).
    pub fs: u8,
    /// `Rm`: run mode for insignificant code groups.
    pub rm: u8,
    /// Bytes beyond the 26-byte standard PIH (intoPIX vendor extension).
    pub extension: Vec<u8>,
}

/// One CDT entry: bit depth, sampling factors and any vendor trailer byte(s).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentInfo {
    /// `Bc`: component bit depth.
    pub bit_depth: u8,
    /// `Sx`: horizontal sampling factor.
    pub sx: u8,
    /// `Sy`: vertical sampling factor.
    pub sy: u8,
    /// Per-component trailer bytes (TicoRAW: decomposition levels `Nlx << 4 | Nly`).
    pub extension: Vec<u8>,
}

/// Every main-header marker segment up to the first slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainHeader {
    /// CAP payload (TicoRAW: `CONTACT_INTOPIX_` and vendor bytes).
    pub cap: Vec<u8>,
    /// The picture header.
    pub pih: PictureHeader,
    /// One CDT entry per component.
    pub components: Vec<ComponentInfo>,
    /// (gain, priority) per band in WGT order.
    pub weights: Vec<(u8, u8)>,
    /// Other segments (marker, payload) kept verbatim: NLT, CWD, CTS, CRG, COM.
    pub other: Vec<(u16, Vec<u8>)>,
    /// Offset of the first SLH marker within the codestream.
    pub first_slice: usize,
}

pub(crate) fn be16(b: &[u8], at: usize) -> Result<u16> {
    b.get(at..at + 2)
        .map(|s| u16::from_be_bytes([s[0], s[1]]))
        .ok_or(TicoError::Truncated {
            what: "codestream",
            offset: at,
        })
}

fn parse_pih(p: &[u8]) -> Result<PictureHeader> {
    if p.len() < 24 {
        return Err(TicoError::Header(format!(
            "PIH payload is {} bytes, need 24",
            p.len()
        )));
    }
    let u16_at = |i: usize| u16::from_be_bytes([p[i], p[i + 1]]);
    Ok(PictureHeader {
        lcod: u32::from_be_bytes([p[0], p[1], p[2], p[3]]),
        ppih: u16_at(4),
        plev: u16_at(6),
        wf: u16_at(8),
        hf: u16_at(10),
        cw: u16_at(12),
        hsl: u16_at(14),
        nc: p[16],
        ng: p[17],
        ss: p[18],
        bw: p[19],
        fq: p[20] >> 4,
        br: p[20] & 15,
        fslc: p[21] >> 7,
        ppoc: (p[21] >> 4) & 7,
        cpih: p[21] & 15,
        nlx: p[22] >> 4,
        nly: p[22] & 15,
        lh: p[23] >> 7,
        rl: (p[23] >> 6) & 1,
        qpih: (p[23] >> 4) & 3,
        fs: (p[23] >> 2) & 3,
        rm: p[23] & 3,
        extension: p[24..].to_vec(),
    })
}

/// Parses SOC through the last main-header segment.
///
/// # Errors
///
/// [`TicoError::Header`] when SOC, CAP, PIH, CDT or WGT is missing, a segment has an
/// invalid length or an unexpected marker, the PIH is shorter than 24 bytes, or the CDT
/// does not divide into `Nc` entries; [`TicoError::Truncated`] when the input ends first.
pub fn parse_main_header(cs: &[u8]) -> Result<MainHeader> {
    if be16(cs, 0)? != SOC {
        return Err(TicoError::Header("missing SOC marker".into()));
    }
    let mut pos = 2;
    let (mut cap, mut pih, mut cdt, mut wgt) = (None, None, None, None);
    let mut other = Vec::new();
    loop {
        let marker = be16(cs, pos)?;
        if marker == SLH {
            break;
        }
        let len = be16(cs, pos + 2)? as usize;
        if len < 2 {
            return Err(TicoError::Header(format!(
                "segment {marker:#06x} has length {len}"
            )));
        }
        let payload = cs.get(pos + 4..pos + 2 + len).ok_or(TicoError::Truncated {
            what: "marker segment",
            offset: pos,
        })?;
        match marker {
            CAP => cap = Some(payload.to_vec()),
            PIH => pih = Some(parse_pih(payload)?),
            CDT => cdt = Some(payload.to_vec()),
            WGT => {
                wgt = Some(
                    payload
                        .chunks_exact(2)
                        .map(|c| (c[0], c[1]))
                        .collect::<Vec<_>>(),
                );
            }
            NLT | CWD | CTS | CRG | COM => other.push((marker, payload.to_vec())),
            m => {
                return Err(TicoError::Header(format!(
                    "unexpected marker {m:#06x} at {pos}"
                )));
            }
        }
        pos += 2 + len;
    }
    let pih = pih.ok_or_else(|| TicoError::Header("missing PIH".into()))?;
    let cdt = cdt.ok_or_else(|| TicoError::Header("missing CDT".into()))?;
    let nc = pih.nc as usize;
    if nc == 0 || cdt.len() % nc != 0 {
        return Err(TicoError::Header(format!(
            "CDT of {} bytes for {nc} components",
            cdt.len()
        )));
    }
    let per = cdt.len() / nc;
    if per < 2 {
        return Err(TicoError::Header(format!("CDT entries of {per} bytes")));
    }
    let components = cdt
        .chunks_exact(per)
        .map(|c| ComponentInfo {
            bit_depth: c[0],
            sx: c[1] >> 4,
            sy: c[1] & 15,
            extension: c[2..].to_vec(),
        })
        .collect();
    Ok(MainHeader {
        cap: cap.ok_or_else(|| TicoError::Header("missing CAP".into()))?,
        pih,
        components,
        weights: wgt.ok_or_else(|| TicoError::Header("missing WGT".into()))?,
        other,
        first_slice: pos,
    })
}

/// Parameters of the TicoRAW output transfer curve carried in the PIH vendor bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferCurve {
    /// Output black offset (`B`).
    pub black: i64,
    /// Q16 gain applied above the knee (`A`).
    pub gain: i64,
    /// Knee in the 16-bit code domain (`C`).
    pub knee: i64,
}

impl TransferCurve {
    /// Decodes `B`, `A` and `C` from the 13 vendor bytes that follow the standard PIH.
    ///
    /// # Errors
    ///
    /// [`TicoError::Unsupported`] when fewer than 12 vendor bytes are present.
    pub fn from_extension(t: &[u8]) -> Result<Self> {
        if t.len() < 12 {
            return Err(TicoError::Unsupported(format!(
                "PIH vendor extension of {} bytes",
                t.len()
            )));
        }
        Ok(Self {
            black: i64::from(t[3] & 0x0F) << 8 | i64::from(t[4]),
            gain: i64::from(t[7] & 0x03) << 16 | i64::from(t[8]) << 8 | i64::from(t[9]),
            knee: i64::from(t[10]) << 8 | i64::from(t[11]),
        })
    }

    /// Maps one inverse-Star-Tetrix output sample to a 14-bit raw value.
    #[inline]
    #[must_use]
    pub fn apply(&self, v: i32) -> u16 {
        let i = (i64::from(v) + 32768).clamp(0, 65535);
        let d = i - self.knee;
        let q = if d < 0 {
            -(d * d)
        } else {
            let g = (d * self.gain + 32768) >> 16;
            g * g
        };
        (self.black + ((q + 163_840) >> 18)).clamp(0, 16383) as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NIKON_EXT: [u8; 13] = [
        0x50, 0x88, 0x70, 0x83, 0xF0, 0x15, 0x23, 0xD1, 0x49, 0xCD, 0x3F, 0x7F, 0x07,
    ];

    #[test]
    fn decodes_vendor_curve_parameters() {
        let c = TransferCurve::from_extension(&NIKON_EXT).unwrap();
        assert_eq!((c.black, c.gain, c.knee), (1008, 84429, 16255));
    }

    #[test]
    fn curve_matches_reference_vectors() {
        let c = TransferCurve::from_extension(&NIKON_EXT).unwrap();
        let cases = [
            (-32768, 0),
            (-20000, 962),
            (-16513, 1008),
            (-16000, 1010),
            (-10000, 1277),
            (0, 2734),
            (10000, 5458),
            (20000, 9449),
            (26000, 12451),
            (30000, 14705),
            (32767, 16383),
        ];
        for (v, want) in cases {
            assert_eq!(c.apply(v), want, "v = {v}");
        }
    }

    #[test]
    fn parses_nikon_main_header() {
        let mut cs = vec![0xFF, 0x10, 0xFF, 0x50, 0x00, 0x22];
        cs.extend_from_slice(b"CONTACT_INTOPIX_");
        cs.extend_from_slice(&[0xEF, 0xC0]);
        cs.extend_from_slice(&[0; 14]);
        cs.extend_from_slice(&[
            0xFF, 0x12, 0x00, 0x27, 0x00, 0xE9, 0xA2, 0x00, 0, 0, 0, 0, 0x17, 0xB0, 0x0F, 0xC8,
        ]);
        cs.extend_from_slice(&[0, 0, 0x00, 0x10, 4, 4, 8, 0x12, 0x44, 0x10, 0x51, 0x14]);
        cs.extend_from_slice(&NIKON_EXT);
        cs.extend_from_slice(&[
            0xFF, 0x13, 0x00, 0x0E, 0x0E, 0x11, 0x51, 0x0E, 0x11, 0x51, 0x0E, 0x11, 0x00, 0x0E,
            0x11, 0x51,
        ]);
        cs.extend_from_slice(&[0xFF, 0x14, 0x00, 0x06, 3, 2, 3, 19]);
        cs.extend_from_slice(&[0xFF, 0x20, 0x00, 0x04, 0x00, 0x00]);
        let h = parse_main_header(&cs).unwrap();
        assert_eq!(
            (h.pih.wf, h.pih.hf, h.pih.nc, h.pih.hsl),
            (6064, 4040, 4, 16)
        );
        assert_eq!((h.pih.fq, h.pih.br, h.pih.ppoc, h.pih.cpih), (4, 4, 1, 0));
        assert_eq!((h.pih.nlx, h.pih.nly, h.pih.qpih, h.pih.fs), (5, 1, 1, 1));
        assert_eq!(h.components[2].extension, vec![0]);
        assert_eq!(h.weights, vec![(3, 2), (3, 19)]);
        assert_eq!(h.first_slice, cs.len() - 6);
    }
}
