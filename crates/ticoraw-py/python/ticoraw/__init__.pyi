"""Bit-exact decoder for Nikon High Efficiency (HE/HE*) NEF raw files, with a DNG writer."""

import os
from typing import Any, Literal, final

import numpy as np
import numpy.typing as npt
from typing_extensions import TypeAlias

__all__ = [
    "Container",
    "Dng",
    "Entropy",
    "Header",
    "Jpeg",
    "RawImage",
    "TicoError",
    "Truncated",
    "Unsupported",
    "__version__",
    "inspect",
    "read",
    "to_dng",
]

__version__: str

_Source: TypeAlias = str | os.PathLike[str] | bytes | bytearray
"""A NEF: a path, or the file's contents."""

@final
class RawImage:
    """A decoded HE/HE* raw image. Attribute names follow rawpy where the meaning is the same."""

    color_desc: bytes
    """Channel letters that ``raw_pattern`` indexes into: always ``b"RGBG"``."""

    @property
    def raw_image(self) -> npt.NDArray[np.uint16]:
        """The whole CFA mosaic, shape ``(height, width)``, 14-bit values, no black subtracted."""
    @property
    def width(self) -> int: ...
    @property
    def height(self) -> int: ...
    @property
    def make(self) -> str: ...
    @property
    def model(self) -> str: ...
    @property
    def cfa_pattern(self) -> tuple[tuple[int, int], tuple[int, int]]:
        """DNG ``CFAPattern`` codes (0 = R, 1 = G, 2 = B), row-major: ``((0, 1), (1, 2))`` for RGGB."""
    @property
    def raw_pattern(self) -> npt.NDArray[np.uint8]:
        """rawpy's 2x2 pattern of indices into ``color_desc``: ``[[0, 1], [3, 2]]`` for RGGB."""
    @property
    def black_level_per_channel(self) -> list[int]:
        """Black level per ``color_desc`` channel (R, G, B, G2)."""
    @property
    def white_level(self) -> int: ...
    @property
    def color_matrix(self) -> npt.NDArray[np.float64] | None:
        """XYZ(D65) to camera 3x3 matrix (DNG ``ColorMatrix1``), or None for unknown bodies."""
    @property
    def camera_whitebalance(self) -> list[float] | None:
        """As-shot white-balance multipliers (R, G, B, G2) normalised to G = 1, or None."""
    @property
    def orientation(self) -> int:
        """TIFF Orientation (1 = normal, 6 / 8 = portrait)."""
    @property
    def crop(self) -> tuple[int, int, int, int] | None:
        """Recommended crop ``(x, y, width, height)`` inside the mosaic, or None."""
    @property
    def baseline_exposure(self) -> float | None:
        """EV a raw processor should add (DNG ``BaselineExposure``), or None."""

def read(source: _Source) -> RawImage:
    """Decode a Nikon HE/HE* NEF. The GIL is released while reading and decoding."""

def inspect(source: _Source) -> dict[str, Any]:
    """Container and codestream facts of a NEF, without decoding pixels."""

def to_dng(
    source: _Source,
    dest: str | os.PathLike[str] | None = None,
    *,
    compression: Literal["lossless", "uncompressed"] = "lossless",
    preview: bool = True,
    xmp: str | bytes | None = None,
) -> bytes | None:
    """Convert a NEF to a complete DNG.

    Returns the DNG as bytes when ``dest`` is None; otherwise writes it to ``dest`` and
    returns None. The write is atomic: a temporary file in the same folder is renamed over
    ``dest`` once complete, so a failure leaves an existing file untouched. ``ValueError`` if
    ``dest`` is the source NEF (also through a symlink or hard link). ``xmp`` replaces the
    NEF's XMP packet; an empty ``xmp`` counts as None and keeps it.
    """

class TicoError(Exception):
    """Base class of every error raised for input ticoraw cannot read, decode or convert."""

class Truncated(TicoError):
    """The input ended before a structure was complete."""

    what: str
    offset: int

class Container(TicoError):
    """The TIFF/NEF container is malformed, is not a Nikon file, or lacks a required tag."""

class Header(TicoError):
    """A JPEG XS marker segment of the codestream is malformed or out of order."""

class Unsupported(TicoError):
    """A classic (non-HE) NEF, or a codestream outside the supported profile."""

class Entropy(TicoError):
    """Entropy-coded data is inconsistent with its declared sizes."""

class Jpeg(TicoError):
    """A lossless JPEG stream is malformed."""

class Dng(TicoError):
    """The DNG could not be written."""
