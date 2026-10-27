//! `ticoraw decode`: one NEF -> complete DNG (via `ticoraw-dng`) or 16-bit PGM.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use ticoraw::CfaImage;
use ticoraw_dng::{CarriedMetadata, Compression, Preview};

use crate::args::{Arg, Args};
use crate::{CliError, ms, read_input, unexpected};

pub const HELP: &str = "\
Decode one NEF to a complete DNG (or a 16-bit PGM)

Usage: ticoraw decode [options] <file.nef>

Decodes the High Efficiency (TicoRAW) raw strip and writes a complete DNG: the CFA mosaic as
lossless JPEG tiles, the camera's EXIF, GPS, XMP and MakerNote, a preview and a thumbnail,
the colour matrix, as-shot white balance, orientation and default crop, and the input's
file name as OriginalRawFileName. A PGM is the bare mosaic as big-endian 16-bit samples
(maxval 16383 for 14-bit data, else 65535). Output is written atomically and never
overwrites the input. To convert many files, use `ticoraw convert`.

Options:
  -o, --output <path>    Output file [default: the input path with a .dng/.pgm extension]
  --format <dng|pgm>     Output format [default: dng]
  --uncompressed         Store the DNG's mosaic uncompressed (about three times larger)
  --no-preview           Leave out the DNG's embedded preview (the thumbnail stays)
  -f, --force            Overwrite an existing output file
  -h, --help             Show this help

Exit status: 0 on success, 1 on failure, 2 on usage errors.
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Dng,
    Pgm,
}

impl Format {
    fn extension(self) -> &'static str {
        match self {
            Self::Dng => "dng",
            Self::Pgm => "pgm",
        }
    }
}

#[derive(Debug)]
pub struct Options {
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub format: Format,
    pub compression: Compression,
    pub preview: Preview,
    pub force: bool,
}

/// Returns `None` when help was requested.
pub fn parse(args: &mut Args) -> Result<Option<Options>, String> {
    let mut input = None;
    let mut output = None;
    let mut format = Format::Dng;
    let mut compression = Compression::LosslessJpeg;
    let mut preview = Preview::Auto;
    let mut force = false;
    while let Some(arg) = args.next_arg()? {
        match arg {
            Arg::Opt(s) => match s.as_str() {
                "-h" | "--help" => return Ok(None),
                "-o" | "--output" => output = Some(PathBuf::from(args.value(&s)?)),
                "--format" => {
                    format = match args.value(&s)?.to_str() {
                        Some("dng") => Format::Dng,
                        Some("pgm") => Format::Pgm,
                        other => {
                            return Err(format!(
                                "invalid --format {:?} (expected dng or pgm)",
                                other.unwrap_or("<non-UTF-8>")
                            ));
                        }
                    }
                }
                "--uncompressed" => compression = Compression::Uncompressed,
                "--no-preview" => preview = Preview::None,
                "-f" | "--force" => force = true,
                _ => return Err(format!("unknown option {s}")),
            },
            Arg::Pos(p) if input.is_none() => input = Some(PathBuf::from(p)),
            Arg::Pos(p) => return Err(unexpected(&p)),
        }
    }
    let input = input.ok_or("no input file")?;
    let dng_only = compression != Compression::LosslessJpeg || preview != Preview::Auto;
    if format == Format::Pgm && dng_only {
        return Err("--uncompressed and --no-preview apply to DNG output only".into());
    }
    Ok(Some(Options {
        input,
        output,
        format,
        compression,
        preview,
        force,
    }))
}

pub fn run(o: &Options) -> Result<ExitCode, CliError> {
    let output = o
        .output
        .clone()
        .unwrap_or_else(|| o.input.with_extension(o.format.extension()));
    if same_file(&o.input, &output) {
        return Err(CliError::Failure(format!(
            "output {} is the input file",
            output.display()
        )));
    }

    let t0 = Instant::now();
    let nef = read_input(&o.input)?;
    let t_read = t0.elapsed();

    let t1 = Instant::now();
    let raw = ticoraw::decode_nef(&nef)
        .map_err(|e| CliError::Failure(format!("{}: {e}", o.input.display())))?;
    let t_decode = t1.elapsed();

    if o.format == Format::Dng && raw.color.color_matrix.is_none() {
        eprintln!(
            "warning: no colour profile for {:?}; the DNG gets an identity colour matrix and \
             white level {}, so its colours will be wrong in raw processors",
            raw.model, raw.color.white_level
        );
    }

    let t2 = Instant::now();
    let bytes = match o.format {
        Format::Dng => {
            let fail =
                |e: ticoraw_dng::Error| CliError::Failure(format!("{}: {e}", o.input.display()));
            let carried = CarriedMetadata::from_nef(&nef).map_err(fail)?;
            let opts = ticoraw_dng::Options {
                compression: o.compression,
                preview: o.preview,
                original_raw_file_name: o
                    .input
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned()),
                ..ticoraw_dng::Options::default()
            };
            ticoraw_dng::write(&raw, &carried, &opts).map_err(fail)?
        }
        Format::Pgm => encode_pgm(&raw.image),
    };
    let t_encode = t2.elapsed();

    let t3 = Instant::now();
    write_output(&output, &bytes, o.force)?;
    let t_write = t3.elapsed();

    let img = &raw.image;
    let mp = (img.width * img.height) as f64 / 1e6;
    let c = &raw.color;
    let mut out = io::stdout().lock();
    writeln!(out, "{} -> {}", o.input.display(), output.display())?;
    writeln!(
        out,
        "  {} {}, {} x {} CFA {}, black {:?}, white {}",
        raw.make,
        raw.model,
        img.width,
        img.height,
        c.cfa_pattern
            .map(crate::cfa_color)
            .iter()
            .collect::<String>(),
        c.black_level,
        c.white_level
    )?;
    writeln!(
        out,
        "  read {}, decode {} ({:.0} MP/s), encode {}, write {} ({:.1} MB); total {}",
        ms(t_read),
        ms(t_decode),
        mp / t_decode.as_secs_f64().max(1e-9),
        ms(t_encode),
        ms(t_write),
        bytes.len() as f64 / 1e6,
        ms(t0.elapsed())
    )?;
    Ok(ExitCode::SUCCESS)
}

