//! The crate's error type. [`TicoError`] and [`Result`] are re-exported at the crate root.

use std::fmt;

/// Every way decoding can fail. Expected failures are values, never panics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicoError {
    /// Input ended before a structure was complete.
    Truncated {
        /// The structure that was cut short.
        what: &'static str,
        /// Byte offset, within the buffer being parsed, where the structure starts or
        /// where reading stopped.
        offset: usize,
    },
    /// A TIFF/NEF/DNG container is malformed.
    Container(String),
    /// A JPEG XS marker segment is malformed or out of order.
    Header(String),
    /// The codestream uses a feature this decoder does not implement.
    Unsupported(String),
    /// Entropy-coded data is inconsistent with its declared sizes.
    Entropy(String),
    /// A lossless JPEG (SOF3) stream inside a DNG is malformed.
    Jpeg(String),
}

impl fmt::Display for TicoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { what, offset } => write!(f, "truncated {what} at byte {offset}"),
            Self::Container(m) => write!(f, "container: {m}"),
            Self::Header(m) => write!(f, "codestream header: {m}"),
            Self::Unsupported(m) => write!(f, "unsupported: {m}"),
            Self::Entropy(m) => write!(f, "entropy data: {m}"),
            Self::Jpeg(m) => write!(f, "lossless jpeg: {m}"),
        }
    }
}

impl std::error::Error for TicoError {}

/// `Result` with [`TicoError`] as the error type.
pub type Result<T> = std::result::Result<T, TicoError>;
