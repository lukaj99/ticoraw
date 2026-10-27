//! A tiny command-line lexer: options (`-h`, `--json`, `--output=x`, `-o x`) and positionals.

use std::ffi::OsString;

/// One lexed command-line token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    /// An option name including its dashes, e.g. `--json` or `-o`.
    Opt(String),
    /// A positional argument (anything after `--`, `-` alone, or not starting with `-`).
    Pos(OsString),
}

/// Lexes arguments on demand so each command decides which options take values.
pub struct Args {
    items: std::vec::IntoIter<OsString>,
    /// The `value` of a `--name=value` token, waiting for `value()` to claim it.
    attached: Option<(String, OsString)>,
    positional_only: bool,
}

impl Args {
    pub fn new(items: Vec<OsString>) -> Self {
        Self {
            items: items.into_iter(),
            attached: None,
            positional_only: false,
        }
    }

    /// Returns the next token, or a usage message for malformed input.
    pub fn next_arg(&mut self) -> Result<Option<Arg>, String> {
        if let Some((name, _)) = self.attached.take() {
            return Err(format!("option {name} does not take a value"));
        }
        let Some(item) = self.items.next() else {
            return Ok(None);
        };
        if self.positional_only {
            return Ok(Some(Arg::Pos(item)));
        }
        let Some(text) = item.to_str() else {
            // Non-UTF-8 options are never valid; non-UTF-8 paths are fine.
            return if item.as_encoded_bytes().first() == Some(&b'-') {
                Err(format!("invalid option {}", item.to_string_lossy()))
            } else {
                Ok(Some(Arg::Pos(item)))
            };
        };
        if text == "--" {
            self.positional_only = true;
            return self.next_arg();
        }
        if text == "-" || !text.starts_with('-') {
            return Ok(Some(Arg::Pos(item)));
        }
        if let Some((name, value)) = text.split_once('=').filter(|_| text.starts_with("--")) {
            self.attached = Some((name.to_owned(), value.into()));
            return Ok(Some(Arg::Opt(name.to_owned())));
        }
        Ok(Some(Arg::Opt(text.to_owned())))
    }

    /// Claims the value of option `name`: the `=value` suffix or the following argument.
    pub fn value(&mut self, name: &str) -> Result<OsString, String> {
        if let Some((_, v)) = self.attached.take() {
            return Ok(v);
        }
        self.items
            .next()
            .ok_or_else(|| format!("option {name} requires a value"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex(items: &[&str]) -> Args {
        Args::new(items.iter().map(OsString::from).collect())
    }

    #[test]
    fn options_values_and_positionals() {
        let mut a = lex(&[
            "a.nef",
            "--json",
            "-o",
            "x.dng",
            "--format=pgm",
            "-",
            "--",
            "-z",
        ]);
        assert_eq!(a.next_arg(), Ok(Some(Arg::Pos("a.nef".into()))));
        assert_eq!(a.next_arg(), Ok(Some(Arg::Opt("--json".into()))));
        assert_eq!(a.next_arg(), Ok(Some(Arg::Opt("-o".into()))));
        assert_eq!(a.value("-o"), Ok("x.dng".into()));
        assert_eq!(a.next_arg(), Ok(Some(Arg::Opt("--format".into()))));
        assert_eq!(a.value("--format"), Ok("pgm".into()));
        assert_eq!(a.next_arg(), Ok(Some(Arg::Pos("-".into()))));
        assert_eq!(a.next_arg(), Ok(Some(Arg::Pos("-z".into()))));
        assert_eq!(a.next_arg(), Ok(None));
    }

    #[test]
    fn value_errors() {
        let mut a = lex(&["--json=yes"]);
        assert_eq!(a.next_arg(), Ok(Some(Arg::Opt("--json".into()))));
        assert!(a.next_arg().is_err());
        let mut a = lex(&["-o"]);
        assert_eq!(a.next_arg(), Ok(Some(Arg::Opt("-o".into()))));
        assert!(a.value("-o").is_err());
    }
}
