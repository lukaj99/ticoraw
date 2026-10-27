# Security policy

`ticoraw` reads raw files that come from anywhere: memory cards, downloads, email
attachments, web uploads. The library is meant to be safe to run on a file you do not
trust, inside a server or a browser. If you find a file that breaks that promise, please
tell us privately first, so a fix can ship before the problem is public.

## How to report

Use GitHub's private vulnerability reporting:

1. Go to <https://github.com/lukaj99/ticoraw/security/advisories/new> (or the
   **Security** tab of the repository, then **Report a vulnerability**).
2. Describe what happens, which version or commit you tested (`ticoraw --version`, or
   `git rev-parse HEAD` for a clone) and your platform.
3. Attach the smallest input that triggers it. A crafted or truncated file is ideal;
   please do not attach personal photos (see [what to send](#what-to-send)).

Please do **not** open a public issue, discussion or pull request for a suspected
vulnerability. Only the maintainer can see a private report.

The project has one maintainer, so responses are best-effort. You should normally hear
back within a week. Once the problem is confirmed, we agree a disclosure date with you, fix
it on a private branch, publish the fix and a GitHub security advisory (with a CVE where
one is warranted), and credit you unless you prefer otherwise.

## Scope

In scope: anything that untrusted input can do to the `ticoraw` library or the `ticoraw`
command-line tool beyond returning an error. In particular:

- **Panics** (including integer-overflow panics in debug builds, index out of bounds,
  `unwrap` on bad data) from any public function, e.g. `decode_nef`, `inspect_nef`,
  `inspect_nef_prefix`, `decode_ticoraw`, `dng::decode_dng_cfa`,
  `dng::decode_lossless_jpeg`;
- **Memory safety** problems of any kind. The crates forbid `unsafe` code, so this would
  most likely be a bug in a dependency or in the Rust toolchain, and we still want to know;
- **Denial of service:** a small input that makes the decoder allocate far more memory
  than its size justifies, hang, recurse without bound or take far longer than a normal
  frame of the same dimensions;
- **CLI file handling:** `ticoraw` writing outside the output path it was given, leaving a
  partial file, overwriting its own input, or reading files that `scan` is documented to
  skip (non-regular files, symbolic links to directories).

Out of scope (please open a normal issue instead):

- a crafted file that decodes without error to **wrong or garbage pixels**. This is
  documented behaviour: the inverse transforms use wrapping arithmetic, so a stream that
  passes every check can produce nonsense, but it cannot crash;
- a genuine camera file that decodes differently from Adobe's converter (a correctness
  bug; see the bug report form);
- slow decoding of a large but valid frame;
- problems in other software that opens DNGs written by `ticoraw`;
- patent and licensing questions (see the patent notice in the README).

## Supported versions

Fixes go into the `main` branch and the next release. `ticoraw` is before 1.0, so only the
latest release is supported; there are no backports to older versions.

## What to send

Raw files carry personal data such as the camera's serial number, and sometimes GPS
location (see [docs/SAMPLES.md](docs/SAMPLES.md#what-a-nef-file-contains)). A security
report almost never needs a real photo: a fuzzer output, a hand-edited header or a
truncated file shows the bug just as well. If only a real photo reproduces it, say so in
the report and we will work out together how to share it.
