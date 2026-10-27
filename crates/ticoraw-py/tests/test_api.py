"""API and error-path tests that need no camera files."""

import pathlib
import random
import re

import pytest

import ticoraw

ERRORS = [
    ticoraw.Truncated,
    ticoraw.Container,
    ticoraw.Header,
    ticoraw.Unsupported,
    ticoraw.Entropy,
    ticoraw.Jpeg,
    ticoraw.Dng,
]

# A little-endian TIFF with one empty IFD: a valid container, but not a Nikon file.
EMPTY_TIFF = b"II*\x00\x08\x00\x00\x00" + b"\x00\x00" + b"\x00\x00\x00\x00"


def test_version():
    assert re.fullmatch(r"\d+\.\d+\.\d+(\S*)", ticoraw.__version__)


@pytest.mark.parametrize("cls", ERRORS, ids=lambda c: c.__name__)
def test_exception_hierarchy(cls):
    assert issubclass(cls, ticoraw.TicoError)
    assert issubclass(ticoraw.TicoError, Exception)
    assert cls.__module__ == "ticoraw"


def test_raw_image_is_not_constructible():
    with pytest.raises(TypeError):
        ticoraw.RawImage()


def test_color_desc_is_a_class_attribute():
    assert ticoraw.RawImage.color_desc == b"RGBG"


@pytest.mark.parametrize("func", [ticoraw.read, ticoraw.inspect, ticoraw.to_dng])
def test_empty_input_is_truncated(func):
    with pytest.raises(ticoraw.Truncated) as info:
        func(b"")
    assert info.value.what == "TIFF header"
    assert info.value.offset == 0
    assert "truncated" in str(info.value)


@pytest.mark.parametrize("func", [ticoraw.read, ticoraw.inspect, ticoraw.to_dng])
def test_non_nikon_tiff_is_a_container_error(func):
    with pytest.raises(ticoraw.Container, match="not a Nikon file"):
        func(EMPTY_TIFF)


def test_bytearray_source():
    with pytest.raises(ticoraw.Container):
        ticoraw.read(bytearray(EMPTY_TIFF))


@pytest.mark.parametrize("seed", range(64))
def test_garbage_never_escapes_tico_error(seed):
    rng = random.Random(seed)
    size = rng.choice([1, 7, 8, 64, 1000, 65536])
    data = bytes(rng.getrandbits(8) for _ in range(size))
    # Half the cases start with a TIFF header, so parsing gets past the first check.
    if seed % 2:
        data = (b"II*\x00" if seed % 4 == 1 else b"MM\x00*") + data
    for func in (ticoraw.read, ticoraw.inspect, ticoraw.to_dng):
        with pytest.raises(ticoraw.TicoError):
            func(data)


@pytest.mark.parametrize("bad", [1, 2.5, None, ["a.nef"], memoryview(b"")])
def test_wrong_source_type(bad):
    with pytest.raises(TypeError, match="source must be"):
        ticoraw.read(bad)


def test_missing_file(tmp_path):
    missing = tmp_path / "missing.NEF"
    for source in (missing, str(missing)):
        with pytest.raises(FileNotFoundError) as info:
            ticoraw.read(source)
        assert info.value.filename == str(missing)


def test_directory_is_an_os_error(tmp_path):
    with pytest.raises(OSError):
        ticoraw.inspect(tmp_path)


def test_garbage_file(tmp_path):
    path = tmp_path / "garbage.NEF"
    path.write_bytes(b"\x00" * 4096)
    with pytest.raises(ticoraw.Container):
        ticoraw.read(pathlib.Path(path))


def test_to_dng_rejects_bad_options():
    with pytest.raises(ValueError, match="compression"):
        ticoraw.to_dng(EMPTY_TIFF, compression="zip")
    with pytest.raises(TypeError, match="xmp"):
        ticoraw.to_dng(EMPTY_TIFF, xmp=123)
    with pytest.raises(TypeError):
        ticoraw.to_dng(EMPTY_TIFF, None, "lossless")  # options are keyword-only


def test_to_dng_failure_writes_nothing(tmp_path):
    dest = tmp_path / "out.dng"
    with pytest.raises(ticoraw.Container):
        ticoraw.to_dng(EMPTY_TIFF, dest)
    assert not dest.exists()
