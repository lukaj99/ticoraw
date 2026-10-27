//! Public conformance suite: the CC0 samples in `conformance/manifest.toml`.
//!
//! Every sample must decode to the pinned SHA-256 of its CFA mosaic, which is also the hash
//! of the mosaic in the DNG Adobe's converter wrote from the same file. The colour metadata
//! `decode_nef` reports, and the tags `encode_uncompressed_dng` writes, must equal the
//! values Adobe's DNG carries.
//!
//! Download the samples with `conformance/fetch.sh`, then point `TICORAW_CONFORMANCE_DIR`
//! at the cache directory:
//!
//! ```text
//! conformance/fetch.sh ~/.cache/ticoraw-conformance
//! TICORAW_CONFORMANCE_DIR=~/.cache/ticoraw-conformance \
//!     cargo test --release --test conformance -- --nocapture
//! ```
//!
//! Without the variable only the manifest and the SHA-256 implementation are checked. The
//! manifest is read with a small hand-written parser for the TOML subset it is written in,
//! so this test needs no dependencies.

use std::fmt::Write as _;
use std::path::Path;
use std::time::Instant;

use ticoraw::tiff::{self, Ifd, Tiff};
use ticoraw::{Crop, DecodedRaw, NikonCompression};

const MANIFEST: &str = include_str!("../../../conformance/manifest.toml");
const SCHEMA: i64 = 1;

#[test]
fn manifest_is_well_formed() {
    let m = parse_manifest(MANIFEST).unwrap_or_else(|e| panic!("conformance/manifest.toml: {e}"));
    assert!(!m.oracle.is_empty());
    assert!(!m.samples.is_empty());
    for (i, s) in m.samples.iter().enumerate() {
        assert!(
            m.samples[..i].iter().all(|t| t.id != s.id),
            "duplicate sample id {}",
            s.id
        );
    }
}

#[test]
fn public_samples_match_adobe_verified_output() {
    let Some(dir) = std::env::var_os("TICORAW_CONFORMANCE_DIR") else {
        println!(
            "conformance: TICORAW_CONFORMANCE_DIR is not set; skipping \
             (download the samples with conformance/fetch.sh <dir>)"
        );
        return;
    };
    let dir = Path::new(&dir);
    let m = parse_manifest(MANIFEST).unwrap_or_else(|e| panic!("conformance/manifest.toml: {e}"));
    println!(
        "\nconformance: {} samples, reference {}",
        m.samples.len(),
        m.oracle
    );

    let rows: Vec<Row> = m.samples.iter().map(|s| check(s, dir)).collect();

    let w = rows.iter().map(|r| r.id.len()).max().unwrap_or(0);
    println!(
        "\n{:<w$} {:<12} {:<4} {:<5} {:>10} {:>6}  {:<16}  result",
        "id", "body", "mode", "frame", "photosites", "ms", "cfa sha256"
    );
    for r in &rows {
        println!(
            "{:<w$} {:<12} {:<4} {:<5} {:>10} {:>6}  {:<16}  {}",
            r.id,
            r.body,
            r.mode,
            r.frame,
            r.dims,
            r.decode_ms
                .map_or_else(|| "-".to_owned(), |ms| ms.to_string()),
            r.cfa_prefix,
            if r.failures.is_empty() { "ok" } else { "FAIL" }
        );
    }
    let failed: Vec<&Row> = rows.iter().filter(|r| !r.failures.is_empty()).collect();
    println!(
        "\nconformance: {} of {} samples pass",
        rows.len() - failed.len(),
        rows.len()
    );
    if !failed.is_empty() {
        let lines: Vec<String> = failed
            .iter()
            .flat_map(|r| r.failures.iter().map(move |f| format!("  {}: {f}", r.id)))
            .collect();
        panic!(
            "{} conformance samples failed:\n{}",
            failed.len(),
            lines.join("\n")
        );
    }
}

/// One line of the result table.
struct Row {
    id: String,
    body: String,
    mode: &'static str,
    frame: String,
    dims: String,
    decode_ms: Option<u128>,
    /// First 16 hex digits of the decoded CFA hash, or `-` when nothing was decoded.
    cfa_prefix: String,
    failures: Vec<String>,
}

