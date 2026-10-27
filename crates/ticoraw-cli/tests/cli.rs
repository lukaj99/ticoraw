//! End-to-end tests of the `ticoraw` binary.
//!
//! Synthetic NEFs (from `support/synthetic.rs`) exercise `convert`, `decode` and `report`
//! without any camera files. Tests that need real NEFs run only when `TICORAW_CORPUS` names
//! a directory holding at least one HE/HE* NEF together with an Adobe DNG converted from it
//! (matched through the DNG's OriginalRawFileName); they only read from it. Everything
//! written goes under Cargo's per-target temp directory.

#[path = "../../ticoraw/tests/common/mod.rs"]
mod common;
#[path = "support/synthetic.rs"]
mod synthetic;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use ticoraw::tiff::{self as rd, Tiff};

fn ticoraw(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ticoraw"))
        .args(args)
        .output()
        .expect("run ticoraw")
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("exit code")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A fresh, empty scratch directory for one test.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn s(p: &Path) -> &str {
    p.to_str().expect("UTF-8 path")
}

#[test]
fn version_and_top_level_help() {
    let o = ticoraw(&["--version"]);
    assert_eq!(code(&o), 0);
    assert_eq!(
        stdout(&o).trim(),
        format!("ticoraw {}", env!("CARGO_PKG_VERSION"))
    );
    for args in [&["--help"][..], &["-h"], &["help"]] {
        let o = ticoraw(args);
        assert_eq!(code(&o), 0, "{args:?}");
        let out = stdout(&o);
        for cmd in ["convert", "inspect", "decode", "verify", "scan", "report"] {
            assert!(out.contains(cmd), "{args:?} lists {cmd}");
        }
    }
}

#[test]
fn per_command_help() {
    for cmd in ["convert", "inspect", "decode", "verify", "scan", "report"] {
        for args in [vec![cmd, "--help"], vec![cmd, "-h"], vec!["help", cmd]] {
            let o = ticoraw(&args);
            assert_eq!(code(&o), 0, "{args:?}");
            assert!(
                stdout(&o).contains(&format!("Usage: ticoraw {cmd}")),
                "{args:?}"
            );
            assert!(stderr(&o).is_empty(), "{args:?}");
        }
    }
}

#[test]
fn usage_errors_exit_2() {
    let cases: &[&[&str]] = &[
        &[],
        &["frobnicate"],
        &["--frob"],
        &["help", "frob"],
        &["inspect"],
        &["inspect", "--yaml", "a.nef"],
        &["inspect", "--json=1", "a.nef"],
        &["decode"],
        &["decode", "a.nef", "b.nef"],
        &["decode", "--format", "tiff", "a.nef"],
        &["decode", "a.nef", "-o"],
        &["verify"],
        &["verify", "a.nef"],
        &["verify", "a.nef", "b.dng", "c.dng"],
        &["scan"],
        &["scan", "a", "b"],
        &["scan", "--deep", "a"],
        &["decode", "--format", "pgm", "--uncompressed", "a.nef"],
        &["decode", "--no-preview", "--format=pgm", "a.nef"],
        &["convert"],
        &["convert", "--skip-existing", "--force", "a"],
        &["convert", "-j", "0", "a"],
        &["convert", "--jobs=many", "a"],
        &["convert", "-o"],
        &["convert", "--frob", "a"],
        &["report"],
        &["report", "a.nef", "b.nef"],
        &["report", "--json", "a.nef"],
    ];
    for args in cases {
        let o = ticoraw(args);
        assert_eq!(code(&o), 2, "{args:?}");
        assert!(stdout(&o).is_empty(), "{args:?}");
        assert!(stderr(&o).contains("Usage: ticoraw"), "{args:?}");
    }
}

#[test]
fn failures_exit_1() {
    let dir = scratch("failures");
    let junk = dir.join("junk.nef");
    std::fs::write(&junk, b"definitely not a TIFF").unwrap();
    let missing = dir.join("missing.nef");

    let o = ticoraw(&["inspect", s(&missing)]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("missing.nef"));

    let o = ticoraw(&["inspect", "--json", s(&junk)]);
    assert_eq!(code(&o), 1);
    let json = stdout(&o);
    assert!(json.trim_start().starts_with('['), "{json}");
    assert!(json.contains("\"error\": \"container:"), "{json}");

    let o = ticoraw(&["decode", s(&junk)]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).starts_with("ticoraw: error:"));
    assert!(
        !dir.join("junk.dng").exists(),
        "no output on decode failure"
    );

    let o = ticoraw(&["verify", s(&junk), s(&missing)]);
    assert_eq!(code(&o), 1);

    let o = ticoraw(&["scan", s(&missing)]);
    assert_eq!(code(&o), 1);
}

