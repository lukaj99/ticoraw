//! A minimal JSON value with a pretty printer, enough for `inspect --json`.

use std::fmt::{self, Write};

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Int(i64),
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

    fn write(&self, out: &mut String, indent: usize) -> fmt::Result {
        let pad = |out: &mut String, n: usize| out.extend(std::iter::repeat_n(' ', n));
        match self {
            Self::Null => out.push_str("null"),
            Self::Int(n) => write!(out, "{n}")?,
            Self::Str(s) => write_string(out, s)?,
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
            c if c < ' ' || c == '\u{7f}' => write!(out, "\\u{:04x}", c as u32)?,
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
            Json::str("a\"b\\c\n\u{1}é").to_string(),
            r#""a\"b\\c\n\u0001é""#
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
}
