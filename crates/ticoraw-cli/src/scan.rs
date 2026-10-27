//! `ticoraw scan`: classify (and optionally decode) every NEF in a directory.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use ticoraw::{NikonCompression, RawCodec};

use crate::args::{Arg, Args};
use crate::inspect::compression_short;
use crate::{CliError, ms, printable, shown, unexpected};

pub const HELP: &str = "\
Classify every NEF in a directory, optionally decoding them all

Usage: ticoraw scan [options] <dir>

Lists each .nef/.nrw file (any case) as TicoRAW (HE/HE*), classic Nikon, or error, then prints counts.
Files are only read, never modified.

Options:
  --decode         Decode every TicoRAW file; report failures and throughput
  -r, --recursive  Descend into subdirectories (symbolic links to directories are skipped)
  -h, --help       Show this help

Exit status: 0 when every file was classified (and decoded), 1 otherwise, 2 on usage errors.
";

#[derive(Debug)]
pub struct Options {
    pub dir: PathBuf,
    pub decode: bool,
    pub recursive: bool,
}

/// Returns `None` when help was requested.
pub fn parse(args: &mut Args) -> Result<Option<Options>, String> {
    let mut dir = None;
    let (mut decode, mut recursive) = (false, false);
    while let Some(arg) = args.next_arg()? {
        match arg {
            Arg::Opt(s) => match s.as_str() {
                "-h" | "--help" => return Ok(None),
                "--decode" => decode = true,
                "-r" | "--recursive" => recursive = true,
                _ => return Err(format!("unknown option {s}")),
            },
            Arg::Pos(p) if dir.is_none() => dir = Some(PathBuf::from(p)),
            Arg::Pos(p) => return Err(unexpected(&p)),
        }
    }
    Ok(Some(Options {
        dir: dir.ok_or("no directory given")?,
        decode,
        recursive,
    }))
}

/// What a file turned out to be. Its strings come from the file, made [`printable`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Class {
    TicoRaw {
        model: String,
        compression: Option<NikonCompression>,
        unsupported: Option<String>,
    },
    Classic {
        model: String,
        compression: Option<NikonCompression>,
    },
    Error(String),
}

/// Bytes first read per file when only classifying; enough for the TIFF structures and
/// MakerNote. When the raw strip starts later (embedded previews can push it past 4 MiB),
/// the read grows to cover the strip's main header.
const PREFIX_LEN: usize = 1 << 20;

/// Bytes of the raw strip needed to parse the codestream's main header.
const STRIP_HEAD_LEN: usize = 64 << 10;

/// File contents for scanning: the whole file when decoding, otherwise the shortest
/// prefix (plus the file length) that classifies it. Only regular files are read, and
/// never past their stat'd length.
pub fn read_for_scan(path: &Path, whole: bool) -> io::Result<ScanInput> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let file_len = usize::try_from(meta.len()).unwrap_or(usize::MAX);
    let read = |n: usize| -> io::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(n.min(file_len));
        std::fs::File::open(path)?
            .take(n as u64)
            .read_to_end(&mut buf)?;
        Ok(buf)
    };
    if whole {
        return Ok(ScanInput {
            bytes: read(file_len)?,
            file_len,
        });
    }
    let mut want = PREFIX_LEN;
    loop {
        let bytes = read(want)?;
        let next = match ticoraw::inspect_nef_prefix(&bytes, file_len) {
            Err(ticoraw::TicoError::Truncated { offset, .. }) if bytes.len() < file_len => offset
                .saturating_add(STRIP_HEAD_LEN)
                .max(2 * want)
                .min(file_len),
            _ => return Ok(ScanInput { bytes, file_len }),
        };
        if next <= want {
            return Ok(ScanInput { bytes, file_len });
        }
        want = next;
    }
}

pub struct ScanInput {
    pub bytes: Vec<u8>,
    pub file_len: usize,
}