#[test]
fn decode_never_overwrites_its_input() {
    let dir = scratch("self_overwrite");
    let input = dir.join("photo.dng");
    std::fs::write(&input, b"original").unwrap();
    for args in [
        vec!["decode", s(&input)],
        vec!["decode", "--force", s(&input)],
    ] {
        let o = ticoraw(&args);
        assert_eq!(code(&o), 1, "{args:?}");
        assert!(stderr(&o).contains("is the input file"), "{args:?}");
    }
    assert_eq!(std::fs::read(&input).unwrap(), b"original");
}

#[test]
fn scan_classifies_and_counts() {
    let dir = scratch("scan");
    let o = ticoraw(&["scan", s(&dir)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("0 NEF files"));

    std::fs::write(dir.join("a.NEF"), b"junk").unwrap();
    std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("b.nef"), b"junk").unwrap();

    let o = ticoraw(&["scan", s(&dir)]);
    assert_eq!(code(&o), 1);
    let out = stdout(&o);
    assert!(out.contains("1 NEF files"), "{out}");
    assert!(out.contains("error    1"), "{out}");

    let o = ticoraw(&["scan", "--recursive", "--decode", s(&dir)]);
    assert_eq!(code(&o), 1);
    let out = stdout(&o);
    assert!(out.contains("2 NEF files"), "{out}");
    assert!(out.contains("error    2"), "{out}");
    assert!(out.contains("decoded 0 of 0 TicoRAW files"), "{out}");
}

/// An ASCII tag of IFD0.
fn ifd0_ascii(tiff: &[u8], tag: u16) -> Option<String> {
    let t = Tiff::parse(tiff, 0).ok()?;
    let ifd0 = t.ifd(t.first_ifd).ok()?;
    Some(t.ascii(ifd0.get(tag)?))
}

/// IFD0's XMP packet (tag 0x02BC).
fn ifd0_xmp(tiff: &[u8]) -> Option<Vec<u8>> {
    let t = Tiff::parse(tiff, 0).ok()?;
    let ifd0 = t.ifd(t.first_ifd).ok()?;
    Some(t.bytes(ifd0.get(0x02BC)?).to_vec())
}

/// The Compression tag of each SubIFD of a DNG: 7 for lossless JPEG, 1 for uncompressed,
/// 7 again for a JPEG preview.
fn sub_ifd_compressions(dng: &[u8]) -> Vec<u64> {
    let t = Tiff::parse(dng, 0).unwrap();
    let ifd0 = t.ifd(t.first_ifd).unwrap();
    let subs = t.uints(ifd0.get(0x14A).expect("SubIFDs")).unwrap();
    subs.into_iter()
        .map(|o| {
            let ifd = t.ifd(u32::try_from(o).unwrap()).unwrap();
            rd::first_uint(&t, &ifd, 0x103).unwrap().unwrap()
        })
        .collect()
}

/// A directory with an HE* NEF (plus a sidecar), a classic NEF, a junk "NEF", a text file
/// and an HE* NEF in a subdirectory.
fn synthetic_tree(name: &str) -> PathBuf {
    let dir = scratch(name);
    std::fs::write(dir.join("he.NEF"), synthetic::he_nef(1)).unwrap();
    std::fs::write(
        dir.join("HE.xmp"),
        b"<x:xmpmeta>sidecar rating 5</x:xmpmeta>",
    )
    .unwrap();
    std::fs::write(dir.join("classic.nef"), synthetic::classic_nef()).unwrap();
    std::fs::write(dir.join("junk.nef"), b"not a TIFF").unwrap();
    std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("deep.nrw"), synthetic::he_nef(2)).unwrap();
    dir
}

/// The JSON Lines of `o`'s stdout that carry `"event":"<event>"`.
fn events(o: &Output, event: &str) -> Vec<String> {
    let tag = format!("\"event\":\"{event}\"");
    stdout(o)
        .lines()
        .filter(|l| l.contains(&tag))
        .map(str::to_owned)
        .collect()
}

