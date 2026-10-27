//! End-to-end tests of the `ticoraw` binary.
//!
//! Tests that need real NEFs run only when `TICORAW_CORPUS` names a directory holding at
//! least one HE/HE* NEF together with an Adobe DNG converted from it (matched through the
//! DNG's OriginalRawFileName); they only read from it. Everything written goes under Cargo's
//! per-target temp directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
        for cmd in ["inspect", "decode", "verify", "scan"] {
            assert!(out.contains(cmd), "{args:?} lists {cmd}");
        }
    }
}

#[test]
fn per_command_help() {
    for cmd in ["inspect", "decode", "verify", "scan"] {
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