fn check(s: &Sample, dir: &Path) -> Row {
    let mut row = Row {
        id: s.id.clone(),
        body: s.body.clone(),
        mode: s.mode.label(),
        frame: s.frame.clone(),
        dims: format!("{}x{}", s.width, s.height),
        decode_ms: None,
        cfa_prefix: "-".to_owned(),
        failures: Vec::new(),
    };
    let path = dir.join(format!("{}.nef", s.id));
    let nef = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            row.failures.push(format!(
                "cannot read {}: {e} (run conformance/fetch.sh {})",
                path.display(),
                dir.display()
            ));
            return row;
        }
    };
    let nef_sha = Sha256::digest(&nef);
    if nef.len() as u64 != s.nef_bytes || nef_sha != s.nef_sha256 {
        row.failures.push(format!(
            "{} is not the pinned sample: {} bytes, SHA-256 {} (want {} bytes, {})",
            path.display(),
            nef.len(),
            hex(&nef_sha),
            s.nef_bytes,
            hex(&s.nef_sha256)
        ));
        return row;
    }

    match ticoraw::inspect_nef(&nef) {
        Ok(i) => {
            let want = match s.mode {
                Mode::He => NikonCompression::HighEfficiency,
                Mode::HeStar => NikonCompression::HighEfficiencyStar,
            };
            if i.compression != Some(want) {
                row.failures
                    .push(format!("compression {:?}, want {want:?}", i.compression));
            }
        }
        Err(e) => row.failures.push(format!("inspect_nef: {e}")),
    }

    let t0 = Instant::now();
    let raw = match ticoraw::decode_nef(&nef) {
        Ok(r) => r,
        Err(e) => {
            row.failures.push(format!("decode_nef: {e}"));
            return row;
        }
    };
    row.decode_ms = Some(t0.elapsed().as_millis());
    drop(nef);

    let cfa_sha = cfa_sha256(&raw.image.pixels);
    row.cfa_prefix = hex(&cfa_sha[..8]);
    let f = &mut row.failures;
    expect(f, "model", &raw.model, &s.body);
    expect(
        f,
        "dimensions",
        &(raw.image.width, raw.image.height),
        &(s.width, s.height),
    );
    if cfa_sha != s.cfa_sha256 {
        f.push(format!(
            "CFA SHA-256 {}, want {}",
            hex(&cfa_sha),
            hex(&s.cfa_sha256)
        ));
    }
    expect(f, "CFA pattern", &raw.color.cfa_pattern, &s.cfa_pattern);
    expect(f, "black level", &raw.color.black_level, &s.black_level);
    expect(f, "white level", &raw.color.white_level, &s.white_level);
    expect_ev(
        f,
        "baseline exposure",
        raw.baseline_exposure
            .map(|(n, d)| (i64::from(n), i64::from(d))),
        s.baseline_exposure,
    );
    expect(f, "orientation", &raw.orientation, &s.orientation);
    expect(f, "crop", &raw.crop, &Some(s.crop));
    check_dng(f, &raw, s);
    row
}

/// The DNG metadata snapshot: what `encode_uncompressed_dng` writes must read back as the
/// expected tags, and its mosaic must be the decoded one.
fn check_dng(f: &mut Vec<String>, raw: &DecodedRaw, s: &Sample) {
    let dng = match ticoraw::dng::encode_uncompressed_dng(raw) {
        Ok(d) => d,
        Err(e) => {
            f.push(format!("encode_uncompressed_dng: {e}"));
            return;
        }
    };
    if let Err(e) = check_dng_tags(f, &dng, s) {
        f.push(format!("DNG tags: {e}"));
    }
    match ticoraw::dng::decode_dng_cfa(&dng) {
        Ok(d) => {
            if d.image != raw.image {
                f.push("DNG: CFA image differs from the decoded mosaic".to_owned());
            }
            expect(f, "DNG CFAPattern", &d.cfa_pattern, &s.cfa_pattern);
            expect(f, "DNG BlackLevel", &d.black_level, &s.black_level);
            expect(f, "DNG WhiteLevel", &d.white_level, &s.white_level);
        }
        Err(e) => f.push(format!("decode_dng_cfa on our DNG: {e}")),
    }
}

