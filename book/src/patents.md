# Patents and downloads

Short version: **the source code is free to read, build and use, and that is how ticoraw is
installed today. Ready-made downloads are planned, but are waiting on a review of patent
licensing.** Nothing here is cause for alarm, and nothing has gone wrong. This page explains
why, in plain words.

## Why there are no downloads yet

HE and HE\* files are compressed with **TicoRAW**, which builds on an international standard
called **JPEG XS** (ISO/IEC 21122).

- JPEG XS is covered by patents. Companies that ship JPEG XS decoders can license those
  patents from a patent pool, run by Vectis IP (<https://www.jpegxspool.com/>), with
  Fraunhofer-Gesellschaft and intoPIX among the licensors.
- intoPIX, which created TicoRAW, also describes it as patented technology.
- The pool licenses decoders **per copy**. So handing out ready-made, compiled copies of a
  decoder (an app, a download, a package) is the kind of distribution such licences are
  about.

ticoraw publishes **source code**, for interoperability and research: so that photographers
can read their own raw files, and so that the format is documented. Before the project
hands out compiled copies itself, the licensing question is being reviewed properly.

## What is available, and what is coming

| Channel | Status |
|---|---|
| Source code on GitHub | **Available** |
| Building it yourself with `cargo install` ([install guide](photographers/install.md)) | **Available** |
| Rust crates as Git dependencies ([Rust guide](libraries/rust.md)) | **Available** |
| Building the C library, Python package or JavaScript package yourself | **Available** (see the [library guides](libraries/index.md)) |
| Homebrew formula that builds from source on your Mac | Planned: ready, but the project's Homebrew tap is not published yet |
| Source crates on crates.io | Planned |
| Prebuilt `ticoraw` downloads for macOS, Windows and Linux (and Scoop and winget packages) | **Coming, pending licensing** |
| Python wheels on PyPI (`pip install ticoraw`) | **Coming, pending licensing** |
| npm package (`npm install ticoraw`) | **Coming, pending licensing** |
| A converter that runs in your web browser, on this website | **Coming, pending licensing** |

The project's CI already builds and tests the binaries, wheels and npm package, but does
not publish them until the review allows it. When that changes, this page and the install
guide will say so.

## Patent notice

**Please read this before using or distributing this software.**

- JPEG XS (ISO/IEC 21122) is covered by essential patents. A patent pool for JPEG XS is
  administered by Vectis IP (<https://www.jpegxspool.com/>), with Fraunhofer-Gesellschaft
  and intoPIX among the licensors.
- intoPIX describes TicoRAW as patented technology and holds patents on it.
- This project provides **source code** for interoperability and research purposes: so that
  photographers can read their own raw files and so that the format is documented. It does
  not grant, and cannot grant, any patent licence. The MIT and Apache-2.0 licences of this
  project cover its copyright only.
- For this reason the project does not distribute compiled binaries.
- **You are responsible** for determining whether you need a patent licence to use,
  build, or distribute this software or binaries made from it in your jurisdiction, and
  for obtaining one if so.
- This notice is not legal advice.

The same notice is in the project's
[README](https://github.com/lukaj99/ticoraw/blob/main/README.md#patent-notice).

## What this means for you

- **Photographers** who build ticoraw for their own use: the install guide is all you need.
  Whether a licence applies to you depends on where you live, and only a lawyer can tell
  you for certain.
- **Developers** who want to ship ticoraw inside their own product: you will be distributing
  compiled decoders, so check the licensing situation for your product first.
- **Nothing changes for your photos.** Whatever the review decides, DNGs you have already
  made stay ordinary DNG files.
