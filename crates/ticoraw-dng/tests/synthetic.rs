//! Corpus-independent tests of `convert`: synthetic NEFs (a TicoRAW codestream from the
//! `ticoraw` test encoder in a NEF container carrying EXIF, Interoperability, GPS, XMP, a
//! Nikon MakerNote and a JPEG preview) must convert to DNGs that carry all of it and decode
//! to the same mosaic, and mutated or truncated NEFs must never make `convert` panic.

#[path = "../../ticoraw/tests/common/mod.rs"]
mod common;

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, Once, PoisonError};

use common::Rng;
use common::encoder::{self, Options as CsOptions};
use common::tiff::{Order, TiffWriter, Value};
use ticoraw::tiff::{self as rd, ByteOrder, Tiff};
use ticoraw_dng::{CarriedMetadata, Compression, Options, Preview, PreviewColorSpace};

const W: usize = 64;
const H: usize = 48;

/// What the synthetic NEF carries besides the raw strip.
#[derive(Clone, Copy)]
struct Spec {
    order: Order,
    /// MakerNote 0x001E colour space: 1 = sRGB, 2 = Adobe RGB, 4 = HLG.
    color_space: u16,
    /// Long side of the embedded JPEG preview (0 = none).
    preview_long_side: u16,
}

fn jpeg(width: u16, height: u16) -> Vec<u8> {
    let mut j = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut j, 60);
    enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_1);
    let px: Vec<u8> = (0..usize::from(width) * usize::from(height) * 3)
        .map(|i| (i % 251) as u8)
        .collect();
    enc.encode(&px, width, height, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    j
}

/// Nikon type-3 MakerNote with white balance, black level, crop, colour space and the HE*
/// compression code.
fn maker_note(color_space: u16) -> Vec<u8> {
    let mut code = vec![0u8; 16];
    code[..4].copy_from_slice(b"0100");
    code[10..12].copy_from_slice(&14u16.to_le_bytes());
    let entries = [
        (
            0x000C,
            Value::Rational(vec![(924, 512), (717, 512), (1, 1), (1, 1)]),
        ),
        (0x001E, Value::Short(vec![color_space])),
        (0x003D, Value::Short(vec![1008, 1009, 1010, 1011])),
        (0x0045, Value::Short(vec![2, 2, 60, 44])),
        (0x0051, Value::Undefined(code)),
    ];
    let mut t = TiffWriter::new(Order::Big);
    let ifd = t.ifd(&entries, 0);
    t.set_first_ifd(ifd);
    let mut mn = b"Nikon\0\x02\x11\0\0".to_vec();
    mn.extend_from_slice(&t.out);
    mn
}