fn check_dng_tags(f: &mut Vec<String>, dng: &[u8], s: &Sample) -> ticoraw::Result<()> {
    let t = Tiff::parse(dng, 0)?;
    let ifd0 = t.ifd(t.first_ifd)?;
    let ucm = ifd0.get(0xC614).map(|e| t.ascii(e));
    expect(
        f,
        "DNG UniqueCameraModel",
        &ucm.as_deref(),
        &Some(s.unique_camera_model.as_str()),
    );
    let ev = match ifd0.get(0xC62A) {
        Some(e) => t.rationals(e)?.first().copied(),
        None => None,
    };
    expect_ev(f, "DNG BaselineExposure", ev, s.baseline_exposure);
    expect(
        f,
        "DNG Orientation",
        &tiff::first_uint(&t, &ifd0, 0x112)?,
        &Some(u64::from(s.orientation)),
    );
    let raw_ifd = tiff::all_ifds(&t)?.into_iter().find(|ifd| {
        tiff::first_uint(&t, ifd, 0xFE).ok().flatten() == Some(0)
            && tiff::first_uint(&t, ifd, 0x106).ok().flatten() == Some(32803)
    });
    let Some(raw_ifd) = raw_ifd else {
        f.push("DNG: no full-resolution CFA IFD".to_owned());
        return Ok(());
    };
    let c = s.crop;
    let want = |a: usize, b: usize| Some(vec![a as u64, b as u64]);
    expect(
        f,
        "DNG DefaultCropOrigin",
        &crop_values(&t, &raw_ifd, 0xC61F)?,
        &want(c.x, c.y),
    );
    expect(
        f,
        "DNG DefaultCropSize",
        &crop_values(&t, &raw_ifd, 0xC620)?,
        &want(c.width, c.height),
    );
    Ok(())
}

/// A DefaultCrop tag's integer values; DNG allows SHORT, LONG or RATIONAL.
fn crop_values(t: &Tiff<'_>, ifd: &Ifd, tag: u16) -> ticoraw::Result<Option<Vec<u64>>> {
    let Some(e) = ifd.get(tag) else {
        return Ok(None);
    };
    if e.typ == 5 {
        return Ok(Some(
            t.rationals(e)?
                .into_iter()
                .map(|(n, d)| if d == 1 { n as u64 } else { u64::MAX })
                .collect(),
        ));
    }
    t.uints(e).map(Some)
}

fn expect<T: PartialEq + std::fmt::Debug + ?Sized>(
    f: &mut Vec<String>,
    what: &str,
    got: &T,
    want: &T,
) {
    if got != want {
        f.push(format!("{what} {got:?}, want {want:?}"));
    }
}

fn expect_ev(f: &mut Vec<String>, what: &str, got: Option<(i64, i64)>, want: f64) {
    let ok = got.is_some_and(|(n, d)| d != 0 && (n as f64 / d as f64 - want).abs() < 1e-9);
    if !ok {
        f.push(format!("{what} {got:?}, want {want:+.2} EV"));
    }
}

/// SHA-256 of the mosaic as row-major little-endian u16 samples.
fn cfa_sha256(pixels: &[u16]) -> [u8; 32] {
    let mut h = Sha256::new();
    let mut buf = Vec::with_capacity(1 << 17);
    for chunk in pixels.chunks(1 << 16) {
        buf.clear();
        buf.extend(chunk.iter().flat_map(|v| v.to_le_bytes()));
        h.update(&buf);
    }
    h.finish()
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .fold(String::with_capacity(2 * b.len()), |mut s, x| {
            let _ = write!(s, "{x:02x}");
            s
        })
}

// ---------------------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------------------

/// HE or HE*, as MakerNote 0x0051 reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    He,
    HeStar,
}

impl Mode {
    fn label(self) -> &'static str {
        match self {
            Self::He => "HE",
            Self::HeStar => "HE*",
        }
    }
}

/// One `[[sample]]` table: where the file comes from and what it must decode to.
#[derive(Debug)]
struct Sample {
    id: String,
    body: String,
    mode: Mode,
    frame: String,
    nef_sha256: [u8; 32],
    nef_bytes: u64,
    width: usize,
    height: usize,
    cfa_sha256: [u8; 32],
    cfa_pattern: [u8; 4],
    black_level: [u16; 4],
    white_level: u16,
    baseline_exposure: f64,
    orientation: u16,
    crop: Crop,
    unique_camera_model: String,
}

#[derive(Debug)]
struct Manifest {
    oracle: String,
    samples: Vec<Sample>,
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Str(String),
    Int(i64),
    Num(f64),
    Ints(Vec<i64>),
}

/// The `key = value` lines of one table, in file order, with the line each came from.
#[derive(Debug, Default)]
struct Table {
    line: usize,
    entries: Vec<(String, Value, usize)>,
}

