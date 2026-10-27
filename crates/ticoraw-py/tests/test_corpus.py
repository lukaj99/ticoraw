"""Ground truth against DNGs converted by Adobe software, read independently through rawpy.

Runs when TICORAW_CORPUS names a folder of NEFs and DNGs made from them (the folder is only
read). Each DNG is paired with its NEF through the DNG's OriginalRawFileName tag, or else
by file stem. Pairs whose NEF is not HE/HE*, and DNGs that hold no CFA mosaic (for example
linear "Enhanced" DNGs), are skipped. The rawpy comparisons need rawpy installed.
"""

import os
import pathlib
import struct

import numpy as np
import pytest
import ticoraw

try:
    import rawpy
except ImportError:
    rawpy = None

CORPUS = os.environ.get("TICORAW_CORPUS")
ORIGINAL_RAW_FILE_NAME = 0xC68B


def original_raw_file_name(dng):
    """The OriginalRawFileName tag of IFD0, or None. Reads only the first MiB."""
    with open(dng, "rb") as f:
        head = f.read(1 << 20)
    order = {b"II": "<", b"MM": ">"}.get(head[:2])
    if order is None:
        return None
    try:
        (ifd,) = struct.unpack_from(order + "I", head, 4)
        (count,) = struct.unpack_from(order + "H", head, ifd)
        for i in range(count):
            entry = ifd + 2 + 12 * i
            tag, _typ, n, value = struct.unpack_from(order + "HHII", head, entry)
            if tag == ORIGINAL_RAW_FILE_NAME:
                data = head[entry + 8 : entry + 8 + n] if n <= 4 else head[value : value + n]
                return data.split(b"\0")[0].decode("utf-8", "replace") or None
    except struct.error:
        return None
    return None


def find_nef(dng):
    names = {p.name.lower(): p for p in dng.parent.iterdir()}
    original = original_raw_file_name(dng)
    if original and original.lower() in names:
        return names[original.lower()]
    for ext in (".nef", ".nrw"):
        candidate = names.get(dng.stem.lower() + ext)
        if candidate:
            return candidate
    return None


def corpus_dngs():
    if not CORPUS:
        return []
    return sorted(p for p in pathlib.Path(CORPUS).iterdir() if p.suffix.lower() == ".dng")


DNGS = corpus_dngs()


@pytest.fixture(params=DNGS or [None], ids=lambda p: p.name if p else "no-corpus")
def pair(request):
    dng = request.param
    if dng is None:
        pytest.skip("TICORAW_CORPUS is not set or holds no DNG")
    nef = find_nef(dng)
    if nef is None:
        pytest.skip(f"no NEF for {dng.name}")
    if ticoraw.inspect(nef)["codec"] != "ticoraw":
        pytest.skip(f"{nef.name} is not HE/HE*")
    return nef, dng


def test_decodes(pair):
    nef, _dng = pair
    raw = ticoraw.read(nef)
    info = ticoraw.inspect(nef)
    assert raw.raw_image.shape == (info["height"], info["width"])
    assert int(raw.raw_image.max()) <= 16383


def test_matches_reference_dng_through_rawpy(pair):
    if rawpy is None:
        pytest.skip("rawpy is not installed")
    nef, dng = pair
    raw = ticoraw.read(nef)
    with rawpy.imread(str(dng)) as ref:
        if ref.raw_pattern is None or ref.raw_image.shape != raw.raw_image.shape:
            pytest.skip(f"{dng.name} holds no CFA mosaic of the NEF's size")
        mismatches = int(np.count_nonzero(ref.raw_image != raw.raw_image))
        assert mismatches == 0, f"{mismatches} photosites differ"
        assert ref.raw_pattern.tolist() == raw.raw_pattern.tolist()
        assert list(ref.black_level_per_channel) == raw.black_level_per_channel
        assert ref.white_level == raw.white_level
        wb = ref.camera_whitebalance
        assert raw.camera_whitebalance[:3] == pytest.approx(
            [wb[0] / wb[1], 1.0, wb[2] / wb[1]], rel=1e-3
        )
        # LibRaw leaves the matrix at zero for bodies newer than its own tables.
        if np.any(ref.rgb_xyz_matrix):
            np.testing.assert_allclose(raw.color_matrix, ref.rgb_xyz_matrix[:3], atol=1e-4)
        s = ref.sizes
        if raw.crop is not None:
            assert raw.crop == (s.crop_left_margin, s.crop_top_margin, s.crop_width, s.crop_height)


def test_classic_nef_is_unsupported():
    if not CORPUS:
        pytest.skip("TICORAW_CORPUS is not set")
    nefs = sorted(p for p in pathlib.Path(CORPUS).iterdir() if p.suffix.lower() == ".nef")
    for nef in nefs[:50]:
        if ticoraw.inspect(nef)["codec"] == "classic":
            with pytest.raises(ticoraw.Unsupported):
                ticoraw.read(nef)
            return
    pytest.skip("no classic NEF among the first 50")
