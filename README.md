# ticoraw

Convert Nikon High Efficiency (HE/HE\*) raw files to DNGs that open everywhere — free,
private, bit-exact.

[![CI](https://github.com/lukaj99/ticoraw/actions/workflows/ci.yml/badge.svg)](https://github.com/lukaj99/ticoraw/actions/workflows/ci.yml)
[![Conformance](https://github.com/lukaj99/ticoraw/actions/workflows/conformance.yml/badge.svg)](https://github.com/lukaj99/ticoraw/actions/workflows/conformance.yml)
[![Licence: MIT OR Apache-2.0](https://img.shields.io/badge/licence-MIT%20OR%20Apache--2.0-blue.svg)](#licence)

Recent Nikon Z cameras can save raw files in **High Efficiency** (HE) and **High
Efficiency\*** (HE\*) modes. Many raw editors cannot open these files yet. `ticoraw` turns
them into standard DNG files, on your own computer, with exactly the pixel values Adobe's
converter produces.

> **Install from source for now.** Prebuilt downloads, PyPI and npm packages and a web
> converter are planned, but they are on hold while a patent-licensing question is
> reviewed (see the [patent notice](#patent-notice)). Building from source takes one
> command, shown below.

## Who is this for?

- **Photographers** whose editor will not open HE/HE\* files: see the
  [quick start](#quick-start-for-photographers).
- **Power users** who convert whole cards or script their workflow: see the
  [command-line tool](#the-command-line-tool).
- **Developers** who want HE/HE\* support in their own app: see the
  [libraries](#libraries) for Rust, C, Python and JavaScript.
- **Format experts** who build or audit decoders: see [docs/FORMAT.md](docs/FORMAT.md) and
  the [conformance kit](conformance/README.md).

## Quick start for photographers

You type a few commands in a terminal (Terminal on macOS, PowerShell on Windows). The
[step-by-step install guide](https://lukaj99.github.io/ticoraw/photographers/install.html)
walks through each one and says what you should see.

**1. Install Rust**, the tool that builds `ticoraw`, and the system tools it needs to link
programs:

- **macOS:** first install Apple's command line tools, then Rust:

  ```sh
  xcode-select --install
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

  `xcode-select` opens a dialog: click **Install** and wait. If it says the tools are
  already installed, go on.
- **Linux:** first install a C compiler (and curl) with your package manager, then Rust:

  ```sh
  sudo apt install build-essential curl        # Debian, Ubuntu, Mint
  sudo dnf install gcc curl                    # Fedora
  sudo pacman -S --needed base-devel curl      # Arch, Manjaro
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

- **Windows:** download and run `rustup-init.exe` from <https://rustup.rs>. If it says you
  need the **Visual Studio C++ Build Tools** (Microsoft's C++ compiler, MSVC), let it
  install them with the **Desktop development with C++** workload.

Choose the default installation when `rustup` asks. Then **close the terminal and open a
new one**, so that it finds `cargo`. `cargo --version` should print 1.85 or higher.

**2. Install `ticoraw`.** This downloads the source code and builds it, which takes a few
minutes the first time:

```sh
cargo install --git https://github.com/lukaj99/ticoraw ticoraw-cli
```

**3. Convert your card.** Point `ticoraw convert` at the card (or any folder) and say where
the DNGs should go:

```sh
ticoraw convert --recursive /Volumes/NIKON/DCIM -o ~/Pictures/DNG     # macOS
ticoraw convert --recursive E:\DCIM -o C:\Photos\DNG                  # Windows
```

`ticoraw` keeps the card's folder structure, skips files that are not HE/HE\* (your editor
already reads those), and never changes your originals. It prints one line per file and a
summary at the end.

**Then open the DNGs** in your editor. If a DNG does not open or looks wrong, please
[tell us](https://github.com/lukaj99/ticoraw/issues/new?template=bug_report.yml).

To update later, run step 2 again with `--force` at the end.

## What you get

Each DNG is meant to stand in for the one Adobe DNG Converter would write:

- **Identical pixels.** Every photosite equals Adobe's output, on every camera listed below.
- **Lossless.** The raw data is stored with lossless compression; nothing is thrown away.
- **Your metadata.** The camera's EXIF (date, exposure, lens), GPS position, XMP (ratings,
  keywords; or your Lightroom sidecar with `--with-sidecar`) and the Nikon MakerNote.
- **Previews.** A thumbnail and a large preview (the camera's own JPEG when there is one),
  so file browsers show the picture.
- **Adobe's rendering metadata.** Colour matrix, white and black levels, baseline
  exposure, default crop, orientation and Adobe's camera name, all equal to the values
  Adobe writes, plus the camera's as-shot white balance. Because the camera name matches,
  Lightroom and Camera Raw apply their own camera profile.
- **About Adobe's size** (checked on a Z6III HE\* frame): 1.2 to 2.2 times the size of the
  NEF on the public samples (see [performance](#performance)).

What a DNG contains, tag by tag: [crates/ticoraw-dng/README.md](crates/ticoraw-dng/README.md).

## Compatibility

Every Nikon body known to write HE/HE\* is supported. Each entry was checked photosite by
photosite against **Adobe DNG Converter 18.7**, together with the levels, colour matrix,
baseline exposure, orientation, crop and camera name.

| Body | HE | HE\* |
|---|---|---|
| Z 9 | Verified | Verified |
| Z 8 | Verified | Verified |
| Z f | Verified | Verified |
| Z6III | Verified | Verified, plus 11 more photos against Adobe Lightroom |
| Z 5 II | Verified (DX crop) | Verified |
| Z 50 II | Verified (1:1 crop) | Verified |
| ZR | No public sample yet | Verified (HLG tone mode) |

The tests cover FX, DX, 1:1 and 16:9 frames. Details, sample sources and known gaps:
[docs/compat.md](docs/compat.md). Have a body, firmware or mode that is not listed? A
sample helps everyone: see [docs/SAMPLES.md](docs/SAMPLES.md).

Classic NEF compression (lossless, lossy, uncompressed) is out of scope: almost every raw
editor already opens those files. Codestreams outside the supported profile are rejected
with a clear error rather than decoded wrongly.

## The command-line tool

| Command | What it does | Example |
|---|---|---|
| `convert` | Convert files or whole folders to complete DNGs | `ticoraw convert --recursive /Volumes/NIKON/DCIM -o ~/Pictures/DNG` |
| `decode` | Convert one file, or write its raw mosaic as a 16-bit PGM | `ticoraw decode DSC_0001.NEF` |
| `inspect` | Describe a file without decoding it | `ticoraw inspect --json DSC_0001.NEF` |
| `verify` | Compare a decode with a reference DNG, photosite by photosite | `ticoraw verify DSC_0001.NEF DSC_0001.dng` |
| `scan` | List the HE/HE\* and classic NEFs in a folder | `ticoraw scan --recursive ~/Pictures/import` |
| `report` | Print a privacy-safe JSON report for a bug report | `ticoraw report DSC_0001.NEF` |

Useful `convert` options: `--skip-existing` to resume, `--force` to overwrite,
`--with-sidecar` to use Lightroom's `.xmp` sidecar, `--json` for scripts and `-j` to set
how many files run at once. Every command explains itself with `--help`. Exit status is 0
on success, 1 when a file failed and 2 for a usage error.

The full reference, with every flag: [crates/ticoraw-cli/README.md](crates/ticoraw-cli/README.md).

## Libraries

All bindings share one Rust core, so they all produce the same, bit-exact output. The
Rust, C and JavaScript libraries take bytes and return bytes; the Python package also
accepts file paths.

| Language | Where | How to get it today |
|---|---|---|
| Rust | [`ticoraw`](crates/ticoraw/README.md) (decoder), [`ticoraw-dng`](crates/ticoraw-dng/README.md) (DNG writer) | Git dependency; not on crates.io yet |
| C and C++ | [`ticoraw-capi`](crates/ticoraw-capi/README.md), header `include/ticoraw.h` | Build from source with `cargo build -p ticoraw-capi --release` |
| Python | [`crates/ticoraw-py`](crates/ticoraw-py/README.md), numpy arrays, rawpy-style names | `pip install` from a clone; PyPI wheels pending licensing review |
| JavaScript | [`crates/ticoraw-wasm`](crates/ticoraw-wasm/README.md), WebAssembly for browsers and Node.js | Build with wasm-pack; npm package pending licensing review |

In Rust:

```toml
[dependencies]
ticoraw = { git = "https://github.com/lukaj99/ticoraw" }
ticoraw-dng = { git = "https://github.com/lukaj99/ticoraw" }
```

```rust
let nef = std::fs::read("DSC_0001.NEF")?;

// Decode to the raw mosaic plus levels, colour, orientation and crop.
let raw = ticoraw::decode_nef(&nef)?;
println!("{} {}x{}", raw.model, raw.image.width, raw.image.height);

// Or convert straight to a complete DNG.
let dng = ticoraw_dng::convert(&nef, &ticoraw_dng::Options::default())?;
std::fs::write("DSC_0001.dng", dng)?;
```

The decoder has no `unsafe` code and no dependencies without its `parallel` feature, and it
builds for `wasm32-unknown-unknown`. Malformed input returns an error; it never panics.

## Performance

Measured on an Apple M5 Pro (15 cores), release build:

| Task | Time |
|---|---|
| Decode one 24.5 MP frame (Z6III), all cores | about 30 to 35 ms |
| Decode the same frame, one core | about 285 ms |
| Decode and write a complete DNG of that frame, including file I/O | about 75 ms |
| `ticoraw convert` on the 13 public samples (251 MB of NEFs, 11 to 46 MP each) | 0.7 to 0.8 s: 14 to 19 files per second |

| Sample | NEF | DNG |
|---|---|---|
| Z6III HE, 24.5 MP | 13.4 MB | 28.4 MB |
| Z 9 HE\*, 45.7 MP | 35.6 MB | 46.7 MB |
| ZR HE\*, 24.5 MP | 15.8 MB | 19.0 MB |
| All 13 samples | 250.8 MB | 405.1 MB |

HE files grow more than HE\* files: the camera stores HE at a lower bit rate, but a
lossless DNG stores the full decoded image, which is about the same size in both modes. A Z6III HE\* frame comes out at about the
size of Adobe's own DNG. In a browser or Node.js, the WebAssembly build converts a 24.5 MP
frame in about 0.6 s on one thread
([details](crates/ticoraw-wasm/README.md#performance-and-size)).

## How it is verified

- **Ground truth.** Real NEFs are compared with DNGs that Adobe DNG Converter and
  Lightroom wrote from them, at every photosite. Adobe tools are used only as black boxes.
  The suite runs whenever `TICORAW_CORPUS` names a folder of pairs
  ([CONTRIBUTING.md](CONTRIBUTING.md#running-the-tests)).
- **Conformance kit.** [conformance/](conformance/README.md) pins 13 public CC0 samples
  and the Adobe-verified hash of each decoded image. CI checks them on every pull request
  and downloads them afresh every week. Other decoders can use the same kit.
- **Fuzzing.** Nine [cargo-fuzz targets](fuzz/README.md) run every night against the
  container, codestream, DNG and lossless JPEG parsers.
- **Benchmarks.** [Criterion benchmarks](crates/ticoraw/benches/decode.rs) measure speed,
  and an instruction-count gate fails any pull request that makes decoding execute more
  than 5% more instructions ([bench.yml](.github/workflows/bench.yml)).
- **Supply chain.** [cargo-deny](deny.toml) checks advisories, licences and sources.

## Documentation

- The documentation website, <https://lukaj99.github.io/ticoraw/>, is published from the
  `book/` folder of this repository: guides for each kind of user.
- [docs/FORMAT.md](docs/FORMAT.md): the HE/HE\* format, fully written up.
- [docs/compat.md](docs/compat.md): cameras, samples and how they were checked.
- [docs/superpowers/specs/2026-10-27-ticoraw-design.md](docs/superpowers/specs/2026-10-27-ticoraw-design.md):
  how the code is organised and why.
- [CHANGELOG.md](CHANGELOG.md): what changed in each version.

## Patent notice

**Please read this before using or distributing this software.**

- JPEG XS (ISO/IEC 21122), the codec inside HE/HE\* files, is covered by essential
  patents. A patent pool for JPEG XS is administered by Vectis IP
  (<https://www.jpegxspool.com/>), with Fraunhofer-Gesellschaft and intoPIX among the
  licensors.
- intoPIX describes TicoRAW as patented technology and holds patents on it.
- This project provides **source code** for interoperability and research purposes: so
  that photographers can read their own raw files and so that the format is documented.
  It does not grant, and cannot grant, any patent licence. The MIT and Apache-2.0 licences
  of this project cover its copyright only.
- For this reason the project does not distribute compiled decoders today: no prebuilt
  binaries, Python wheels, npm packages or hosted web converter. These are on hold pending
  a patent-licensing review, and may follow if that review allows it. The release
  workflows build and test them but publish nothing compiled
  ([packaging/README.md](packaging/README.md#the-patent-gate)).
- **You are responsible** for determining whether you need a patent licence to use,
  build, or distribute this software or binaries made from it in your jurisdiction, and
  for obtaining one if so.
- This notice is not legal advice.

## Contributing and getting help

Contributions of every size are welcome, and you do not need to know anything about
wavelets to start. [CONTRIBUTING.md](CONTRIBUTING.md) explains the setup, the tests and the
bit-exact rule.

- Questions and help: [SUPPORT.md](SUPPORT.md) and
  [Discussions](https://github.com/lukaj99/ticoraw/discussions).
- Bugs, camera support and ideas: the
  [issue forms](https://github.com/lukaj99/ticoraw/issues/new/choose). For a file that
  misbehaves, paste the output of `ticoraw report <file>`; it contains nothing personal.
- Security problems: privately, as described in [SECURITY.md](SECURITY.md).
- Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## Acknowledgements

`ticoraw` is a clean-room implementation: no code was copied from the projects below, and
in particular none from LGPL-licensed sources. It would not exist without the public work
of:

- the authors of ISO/IEC 21122 and of **SVT-JPEG-XS**
  (<https://github.com/OpenVisualCloud/SVT-JPEG-XS>), for an open account of JPEG XS;
- **LibRaw PR #826** (<https://github.com/LibRaw/LibRaw/pull/826>) and **RdWing**, whose
  review established the exact transfer curve and verified it against Adobe;
- **darktable PR #22743** (<https://github.com/darktable-org/darktable/pull/22743>);
- **dnglab PR #835** (<https://github.com/dnglab/dnglab/pull/835>);
- Richter, Fößel, Descampe and Rouvroy, "Bayer CFA Pattern Compression With JPEG XS",
  IEEE TIP 30 (2021), <https://doi.org/10.1109/TIP.2021.3095421>;
- the photographers who published CC0 samples on [raw.pixls.us](https://raw.pixls.us/)
  and [RawDB](https://rawdb.dnglab.org/).

[docs/FORMAT.md](docs/FORMAT.md#21-sources-and-acknowledgements) says what each
contributed.

## Licence

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT licence ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option. Third-party licences are listed in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in the work by you, as defined in the Apache-2.0 licence, shall be dual licensed
as above, without any additional terms or conditions.

## Trademarks

Nikon is a trademark of Nikon Corporation. intoPIX and TicoRAW are trademarks of intoPIX
SA. Adobe, Lightroom and DNG are trademarks of Adobe Inc. Topaz Photo AI is a trademark of
Topaz Labs. All other names are the property of their respective owners. This project is
independent and is not affiliated with, endorsed by, or sponsored by any of them.