/// Whether `a` and `b` name the same existing file: same canonical path, or (on Unix) the same
/// inode, which also catches hard links.
pub fn same_file(a: &Path, b: &Path) -> bool {
    if let (Ok(x), Ok(y)) = (a.canonicalize(), b.canonicalize()) {
        if x == y {
            return true;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) {
            return x.dev() == y.dev() && x.ino() == y.ino();
        }
    }
    false
}

/// Writes `bytes` to `path` atomically: everything goes to a temporary file in the same
/// directory first, which then replaces `path` (`force`) or is linked in only if `path`
/// does not exist yet. A failed write never leaves a partial file or destroys an old one.
pub fn write_output(path: &Path, bytes: &[u8], force: bool) -> Result<(), CliError> {
    let exists = || {
        CliError::Failure(format!(
            "{} already exists (use --force to overwrite)",
            path.display()
        ))
    };
    if !force && path.symlink_metadata().is_ok() {
        return Err(exists());
    }
    let name = path
        .file_name()
        .map_or_else(|| "output".into(), |n| n.to_string_lossy());
    let tmp = path.with_file_name(format!(".{name}.ticoraw-{}.tmp", std::process::id()));
    let result = (|| -> Result<(), CliError> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| CliError::Failure(format!("creating {}: {e}", tmp.display())))?;
        f.write_all(bytes)
            .and_then(|()| f.sync_all())
            .map_err(|e| CliError::Failure(format!("writing {}: {e}", path.display())))?;
        drop(f);
        let placed = if force {
            std::fs::rename(&tmp, path)
        } else {
            // A hard link fails if `path` appeared meanwhile; fall back to a checked
            // rename on filesystems without hard links.
            std::fs::hard_link(&tmp, path).or_else(|e| match e.kind() {
                io::ErrorKind::AlreadyExists => Err(e),
                _ if path.symlink_metadata().is_ok() => {
                    Err(io::Error::from(io::ErrorKind::AlreadyExists))
                }
                _ => std::fs::rename(&tmp, path),
            })
        };
        placed.map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                exists()
            } else {
                CliError::Failure(format!("writing {}: {e}", path.display()))
            }
        })
    })();
    // Gone already after a rename; otherwise clean up the temporary file.
    let _ = std::fs::remove_file(&tmp);
    result
}

/// Binary (P5) PGM with big-endian 16-bit samples.
pub fn encode_pgm(img: &CfaImage) -> Vec<u8> {
    let max = img.pixels.iter().copied().max().unwrap_or(0);
    let maxval = if max <= 16383 { 16383 } else { 65535 };
    let header = format!("P5\n{} {}\n{maxval}\n", img.width, img.height);
    let mut out = Vec::with_capacity(header.len() + img.pixels.len() * 2);
    out.extend_from_slice(header.as_bytes());
    out.extend(img.pixels.iter().flat_map(|p| p.to_be_bytes()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn same_file_sees_through_hard_links_and_atomic_write_keeps_old_on_refusal() {
        let dir = std::env::temp_dir().join(format!("ticoraw-decode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = (dir.join("a.nef"), dir.join("b.nef"), dir.join("c.dng"));
        for p in [&a, &b, &c] {
            let _ = std::fs::remove_file(p);
        }
        std::fs::write(&a, b"raw").unwrap();
        std::fs::hard_link(&a, &b).unwrap();
        assert!(same_file(&a, &b));
        assert!(!same_file(&a, &c));

        write_output(&c, b"one", false).unwrap();
        assert!(write_output(&c, b"two", false).is_err());
        assert_eq!(std::fs::read(&c).unwrap(), b"one");
        write_output(&c, b"three", true).unwrap();
        assert_eq!(std::fs::read(&c).unwrap(), b"three");
        let leftovers = std::fs::read_dir(&dir).unwrap().filter_map(Result::ok);
        assert!(
            leftovers
                .into_iter()
                .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pgm_layout() {
        let img = CfaImage {
            width: 2,
            height: 1,
            pixels: vec![0x0102, 16383],
        };
        assert_eq!(encode_pgm(&img), b"P5\n2 1\n16383\n\x01\x02\x3f\xff");
        let img = CfaImage {
            width: 1,
            height: 1,
            pixels: vec![65535],
        };
        assert!(encode_pgm(&img).starts_with(b"P5\n1 1\n65535\n"));
    }
}
