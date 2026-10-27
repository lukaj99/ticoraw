//! Python bindings for the `ticoraw` decoder and the `ticoraw-dng` writer.
//!
//! This crate is a thin imperative shell: it turns Python arguments into bytes and options,
//! runs the pure Rust core with the GIL released, and turns the results back into Python
//! objects (a zero-copy numpy mosaic, dicts, `bytes`) or exceptions. All decoding logic,
//! and with it the bit-exact guarantee, stays in `ticoraw`.
//!
//! Panics never cross into Python: pyo3 turns any panic at the function boundary into a
//! `pyo3_runtime.PanicException`, and the core returns every expected failure as a value.

use std::path::{Path, PathBuf};

use numpy::ndarray::Array2;
use numpy::{IntoPyArray, PyArray2};
use pyo3::create_exception;
use pyo3::exceptions::{PyException, PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict, PyString};
use ticoraw::{DecodedRaw, NefInspection, NikonCompression, RawCodec, TicoError as CoreError};
use ticoraw_dng::{Compression, Options, Preview};

create_exception!(
    ticoraw,
    TicoError,
    PyException,
    "Base class of every error ticoraw raises for input it cannot read, decode or convert."
);
create_exception!(
    ticoraw,
    Truncated,
    TicoError,
    "The input ended before a structure was complete. `what` names the structure and \
     `offset` the byte where it starts or where reading stopped."
);
create_exception!(
    ticoraw,
    Container,
    TicoError,
    "The TIFF/NEF container is malformed, is not a Nikon file, or lacks a required tag."
);
create_exception!(
    ticoraw,
    Header,
    TicoError,
    "A JPEG XS marker segment of the codestream is malformed or out of order."
);
create_exception!(
    ticoraw,
    Unsupported,
    TicoError,
    "The file is valid but not decodable here: a classic (non-HE) NEF, or a codestream \
     outside the supported profile."
);
create_exception!(
    ticoraw,
    Entropy,
    TicoError,
    "Entropy-coded data is inconsistent with its declared sizes."
);
create_exception!(
    ticoraw,
    Jpeg,
    TicoError,
    "A lossless JPEG stream is malformed."
);
create_exception!(
    ticoraw,
    Dng,
    TicoError,
    "The DNG could not be written: too large for classic TIFF, or the preview encoder failed."
);

/// Maps a core error to its exception class, with the same message as the Rust `Display`.
fn core_err(py: Python<'_>, e: &CoreError) -> PyErr {
    let msg = e.to_string();
    match e {
        CoreError::Truncated { what, offset } => {
            let err = Truncated::new_err(msg);
            let value = err.value(py);
            match value
                .setattr("what", *what)
                .and_then(|()| value.setattr("offset", *offset))
            {
                Ok(()) => err,
                Err(attr_err) => attr_err,
            }
        }
        CoreError::Container(_) => Container::new_err(msg),
        CoreError::Header(_) => Header::new_err(msg),
        CoreError::Unsupported(_) => Unsupported::new_err(msg),
        CoreError::Entropy(_) => Entropy::new_err(msg),
        CoreError::Jpeg(_) => Jpeg::new_err(msg),
    }
}

fn dng_err(py: Python<'_>, e: &ticoraw_dng::Error) -> PyErr {
    match e {
        ticoraw_dng::Error::Decode(inner) => core_err(py, inner),
        ticoraw_dng::Error::InvalidImage(_)
        | ticoraw_dng::Error::InvalidMetadata(_)
        | ticoraw_dng::Error::TooLarge(_)
        | ticoraw_dng::Error::Preview(_) => Dng::new_err(e.to_string()),
    }
}

