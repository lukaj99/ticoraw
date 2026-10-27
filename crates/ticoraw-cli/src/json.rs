//! A minimal JSON value with a pretty and a one-line printer, enough for `inspect --json`,
//! `convert --json` events and `report`.

use std::fmt::{self, Write};

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    /// Written with three decimals; a non-finite value is written as `null`.
    Float(f64),
    Str(String),
    Array(Vec<Json>),
    /// Keys keep insertion order.
    Object(Vec<(&'static str, Json)>),
}

impl Json {
    pub fn str(s: impl Into<String>) -> Self {
        Self::Str(s.into())
    }

    pub fn uint(n: impl Into<u64>) -> Self {
        Self::Int(i64::try_from(n.into()).unwrap_or(i64::MAX))
    }

    pub fn opt<T>(v: Option<T>, f: impl FnOnce(T) -> Self) -> Self {
        v.map_or(Self::Null, f)
    }

    /// The value on one line without insignificant whitespace (for JSON Lines output).
    pub fn compact(&self) -> String {
        let mut s = String::new();
        // Writing to a String cannot fail.
        let _ = self.write_compact(&mut s);
        s
    }

    fn write_compact(&self, out: &mut String) -> fmt::Result {
        match self {
            Self::Array(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write_compact(out)?;
                }
                out.push(']');
            }
            Self::Object(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(out, k)?;
                    out.push(':');
                    v.write_compact(out)?;
                }
                out.push('}');
            }
            scalar => scalar.write_scalar(out)?,
        }
        Ok(())
    }

    /// Writes a scalar; arrays and objects write nothing.
    fn write_scalar(&self, out: &mut String) -> fmt::Result {
        match self {
            Self::Bool(b) => write!(out, "{b}")?,
            Self::Int(n) => write!(out, "{n}")?,
            Self::Float(x) if x.is_finite() => write!(out, "{x:.3}")?,
            Self::Null | Self::Float(_) => out.push_str("null"),
            Self::Str(s) => write_string(out, s)?,
            Self::Array(_) | Self::Object(_) => {}
        }
        Ok(())
    }

    fn write(&self, out: &mut String, indent: usize) -> fmt::Result {
        let pad = |out: &mut String, n: usize| out.extend(std::iter::repeat_n(' ', n));
        match self {
            Self::Array(items) if items.is_empty() => out.push_str("[]"),
            Self::Object(fields) if fields.is_empty() => out.push_str("{}"),
            // Arrays of scalars stay on one line (e.g. black levels).
            Self::Array(items) if items.iter().all(Self::is_scalar) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    v.write(out, indent)?;
                }
                out.push(']');
            }
            Self::Array(items) => {
                out.push_str("[\n");
                for (i, v) in items.iter().enumerate() {
                    pad(out, indent + 2);
                    v.write(out, indent + 2)?;
                    out.push_str(if i + 1 < items.len() { ",\n" } else { "\n" });
                }
                pad(out, indent);
                out.push(']');
            }
            Self::Object(fields) => {
                out.push_str("{\n");
                for (i, (k, v)) in fields.iter().enumerate() {
                    pad(out, indent + 2);
                    write_string(out, k)?;
                    out.push_str(": ");
                    v.write(out, indent + 2)?;
                    out.push_str(if i + 1 < fields.len() { ",\n" } else { "\n" });
                }
                pad(out, indent);
                out.push('}');
            }
            scalar => scalar.write_scalar(out)?,
        }
        Ok(())
    }

    fn is_scalar(&self) -> bool {
        !matches!(self, Self::Array(_) | Self::Object(_))
    }
}

impl fmt::Display for Json {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        self.write(&mut s, 0)?;
        f.write_str(&s)
    }
}

fn write_string(out: &mut String, s: &str) -> fmt::Result {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // All control characters (C0, DEL and C1), so that JSON shown on a terminal
            // cannot carry escape sequences from a file's strings.
            c if c.is_control() => write!(out, "\\u{:04x}", c as u32)?,
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_strings() {
        assert_eq!(
            Json::str("a\"b\\c\n\u{1}é\u{1b}[2J\u{7f}\u{9b}").to_string(),
            r#""a\"b\\c\n\u0001é\u001b[2J\u007f\u009b""#
        );
    }

    #[test]
    fn pretty_prints_nested_values() {
        let v = Json::Array(vec![Json::Object(vec![
            ("n", Json::Int(-3)),
            ("none", Json::Null),
            (
                "levels",
                Json::Array(vec![Json::uint(1u16), Json::uint(2u16)]),
            ),
            ("empty", Json::Object(vec![])),
        ])]);
        let expected = "[\n  {\n    \"n\": -3,\n    \"none\": null,\n    \
                        \"levels\": [1, 2],\n    \"empty\": {}\n  }\n]";
        assert_eq!(v.to_string(), expected);
    }

    #[test]
    fn compact_lines_and_scalars() {
        let v = Json::Object(vec![
            ("ok", Json::Bool(true)),
            ("s", Json::Float(1.23456)),
            ("nan", Json::Float(f64::NAN)),
            ("a", Json::Array(vec![Json::Int(1), Json::str("x")])),
            ("o", Json::Object(vec![("n", Json::Null)])),
        ]);
        assert_eq!(
            v.compact(),
            r#"{"ok":true,"s":1.235,"nan":null,"a":[1,"x"],"o":{"n":null}}"#
        );
        assert!(v.to_string().contains("\"ok\": true,\n"));
    }
}