/// A NEF laid out like a camera file: raw strip and preview first, then the IFDs.
fn build_nef(spec: Spec, codestream: &[u8]) -> Vec<u8> {
    let mut t = TiffWriter::new(spec.order);
    let strip_at = t.blob(codestream);
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
            (0x0117, Value::Long(vec![codestream.len() as u32])),
            (0x828D, Value::Short(vec![2, 2])),
            (0x828E, Value::Byte(vec![0, 1, 1, 2])),
        ],
        0,
    );
    let mut subs = vec![raw];
    if spec.preview_long_side > 0 {
        let j = jpeg(spec.preview_long_side, 16);
        let at = t.blob(&j);
        subs.push(t.ifd(
            &[
                (0x00FE, Value::Long(vec![1])),
                (0x0103, Value::Short(vec![6])),
                (0x0201, Value::Long(vec![at])),
                (0x0202, Value::Long(vec![j.len() as u32])),
            ],
            0,
        ));
    }
    let interop = t.ifd(&[(0x0001, Value::Ascii("R98".into()))], 0);
    let exif = t.ifd(
        &[
            (0x829A, Value::Rational(vec![(1, 320)])),
            (0x8827, Value::Short(vec![400])),
            (0x9003, Value::Ascii("2025:03:16 15:25:19".into())),
            (0x927C, Value::Undefined(maker_note(spec.color_space))),
            (0xA005, Value::Long(vec![interop])),
            (0xA431, Value::Ascii("0000001".into())),
            (
                0xA432,
                Value::Rational(vec![(240, 10), (2000, 10), (40, 10), (63, 10)]),
            ),
            (0xA434, Value::Ascii("TEST LENS 24-200mm".into())),
        ],
        0,
    );
    let gps = t.ifd(
        &[
            (0x0000, Value::Byte(vec![2, 3, 0, 0])),
            (0x0001, Value::Ascii("N".into())),
            (0x0002, Value::Rational(vec![(10, 1), (20, 1), (30, 1)])),
        ],
        0,
    );
    let ifd0 = t.ifd(
        &[
            (0x00FE, Value::Long(vec![1])),
            (0x010F, Value::Ascii("NIKON CORPORATION".into())),
            (0x0110, Value::Ascii("NIKON Z6_3".into())),
            (0x0112, Value::Short(vec![8])),
            (0x0132, Value::Ascii("2025:03:16 15:25:19".into())),
            (0x013B, Value::Ascii("    ".into())), // blank: not carried
            (0x014A, Value::Long(subs)),
            (
                0x02BC,
                Value::Byte(b"<x:xmpmeta>synthetic</x:xmpmeta>".to_vec()),
            ),
            (0x8769, Value::Long(vec![exif])),
            (0x8825, Value::Long(vec![gps])),
        ],
        0,
    );
    t.set_first_ifd(ifd0);
    t.out
}

fn codestream() -> Vec<u8> {
    encoder::encode_random(&CsOptions::new(W, H, 2), 11).codestream
}

const SDR: Spec = Spec {
    order: Order::Little,
    color_space: 1,
    preview_long_side: 1100,
};

#[test]
fn converts_with_all_metadata() {
    let nef = build_nef(SDR, &codestream());
    let raw = ticoraw::decode_nef(&nef).unwrap();
    for compression in [Compression::LosslessJpeg, Compression::Uncompressed] {
        let opts = Options {
            compression,
            original_raw_file_name: Some("SYN_0001.NEF".into()),
            ..Options::default()
        };
        let dng = ticoraw_dng::convert(&nef, &opts).unwrap();
        let back = ticoraw::dng::decode_dng_cfa(&dng).unwrap();
        assert_eq!(back.image, raw.image, "{compression:?}");
        assert_eq!(back.black_level, [1008, 1009, 1010, 1011]);

        let t = Tiff::parse(&dng, 0).unwrap();
        let ifd0 = t.ifd(t.first_ifd).unwrap();
        let u = |ifd: &rd::Ifd, tag| rd::first_uint(&t, ifd, tag).unwrap();
        assert_eq!(u(&ifd0, 0x112), Some(8));
        assert_eq!(t.ascii(ifd0.get(0x0132).unwrap()), "2025:03:16 15:25:19");
        assert!(ifd0.get(0x013B).is_none(), "blank Artist is dropped");
        assert_eq!(t.ascii(ifd0.get(0xC68B).unwrap()), "SYN_0001.NEF");
        assert_eq!(t.ascii(ifd0.get(0xC614).unwrap()), "Nikon Z 6 3");
        assert_eq!(
            t.bytes(ifd0.get(0x02BC).unwrap()),
            b"<x:xmpmeta>synthetic</x:xmpmeta>"
        );
        assert_eq!(
            t.rationals(ifd0.get(0xC630).unwrap()).unwrap()[0],
            (240, 10)
        );
        let private = t.bytes(ifd0.get(0xC634).unwrap());
        assert!(private.starts_with(b"Adobe\0MakN"));
        assert_eq!(&private[14..16], b"II");
        assert_eq!(&private[20..], &maker_note(1)[..]);

        let exif = t.ifd(u(&ifd0, 0x8769).unwrap() as u32).unwrap();
        assert_eq!(u(&exif, 0x8827), Some(400));
        assert!(exif.get(0x927C).is_none());
        assert_eq!(t.bytes(exif.get(0x9000).unwrap()), b"0231");
        assert_eq!(t.ascii(exif.get(0xA434).unwrap()), "TEST LENS 24-200mm");
        let interop = t.ifd(u(&exif, 0xA005).unwrap() as u32).unwrap();
        assert_eq!(t.ascii(interop.get(1).unwrap()), "R98");
        let gps = t.ifd(u(&ifd0, 0x8825).unwrap() as u32).unwrap();
        assert_eq!(t.ascii(gps.get(1).unwrap()), "N");
        assert_eq!(
            t.rationals(gps.get(2).unwrap()).unwrap(),
            vec![(10, 1), (20, 1), (30, 1)]
        );

        // The camera JPEG is carried unchanged as SubIFD 1.
        let subs = t.uints(ifd0.get(0x14A).unwrap()).unwrap();
        let prev = t.ifd(subs[1] as u32).unwrap();
        let at = u(&prev, 0x111).unwrap() as usize;
        let j = jpeg(1100, 16);
        assert_eq!(&dng[at..at + j.len()], &j[..]);
        assert_eq!((u(&prev, 0x100), u(&prev, 0x101)), (Some(1100), Some(16)));
        assert_eq!(u(&prev, 0xC71A), Some(2));
    }
}