impl Table {
    fn take(&mut self, key: &str) -> Result<(Value, usize), String> {
        let i = self
            .entries
            .iter()
            .position(|(k, _, _)| k == key)
            .ok_or_else(|| format!("table at line {}: missing `{key}`", self.line))?;
        let (_, v, line) = self.entries.remove(i);
        Ok((v, line))
    }

    fn str(&mut self, key: &str) -> Result<String, String> {
        match self.take(key)? {
            (Value::Str(s), _) => Ok(s),
            (v, line) => Err(format!("line {line}: `{key}` must be a string, not {v:?}")),
        }
    }

    fn int<T: TryFrom<i64>>(&mut self, key: &str) -> Result<T, String> {
        match self.take(key)? {
            (Value::Int(n), line) => {
                T::try_from(n).map_err(|_| format!("line {line}: `{key}` = {n} is out of range"))
            }
            (v, line) => Err(format!(
                "line {line}: `{key}` must be an integer, not {v:?}"
            )),
        }
    }

    fn num(&mut self, key: &str) -> Result<f64, String> {
        match self.take(key)? {
            (Value::Num(x), _) => Ok(x),
            (Value::Int(n), _) => Ok(n as f64),
            (v, line) => Err(format!("line {line}: `{key}` must be a number, not {v:?}")),
        }
    }

    fn ints<T: TryFrom<i64>, const N: usize>(&mut self, key: &str) -> Result<[T; N], String> {
        let (v, line) = self.take(key)?;
        let Value::Ints(v) = v else {
            return Err(format!(
                "line {line}: `{key}` must be an array of integers, not {v:?}"
            ));
        };
        let len = v.len();
        let items: Vec<T> = v
            .into_iter()
            .map(|n| {
                T::try_from(n).map_err(|_| format!("line {line}: `{key}`: {n} is out of range"))
            })
            .collect::<Result<_, _>>()?;
        items
            .try_into()
            .map_err(|_| format!("line {line}: `{key}` must have {N} items, not {len}"))
    }

    fn sha256(&mut self, key: &str) -> Result<[u8; 32], String> {
        let s = self.str(key)?;
        let bytes = s.as_bytes();
        let ok = bytes.len() == 64 && bytes.iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'));
        if !ok {
            return Err(format!(
                "sample table at line {}: `{key}` must be 64 lowercase hex digits",
                self.line
            ));
        }
        let mut out = [0; 32];
        for (o, pair) in out.iter_mut().zip(bytes.chunks(2)) {
            let digit = |c: u8| {
                if c.is_ascii_digit() {
                    c - b'0'
                } else {
                    c - b'a' + 10
                }
            };
            *o = (digit(pair[0]) << 4) | digit(pair[1]);
        }
        Ok(out)
    }

    fn finish(self) -> Result<(), String> {
        match self.entries.first() {
            Some((k, _, line)) => Err(format!("line {line}: unknown key `{k}`")),
            None => Ok(()),
        }
    }
}

fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let mut top = Table {
        line: 1,
        entries: Vec::new(),
    };
    let mut samples: Vec<Table> = Vec::new();
    for (i, raw_line) in text.lines().enumerate() {
        let n = i + 1;
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line == "[[sample]]" {
            samples.push(Table {
                line: n,
                entries: Vec::new(),
            });
            continue;
        }
        if line.starts_with('[') {
            return Err(format!("line {n}: only [[sample]] tables are allowed"));
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {n}: expected `key = value`"))?;
        let key = key.trim();
        if key.is_empty()
            || !key
                .bytes()
                .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_'))
        {
            return Err(format!("line {n}: bad key `{key}`"));
        }
        let value = parse_value(value.trim()).map_err(|e| format!("line {n}: {e}"))?;
        let table = samples.last_mut().unwrap_or(&mut top);
        if table.entries.iter().any(|(k, _, _)| k == key) {
            return Err(format!("line {n}: duplicate key `{key}`"));
        }
        table.entries.push((key.to_owned(), value, n));
    }

    let schema: i64 = top.int("schema")?;
    if schema != SCHEMA {
        return Err(format!("schema {schema} is not the supported {SCHEMA}"));
    }
    let oracle = top.str("oracle")?;
    top.finish()?;
    let samples = samples
        .into_iter()
        .map(parse_sample)
        .collect::<Result<_, _>>()?;
    Ok(Manifest { oracle, samples })
}

