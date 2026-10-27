//! `ticoraw convert`: many NEFs -> complete DNGs, a bounded number at a time.
//!
//! The main thread plans the work (expands directories, picks output paths, finds sidecars),
//! then worker threads each take the next file, read it, convert it and write the DNG. Only
//! the main thread prints, from the events the workers send.

use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal, Write};
use std::num::NonZeroUsize;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ticoraw::{NikonCompression, RawCodec};
use ticoraw_dng::{CarriedMetadata, Compression, Preview};

use crate::CliError;
use crate::args::{Arg, Args};
use crate::decode::{same_file, write_output};
use crate::json::Json;
use crate::scan::{collect, read_for_scan};

pub const HELP: &str = "\
Convert High Efficiency (HE/HE*) NEFs to complete DNGs

Usage: ticoraw convert [options] <file|dir>...

Converts every HE/HE* NEF given, or found as a .nef/.nrw file (any case) in a given
directory, to a DNG that keeps the camera's EXIF, GPS, XMP and MakerNote, with the mosaic
in lossless JPEG tiles, a preview, and the NEF's file name as OriginalRawFileName. Classic
(not High Efficiency) NEFs are skipped: editors already support them. Inputs are only read.

Each DNG is written next to its NEF, or with --output under that directory; files found in a
given directory keep their path relative to it there. DNGs are written atomically (a
temporary file, then a rename) and an existing file is never replaced without --force.

Options:
  -o, --output <dir>  Write the DNGs under <dir>, which is created if needed
  -r, --recursive     Descend into subdirectories (symbolic links to directories are skipped)
  --skip-existing     Skip a NEF whose DNG already exists
  -f, --force         Replace existing DNGs
  --uncompressed      Store the mosaic uncompressed (about three times larger)
  --no-preview        Leave out the full-size preview (the thumbnail stays)
  --with-sidecar      Use the sidecar <name>.xmp (any case) next to each NEF, as written by
                      Lightroom, Bridge or Capture One, as the DNG's XMP instead of the
                      NEF's own packet
  -j, --jobs <n>      Files converted at once [default: the number of cores, at most one
                      per 2 GB of memory]
  --json              Print JSON Lines (one event per file, then a summary) instead of text
  -h, --help          Show this help

On a terminal, a progress line is shown on stderr.

Exit status: 0 when no file failed (skipped files are not failures), 1 when any file
failed, 2 on usage errors.
";

/// Physical memory reserved per file in flight: a 45 MP frame needs about 1 GB while it is
/// converted, and half of the memory is left to everything else.
const MEMORY_PER_JOB: u64 = 2 << 30;

/// Largest sidecar accepted; real ones are a few kilobytes.
const MAX_SIDECAR: u64 = 16 << 20;

/// What to do when a DNG is already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Existing {
    Fail,
    Skip,
    Replace,
}

#[derive(Debug)]
pub struct Options {
    pub inputs: Vec<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub recursive: bool,
    pub existing: Existing,
    pub compression: Compression,
    pub preview: Preview,
    pub with_sidecar: bool,
    pub jobs: Option<NonZeroUsize>,
    pub json: bool,
}

/// Returns `None` when help was requested.
pub fn parse(args: &mut Args) -> Result<Option<Options>, String> {
    let mut o = Options {
        inputs: Vec::new(),
        out_dir: None,
        recursive: false,
        existing: Existing::Fail,
        compression: Compression::LosslessJpeg,
        preview: Preview::Auto,
        with_sidecar: false,
        jobs: None,
        json: false,
    };
    let (mut skip, mut force) = (false, false);
    while let Some(arg) = args.next_arg()? {
        match arg {
            Arg::Opt(s) => match s.as_str() {
                "-h" | "--help" => return Ok(None),
                "-o" | "--output" => o.out_dir = Some(PathBuf::from(args.value(&s)?)),
                "-r" | "--recursive" => o.recursive = true,
                "--skip-existing" => skip = true,
                "-f" | "--force" => force = true,
                "--uncompressed" => o.compression = Compression::Uncompressed,
                "--no-preview" => o.preview = Preview::None,
                "--with-sidecar" => o.with_sidecar = true,
                "-j" | "--jobs" => {
                    let v = args.value(&s)?;
                    let n = v.to_str().and_then(|t| t.parse::<NonZeroUsize>().ok());
                    o.jobs = Some(n.ok_or_else(|| {
                        format!(
                            "invalid {s} {:?} (expected a positive integer)",
                            v.to_string_lossy()
                        )
                    })?);
                }
                "--json" => o.json = true,
                _ => return Err(format!("unknown option {s}")),
            },
            Arg::Pos(p) => o.inputs.push(p.into()),
        }
    }
    o.existing = match (skip, force) {
        (true, true) => return Err("--skip-existing and --force exclude each other".into()),
        (true, false) => Existing::Skip,
        (false, true) => Existing::Replace,
        (false, false) => Existing::Fail,
    };
    if o.inputs.is_empty() {
        return Err("no input files or directories".into());
    }
    Ok(Some(o))
}