/// An `OSError` built the way Python's own `open()` builds one, so that it arrives as the
/// matching subclass (`FileNotFoundError`, `PermissionError`, ...) with `filename` set.
fn os_error(e: &std::io::Error, path: &Path) -> PyErr {
    let name = path.display().to_string();
    match e.raw_os_error() {
        Some(code) => {
            let full = e.to_string();
            let suffix = format!(" (os error {code})");
            let msg = full.strip_suffix(&suffix).unwrap_or(&full).to_owned();
            // On Windows the raw code is a Win32 error: Python derives errno from `winerror`.
            #[cfg(windows)]
            {
                PyOSError::new_err((0, msg, name, code))
            }
            #[cfg(not(windows))]
            {
                PyOSError::new_err((code, msg, name))
            }
        }
        None => PyOSError::new_err(format!("{name}: {e}")),
    }
}

/// Where the NEF bytes come from.
enum Source<'py> {
    /// An immutable `bytes` object, read in place.
    Bytes(Bound<'py, PyBytes>),
    /// A copy of a mutable buffer (`bytearray`), so no Python thread can change it while the
    /// GIL is released.
    Copied(Vec<u8>),
    /// A file, read with the GIL released.
    Path(PathBuf),
}

impl<'py> Source<'py> {
    fn parse(obj: &Bound<'py, PyAny>) -> PyResult<Self> {
        if let Ok(b) = obj.cast::<PyBytes>() {
            return Ok(Self::Bytes(b.clone()));
        }
        if let Ok(b) = obj.cast::<PyByteArray>() {
            return Ok(Self::Copied(b.to_vec()));
        }
        if obj.is_instance_of::<PyString>() || obj.hasattr("__fspath__")? {
            return obj.extract::<PathBuf>().map(Self::Path);
        }
        Err(PyTypeError::new_err(format!(
            "source must be a path (str or os.PathLike), bytes or bytearray, not {}",
            obj.get_type().name()?
        )))
    }

    fn path(&self) -> Option<&Path> {
        match self {
            Self::Path(p) => Some(p),
            Self::Bytes(_) | Self::Copied(_) => None,
        }
    }

    /// Runs `f` over the NEF bytes with the GIL released, reading the file first for a path.
    fn with_bytes<T, F>(&self, py: Python<'_>, f: F) -> PyResult<T>
    where
        T: Send,
        F: FnOnce(&[u8]) -> T + Send,
    {
        match self {
            Self::Bytes(b) => {
                let data = b.as_bytes();
                Ok(py.detach(|| f(data)))
            }
            Self::Copied(v) => Ok(py.detach(|| f(v))),
            Self::Path(p) => py
                .detach(|| std::fs::read(p).map(|d| f(&d)))
                .map_err(|e| os_error(&e, p)),
        }
    }
}

/// A decoded HE/HE* raw image: the CFA mosaic plus what is needed to interpret it.
///
/// Attribute names follow rawpy where the meaning is the same (`raw_image`, `raw_pattern`,
/// `color_desc`, `black_level_per_channel`, `white_level`, `camera_whitebalance`).
#[pyclass(module = "ticoraw", frozen)]
#[allow(clippy::struct_field_names)] // `raw_image` is rawpy's name.
struct RawImage {
    /// The full CFA mosaic, `height x width` `uint16` (14-bit values, no black subtracted).
    #[pyo3(get)]
    raw_image: Py<PyArray2<u16>>,
    #[pyo3(get)]
    width: usize,
    #[pyo3(get)]
    height: usize,
    #[pyo3(get)]
    make: String,
    #[pyo3(get)]
    model: String,
    /// DNG `CFAPattern` codes (0 = R, 1 = G, 2 = B) as `((top_left, top_right), (bottom_left,
    /// bottom_right))`.
    #[pyo3(get)]
    cfa_pattern: ((u8, u8), (u8, u8)),
    /// rawpy's `raw_pattern`: 2x2 indices into `color_desc` (`b"RGBG"`), so the green that
    /// shares a row with blue is 3.
    #[pyo3(get)]
    raw_pattern: Py<PyArray2<u8>>,
    /// Black level per `color_desc` channel (R, G, B, G2), like rawpy.
    #[pyo3(get)]
    black_level_per_channel: [u16; 4],
    #[pyo3(get)]
    white_level: u16,
    /// XYZ(D65) -> camera 3x3 matrix (DNG `ColorMatrix1`), or `None` for unknown bodies.
    #[pyo3(get)]
    color_matrix: Option<Py<PyArray2<f64>>>,
    /// As-shot white balance multipliers (R, G, B, G2) normalised to G = 1, or `None`.
    #[pyo3(get)]
    camera_whitebalance: Option<[f64; 4]>,
    #[pyo3(get)]
    orientation: u16,
    /// Recommended crop `(x, y, width, height)` inside the mosaic, or `None`.
    #[pyo3(get)]
    crop: Option<(usize, usize, usize, usize)>,
    /// Exposure (EV) a raw processor should add, as in DNG `BaselineExposure`, or `None`.
    #[pyo3(get)]
    baseline_exposure: Option<f64>,
}

#[pymethods]
impl RawImage {
    /// The channel letters `raw_pattern` indexes into: always `b"RGBG"`.
    #[classattr]
    fn color_desc(py: Python<'_>) -> Py<PyBytes> {
        PyBytes::new(py, b"RGBG").unbind()
    }