#[test]
fn convert_classifies_converts_and_mirrors_directories() {
    let dir = synthetic_tree("convert");
    let out = dir.join("out");
    let o = ticoraw(&["convert", "--json", s(&dir), "-o", s(&out)]);
    assert_eq!(code(&o), 1, "the junk file fails: {}", stdout(&o));
    assert_eq!(events(&o, "start").len(), 1);
    let converted = events(&o, "converted");
    assert_eq!(converted.len(), 1, "{converted:?}");
    assert!(converted[0].contains("\"warning\":null"), "{converted:?}");
    let skipped = events(&o, "skipped");
    assert_eq!(skipped.len(), 1, "{skipped:?}");
    assert!(skipped[0].contains("\"reason\":\"classic\""), "{skipped:?}");
    assert!(skipped[0].contains("already supports it"), "{skipped:?}");
    let failed = events(&o, "failed");
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(failed[0].contains("junk.nef"), "{failed:?}");
    let summary = events(&o, "summary");
    assert!(
        summary[0].contains("\"files\":3,\"converted\":1,\"skipped\":1,\"failed\":1"),
        "{summary:?}"
    );

    // The DNG is complete, decodes to the NEF's mosaic and names its source.
    let dng = std::fs::read(out.join("he.dng")).unwrap();
    let nef = std::fs::read(dir.join("he.NEF")).unwrap();
    let raw = ticoraw::decode_nef(&nef).unwrap();
    assert_eq!(ticoraw::dng::decode_dng_cfa(&dng).unwrap().image, raw.image);
    assert_eq!(ifd0_ascii(&dng, 0xC68B).as_deref(), Some("he.NEF"));
    assert_eq!(sub_ifd_compressions(&dng), vec![7, 7]);
    assert!(
        ifd0_xmp(&dng).is_some_and(|x| {
            let secret = synthetic::XMP_SECRET.as_bytes();
            x.windows(secret.len()).any(|w| w == secret)
        }),
        "the NEF's own XMP is carried by default"
    );
    let o = ticoraw(&["verify", s(&dir.join("he.NEF")), s(&out.join("he.dng"))]);
    assert_eq!(code(&o), 0, "{}", stdout(&o));
    assert!(!out.join("classic.dng").exists());
    assert!(!out.join("sub").exists(), "not recursive without -r");

    // Existing DNGs: refused, skipped, or replaced (here with the sidecar's XMP). The
    // subdirectory is mirrored with -r.
    let o = ticoraw(&["convert", s(&dir.join("he.NEF")), "-o", s(&out)]);
    assert_eq!(code(&o), 1);
    assert!(stdout(&o).contains("already exists"), "{}", stdout(&o));
    let o = ticoraw(&[
        "convert",
        "-r",
        "--skip-existing",
        s(&dir),
        "--output",
        s(&out),
    ]);
    assert_eq!(code(&o), 1, "junk still fails");
    let text = stdout(&o);
    assert!(text.contains("he.dng already exists"), "{text}");
    assert!(
        text.contains("4 files: 1 converted, 2 skipped, 1 failed"),
        "{text}"
    );
    assert!(text.contains("files/s"), "{text}");
    let deep = std::fs::read(out.join("sub").join("deep.dng")).unwrap();
    assert_eq!(ifd0_ascii(&deep, 0xC68B).as_deref(), Some("deep.nrw"));

    let o = ticoraw(&[
        "convert",
        "--force",
        "--with-sidecar",
        "--uncompressed",
        "--no-preview",
        "-j",
        "1",
        s(&dir.join("he.NEF")),
        s(&dir.join("classic.nef")),
        "-o",
        s(&out),
    ]);
    assert_eq!(code(&o), 0, "{}", stdout(&o));
    assert!(stdout(&o).contains("HE.xmp"), "{}", stdout(&o));
    let dng = std::fs::read(out.join("he.dng")).unwrap();
    assert_eq!(
        ifd0_xmp(&dng).as_deref(),
        Some(&b"<x:xmpmeta>sidecar rating 5</x:xmpmeta>"[..])
    );
    assert_eq!(
        sub_ifd_compressions(&dng),
        vec![1],
        "uncompressed, no preview"
    );
    assert_eq!(ticoraw::dng::decode_dng_cfa(&dng).unwrap().image, raw.image);
}

