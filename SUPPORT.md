# Getting help

Thanks for trying `ticoraw`. Here is where to go, depending on what you need.

| You want to... | Go to |
|---|---|
| Ask a question, get help installing, share how you use `ticoraw` | [GitHub Discussions](https://github.com/lukaj99/ticoraw/discussions) |
| Report something that does not work | [Bug report](https://github.com/lukaj99/ticoraw/issues/new?template=bug_report.yml) |
| Get a camera, firmware or mode supported, or offer a sample | [Camera support](https://github.com/lukaj99/ticoraw/issues/new?template=camera_support.yml) (and [docs/SAMPLES.md](docs/SAMPLES.md)) |
| Suggest a feature | [Feature request](https://github.com/lukaj99/ticoraw/issues/new?template=feature_request.yml) |
| Report a security problem | Privately, as described in [SECURITY.md](SECURITY.md); never in a public issue |
| Contribute code or documentation | [CONTRIBUTING.md](CONTRIBUTING.md) |

`ticoraw` has one maintainer who works on it in their spare time, so answers can take a
few days. Discussions are public, so other users may answer sooner.

## Before you ask

A few quick checks solve most problems:

1. **Is your file actually High Efficiency?** `ticoraw` only decodes Nikon **HE** and
   **HE\*** NEFs. Run `ticoraw inspect <file.NEF>` and look at the `codec` line. If it
   says `classic Nikon (not TicoRAW)`, the file uses Nikon's older lossless, lossy or
   uncompressed formats: `ticoraw` is the wrong tool, and almost every raw editor already
   opens that file.
2. **Is your camera listed?** The README's status table and
   [docs/compat.md](docs/compat.md) list the bodies verified so far. Others may work but
   are unverified: please tell us through the camera support form.
3. **Is your copy up to date?** `ticoraw --version` shows what you have. Reinstall with
   `cargo install --git https://github.com/lukaj99/ticoraw ticoraw-cli --force` to get the
   latest `main`.
4. **Does your Rust toolchain meet the minimum?** Building needs Rust 1.85 or later
   (`rustc --version`; update with `rustup update`).

## Prebuilt binaries

For now the project ships source code only: prebuilt downloads, PyPI and npm packages and
a hosted web converter are planned but on hold while JPEG XS patent licensing is reviewed
(see the patent notice in the README). The README explains how to build `ticoraw`
yourself with one command; there is no need to open an issue asking for binaries.

## When you post

- Paste the output of `ticoraw --version` and, for a file that misbehaves,
  `ticoraw inspect --json <file.NEF>`. That output has no pixels and no serial number,
  but its `path` field shows where the file lives on your computer: shorten it if it
  contains your name.
- Paste text, not screenshots of text.
- Please **do not attach NEF files** to issues or discussions. They contain your camera's
  serial number and often more. If a sample would help, the
  [sample guide](docs/SAMPLES.md) explains how to share one safely under CC0.

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