    fn __repr__(&self) -> String {
        format!(
            "RawImage(make={:?}, model={:?}, width={}, height={})",
            self.make, self.model, self.width, self.height
        )
    }
}

impl RawImage {
    fn new(py: Python<'_>, raw: DecodedRaw) -> PyResult<Self> {
        let DecodedRaw {
            make,
            model,
            image,
            color,
            orientation,
            crop,
            baseline_exposure,
        } = raw;
        let (width, height) = (image.width, image.height);
        let mosaic = Array2::from_shape_vec((height, width), image.pixels).map_err(|e| {
            TicoError::new_err(format!("decoded mosaic does not match its size: {e}"))
        })?;
        let pattern = rawpy_pattern(color.cfa_pattern);
        let raw_pattern = Array2::from_shape_vec((2, 2), pattern.to_vec())
            .map_err(|e| TicoError::new_err(format!("raw_pattern: {e}")))?;
        let color_matrix = match color.color_matrix.and_then(matrix_f64) {
            Some(m) => Some(
                Array2::from_shape_vec((3, 3), m.to_vec())
                    .map_err(|e| TicoError::new_err(format!("color_matrix: {e}")))?
                    .into_pyarray(py)
                    .unbind(),
            ),
            None => None,
        };
        let p = color.cfa_pattern;
        Ok(Self {
            // Moves the Vec into numpy without copying it.
            raw_image: mosaic.into_pyarray(py).unbind(),
            width,
            height,
            make,
            model,
            cfa_pattern: ((p[0], p[1]), (p[2], p[3])),
            raw_pattern: raw_pattern.into_pyarray(py).unbind(),
            black_level_per_channel: per_channel(pattern, color.black_level),
            white_level: color.white_level,
            color_matrix,
            camera_whitebalance: color.as_shot_neutral.and_then(white_balance),
            orientation,
            crop: crop.map(|c| (c.x, c.y, c.width, c.height)),
            baseline_exposure: baseline_exposure.and_then(|(n, d)| ratio(n, d)),
        })
    }
}

/// rawpy's `raw_pattern` for DNG CFA codes: R = 0, G = 1, B = 2, and 3 for the green in the
/// row that holds blue (`color_desc` is `RGBG`).
fn rawpy_pattern(cfa: [u8; 4]) -> [u8; 4] {
    let mut out = cfa;
    for p in 0..4 {
        let neighbour = cfa[p ^ 1];
        if cfa[p] == 1 && neighbour == 2 {
            out[p] = 3;
        }
    }
    out
}

/// Re-indexes per-2x2-position values by rawpy channel index.
fn per_channel(pattern: [u8; 4], per_position: [u16; 4]) -> [u16; 4] {
    let mut out = [0; 4];
    for (&channel, &value) in pattern.iter().zip(&per_position) {
        if let Some(slot) = out.get_mut(usize::from(channel)) {
            *slot = value;
        }
    }
    out
}

/// White-balance multipliers (R, G, B, G2) from an as-shot neutral, normalised to G = 1.
fn white_balance(neutral: [(u32, u32); 3]) -> Option<[f64; 4]> {
    let [r, g, b] = neutral.map(|(n, d)| ratio(i64::from(n), i64::from(d)).filter(|v| *v > 0.0));
    let (r, g, b) = (r?, g?, b?);
    Some([g / r, 1.0, g / b, 1.0])
}

fn ratio(n: impl Into<i64>, d: impl Into<i64>) -> Option<f64> {
    let d = d.into();
    (d != 0).then(|| n.into() as f64 / d as f64)
}

fn matrix_f64(m: [(i32, i32); 9]) -> Option<[f64; 9]> {
    let mut out = [0.0; 9];
    for (slot, (n, d)) in out.iter_mut().zip(m) {
        *slot = ratio(n, d)?;
    }
    Some(out)
}

/// Decodes a Nikon HE/HE* NEF.
///
/// `source` is a path (`str` or `os.PathLike`) or the file's contents (`bytes` or
/// `bytearray`). The GIL is released while the file is read and decoded.
#[pyfunction]
fn read(py: Python<'_>, source: &Bound<'_, PyAny>) -> PyResult<RawImage> {
    let src = Source::parse(source)?;
    let raw = src
        .with_bytes(py, ticoraw::decode_nef)?
        .map_err(|e| core_err(py, &e))?;
    RawImage::new(py, raw)
}

/// Reports container and codestream facts of a NEF without decoding pixels.
#[pyfunction]
fn inspect<'py>(py: Python<'py>, source: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyDict>> {
    let src = Source::parse(source)?;
    let info = src
        .with_bytes(py, ticoraw::inspect_nef)?
        .map_err(|e| core_err(py, &e))?;
    inspection_dict(py, &info)
}

fn inspection_dict<'py>(py: Python<'py>, info: &NefInspection) -> PyResult<Bound<'py, PyDict>> {
    let d = PyDict::new(py);
    d.set_item("make", &info.make)?;
    d.set_item("model", &info.model)?;
    d.set_item(
        "unique_camera_model",
        ticoraw::unique_camera_model(&info.make, &info.model),
    )?;
    d.set_item("width", info.width)?;
    d.set_item("height", info.height)?;
    d.set_item("bits_per_sample", info.bits_per_sample)?;
    d.set_item(
        "codec",
        match info.codec {
            RawCodec::TicoRaw => "ticoraw",
            RawCodec::Classic => "classic",
        },
    )?;
    d.set_item(
        "compression",
        info.compression.map(|c| match c {
            NikonCompression::HighEfficiency => "HE",
            NikonCompression::HighEfficiencyStar => "HE*",
            NikonCompression::Lossless => "lossless",
            NikonCompression::Other(_) => "other",
        }),
    )?;
    d.set_item(
        "compression_code",
        info.compression.map(NikonCompression::code),
    )?;
    d.set_item("strip_offset", info.strip_offset)?;
    d.set_item("strip_len", info.strip_len)?;
    d.set_item("black_level", info.black_level)?;
    let codestream = match &info.codestream {
        Some(cs) => {
            let c = PyDict::new(py);
            c.set_item("frame_width", cs.header.pih.wf)?;
            c.set_item("frame_height", cs.header.pih.hf)?;
            c.set_item("components", cs.header.components.len())?;
            c.set_item("component_width", cs.component_width)?;
            c.set_item("component_height", cs.component_height)?;
            c.set_item("slices", cs.slices)?;
            c.set_item("precincts", cs.precincts)?;
            c.set_item("bands", cs.bands)?;
            let curve = PyDict::new(py);
            curve.set_item("black", cs.curve.black)?;
            curve.set_item("gain", cs.curve.gain)?;
            curve.set_item("knee", cs.curve.knee)?;
            c.set_item("transfer_curve", curve)?;
            c.set_item("unsupported", cs.unsupported.as_deref())?;
            Some(c)
        }
        None => None,
    };
    d.set_item("codestream", codestream)?;
    Ok(d)
}