#[test]
fn preview_source_follows_tone_mode_and_size() {
    let cs = codestream();
    let preview_len = |spec: Spec| {
        let nef = build_nef(spec, &cs);
        let carried = CarriedMetadata::from_nef(&nef).unwrap();
        let dng = ticoraw_dng::convert(&nef, &Options::default()).unwrap();
        let t = Tiff::parse(&dng, 0).unwrap();
        let ifd0 = t.ifd(t.first_ifd).unwrap();
        let subs = t.uints(ifd0.get(0x14A).unwrap()).unwrap();
        let prev = t.ifd(subs[1] as u32).unwrap();
        let w = rd::first_uint(&t, &prev, 0x100).unwrap();
        (carried.preview.map(|p| p.color_space), w)
    };
    // SDR with a large JPEG: carried. Adobe RGB frames label it as such.
    assert_eq!(
        preview_len(SDR),
        (Some(PreviewColorSpace::Srgb), Some(1100))
    );
    let adobe = Spec {
        color_space: 2,
        ..SDR
    };
    assert_eq!(
        preview_len(adobe),
        (Some(PreviewColorSpace::AdobeRgb), Some(1100))
    );
    // HLG, or a JPEG too small to be a preview: rendered from the crop (60x44) at half size.
    let hlg = Spec {
        color_space: 4,
        ..SDR
    };
    assert_eq!(preview_len(hlg), (None, Some(30)));
    let small = Spec {
        preview_long_side: 160,
        ..SDR
    };
    assert_eq!(preview_len(small), (None, Some(30)));
}

#[test]
fn big_endian_nefs_give_big_endian_dngs() {
    let spec = Spec {
        order: Order::Big,
        ..SDR
    };
    let nef = build_nef(spec, &codestream());
    let dng = ticoraw_dng::convert(&nef, &Options::default()).unwrap();
    let t = Tiff::parse(&dng, 0).unwrap();
    assert_eq!(t.order, ByteOrder::Big);
    let ifd0 = t.ifd(t.first_ifd).unwrap();
    let exif = t
        .ifd(rd::first_uint(&t, &ifd0, 0x8769).unwrap().unwrap() as u32)
        .unwrap();
    assert_eq!(rd::first_uint(&t, &exif, 0x8827).unwrap(), Some(400));
    assert_eq!(&t.bytes(ifd0.get(0xC634).unwrap())[14..16], b"MM");
    let raw = ticoraw::decode_nef(&nef).unwrap();
    assert_eq!(ticoraw::dng::decode_dng_cfa(&dng).unwrap().image, raw.image);
}

