//! Synthetic NEFs for the CLI tests: a TicoRAW (HE*) codestream from the `ticoraw` test
//! encoder, or a non-TicoRAW strip, in a NEF container that carries every kind of personal
//! data a real camera file does (serial numbers, owner, artist, copyright, dates, GPS).
//!
//! Shared by the unit tests in `src/` and the integration tests in `tests/cli.rs`; both
//! declare `ticoraw/tests/common` as the crate-level module `common`.
#![allow(dead_code)]

use crate::common::encoder::{self, Options as CsOptions};
use crate::common::tiff::{Order, TiffWriter, Value};

/// CFA width and height of the synthetic frames.
pub const W: usize = 64;
pub const H: usize = 48;

/// IFD0 `Software`: the firmware a report must show.
pub const FIRMWARE: &str = "Ver.01.10";

/// EXIF BodySerialNumber.
pub const SERIAL: &str = "BODYSERIAL4711";
/// Nikon MakerNote 0x001D (SerialNumber).
pub const MAKER_NOTE_SERIAL: &str = "MNSERIAL31337";
/// EXIF LensSerialNumber.
pub const LENS_SERIAL: &str = "LENSSERIAL99887";
/// EXIF CameraOwnerName.
pub const OWNER: &str = "Jane Q Photographer";
/// IFD0 Artist.
pub const ARTIST: &str = "Artist Jane Q";
/// IFD0 Copyright.
pub const COPYRIGHT: &str = "Copyright Jane Q 2025";
/// IFD0 DateTime.
pub const DATE_TIME: &str = "2025:03:16 15:25:19";
/// EXIF DateTimeOriginal.
pub const DATE_ORIGINAL: &str = "2024:12:31 23:59:58";
/// EXIF ImageUniqueID.
pub const IMAGE_ID: &str = "UNIQUEID0123456789ABCDEF";
/// GPS GPSMapDatum.
pub const GPS_DATUM: &str = "GPSDATUMSECRET";
/// A word inside the embedded XMP packet.
pub const XMP_SECRET: &str = "xmpsecretkeyword";

/// Every personal string of the synthetic NEFs. `report` output must contain none of them.
pub const PERSONAL: &[&str] = &[
    SERIAL,
    MAKER_NOTE_SERIAL,
    LENS_SERIAL,
    OWNER,
    ARTIST,
    COPYRIGHT,
    DATE_TIME,
    DATE_ORIGINAL,
    "2025:03:16",
    "2024:12:31",
    IMAGE_ID,
    GPS_DATUM,
    XMP_SECRET,
];

/// Nikon type-3 MakerNote with white balance, black level, serial number and the given
/// compression code (14 = HE*, 3 = lossless).
fn maker_note(compression_code: u16) -> Vec<u8> {
    let mut code = vec![0u8; 16];
    code[..4].copy_from_slice(b"0100");
    code[10..12].copy_from_slice(&compression_code.to_le_bytes());
    let entries = [
        (
            0x000C,
            Value::Rational(vec![(924, 512), (717, 512), (1, 1), (1, 1)]),
        ),
        (0x001D, Value::Ascii(MAKER_NOTE_SERIAL.into())),
        (0x003D, Value::Short(vec![1008, 1009, 1010, 1011])),
        (0x0051, Value::Undefined(code)),
    ];
    let mut t = TiffWriter::new(Order::Big);
    let ifd = t.ifd(&entries, 0);
    t.set_first_ifd(ifd);
    let mut mn = b"Nikon\0\x02\x11\0\0".to_vec();
    mn.extend_from_slice(&t.out);
    mn
}

/// A NEF laid out like a camera file around `strip`.
fn build(strip: &[u8], compression_code: u16) -> Vec<u8> {
    let mut t = TiffWriter::new(Order::Little);
    let strip_at = t.blob(strip);
    let raw = t.ifd(
        &[
            (0x00FE, Value::Long(vec![0])),
            (0x0100, Value::Long(vec![W as u32])),
            (0x0101, Value::Long(vec![H as u32])),
            (0x0102, Value::Short(vec![14])),
            (0x0103, Value::Short(vec![34713])),
            (0x0106, Value::Short(vec![32803])),
            (0x0111, Value::Long(vec![strip_at])),
            (0x0115, Value::Short(vec![1])),
            (0x0116, Value::Long(vec![H as u32])),
            (0x0117, Value::Long(vec![strip.len() as u32])),
            (0x828D, Value::Short(vec![2, 2])),
            (0x828E, Value::Byte(vec![0, 1, 1, 2])),
        ],
        0,
    );
    let exif = t.ifd(
        &[
            (0x829A, Value::Rational(vec![(1, 320)])),
            (0x8827, Value::Short(vec![400])),
            (0x9003, Value::Ascii(DATE_ORIGINAL.into())),
            (0x927C, Value::Undefined(maker_note(compression_code))),
            (0xA420, Value::Ascii(IMAGE_ID.into())),
            (0xA430, Value::Ascii(OWNER.into())),
            (0xA431, Value::Ascii(SERIAL.into())),
            (0xA434, Value::Ascii("TEST LENS 24-200mm".into())),
            (0xA435, Value::Ascii(LENS_SERIAL.into())),
        ],
        0,
    );
    let gps = t.ifd(
        &[
            (0x0000, Value::Byte(vec![2, 3, 0, 0])),
            (0x0001, Value::Ascii("N".into())),
            (0x0002, Value::Rational(vec![(47, 1), (22, 1), (3141, 100)])),
            (0x0012, Value::Ascii(GPS_DATUM.into())),
        ],
        0,
    );
    let xmp = format!("<x:xmpmeta><dc:subject>{XMP_SECRET}</dc:subject></x:xmpmeta>");
    let ifd0 = t.ifd(
        &[
            (0x00FE, Value::Long(vec![1])),
            (0x010F, Value::Ascii("NIKON CORPORATION".into())),
            (0x0110, Value::Ascii("NIKON Z6_3".into())),
            (0x0112, Value::Short(vec![1])),
            (0x0131, Value::Ascii(FIRMWARE.into())),
            (0x0132, Value::Ascii(DATE_TIME.into())),
            (0x013B, Value::Ascii(ARTIST.into())),
            (0x014A, Value::Long(vec![raw])),
            (0x02BC, Value::Byte(xmp.into_bytes())),
            (0x8298, Value::Ascii(COPYRIGHT.into())),
            (0x8769, Value::Long(vec![exif])),
            (0x8825, Value::Long(vec![gps])),
        ],
        0,
    );
    t.set_first_ifd(ifd0);
    t.out
}

/// A High Efficiency* NEF that decodes.
#[must_use]
pub fn he_nef(seed: u64) -> Vec<u8> {
    let cs = encoder::encode_random(&CsOptions::new(W, H, 2), seed).codestream;
    build(&cs, 14)
}

/// A classic (lossless compressed, not TicoRAW) NEF.
#[must_use]
pub fn classic_nef() -> Vec<u8> {
    let strip: Vec<u8> = (0..4096u32).map(|i| (i * 7 % 251) as u8).collect();
    build(&strip, 3)
}