fn parse_sample(mut t: Table) -> Result<Sample, String> {
    let line = t.line;
    let id = t.str("id")?;
    if id.is_empty()
        || !id
            .bytes()
            .all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_'))
    {
        return Err(format!("line {line}: sample id `{id}` must be [a-z0-9_]+"));
    }
    let err = |what: String| format!("sample `{id}` (line {line}): {what}");
    let mode = match t.str("mode")?.as_str() {
        "HE" => Mode::He,
        "HE*" => Mode::HeStar,
        m => return Err(err(format!("mode `{m}` must be HE or HE*"))),
    };
    let url = t.str("url")?;
    if !url.starts_with("https://") || url.contains(char::is_whitespace) {
        return Err(err(format!("url `{url}` must be https:// without spaces")));
    }
    let licence = t.str("licence")?;
    if licence != "CC0-1.0" {
        return Err(err(format!(
            "licence `{licence}`: only CC0-1.0 samples are accepted"
        )));
    }
    let cfa_pattern = match t.str("cfa_pattern")?.as_str() {
        "RGGB" => [0, 1, 1, 2],
        "GRBG" => [1, 0, 2, 1],
        "GBRG" => [1, 2, 0, 1],
        "BGGR" => [2, 1, 1, 0],
        p => return Err(err(format!("cfa_pattern `{p}` is not a Bayer pattern"))),
    };
    let s = Sample {
        body: t.str("body")?,
        mode,
        frame: t.str("frame")?,
        nef_sha256: t.sha256("nef_sha256")?,
        nef_bytes: t.int("nef_bytes")?,
        width: t.int("width")?,
        height: t.int("height")?,
        cfa_sha256: t.sha256("cfa_sha256")?,
        cfa_pattern,
        black_level: t.ints("black_level")?,
        white_level: t.int("white_level")?,
        baseline_exposure: t.num("baseline_exposure")?,
        orientation: t.int("orientation")?,
        crop: {
            let [x, y, width, height] = t.ints::<usize, 4>("crop")?;
            Crop {
                x,
                y,
                width,
                height,
            }
        },
        unique_camera_model: t.str("unique_camera_model")?,
        id: id.clone(),
    };
    t.finish()?;
    let c = s.crop;
    if s.width == 0 || s.height == 0 || c.width == 0 || c.height == 0 {
        return Err(err("empty frame or crop".to_owned()));
    }
    if c.x + c.width > s.width || c.y + c.height > s.height {
        return Err(err("crop does not fit inside the frame".to_owned()));
    }
    if !(1..=8).contains(&s.orientation) {
        return Err(err(format!("orientation {} is not 1..=8", s.orientation)));
    }
    if s.body.is_empty() || s.frame.is_empty() || s.unique_camera_model.is_empty() {
        return Err(err(
            "body, frame and unique_camera_model must not be empty".to_owned()
        ));
    }
    Ok(s)
}

/// `line` up to a `#` outside a string.
fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn parse_value(v: &str) -> Result<Value, String> {
    if let Some(rest) = v.strip_prefix('"') {
        let s = rest
            .strip_suffix('"')
            .ok_or_else(|| format!("unterminated string {v}"))?;
        if s.contains(['"', '\\']) || s.chars().any(char::is_control) {
            return Err(format!("string {v} uses escapes or control characters"));
        }
        return Ok(Value::Str(s.to_owned()));
    }
    if let Some(rest) = v.strip_prefix('[') {
        let body = rest
            .strip_suffix(']')
            .ok_or_else(|| format!("unterminated array {v}"))?;
        return body
            .split(',')
            .map(|item| parse_int(item.trim()))
            .collect::<Result<_, _>>()
            .map(Value::Ints);
    }
    if v.contains('.') {
        let digits = v.strip_prefix(['-', '+']).unwrap_or(v);
        let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
        let all_digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        if !all_digits(int) || !all_digits(frac) {
            return Err(format!("bad decimal `{v}`"));
        }
        return v.parse().map(Value::Num).map_err(|e| format!("`{v}`: {e}"));
    }
    parse_int(v).map(Value::Int)
}

fn parse_int(v: &str) -> Result<i64, String> {
    let digits = v.strip_prefix('-').unwrap_or(v);
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return Err(format!("bad integer `{v}`"));
    }
    v.parse().map_err(|e| format!("`{v}`: {e}"))
}