/// One NEF to convert.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Job {
    input: PathBuf,
    output: PathBuf,
    /// The sidecar to use as the DNG's XMP (only with `--with-sidecar`).
    sidecar: Option<PathBuf>,
}

/// The work, in input order, plus inputs that failed before any work started.
#[derive(Debug, Default)]
struct Plan {
    jobs: Vec<Job>,
    failures: Vec<(PathBuf, String)>,
}

/// Expands directories and assigns each NEF its output. A file reached twice is converted
/// once; two NEFs whose DNGs would have the same path (compared ignoring case, as on
/// macOS and Windows) fail after the first.
fn plan(o: &Options) -> Plan {
    let mut p = Plan::default();
    let mut found = Vec::new();
    for arg in &o.inputs {
        match std::fs::metadata(arg) {
            Err(e) => p.failures.push((arg.clone(), format!("reading: {e}"))),
            Ok(m) if m.is_dir() => {
                let mut files = Vec::new();
                collect(arg, o.recursive, &mut files, &mut p.failures);
                files.sort();
                for f in files {
                    let rel = f.strip_prefix(arg).unwrap_or(&f).to_path_buf();
                    found.push((f, rel));
                }
            }
            Ok(_) => {
                let name = arg.file_name().map_or_else(|| arg.clone(), PathBuf::from);
                found.push((arg.clone(), name));
            }
        }
    }
    let mut inputs = HashSet::new();
    let mut outputs: HashMap<String, PathBuf> = HashMap::new();
    let mut sidecars = Sidecars::default();
    for (input, rel) in found {
        if !inputs.insert(input.canonicalize().unwrap_or_else(|_| input.clone())) {
            continue;
        }
        let output = match &o.out_dir {
            Some(d) => d.join(rel),
            None => input.clone(),
        }
        .with_extension("dng");
        let key = output.to_string_lossy().to_lowercase();
        if let Some(first) = outputs.get(&key) {
            let msg = format!(
                "{} is also the output of {}",
                output.display(),
                first.display()
            );
            p.failures.push((input, msg));
            continue;
        }
        outputs.insert(key, input.clone());
        let sidecar = o.with_sidecar.then(|| sidecars.find(&input)).flatten();
        p.jobs.push(Job {
            input,
            output,
            sidecar,
        });
    }
    p
}

/// `.xmp` files per directory, by lower-case stem; each directory is listed once.
#[derive(Debug, Default)]
struct Sidecars(HashMap<PathBuf, HashMap<String, PathBuf>>);

impl Sidecars {
    fn find(&mut self, nef: &Path) -> Option<PathBuf> {
        let dir = nef.parent().unwrap_or(Path::new(""));
        let index = self.0.entry(dir.to_path_buf()).or_insert_with(|| {
            let listed = if dir.as_os_str().is_empty() {
                Path::new(".")
            } else {
                dir
            };
            let mut found: Vec<PathBuf> = std::fs::read_dir(listed)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xmp")))
                .collect();
            found.sort();
            let mut index = HashMap::new();
            for p in found {
                if let Some(stem) = p.file_stem() {
                    let name = p.file_name().map(PathBuf::from).unwrap_or_default();
                    index
                        .entry(stem.to_string_lossy().to_lowercase())
                        .or_insert_with(|| dir.join(name));
                }
            }
            index
        });
        let stem = nef.file_stem()?.to_string_lossy().to_lowercase();
        index.get(&stem).cloned()
    }
}

