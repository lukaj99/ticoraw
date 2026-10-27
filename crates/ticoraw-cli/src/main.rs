//! `ticoraw`: convert Nikon High Efficiency (HE/HE*) NEF raw files to DNG, and inspect,
//! decode and verify them.
//!
//! All file I/O lives here; the `ticoraw` and `ticoraw-dng` libraries only ever see byte
//! slices.

mod args;
mod convert;
mod decode;
mod inspect;
mod interrupt;
mod json;
mod report;
mod scan;
mod verify;

#[cfg(test)]
#[path = "../../ticoraw/tests/common/mod.rs"]
mod common;
#[cfg(test)]
#[path = "../tests/support/synthetic.rs"]
mod synthetic;

use std::borrow::Cow;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use args::{Arg, Args};

const HELP: &str = "\
ticoraw: convert Nikon High Efficiency (HE/HE*) NEF raw files to DNG

Usage: ticoraw <command> [options]

Commands:
  convert   Convert HE/HE* NEFs (files or whole directories) to complete DNGs
  decode    Decode one NEF to a complete DNG (or a 16-bit PGM)
  inspect   Describe NEF containers and TicoRAW codestreams without decoding
  verify    Compare a decoded NEF sample-for-sample against a reference DNG
  scan      Classify every NEF in a directory, optionally decoding them all
  report    Print a privacy-safe JSON bug report for one NEF
  help      Show help for a command

Options:
  -h, --help     Show this help (or `ticoraw <command> --help`)
  -V, --version  Show the version

Exit status: 0 on success, 1 on failure, 2 on usage errors.
";

/// The subcommands, used for dispatch and per-command help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Topic {
    Convert,
    Inspect,
    Decode,
    Verify,
    Scan,
    Report,
}

impl Topic {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "convert" => Self::Convert,
            "inspect" => Self::Inspect,
            "decode" => Self::Decode,
            "verify" => Self::Verify,
            "scan" => Self::Scan,
            "report" => Self::Report,
            _ => return None,
        })
    }

    fn help(self) -> &'static str {
        match self {
            Self::Convert => convert::HELP,
            Self::Inspect => inspect::HELP,
            Self::Decode => decode::HELP,
            Self::Verify => verify::HELP,
            Self::Scan => scan::HELP,
            Self::Report => report::HELP,
        }
    }

    /// The `Usage:` line of the command's help.
    fn usage(self) -> &'static str {
        self.help()
            .lines()
            .find(|l| l.starts_with("Usage:"))
            .unwrap_or("Usage: ticoraw <command> [options]")
    }
}

/// A fully parsed invocation.
#[derive(Debug)]
enum Command {
    Help(Option<Topic>),
    Version,
    Convert(convert::Options),
    Inspect(inspect::Options),
    Decode(decode::Options),
    Verify(verify::Options),
    Scan(scan::Options),
    Report(report::Options),
}

/// A malformed command line (exit status 2).
#[derive(Debug)]
struct UsageError {
    topic: Option<Topic>,
    message: String,
}

/// A failed run (exit status 1).
#[derive(Debug)]
enum CliError {
    /// The message is printed as `ticoraw: error: <message>`.
    Failure(String),
    /// Writing to stdout failed (e.g. a closed pipe); reported tersely.
    Output(io::Error),
}

impl From<io::Error> for CliError {
    fn from(e: io::Error) -> Self {
        Self::Output(e)
    }
}

fn main() -> ExitCode {
    let command = match parse(std::env::args_os().skip(1).collect()) {
        Ok(c) => c,
        Err(UsageError { topic, message }) => {
            let (usage, more) = match topic {
                Some(t) => (t.usage(), format!("ticoraw {} --help", name(t))),
                None => (
                    "Usage: ticoraw <command> [options]",
                    "ticoraw --help".into(),
                ),
            };
            eprintln!("ticoraw: {message}\n\n{usage}\n\nFor more information, try '{more}'.");
            return ExitCode::from(2);
        }
    };
    match run(command) {
        Ok(code) => code,
        Err(CliError::Failure(m)) => {
            eprintln!("ticoraw: error: {}", printable(&m));
            ExitCode::FAILURE
        }
        Err(CliError::Output(e)) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::FAILURE,
        Err(CliError::Output(e)) => {
            eprintln!("ticoraw: error: writing output: {e}");
            ExitCode::FAILURE
        }
    }
}

fn name(t: Topic) -> &'static str {
    match t {
        Topic::Convert => "convert",
        Topic::Inspect => "inspect",
        Topic::Decode => "decode",
        Topic::Verify => "verify",
        Topic::Scan => "scan",
        Topic::Report => "report",
    }
}