#[test]
fn manifest_parser_rejects_malformed_input() {
    let head = "schema = 1\noracle = \"x\"\n";
    let sample = MANIFEST
        .split("\n[[sample]]\n")
        .nth(1)
        .expect("manifest has a sample");
    let ok = format!("{head}[[sample]]\n{sample}");
    assert!(parse_manifest(&ok).is_ok(), "{:?}", parse_manifest(&ok));
    for (bad, why) in [
        (ok.replace("schema = 1", "schema = 2"), "schema"),
        (
            ok.replace("licence = \"CC0-1.0\"", "licence = \"CC-BY-4.0\""),
            "licence",
        ),
        (ok.replace("mode = \"", "mode = \"X"), "mode"),
        (format!("{ok}extra = 1\n"), "unknown key"),
        (format!("{ok}id = \"again\"\n"), "duplicate key"),
        (
            ok.replace("[1008, 1008, 1008, 1008]", "[1008, 1008]"),
            "items",
        ),
        (ok.replace("url = \"https://", "url = \"http://"), "https"),
        (ok.replace("crop = [", "crop = [99999, "), "items"),
        (ok.replace("crop = [12,", "crop = [100,"), "does not fit"),
        (format!("{head}[sample]\n"), "only [[sample]]"),
        (format!("{head}[[sample]]\nid = \"a\\\"b\"\n"), "escapes"),
        (format!("{head}[[sample]]\nid = 0x10\n"), "bad integer"),
    ] {
        let e = parse_manifest(&bad).expect_err(why);
        assert!(e.contains(why), "{why}: got {e}");
    }
}

// ---------------------------------------------------------------------------------------
// SHA-256 (FIPS 180-4)
// ---------------------------------------------------------------------------------------

/// A streaming SHA-256, so the suite needs no hashing dependency.
struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    filled: usize,
    total: u64,
}

const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09_e667,
                0xbb67_ae85,
                0x3c6e_f372,
                0xa54f_f53a,
                0x510e_527f,
                0x9b05_688c,
                0x1f83_d9ab,
                0x5be0_cd19,
            ],
            block: [0; 64],
            filled: 0,
            total: 0,
        }
    }

    fn digest(data: &[u8]) -> [u8; 32] {
        let mut h = Self::new();
        h.update(data);
        h.finish()
    }

    fn update(&mut self, mut data: &[u8]) {
        self.total += data.len() as u64;
        if self.filled > 0 {
            let n = data.len().min(64 - self.filled);
            self.block[self.filled..self.filled + n].copy_from_slice(&data[..n]);
            self.filled += n;
            data = &data[n..];
            if self.filled < 64 {
                return;
            }
            let block = self.block;
            self.compress(&block);
            self.filled = 0;
        }
        let mut blocks = data.chunks_exact(64);
        for b in &mut blocks {
            self.compress(b.try_into().expect("64-byte chunk"));
        }
        let rest = blocks.remainder();
        self.block[..rest.len()].copy_from_slice(rest);
        self.filled = rest.len();
    }

    fn finish(mut self) -> [u8; 32] {
        let bits = self.total.wrapping_mul(8);
        let mut pad = vec![0x80u8];
        pad.resize((119 - self.filled) % 64 + 1, 0);
        pad.extend_from_slice(&bits.to_be_bytes());
        let total = self.total;
        self.update(&pad);
        debug_assert_eq!(self.filled, 0);
        self.total = total;
        let mut out = [0; 32];
        for (o, s) in out.chunks_exact_mut(4).zip(self.state) {
            o.copy_from_slice(&s.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
}

#[test]
fn sha256_matches_fips_180_vectors() {
    let cases: [(&[u8], &str); 3] = [
        (
            b"",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
    ];
    for (msg, want) in cases {
        assert_eq!(hex(&Sha256::digest(msg)), want);
    }
    // One million 'a', fed in uneven pieces to exercise the block buffer.
    let mut h = Sha256::new();
    let piece = [b'a'; 997];
    let mut left = 1_000_000;
    while left > 0 {
        let n = left.min(piece.len());
        h.update(&piece[..n]);
        left -= n;
    }
    assert_eq!(
        hex(&h.finish()),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
    // Every message length around the 55/56/64-byte padding boundaries, one-shot vs streamed.
    let data: Vec<u8> = (0..200u8).collect();
    for len in 0..data.len() {
        let mut s = Sha256::new();
        for b in data[..len].chunks(7) {
            s.update(b);
        }
        assert_eq!(s.finish(), Sha256::digest(&data[..len]), "length {len}");
    }
}
