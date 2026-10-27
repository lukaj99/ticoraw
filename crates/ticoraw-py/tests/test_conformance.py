"""Real decodes of the public conformance samples against conformance/manifest.toml.

Runs when TICORAW_CONFORMANCE_DIR names a folder filled by conformance/fetch.sh (one
`<id>.nef` per sample); skipped otherwise. With rawpy installed, the DNGs written by
`to_dng` are also read back through LibRaw and compared with the decode.
"""

import concurrent.futures
import errno
import hashlib
import io
import os
import pathlib

import numpy as np
import pytest
import ticoraw

try:
    import tomllib
except ImportError:  # Python < 3.11
    try:
        import tomli as tomllib
    except ImportError:
        tomllib = None

try:
    import rawpy
except ImportError:
    rawpy = None

try:
    import resource
except ImportError:  # Windows
    resource = None

MANIFEST = pathlib.Path(__file__).resolve().parents[3] / "conformance" / "manifest.toml"
SAMPLE_DIR = os.environ.get("TICORAW_CONFORMANCE_DIR")


def load_samples():
    if tomllib is None or not MANIFEST.is_file():
        return []
    with MANIFEST.open("rb") as f:
        return tomllib.load(f)["sample"]


SAMPLES = load_samples()


def cfa_sha256(mosaic):
    return hashlib.sha256(np.ascontiguousarray(mosaic, dtype="<u2").tobytes()).hexdigest()


@pytest.fixture(params=SAMPLES or [None], ids=lambda s: s["id"] if s else "no-manifest")
def sample(request):
    s = request.param
    if s is None:
        pytest.skip("manifest unavailable (needs tomllib or tomli)")
    if not SAMPLE_DIR:
        pytest.skip("TICORAW_CONFORMANCE_DIR is not set")
    path = pathlib.Path(SAMPLE_DIR) / f"{s['id']}.nef"
    if not path.is_file():
        pytest.skip(f"{path} is missing; run conformance/fetch.sh")
    return s, path


def test_manifest_is_readable():
    if tomllib is None:
        pytest.skip("needs tomllib or tomli")
    assert len(SAMPLES) >= 13
    for s in SAMPLES:
        assert len(s["cfa_sha256"]) == 64


def test_decode_matches_manifest(sample):
    s, path = sample
    raw = ticoraw.read(path)
    img = raw.raw_image
    assert img is raw.raw_image
    assert not img.flags.owndata  # zero-copy: numpy views the decoder's own buffer
    assert img.flags.c_contiguous
    assert img.dtype == np.uint16
    assert img.shape == (s["height"], s["width"])
    assert (raw.width, raw.height) == (s["width"], s["height"])
    assert cfa_sha256(img) == s["cfa_sha256"]

    codes = [c for row in raw.cfa_pattern for c in row]
    assert "".join("RGB"[c] for c in codes) == s["cfa_pattern"]
    pattern = raw.raw_pattern.ravel().tolist()
    black_per_position = [raw.black_level_per_channel[c] for c in pattern]
    assert black_per_position == s["black_level"]
    assert raw.white_level == s["white_level"]
    assert raw.orientation == s["orientation"]
    assert list(raw.crop) == s["crop"]
    assert raw.baseline_exposure == pytest.approx(s["baseline_exposure"], abs=0.005)
    assert raw.color_matrix is not None and raw.color_matrix.shape == (3, 3)
    assert raw.camera_whitebalance is not None and raw.camera_whitebalance[1] == 1.0

    info = ticoraw.inspect(path)
    assert info["codec"] == "ticoraw"
    assert info["compression"] == s["mode"]
    assert info["unique_camera_model"] == s["unique_camera_model"]
    assert (info["width"], info["height"]) == (s["width"], s["height"])


def test_sources_agree(sample):
    s, path = sample
    data = path.read_bytes()
    for source in (str(path), data, bytearray(data)):
        assert cfa_sha256(ticoraw.read(source).raw_image) == s["cfa_sha256"]


def test_threads_decode_in_parallel(sample):
    s, path = sample
    with concurrent.futures.ThreadPoolExecutor(4) as pool:
        digests = list(pool.map(lambda _: cfa_sha256(ticoraw.read(path).raw_image), range(4)))
    assert digests == [s["cfa_sha256"]] * 4


@pytest.mark.parametrize("compression", ["lossless", "uncompressed"])
def test_to_dng_round_trip(sample, compression):
    s, path = sample
    dng = ticoraw.to_dng(path, compression=compression, preview=compression == "lossless")
    assert isinstance(dng, bytes)
    assert dng[:4] in (b"II*\x00", b"MM\x00*")
    if rawpy is None:
        return
    with rawpy.imread(io.BytesIO(dng)) as r:
        assert cfa_sha256(r.raw_image) == s["cfa_sha256"]
        assert r.raw_pattern.tolist() == ticoraw.read(path).raw_pattern.tolist()
        assert r.white_level == s["white_level"]


def test_to_dng_writes_dest(sample, tmp_path):
    _s, path = sample
    dest = tmp_path / "out.dng"
    xmp = '<x:xmpmeta xmlns:x="adobe:ns:meta/"/>'
    assert ticoraw.to_dng(path, dest, xmp=xmp) is None
    data = dest.read_bytes()
    assert xmp.encode() in data
    assert path.name.encode() in data  # OriginalRawFileName, set for path sources only
    assert ticoraw.to_dng(path, xmp=xmp.encode()) == data
    from_bytes = ticoraw.to_dng(path.read_bytes(), xmp=xmp)
    assert xmp.encode() in from_bytes
    assert len(from_bytes) < len(data)


def test_to_dng_refuses_to_overwrite_source(sample, tmp_path):
    _s, path = sample
    copy = tmp_path / path.name
    copy.write_bytes(path.read_bytes())
    with pytest.raises(ValueError, match="source NEF"):
        ticoraw.to_dng(copy, copy)
    assert copy.read_bytes() == path.read_bytes()


def test_to_dng_replaces_dest_atomically(sample, tmp_path):
    _s, path = sample
    dest = tmp_path / "out.dng"
    dest.write_bytes(b"old")
    ticoraw.to_dng(path, dest)
    good = dest.read_bytes()
    assert good == ticoraw.to_dng(path)
    assert [p.name for p in tmp_path.iterdir()] == ["out.dng"]  # no temporary file left
    if resource is None:
        return
    # A write that fails part-way (here: a 1 MB file-size limit; Python ignores SIGXFSZ)
    # must leave the previous DNG intact and clean up after itself.
    soft, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
    resource.setrlimit(resource.RLIMIT_FSIZE, (1 << 20, hard))
    try:
        with pytest.raises(OSError) as info:
            ticoraw.to_dng(path, dest, compression="uncompressed")
    finally:
        resource.setrlimit(resource.RLIMIT_FSIZE, (soft, hard))
    assert info.value.errno == errno.EFBIG
    assert info.value.filename == str(dest)
    assert dest.read_bytes() == good
    assert [p.name for p in tmp_path.iterdir()] == ["out.dng"]


def test_to_dng_empty_xmp_keeps_the_nef_xmp(sample):
    _s, path = sample
    default = ticoraw.to_dng(path)
    assert ticoraw.to_dng(path, xmp=b"") == default
    assert ticoraw.to_dng(path, xmp="") == default