fn parse_compression(name: &str) -> PyResult<Compression> {
    match name {
        "lossless" => Ok(Compression::LosslessJpeg),
        "uncompressed" => Ok(Compression::Uncompressed),
        other => Err(PyValueError::new_err(format!(
            "compression must be 'lossless' or 'uncompressed', not {other:?}"
        ))),
    }
}

fn xmp_bytes(obj: &Bound<'_, PyAny>) -> PyResult<Vec<u8>> {
    if let Ok(s) = obj.cast::<PyString>() {
        return Ok(s.to_cow()?.as_bytes().to_vec());
    }
    if let Ok(b) = obj.cast::<PyBytes>() {
        return Ok(b.as_bytes().to_vec());
    }
    Err(PyTypeError::new_err(format!(
        "xmp must be str or bytes, not {}",
        obj.get_type().name()?
    )))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// Converts a Nikon HE/HE* NEF into a complete DNG.
///
/// Returns the DNG as `bytes` when `dest` is `None`; otherwise writes it to `dest`
/// (replacing any existing file) and returns `None`.
#[pyfunction]
#[pyo3(signature = (source, dest=None, *, compression="lossless", preview=true, xmp=None))]
fn to_dng<'py>(
    py: Python<'py>,
    source: &Bound<'py, PyAny>,
    dest: Option<PathBuf>,
    compression: &str,
    preview: bool,
    xmp: Option<&Bound<'py, PyAny>>,
) -> PyResult<Option<Bound<'py, PyBytes>>> {
    let src = Source::parse(source)?;
    let opts = Options {
        compression: parse_compression(compression)?,
        preview: if preview {
            Preview::Auto
        } else {
            Preview::None
        },
        extra_xmp: xmp.map(xmp_bytes).transpose()?,
        original_raw_file_name: src
            .path()
            .and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .map(str::to_owned),
        ..Options::default()
    };
    if let (Some(dest), Some(nef)) = (&dest, src.path()) {
        if same_file(nef, dest) {
            return Err(PyValueError::new_err(format!(
                "dest {} is the source NEF; refusing to overwrite it",
                dest.display()
            )));
        }
    }
    let dng = src
        .with_bytes(py, |nef| ticoraw_dng::convert(nef, &opts))?
        .map_err(|e| dng_err(py, &e))?;
    match dest {
        None => Ok(Some(PyBytes::new(py, &dng))),
        Some(dest) => {
            py.detach(|| std::fs::write(&dest, &dng))
                .map_err(|e| os_error(&e, &dest))?;
            Ok(None)
        }
    }
}

/// Bit-exact decoder for Nikon High Efficiency (HE/HE*) NEF raw files, with a DNG writer.
#[pymodule]
#[pyo3(name = "_ticoraw")]
fn ticoraw_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<RawImage>()?;
    m.add_function(wrap_pyfunction!(read, m)?)?;
    m.add_function(wrap_pyfunction!(inspect, m)?)?;
    m.add_function(wrap_pyfunction!(to_dng, m)?)?;
    m.add("TicoError", py.get_type::<TicoError>())?;
    m.add("Truncated", py.get_type::<Truncated>())?;
    m.add("Container", py.get_type::<Container>())?;
    m.add("Header", py.get_type::<Header>())?;
    m.add("Unsupported", py.get_type::<Unsupported>())?;
    m.add("Entropy", py.get_type::<Entropy>())?;
    m.add("Jpeg", py.get_type::<Jpeg>())?;
    m.add("Dng", py.get_type::<Dng>())?;
    Ok(())
}