/// What happened to one NEF that did not fail.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Done {
    Converted {
        bytes_in: u64,
        bytes_out: u64,
        /// Set when the camera has no built-in colour profile.
        warning: Option<String>,
    },
    Skipped(Skip),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Skip {
    /// The DNG exists and `--skip-existing` was given.
    Exists,
    /// Not a High Efficiency NEF.
    Classic(Option<NikonCompression>),
}

impl Skip {
    fn code(self) -> &'static str {
        match self {
            Self::Exists => "exists",
            Self::Classic(_) => "classic",
        }
    }

    fn message(self, job: &Job) -> String {
        match self {
            Self::Exists => format!("{} already exists", job.output.display()),
            Self::Classic(Some(c)) => {
                format!("classic NEF ({c}), not High Efficiency: your editor already supports it")
            }
            Self::Classic(None) => {
                "classic NEF, not High Efficiency: your editor already supports it".into()
            }
        }
    }
}

/// Converts one NEF; every expected failure is an `Err` message.
fn convert_one(job: &Job, o: &Options) -> Result<Done, String> {
    if job.output.symlink_metadata().is_ok() {
        match o.existing {
            Existing::Skip => return Ok(Done::Skipped(Skip::Exists)),
            Existing::Fail => {
                return Err(format!(
                    "{} already exists (use --force to replace it, or --skip-existing)",
                    job.output.display()
                ));
            }
            Existing::Replace => {}
        }
    }
    if same_file(&job.input, &job.output) {
        return Err(format!("output {} is the input file", job.output.display()));
    }
    let head = read_for_scan(&job.input, false).map_err(|e| format!("reading: {e}"))?;
    let info =
        ticoraw::inspect_nef_prefix(&head.bytes, head.file_len).map_err(|e| e.to_string())?;
    if info.codec == RawCodec::Classic {
        return Ok(Done::Skipped(Skip::Classic(info.compression)));
    }
    if let Some(why) = info.codestream.and_then(|c| c.unsupported) {
        return Err(format!(
            "this High Efficiency variant is not supported yet ({why}); \
             `ticoraw report` collects what is needed to add it"
        ));
    }
    let nef = if head.bytes.len() == head.file_len {
        head.bytes
    } else {
        drop(head);
        std::fs::read(&job.input).map_err(|e| format!("reading: {e}"))?
    };
    let extra_xmp = job.sidecar.as_deref().map(read_sidecar).transpose()?;

    let raw = ticoraw::decode_nef(&nef).map_err(|e| e.to_string())?;
    let carried = CarriedMetadata::from_nef(&nef).map_err(|e| e.to_string())?;
    let bytes_in = nef.len() as u64;
    drop(nef);
    let warning = raw.color.color_matrix.is_none().then(|| {
        format!(
            "no colour profile for {:?}: the DNG has an identity colour matrix, so its colours \
             will be wrong in raw processors",
            raw.model
        )
    });
    let opts = ticoraw_dng::Options {
        compression: o.compression,
        preview: o.preview,
        extra_xmp,
        original_raw_file_name: job
            .input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned()),
        ..ticoraw_dng::Options::default()
    };
    let dng = ticoraw_dng::write(&raw, &carried, &opts).map_err(|e| e.to_string())?;
    drop((raw, carried));

    if let Some(dir) = job.output.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    write_output(&job.output, &dng, o.existing == Existing::Replace).map_err(|e| match e {
        CliError::Failure(m) => m,
        CliError::Output(e) => e.to_string(),
    })?;
    Ok(Done::Converted {
        bytes_in,
        bytes_out: dng.len() as u64,
        warning,
    })
}

/// Reads a sidecar and checks that it is an XMP packet.
fn read_sidecar(path: &Path) -> Result<Vec<u8>, String> {
    let fail = |e: &dyn std::fmt::Display| format!("sidecar {}: {e}", path.display());
    let len = std::fs::metadata(path).map_err(|e| fail(&e))?.len();
    if len > MAX_SIDECAR {
        return Err(fail(&format!(
            "{len} bytes is too large for an XMP sidecar"
        )));
    }
    let bytes = std::fs::read(path).map_err(|e| fail(&e))?;
    let has = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
    if !has(b"<x:xmpmeta") && !has(b"<?xpacket") {
        return Err(fail(&"not an XMP packet (no <x:xmpmeta> or <?xpacket>)"));
    }
    Ok(bytes)
}