fn classify(input: &ScanInput) -> Class {
    match ticoraw::inspect_nef_prefix(&input.bytes, input.file_len) {
        Err(e) => Class::Error(printable(&e.to_string()).into_owned()),
        Ok(i) => match i.codec {
            RawCodec::TicoRaw => Class::TicoRaw {
                model: printable(&i.model).into_owned(),
                compression: i.compression,
                unsupported: i
                    .codestream
                    .and_then(|c| c.unsupported)
                    .map(|why| printable(&why).into_owned()),
            },
            RawCodec::Classic => Class::Classic {
                model: printable(&i.model).into_owned(),
                compression: i.compression,
            },
        },
    }
}

/// Running totals for the summary.
#[derive(Debug, Default)]
struct Tally {
    files: usize,
    ticoraw: BTreeMap<String, usize>,
    classic: BTreeMap<String, usize>,
    models: BTreeMap<String, usize>,
    errors: Vec<(PathBuf, String)>,
    decoded: usize,
    decode_failures: Vec<(PathBuf, String)>,
    decode_time: Duration,
    megapixels: f64,
}

pub fn run(o: &Options) -> Result<ExitCode, CliError> {
    let started = Instant::now();
    let mut files = Vec::new();
    let mut tally = Tally::default();
    std::fs::read_dir(&o.dir)
        .map_err(|e| CliError::Failure(format!("reading directory {}: {e}", o.dir.display())))?;
    collect(&o.dir, o.recursive, &mut files, &mut tally.errors);
    files.sort();

    let mut out = io::stdout().lock();
    for path in &files {
        let shown = shown(path.strip_prefix(&o.dir).unwrap_or(path));
        tally.files += 1;
        let bytes = match read_for_scan(path, o.decode) {
            Ok(b) => b,
            Err(e) => {
                let msg = printable(&format!("reading: {e}")).into_owned();
                writeln!(out, "{:<8} {:<8} {:<12} {shown}: {msg}", "error", "-", "-")?;
                tally.errors.push((path.clone(), msg));
                continue;
            }
        };
        let class = classify(&bytes);
        let (kind, comp, model) = match &class {
            Class::TicoRaw {
                model, compression, ..
            } => ("ticoraw", compression_short(*compression), model.as_str()),
            Class::Classic { model, compression } => {
                ("classic", compression_short(*compression), model.as_str())
            }
            Class::Error(_) => ("error", "-".into(), "-"),
        };
        write!(out, "{kind:<8} {comp:<8} {model:<12} {shown}")?;
        match class {
            Class::Error(e) => {
                writeln!(out, ": {e}")?;
                tally.errors.push((path.clone(), e));
                continue;
            }
            Class::Classic { model, .. } => {
                *tally.classic.entry(comp).or_default() += 1;
                *tally.models.entry(model).or_default() += 1;
            }
            Class::TicoRaw {
                model, unsupported, ..
            } => {
                *tally.ticoraw.entry(comp).or_default() += 1;
                *tally.models.entry(model).or_default() += 1;
                if let Some(why) = unsupported {
                    write!(out, " [unsupported: {why}]")?;
                }
                if o.decode {
                    let t = Instant::now();
                    match ticoraw::decode_nef(&bytes.bytes) {
                        Ok(raw) => {
                            let el = t.elapsed();
                            tally.decoded += 1;
                            tally.decode_time += el;
                            tally.megapixels += (raw.image.width * raw.image.height) as f64 / 1e6;
                            write!(out, "  decoded in {}", ms(el))?;
                        }
                        Err(e) => {
                            let e = printable(&e.to_string()).into_owned();
                            write!(out, "  DECODE FAILED: {e}")?;
                            tally.decode_failures.push((path.clone(), e));
                        }
                    }
                }
            }
        }
        writeln!(out)?;
    }
    write_summary(&mut out, &tally, o.decode, started.elapsed())?;
    let ok = tally.errors.is_empty() && tally.decode_failures.is_empty();
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn write_summary(out: &mut impl Write, t: &Tally, decode: bool, wall: Duration) -> io::Result<()> {
    let list = |m: &BTreeMap<String, usize>| {
        let parts: Vec<String> = m.iter().map(|(k, v)| format!("{k} {v}")).collect();
        parts.join(", ")
    };
    let breakdown = |m: &BTreeMap<String, usize>| {
        if m.is_empty() {
            String::new()
        } else {
            format!(" ({})", list(m))
        }
    };
    let count = |m: &BTreeMap<String, usize>| m.values().sum::<usize>();
    writeln!(out)?;
    writeln!(out, "{} NEF files", t.files)?;
    writeln!(
        out,
        "  ticoraw  {}{}",
        count(&t.ticoraw),
        breakdown(&t.ticoraw)
    )?;
    writeln!(
        out,
        "  classic  {}{}",
        count(&t.classic),
        breakdown(&t.classic)
    )?;
    writeln!(out, "  error    {}", t.errors.len())?;
    if !t.models.is_empty() {
        writeln!(out, "  models   {}", list(&t.models))?;
    }
    if decode {
        let secs = t.decode_time.as_secs_f64().max(1e-9);
        writeln!(
            out,
            "decoded {} of {} TicoRAW files, {} failed; {:.1} MP in {:.2} s decode time \
             ({:.0} MP/s, {:.1} files/s); wall {:.2} s including I/O",
            t.decoded,
            count(&t.ticoraw),
            t.decode_failures.len(),
            t.megapixels,
            t.decode_time.as_secs_f64(),
            t.megapixels / secs,
            t.decoded as f64 / secs,
            wall.as_secs_f64()
        )?;
    } else {
        writeln!(out, "wall {:.2} s", wall.as_secs_f64())?;
    }
    for (label, list) in [("error", &t.errors), ("decode failure", &t.decode_failures)] {
        for (p, e) in list {
            writeln!(out, "{label}: {}: {e}", shown(p))?;
        }
    }
    Ok(())
}

/// Gathers `.nef`/`.nrw` files (any case) under `dir`; unreadable subdirectories become errors.
pub fn collect(
    dir: &Path,
    recursive: bool,
    files: &mut Vec<PathBuf>,
    errors: &mut Vec<(PathBuf, String)>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            errors.push((dir.to_path_buf(), format!("reading directory: {e}")));
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                errors.push((dir.to_path_buf(), format!("reading directory: {e}")));
                continue;
            }
        };
        let path = entry.path();
        // `DirEntry::file_type` does not follow symlinks, so linked directories are skipped.
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if is_dir {
            if recursive {
                collect(&path, recursive, files, errors);
            }
        } else if is_nef(&path) {
            files.push(path);
        }
    }
}