fn parse(items: Vec<OsString>) -> Result<Command, UsageError> {
    let top = |message: String| UsageError {
        topic: None,
        message,
    };
    let mut args = Args::new(items);
    let first = match args.next_arg().map_err(top)? {
        None => return Err(top("no command given".into())),
        Some(Arg::Opt(o)) => match o.as_str() {
            "-h" | "--help" => return Ok(Command::Help(None)),
            "-V" | "--version" => return Ok(Command::Version),
            _ => return Err(top(format!("unknown option {o}"))),
        },
        Some(Arg::Pos(p)) => p.to_string_lossy().into_owned(),
    };
    if first == "help" {
        return match args.next_arg().map_err(top)? {
            None => Ok(Command::Help(None)),
            Some(Arg::Pos(p)) => Topic::from_name(&p.to_string_lossy())
                .map(|t| Command::Help(Some(t)))
                .ok_or_else(|| top(format!("unknown command {}", p.to_string_lossy()))),
            Some(Arg::Opt(o)) => Err(top(format!("unknown option {o}"))),
        };
    }
    let topic =
        Topic::from_name(&first).ok_or_else(|| top(format!("unknown command {first:?}")))?;
    let parsed = match topic {
        Topic::Convert => convert::parse(&mut args).map(|o| o.map(Command::Convert)),
        Topic::Report => report::parse(&mut args).map(|o| o.map(Command::Report)),
        Topic::Inspect => inspect::parse(&mut args).map(|o| o.map(Command::Inspect)),
        Topic::Decode => decode::parse(&mut args).map(|o| o.map(Command::Decode)),
        Topic::Verify => verify::parse(&mut args).map(|o| o.map(Command::Verify)),
        Topic::Scan => scan::parse(&mut args).map(|o| o.map(Command::Scan)),
    };
    match parsed {
        Ok(Some(c)) => Ok(c),
        Ok(None) => Ok(Command::Help(Some(topic))),
        Err(message) => Err(UsageError {
            topic: Some(topic),
            message,
        }),
    }
}

fn run(command: Command) -> Result<ExitCode, CliError> {
    match command {
        Command::Help(topic) => {
            io::stdout().write_all(topic.map_or(HELP, Topic::help).as_bytes())?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Version => {
            writeln!(io::stdout(), "ticoraw {}", env!("CARGO_PKG_VERSION"))?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Inspect(o) => inspect::run(&o),
        Command::Decode(o) => decode::run(&o),
        Command::Verify(o) => verify::run(&o),
        Command::Scan(o) => scan::run(&o),
        Command::Convert(o) => convert::run(&o),
        Command::Report(o) => report::run(&o),
    }
}

/// Reads a whole input file, naming it in the error.
fn read_input(path: &Path) -> Result<Vec<u8>, CliError> {
    std::fs::read(path).map_err(|e| CliError::Failure(format!("reading {}: {e}", path.display())))
}

/// `s` with every control character (and bidirectional override) escaped as `\u{..}`, so
/// that text taken from a file (a camera model, a file name) cannot move the cursor, clear
/// the terminal or forge output lines.
fn printable(s: &str) -> Cow<'_, str> {
    let unsafe_char =
        |c: char| c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}');
    if !s.chars().any(unsafe_char) {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if unsafe_char(c) {
            out.extend(c.escape_unicode());
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// A path for text output, through [`printable`].
fn shown(path: &Path) -> String {
    printable(&path.to_string_lossy()).into_owned()
}

/// Formats a duration as milliseconds with one decimal.
fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1e3)
}

/// Single-letter colour name of a CFA pattern code (0 = R, 1 = G, 2 = B).
fn cfa_color(code: u8) -> char {
    match code {
        0 => 'R',
        1 => 'G',
        2 => 'B',
        3 => 'C',
        4 => 'M',
        5 => 'Y',
        _ => '?',
    }
}

/// Rejects a second positional where a command takes exactly one.
fn unexpected(p: &OsString) -> String {
    format!("unexpected argument {:?}", p.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(items: &[&str]) -> Result<Command, UsageError> {
        parse(items.iter().map(OsString::from).collect())
    }

    #[test]
    fn top_level() {
        assert!(matches!(p(&["--help"]), Ok(Command::Help(None))));
        assert!(matches!(p(&["-V"]), Ok(Command::Version)));
        assert!(matches!(
            p(&["help", "scan"]),
            Ok(Command::Help(Some(Topic::Scan)))
        ));
        assert!(matches!(
            p(&["verify", "-h"]),
            Ok(Command::Help(Some(Topic::Verify)))
        ));
        assert!(p(&[]).is_err());
        assert!(p(&["frobnicate"]).is_err());
        assert!(p(&["--frob"]).is_err());
        assert!(p(&["help", "frob"]).is_err());
    }

    #[test]
    fn printable_escapes_control_characters() {
        assert!(matches!(printable("NIKON Z 6_3"), Cow::Borrowed(_)));
        assert_eq!(printable("caf\u{e9} \u{4e2d}"), "caf\u{e9} \u{4e2d}");
        assert_eq!(
            printable("\x1b[2J\x1b]0;PWN\x07X\nY\u{9b}\u{202e}"),
            r"\u{1b}[2J\u{1b}]0;PWN\u{7}X\u{a}Y\u{9b}\u{202e}"
        );
        assert_eq!(shown(Path::new("a/\x1bb.NEF")), r"a/\u{1b}b.NEF");
    }

    #[test]
    fn usage_lines_exist() {
        for t in [
            Topic::Convert,
            Topic::Inspect,
            Topic::Decode,
            Topic::Verify,
            Topic::Scan,
            Topic::Report,
        ] {
            assert!(
                t.usage()
                    .starts_with(&format!("Usage: ticoraw {}", name(t)))
            );
        }
    }
}