#[test]
fn xmp_options() {
    let nef = build_nef(SDR, &codestream());
    let xmp = |opts: &Options| {
        let dng = ticoraw_dng::convert(&nef, opts).unwrap();
        let t = Tiff::parse(&dng, 0).unwrap();
        let ifd0 = t.ifd(t.first_ifd).unwrap();
        ifd0.get(0x02BC).map(|e| t.bytes(e).to_vec())
    };
    let none = Options {
        carry_xmp: false,
        preview: Preview::None,
        ..Options::default()
    };
    assert_eq!(xmp(&none), None);
    let sidecar = Options {
        extra_xmp: Some(b"<sidecar/>".to_vec()),
        ..Options::default()
    };
    assert_eq!(xmp(&sidecar), Some(b"<sidecar/>".to_vec()));
}

#[test]
fn non_he_files_are_errors() {
    assert!(ticoraw_dng::convert(b"", &Options::default()).is_err());
    let mut nef = build_nef(SDR, &codestream());
    let cs_at = 8;
    nef[cs_at..cs_at + 4].copy_from_slice(b"\0\0\0\0"); // no longer TicoRAW
    let err = ticoraw_dng::convert(&nef, &Options::default()).unwrap_err();
    assert!(matches!(err, ticoraw_dng::Error::Decode(_)), "{err}");
}

// ---- robustness: mutated and truncated NEFs never panic ----

static LAST_PANIC: Mutex<Option<String>> = Mutex::new(None);
static PROBE_LOCK: Mutex<()> = Mutex::new(());
static PROBING: AtomicBool = AtomicBool::new(false);

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if !PROBING.load(Ordering::SeqCst) {
                return default(info);
            }
            let at = info
                .location()
                .map(|l| format!("{}:{}", l.file(), l.line()))
                .unwrap_or_default();
            *LAST_PANIC.lock().unwrap_or_else(PoisonError::into_inner) = Some(at);
        }));
    });
}

struct Failures {
    found: Vec<String>,
    _guard: MutexGuard<'static, ()>,
}

impl Failures {
    fn new() -> Self {
        install_panic_hook();
        Self {
            found: Vec::new(),
            _guard: PROBE_LOCK.lock().unwrap_or_else(PoisonError::into_inner),
        }
    }

    fn check(&mut self, input: &[u8], what: impl FnOnce() -> String) {
        PROBING.store(true, Ordering::SeqCst);
        let r = panic::catch_unwind(AssertUnwindSafe(|| {
            let _ = ticoraw_dng::convert(input, &Options::default());
            let _ = CarriedMetadata::from_nef(input);
        }));
        PROBING.store(false, Ordering::SeqCst);
        if r.is_err() {
            let p = LAST_PANIC
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .unwrap_or_default();
            self.found.push(format!("{}: panic at {p}", what()));
        }
    }

    fn assert_none(self) {
        assert!(
            self.found.is_empty(),
            "{} panicking inputs:\n{}",
            self.found.len(),
            self.found[..self.found.len().min(12)].join("\n")
        );
    }
}

#[test]
fn mutated_and_truncated_nefs_never_panic() {
    let cs = codestream();
    let mut failures = Failures::new();
    for (k, order) in [Order::Little, Order::Big].into_iter().enumerate() {
        let nef = build_nef(Spec { order, ..SDR }, &cs);
        // Mutate the container (everything after the codestream), not the pixels.
        let start = 8 + cs.len();
        let mut rng = Rng::new(40 + k as u64);
        let mut buf = nef.clone();
        for _ in 0..2500 {
            let at = start + rng.below((nef.len() - start) as u64) as usize;
            let old = buf[at];
            let new = match rng.below(3) {
                0 => old ^ (1 << rng.below(8)),
                1 => [0x00, 0xFF, 0x7F, 0x80, 0x01][rng.below(5) as usize],
                _ => rng.byte(),
            };
            buf[at] = new;
            failures.check(&buf, || {
                format!("{order:?} byte {at}: {old:#04x} -> {new:#04x}")
            });
            buf[at] = old;
        }
        let n = nef.len();
        for len in (0..256.min(n)).chain((256..n).step_by(29)) {
            failures.check(&nef[..len], || format!("{order:?} truncated to {len}"));
        }
    }
    failures.assert_none();
}