/// `.nef`, or `.nrw` (used by some Nikon bodies for the same container), in any case.
pub fn is_nef(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("nef") || e.eq_ignore_ascii_case("nrw"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nef_extension_any_case() {
        assert!(is_nef(Path::new("a/DSC_0001.NEF")));
        assert!(is_nef(Path::new("b.nef")));
        assert!(is_nef(Path::new("c.NEF.nef")));
        assert!(is_nef(Path::new("zr.NRW")));
        assert!(!is_nef(Path::new("d.dng")));
        assert!(!is_nef(Path::new("nef")));
    }

    #[test]
    fn garbage_is_an_error() {
        let input = ScanInput {
            bytes: b"not a tiff".to_vec(),
            file_len: 10,
        };
        assert!(matches!(classify(&input), Class::Error(_)));
    }

    #[test]
    fn file_strings_are_printable() {
        let mut nef = crate::synthetic::he_nef(1);
        let model = b"NIKON Z6_3";
        let at = nef.windows(model.len()).position(|w| w == model).unwrap();
        nef[at..at + 4].copy_from_slice(b"\x1b[2J");
        let input = ScanInput {
            file_len: nef.len(),
            bytes: nef,
        };
        match classify(&input) {
            Class::TicoRaw { model, .. } => assert_eq!(model, r"\u{1b}[2JN Z6_3"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn summary_counts() {
        let mut t = Tally {
            files: 3,
            ..Tally::default()
        };
        t.ticoraw.insert("HE*".into(), 2);
        t.classic.insert("lossless".into(), 1);
        let mut out = Vec::new();
        write_summary(&mut out, &t, false, Duration::ZERO).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("3 NEF files"));
        assert!(s.contains("ticoraw  2 (HE* 2)"));
        assert!(s.contains("classic  1 (lossless 1)"));
        assert!(s.contains("error    0"));
    }
}
