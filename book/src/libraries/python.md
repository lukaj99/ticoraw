# Python

The `ticoraw` Python package reads HE/HE\* NEFs into numpy arrays and converts them to
DNG. It is built with [pyo3](https://pyo3.rs) and [maturin](https://www.maturin.rs) from
the `crates/ticoraw-py` folder of the repository.

> [!NOTE]
> `pip install ticoraw` from PyPI does not work yet: prebuilt wheels are pending a
> patent-licensing review (see [Patents and downloads](../patents.md)). Installing from a
> clone, as shown below, builds the package on your computer with maturin. It needs Rust
> 1.85 or later from [rustup.rs](https://rustup.rs) and Python 3.9 or later.

The rest of this page is the package's README.

{{#include ../../../crates/ticoraw-py/README.md:2:}}