#[test]
fn convert_writes_next_to_the_input_and_never_over_it() {
    let dir = scratch("convert_beside");
    let nef = dir.join("DSC_0001.NEF");
    std::fs::write(&nef, synthetic::he_nef(4)).unwrap();
    let o = ticoraw(&["convert", s(&nef)]);
    assert_eq!(code(&o), 0, "{}", stdout(&o));
    assert!(dir.join("DSC_0001.dng").exists());
    // No temporary files are left behind.
    let names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");

    // An input whose DNG path is itself is never replaced.
    let input = dir.join("photo.dng");
    std::fs::write(&input, b"original").unwrap();
    for flag in ["--force", "--skip-existing"] {
        let o = ticoraw(&["convert", flag, s(&input)]);
        assert_eq!(std::fs::read(&input).unwrap(), b"original", "{flag}");
        if flag == "--force" {
            assert_eq!(code(&o), 1);
            assert!(stdout(&o).contains("is the input file"), "{}", stdout(&o));
        }
    }

    // A missing input fails, an empty directory converts nothing.
    let o = ticoraw(&["convert", s(&dir.join("missing.nef"))]);
    assert_eq!(code(&o), 1);
    let empty = scratch("convert_empty");
    let o = ticoraw(&["convert", s(&empty)]);
    assert_eq!(code(&o), 0);
    assert!(
        stdout(&o).contains("0 files: 0 converted"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn decode_writes_a_complete_dng() {
    let dir = scratch("decode_synthetic");
    let nef = dir.join("SYN_0001.NEF");
    std::fs::write(&nef, synthetic::he_nef(6)).unwrap();
    let raw = ticoraw::decode_nef(&std::fs::read(&nef).unwrap()).unwrap();
    let o = ticoraw(&["decode", s(&nef)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let dng = std::fs::read(dir.join("SYN_0001.dng")).unwrap();
    assert_eq!(ifd0_ascii(&dng, 0xC68B).as_deref(), Some("SYN_0001.NEF"));
    assert!(ifd0_ascii(&dng, 0x0131).is_some(), "Software");
    assert_eq!(sub_ifd_compressions(&dng), vec![7, 7]);
    assert_eq!(ticoraw::dng::decode_dng_cfa(&dng).unwrap().image, raw.image);

    let out = dir.join("small.dng");
    let o = ticoraw(&[
        "decode",
        "--uncompressed",
        "--no-preview",
        s(&nef),
        "-o",
        s(&out),
    ]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let dng = std::fs::read(&out).unwrap();
    assert_eq!(sub_ifd_compressions(&dng), vec![1]);
    assert_eq!(ticoraw::dng::decode_dng_cfa(&dng).unwrap().image, raw.image);

    let pgm = dir.join("out.pgm");
    let o = ticoraw(&["decode", "--format", "pgm", s(&nef), "-o", s(&pgm)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(std::fs::read(&pgm).unwrap().starts_with(b"P5\n64 48\n"));
}

#[test]
fn report_is_privacy_safe() {
    // A path full of personal words: none of it may appear in the report.
    let dir = scratch("report").join("Jane-Q-Photographer-private");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, nef) in [
        ("DSC_SECRETNAME.NEF", synthetic::he_nef(7)),
        ("CLASSIC_SECRETNAME.NEF", synthetic::classic_nef()),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, nef).unwrap();
        let o = ticoraw(&["report", s(&path)]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let json = stdout(&o);
        assert!(json.trim_start().starts_with('{'), "{json}");
        assert!(json.contains("\"model\": \"NIKON Z6_3\""), "{json}");
        assert!(json.contains("\"decode\": {"), "{json}");
        for secret in synthetic::PERSONAL.iter().copied().chain([
            "SECRETNAME",
            "Jane-Q",
            env!("CARGO_TARGET_TMPDIR"),
        ]) {
            assert!(!json.contains(secret), "{secret} leaked into {json}");
        }
        assert!(stderr(&o).contains("bug-report form"), "{}", stderr(&o));
    }

    let o = ticoraw(&["report", s(&dir.join("missing.nef"))]);
    assert_eq!(code(&o), 1);
    // A file that is not a NEF still gets a report.
    let junk = dir.join("junk.nef");
    std::fs::write(&junk, b"not a TIFF").unwrap();
    let o = ticoraw(&["report", s(&junk)]);
    assert_eq!(code(&o), 0);
    assert!(
        stdout(&o).contains("\"inspect_error\": \"container:"),
        "{}",
        stdout(&o)
    );
}

/// The corpus directory, or `None` (with a note) when `TICORAW_CORPUS` is unset.
fn corpus(test: &str) -> Option<PathBuf> {
    match std::env::var_os("TICORAW_CORPUS") {
        Some(d) => Some(PathBuf::from(d)),
        None => {
            eprintln!("skipping {test}: TICORAW_CORPUS is not set");
            None
        }
    }
}

/// A ground-truth pair from the corpus plus, if present, a second TicoRAW NEF.
struct Pair {
    nef: PathBuf,
    dng: PathBuf,
    other: Option<PathBuf>,
}

fn is_ticoraw(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|b| ticoraw::inspect_nef(&b).ok())
        .is_some_and(|i| i.codec == ticoraw::RawCodec::TicoRaw)
}

/// Finds a DNG whose OriginalRawFileName (tag 0xC68B) names a TicoRAW NEF in `corpus`.
fn corpus_pair(test: &str) -> Option<Pair> {
    let corpus = corpus(test)?;
    let files: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .expect("read corpus dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    let ext = |p: &Path, want: &str| p.extension().is_some_and(|e| e.eq_ignore_ascii_case(want));
    let original_name = |dng: &Path| -> Option<String> {
        let bytes = std::fs::read(dng).ok()?;
        let t = ticoraw::tiff::Tiff::parse(&bytes, 0).ok()?;
        let ifd0 = t.ifd(t.first_ifd).ok()?;
        Some(t.ascii(ifd0.get(0xC68B)?))
    };
    let pair = files.iter().filter(|p| ext(p, "dng")).find_map(|dng| {
        let name = original_name(dng)?;
        let nef = files.iter().find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(&name))
        })?;
        is_ticoraw(nef).then(|| (nef.clone(), dng.clone()))
    });
    let Some((nef, dng)) = pair else {
        eprintln!("skipping {test}: no TicoRAW NEF + DNG pair in TICORAW_CORPUS");
        return None;
    };
    let other = files
        .iter()
        .filter(|p| (ext(p, "nef") || ext(p, "nrw")) && **p != nef)
        .find(|p| is_ticoraw(p))
        .cloned();
    Some(Pair { nef, dng, other })
}

#[test]
fn corpus_inspect() {
    let Some(Pair { nef, .. }) = corpus_pair("corpus_inspect") else {
        return;
    };
    let info = ticoraw::inspect_nef(&std::fs::read(&nef).unwrap()).unwrap();
    let o = ticoraw(&["inspect", s(&nef)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("TicoRAW (JPEG XS)"), "{out}");
    assert!(out.contains("High Efficiency"), "{out}");
    assert!(out.contains("profile supported"), "{out}");

    let o = ticoraw(&["inspect", "--json", s(&nef)]);
    assert_eq!(code(&o), 0);
    let json = stdout(&o);
    let code_field = format!(
        "\"code\": {}",
        info.compression.map_or(0, ticoraw::NikonCompression::code)
    );
    let width_field = format!("\"width\": {}", info.width);
    for needle in [
        "\"codec\": \"ticoraw\"",
        code_field.as_str(),
        width_field.as_str(),
        "\"unsupported\": null",
    ] {
        assert!(json.contains(needle), "{needle} in {json}");
    }
}

#[test]
fn corpus_verify_against_lightroom() {
    let Some(Pair { nef, dng, other }) = corpus_pair("corpus_verify_against_lightroom") else {
        return;
    };
    let o = ticoraw(&["verify", s(&nef), s(&dng)]);
    let out = stdout(&o);
    assert_eq!(code(&o), 0, "{out}");
    assert!(out.contains("mismatches 0\n"), "{out}");
    assert!(out.contains("OK: bit-exact"), "{out}");

    // A different frame must fail.
    if let Some(other) = other {
        let o = ticoraw(&["verify", s(&other), s(&dng)]);
        assert_eq!(code(&o), 1);
        assert!(stdout(&o).contains("FAIL:"));
    }
}

#[test]
fn corpus_decode_round_trips() {
    let Some(Pair { nef, .. }) = corpus_pair("corpus_decode_round_trips") else {
        return;
    };
    let dir = scratch("decode");
    let dng = dir.join("out.dng");
    let o = ticoraw(&["decode", s(&nef), "-o", s(&dng)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stdout(&o).contains("decode "));

    // Refuses to overwrite without --force, then overwrites with it.
    let o = ticoraw(&["decode", s(&nef), "-o", s(&dng)]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("--force"));
    let o = ticoraw(&["decode", "--force", s(&nef), "-o", s(&dng)]);
    assert_eq!(code(&o), 0);

    let raw = ticoraw::decode_nef(&std::fs::read(&nef).unwrap()).unwrap();
    let back = ticoraw::dng::decode_dng_cfa(&std::fs::read(&dng).unwrap()).unwrap();
    assert_eq!(back.image, raw.image, "DNG round trip is lossless");
    assert_eq!(back.cfa_pattern, raw.color.cfa_pattern);
    assert_eq!(back.black_level, raw.color.black_level);
    assert_eq!(back.white_level, raw.color.white_level);

    let o = ticoraw(&["verify", s(&nef), s(&dng)]);
    assert_eq!(code(&o), 0, "{}", stdout(&o));

    let pgm = dir.join("out.pgm");
    let o = ticoraw(&["decode", "--format=pgm", s(&nef), "--output", s(&pgm)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let bytes = std::fs::read(&pgm).unwrap();
    let header = format!("P5\n{} {}\n16383\n", raw.image.width, raw.image.height);
    assert!(bytes.starts_with(header.as_bytes()));
    assert_eq!(bytes.len(), header.len() + raw.image.pixels.len() * 2);
    let at = |i: usize| {
        u16::from_be_bytes([bytes[header.len() + 2 * i], bytes[header.len() + 2 * i + 1]])
    };
    assert_eq!(at(0), raw.image.pixels[0]);
    assert_eq!(
        at(raw.image.pixels.len() - 1),
        *raw.image.pixels.last().unwrap()
    );
}

#[cfg(unix)]
#[test]
fn corpus_scan_decodes() {
    let Some(Pair { nef, other, .. }) = corpus_pair("corpus_scan_decodes") else {
        return;
    };
    // Symlinks keep the scan small without copying (or touching) the corpus.
    let dir = scratch("scan_corpus");
    let files: Vec<PathBuf> = std::iter::once(nef).chain(other).collect();
    for (i, f) in files.iter().enumerate() {
        std::os::unix::fs::symlink(f, dir.join(format!("{i}.nef"))).unwrap();
    }
    let n = files.len();
    let o = ticoraw(&["scan", "--decode", s(&dir)]);
    let out = stdout(&o);
    assert_eq!(code(&o), 0, "{out}");
    assert!(out.contains(&format!("{n} NEF files")), "{out}");
    assert!(out.contains(&format!("ticoraw  {n} (")), "{out}");
    assert!(
        out.contains(&format!("decoded {n} of {n} TicoRAW files, 0 failed")),
        "{out}"
    );
}

#[cfg(unix)]
#[test]
fn corpus_convert_and_report() {
    let Some(Pair { nef, other, .. }) = corpus_pair("corpus_convert_and_report") else {
        return;
    };
    // Symlinks keep the input small without copying (or touching) the corpus.
    let dir = scratch("convert_corpus");
    let input = dir.join("in");
    std::fs::create_dir(&input).unwrap();
    let files: Vec<PathBuf> = std::iter::once(nef.clone()).chain(other).collect();
    for f in &files {
        std::os::unix::fs::symlink(f, input.join(f.file_name().unwrap())).unwrap();
    }
    let out = dir.join("out");
    let o = ticoraw(&["convert", "--json", s(&input), "-o", s(&out)]);
    assert_eq!(code(&o), 0, "{}", stdout(&o));
    assert_eq!(events(&o, "converted").len(), files.len(), "{}", stdout(&o));
    for f in &files {
        let dng = out.join(Path::new(f.file_name().unwrap()).with_extension("dng"));
        let v = ticoraw(&["verify", s(f), s(&dng)]);
        assert_eq!(code(&v), 0, "{}", stdout(&v));
        assert!(stdout(&v).contains("OK: bit-exact"));
    }

    // The report names neither the file nor the camera's serial numbers or dates.
    let bytes = std::fs::read(&nef).unwrap();
    let t = Tiff::parse(&bytes, 0).unwrap();
    let ifd0 = t.ifd(t.first_ifd).unwrap();
    let exif = rd::first_uint(&t, &ifd0, 0x8769).unwrap().unwrap();
    let exif = t.ifd(u32::try_from(exif).unwrap()).unwrap();
    let personal: Vec<String> = [0xA431, 0xA435, 0xA430, 0x9003]
        .iter()
        .filter_map(|&tag| exif.get(tag).map(|e| t.ascii(e).trim().to_owned()))
        .chain(nef.file_stem().map(|n| n.to_string_lossy().into_owned()))
        .filter(|v| v.len() >= 4)
        .collect();
    let o = ticoraw(&["report", s(&nef)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let json = stdout(&o);
    assert!(json.contains("\"ok\": true"), "{json}");
    assert!(json.contains("\"marker\": \"PIH\""), "{json}");
    for p in &personal {
        assert!(!json.contains(p.as_str()), "{p} leaked into {json}");
    }
}