/// The default `--jobs`: one per core, at most one per [`MEMORY_PER_JOB`] of memory.
fn default_jobs() -> usize {
    let cores = std::thread::available_parallelism().map_or(1, NonZeroUsize::get);
    let by_memory = physical_memory().map_or(usize::MAX, |m| {
        usize::try_from(m / MEMORY_PER_JOB).unwrap_or(usize::MAX)
    });
    cores.min(by_memory).max(1)
}

#[cfg(target_os = "linux")]
fn physical_memory() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb = info.lines().find_map(|l| l.strip_prefix("MemTotal:"))?;
    let kb: u64 = kb.trim().trim_end_matches("kB").trim().parse().ok()?;
    kb.checked_mul(1024)
}

#[cfg(target_os = "macos")]
fn physical_memory() -> Option<u64> {
    let out = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    String::from_utf8(out.stdout).ok()?.trim().parse().ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn physical_memory() -> Option<u64> {
    None
}

/// Sent by the workers to the printing main thread.
enum Event {
    Started(usize),
    Finished(usize, Result<Done, String>, Duration),
}

pub fn run(o: &Options) -> Result<ExitCode, CliError> {
    let started = Instant::now();
    let plan = plan(o);
    let jobs = o
        .jobs
        .map_or_else(default_jobs, NonZeroUsize::get)
        .min(plan.jobs.len())
        .max(1);
    let progress = !o.json
        && io::stderr().is_terminal()
        && std::env::var_os("TERM").is_none_or(|t| t != "dumb");
    let mut r = Reporter {
        out: io::stdout(),
        json: o.json,
        progress,
        total: plan.jobs.len() + plan.failures.len(),
        started,
        current: String::new(),
        t: Totals::default(),
    };
    r.start(jobs)?;
    for (input, error) in &plan.failures {
        r.failed(input, None, error)?;
    }

    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let (tx, rx) = mpsc::channel();
    let printed = std::thread::scope(|s| -> Result<(), CliError> {
        for _ in 0..jobs {
            let tx = tx.clone();
            let (next, stop, list) = (&next, &stop, &plan.jobs);
            s.spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = list.get(i) else { break };
                    let _ = tx.send(Event::Started(i));
                    let t = Instant::now();
                    let result = panic::catch_unwind(AssertUnwindSafe(|| convert_one(job, o)))
                        .unwrap_or_else(|_| {
                            Err(
                                "internal error: the converter panicked; please report this \
                                 with the output of `ticoraw report`"
                                    .into(),
                            )
                        });
                    let _ = tx.send(Event::Finished(i, result, t.elapsed()));
                }
            });
        }
        drop(tx);
        let result = r.events(&rx, &plan.jobs);
        if result.is_err() {
            // Output is gone (e.g. a closed pipe): finish the files in flight, start no more.
            stop.store(true, Ordering::Relaxed);
        }
        result
    });
    printed?;
    r.summary()?;
    Ok(if r.t.failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Running totals.
#[derive(Debug, Default)]
struct Totals {
    converted: usize,
    skipped: usize,
    failed: usize,
    bytes_in: u64,
    bytes_out: u64,
    failures: Vec<(PathBuf, String)>,
}

/// Prints events as text or JSON Lines, with a progress line on a terminal.
struct Reporter {
    out: io::Stdout,
    json: bool,
    /// Draw a progress line on stderr.
    progress: bool,
    total: usize,
    started: Instant,
    /// Name of the file started last.
    current: String,
    t: Totals,
}

impl Reporter {
    fn events(&mut self, rx: &mpsc::Receiver<Event>, jobs: &[Job]) -> Result<(), CliError> {
        loop {
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(Event::Started(i)) => {
                    self.current = jobs[i]
                        .input
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                }
                Ok(Event::Finished(i, result, took)) => {
                    let job = &jobs[i];
                    match result {
                        Ok(Done::Converted {
                            bytes_in,
                            bytes_out,
                            warning,
                        }) => self.converted(job, bytes_in, bytes_out, warning, took)?,
                        Ok(Done::Skipped(why)) => self.skipped(job, why)?,
                        Err(e) => self.failed(&job.input, Some(&job.output), &e)?,
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            self.draw_progress();
        }
        self.clear_progress();
        Ok(())
    }

    fn start(&mut self, jobs: usize) -> io::Result<()> {
        if self.json {
            self.line(&Json::Object(vec![
                ("event", Json::str("start")),
                ("ticoraw_version", Json::str(env!("CARGO_PKG_VERSION"))),
                ("files", Json::uint(self.total as u64)),
                ("jobs", Json::uint(jobs as u64)),
            ]))
        } else {
            Ok(())
        }
    }

    fn converted(
        &mut self,
        job: &Job,
        bytes_in: u64,
        bytes_out: u64,
        warning: Option<String>,
        took: Duration,
    ) -> io::Result<()> {
        self.t.converted += 1;
        self.t.bytes_in += bytes_in;
        self.t.bytes_out += bytes_out;
        if self.json {
            return self.line(&Json::Object(vec![
                ("event", Json::str("converted")),
                ("input", Json::str(job.input.to_string_lossy())),
                ("output", Json::str(job.output.to_string_lossy())),
                (
                    "sidecar",
                    Json::opt(job.sidecar.as_ref(), |p| Json::str(p.to_string_lossy())),
                ),
                ("bytes_in", Json::uint(bytes_in)),
                ("bytes_out", Json::uint(bytes_out)),
                ("seconds", Json::Float(took.as_secs_f64())),
                ("warning", Json::opt(warning, Json::Str)),
            ]));
        }
        let sidecar = job
            .sidecar
            .as_ref()
            .map(|p| format!(" + {}", p.display()))
            .unwrap_or_default();
        self.text(&format!(
            "converted  {}{sidecar} -> {}  ({:.1} MB -> {:.1} MB, {:.2} s)",
            job.input.display(),
            job.output.display(),
            mb(bytes_in),
            mb(bytes_out),
            took.as_secs_f64()
        ))?;
        match warning {
            Some(w) => self.text(&format!("warning    {}: {w}", job.input.display())),
            None => Ok(()),
        }
    }

    fn skipped(&mut self, job: &Job, why: Skip) -> io::Result<()> {
        self.t.skipped += 1;
        let message = why.message(job);
        if self.json {
            return self.line(&Json::Object(vec![
                ("event", Json::str("skipped")),
                ("input", Json::str(job.input.to_string_lossy())),
                ("output", Json::str(job.output.to_string_lossy())),
                ("reason", Json::str(why.code())),
                ("message", Json::Str(message)),
            ]));
        }
        self.text(&format!("skipped    {}: {message}", job.input.display()))
    }

    fn failed(&mut self, input: &Path, output: Option<&Path>, error: &str) -> io::Result<()> {
        self.t.failed += 1;
        self.t
            .failures
            .push((input.to_path_buf(), error.to_owned()));
        if self.json {
            return self.line(&Json::Object(vec![
                ("event", Json::str("failed")),
                ("input", Json::str(input.to_string_lossy())),
                (
                    "output",
                    Json::opt(output, |p| Json::str(p.to_string_lossy())),
                ),
                ("error", Json::str(error)),
            ]));
        }
        self.text(&format!("FAILED     {}: {error}", input.display()))
    }

    fn summary(&mut self) -> io::Result<()> {
        let secs = self.started.elapsed().as_secs_f64();
        let t = &self.t;
        let per_sec = |n: f64| n / secs.max(1e-9);
        if self.json {
            let line = Json::Object(vec![
                ("event", Json::str("summary")),
                ("files", Json::uint(self.total as u64)),
                ("converted", Json::uint(t.converted as u64)),
                ("skipped", Json::uint(t.skipped as u64)),
                ("failed", Json::uint(t.failed as u64)),
                ("bytes_in", Json::uint(t.bytes_in)),
                ("bytes_out", Json::uint(t.bytes_out)),
                ("seconds", Json::Float(secs)),
                ("files_per_second", Json::Float(per_sec(t.converted as f64))),
                ("mb_per_second", Json::Float(per_sec(mb(t.bytes_in)))),
            ]);
            return self.line(&line);
        }
        let mut out = self.out.lock();
        if self.total > 0 {
            writeln!(out)?;
        }
        writeln!(
            out,
            "{} file{}: {} converted, {} skipped, {} failed",
            self.total,
            if self.total == 1 { "" } else { "s" },
            t.converted,
            t.skipped,
            t.failed
        )?;
        writeln!(
            out,
            "read {:.1} MB, wrote {:.1} MB in {secs:.2} s: {:.2} files/s, {:.1} MB/s read",
            mb(t.bytes_in),
            mb(t.bytes_out),
            per_sec(t.converted as f64),
            per_sec(mb(t.bytes_in))
        )?;
        for (p, e) in &t.failures {
            writeln!(out, "failed: {}: {e}", p.display())?;
        }
        Ok(())
    }

    fn line(&mut self, v: &Json) -> io::Result<()> {
        let mut out = self.out.lock();
        writeln!(out, "{}", v.compact())?;
        out.flush()
    }

    /// A text line on stdout, above the progress line.
    fn text(&mut self, s: &str) -> io::Result<()> {
        self.clear_progress();
        let mut out = self.out.lock();
        writeln!(out, "{s}")?;
        out.flush()
    }

    fn draw_progress(&self) {
        if !self.progress {
            return;
        }
        let columns = std::env::var("COLUMNS")
            .ok()
            .and_then(|c| c.parse().ok())
            .unwrap_or(80);
        let line = progress_line(
            &self.t,
            self.total,
            self.started.elapsed(),
            &self.current,
            columns,
        );
        // Progress is best effort: a failed write to stderr is ignored.
        let _ = write!(io::stderr().lock(), "\r\x1b[2K{line}");
    }

    fn clear_progress(&self) {
        if self.progress {
            let _ = write!(io::stderr().lock(), "\r\x1b[2K");
        }
    }
}

/// The progress line, cut to fit `columns - 1` characters so that it never wraps.
fn progress_line(
    t: &Totals,
    total: usize,
    elapsed: Duration,
    current: &str,
    columns: usize,
) -> String {
    let done = t.converted + t.skipped + t.failed;
    let secs = elapsed.as_secs_f64();
    let eta = if done > 0 && done < total {
        format!(", ETA {:.0} s", secs / done as f64 * (total - done) as f64)
    } else {
        String::new()
    };
    let width = total.to_string().len();
    let line = format!(
        "[{done:>width$}/{total}] {} converted, {} skipped, {} failed, {:.1} MB/s{eta}  {current}",
        t.converted,
        t.skipped,
        t.failed,
        mb(t.bytes_in) / secs.max(1e-9),
    );
    line.chars().take(columns.saturating_sub(1)).collect()
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(inputs: &[&Path]) -> Options {
        Options {
            inputs: inputs.iter().map(|p| p.to_path_buf()).collect(),
            out_dir: None,
            recursive: false,
            existing: Existing::Fail,
            compression: Compression::LosslessJpeg,
            preview: Preview::Auto,
            with_sidecar: false,
            jobs: None,
            json: false,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ticoraw-convert-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        dir
    }

    #[test]
    fn parses_options() {
        let p = |items: &[&str]| parse(&mut Args::new(items.iter().map(Into::into).collect()));
        let o = p(&["-j", "3", "--skip-existing", "-r", "a", "b"])
            .unwrap()
            .unwrap();
        assert_eq!(o.jobs.map(NonZeroUsize::get), Some(3));
        assert_eq!(o.existing, Existing::Skip);
        assert!(o.recursive);
        assert_eq!(o.inputs.len(), 2);
        let o = p(&["--force", "--uncompressed", "--no-preview", "--jobs=1", "a"])
            .unwrap()
            .unwrap();
        assert_eq!(o.existing, Existing::Replace);
        assert_eq!(o.compression, Compression::Uncompressed);
        assert_eq!(o.preview, Preview::None);
        assert!(p(&["--help"]).unwrap().is_none());
        for bad in [
            &["a", "--skip-existing", "--force"][..],
            &["-j", "0", "a"],
            &["-j", "x", "a"],
            &[],
            &["--frob", "a"],
        ] {
            assert!(p(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn plans_outputs_mirroring_dirs_and_rejects_collisions() {
        let dir = scratch("plan");
        for f in ["a.NEF", "b.nrw", "sub/c.nef", "notes.txt", "B.nef"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let out = dir.join("out");
        let mut o = options(&[&dir, &dir.join("a.NEF")]);
        o.recursive = true;
        o.out_dir = Some(out.clone());
        let p = plan(&o);
        let outputs: Vec<_> = p.jobs.iter().map(|j| j.output.clone()).collect();
        // B.nef and b.nrw both want out/b.dng: the second in sorted order fails. a.NEF given
        // twice is converted once.
        assert_eq!(
            outputs,
            vec![out.join("B.dng"), out.join("a.dng"), out.join("sub/c.dng")]
        );
        assert_eq!(p.failures.len(), 1, "{:?}", p.failures);
        assert!(p.failures[0].1.contains("is also the output of"));

        let p = plan(&options(&[&dir.join("missing.nef"), &dir.join("a.NEF")]));
        assert_eq!(p.jobs[0].output, dir.join("a.dng"));
        assert!(p.failures[0].1.starts_with("reading:"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_sidecars_by_stem_in_any_case() {
        let dir = scratch("sidecar");
        for f in ["a.NEF", "a.XMP", "b.nef", "c.nef", "c.nef.xmp"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let mut s = Sidecars::default();
        assert_eq!(s.find(&dir.join("a.NEF")), Some(dir.join("a.XMP")));
        assert_eq!(s.find(&dir.join("b.nef")), None);
        assert_eq!(s.find(&dir.join("c.nef")), None);
        std::fs::write(dir.join("bad.xmp"), b"<html/>").unwrap();
        assert!(
            read_sidecar(&dir.join("bad.xmp"))
                .unwrap_err()
                .contains("not an XMP")
        );
        std::fs::write(dir.join("ok.xmp"), b"<x:xmpmeta/>").unwrap();
        assert_eq!(read_sidecar(&dir.join("ok.xmp")).unwrap(), b"<x:xmpmeta/>");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn converts_skips_and_never_clobbers() {
        let dir = scratch("one");
        let he = dir.join("he.NEF");
        let classic = dir.join("classic.nef");
        std::fs::write(&he, crate::synthetic::he_nef(3)).unwrap();
        std::fs::write(&classic, crate::synthetic::classic_nef()).unwrap();
        let o = options(&[&dir]);
        let p = plan(&o);
        let job = |name: &str| p.jobs.iter().find(|j| j.input.ends_with(name)).unwrap();

        let done = convert_one(job("he.NEF"), &o).unwrap();
        assert!(
            matches!(done, Done::Converted { warning: None, .. }),
            "{done:?}"
        );
        let dng = std::fs::read(dir.join("he.dng")).unwrap();
        let raw = ticoraw::decode_nef(&std::fs::read(&he).unwrap()).unwrap();
        assert_eq!(ticoraw::dng::decode_dng_cfa(&dng).unwrap().image, raw.image);
        assert_eq!(
            convert_one(job("classic.nef"), &o),
            Ok(Done::Skipped(Skip::Classic(Some(
                NikonCompression::Lossless
            ))))
        );
        assert!(!dir.join("classic.dng").exists());

        // An existing DNG is kept, skipped or replaced.
        let err = convert_one(job("he.NEF"), &o).unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        let skip = Options {
            existing: Existing::Skip,
            ..options(&[&dir])
        };
        assert_eq!(
            convert_one(job("he.NEF"), &skip),
            Ok(Done::Skipped(Skip::Exists))
        );
        std::fs::write(dir.join("he.dng"), b"old").unwrap();
        let force = Options {
            existing: Existing::Replace,
            ..options(&[&dir])
        };
        assert!(convert_one(job("he.NEF"), &force).is_ok());
        assert_eq!(std::fs::read(dir.join("he.dng")).unwrap(), dng);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn progress_line_fits_the_terminal() {
        let t = Totals {
            converted: 2,
            skipped: 1,
            bytes_in: 50_000_000,
            ..Totals::default()
        };
        let line = progress_line(&t, 10, Duration::from_secs(2), "DSC_0004.NEF", 200);
        assert_eq!(
            line,
            "[ 3/10] 2 converted, 1 skipped, 0 failed, 25.0 MB/s, ETA 5 s  DSC_0004.NEF"
        );
        let line = progress_line(&t, 10, Duration::from_secs(2), "DSC_0004.NEF", 40);
        assert_eq!(line.chars().count(), 39);
    }

    #[test]
    fn default_jobs_is_positive() {
        assert!(default_jobs() >= 1);
    }
}
