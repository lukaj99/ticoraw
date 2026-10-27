"""Bit-exact decoder for Nikon High Efficiency (HE/HE*) NEF raw files, with a DNG writer.

>>> import ticoraw
>>> raw = ticoraw.read("DSC_0001.NEF")       # raw.raw_image is a numpy uint16 mosaic
>>> ticoraw.to_dng("DSC_0001.NEF", "DSC_0001.dng")
"""

from ._ticoraw import (
    Container,
    Dng,
    Entropy,
    Header,
    Jpeg,
    RawImage,
    TicoError,
    Truncated,
    Unsupported,
    __version__,
    inspect,
    read,
    to_dng,
)

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
