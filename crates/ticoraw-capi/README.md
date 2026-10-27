# ticoraw-capi

A stable C ABI for [`ticoraw`](https://github.com/lukaj99/ticoraw), the bit-exact decoder
for Nikon **High Efficiency (HE)** and **High Efficiency\* (HE\*)** NEFs, and for
`ticoraw-dng`, its complete DNG writer. Use it from C, C++, or any language with a C FFI.

Like the Rust crates, the library does no I/O: bytes in, bytes out.

```c
#include "ticoraw.h"

TrDngOptions opts = {0};
opts.struct_size = sizeof opts;
opts.original_raw_file_name = "DSC_0001.NEF";

TrBuffer *dng = NULL;
if (tr_nef_to_dng(nef, nef_len, &opts, &dng) != TR_STATUS_OK) {
  fprintf(stderr, "ticoraw: %s\n", tr_last_error_message());
  return 1;
}
fwrite(tr_buffer_data(dng), 1, tr_buffer_len(dng), out);
tr_buffer_free(dng);
```

[`examples/convert.c`](examples/convert.c) is a complete NEF-to-DNG converter.

## Patent gate: source only

**Compiled builds of this library are not published, pending patent-licensing review.**
JPEG XS, which TicoRAW codestreams use, is covered by essential patents; see the
[patent notice](https://github.com/lukaj99/ticoraw#patent-notice). There are no prebuilt
libraries, packages or release assets. Build the library yourself from source; whether you
need a patent licence to build, use or distribute it is your responsibility.

The CI workflow (`.github/workflows/capi.yml`) builds and tests the library and keeps the
results as short-lived workflow artifacts only. Any future publish, deploy or release-asset
step must be guarded by both a manual `workflow_dispatch` input and the repository variable
`ALLOW_BINARY_DISTRIBUTION == 'true'`, which is unset.

## Building

```sh
cargo build -p ticoraw-capi --release
```

This writes, in `target/release/`:

| Platform | Static library | Shared library |
|---|---|---|
| Linux | `libticoraw_capi.a` | `libticoraw_capi.so` |
| macOS | `libticoraw_capi.a` | `libticoraw_capi.dylib` |
| Windows (MSVC) | `ticoraw_capi.lib` | `ticoraw_capi.dll` + import library `ticoraw_capi.dll.lib` |

The header is [`include/ticoraw.h`](include/ticoraw.h). Linking the static library also
needs the system libraries Rust's standard library uses; ask rustc for the exact list:

```sh
cargo rustc -p ticoraw-capi --release --lib --crate-type staticlib -- --print native-static-libs
```

Typical results are `-lSystem -lc -lm` on macOS and `-lgcc_s -lutil -lrt -lpthread -lm -ldl
-lc` on Linux, so for example:

```sh
cc -std=c99 -I crates/ticoraw-capi/include crates/ticoraw-capi/examples/convert.c \
   target/release/libticoraw_capi.a -lpthread -ldl -lm -o convert
```

The default `parallel` feature decodes and encodes on all cores with a rayon thread pool.
Build with `--no-default-features` for a single-threaded library; the output is identical.

## API

All names start with `tr_`. Fallible functions return a `TrStatus`; `TR_STATUS_OK` (0) is
success.

| Function | Purpose |
|---|---|
| `tr_version()`, `tr_abi_version()` | Library version string; ABI version (`TR_ABI_VERSION`) |
| `tr_decode_nef(data, len, &raw)` | Decode a HE/HE\* NEF into a `TrRaw` handle |
| `tr_raw_width/height/pixels/make/model/orientation/white_level(raw)` | Plain values; 0 or `NULL` for a `NULL` handle |
| `tr_raw_cfa_pattern/black_level/color_matrix/as_shot_neutral/crop/baseline_exposure(raw, out)` | Write into caller arrays; `TR_STATUS_NOT_PRESENT` when the value is absent |
| `tr_raw_free(raw)` | Release a `TrRaw` |
| `tr_nef_to_dng(data, len, opts, &buf)` | Convert to a complete DNG in a `TrBuffer` (`opts` may be `NULL`) |
| `tr_inspect_json(data, len, &buf)` | Describe a NEF without decoding pixels, as JSON |
| `tr_buffer_data/len(buf)`, `tr_buffer_free(buf)` | Read and release a `TrBuffer` |
| `tr_last_error_message()` | Message for the last failure on this thread |

`tr_raw_pixels` returns `width * height` 16-bit samples (14-bit values) in row-major order.
`tr_inspect_json` returns the same fields as `ticoraw inspect --json` (without `path`), and
its buffer is NUL-terminated after `tr_buffer_len` bytes.

### Status codes

| Code | Name | Meaning |
|---|---|---|
| 0 | `TR_STATUS_OK` | Success |
| 1 | `TR_STATUS_NULL_POINTER` | A required pointer was `NULL` |
| 2 | `TR_STATUS_INVALID_ARGUMENT` | Unknown option value, `struct_size` too small, non-UTF-8 string |
| 3 | `TR_STATUS_NOT_PRESENT` | Optional value absent (not an error; no message recorded) |
| 10-15 | `TRUNCATED`, `CONTAINER`, `HEADER`, `UNSUPPORTED`, `ENTROPY`, `JPEG` | The decoder's `TicoError` variants |
| 20-23 | `INVALID_IMAGE`, `INVALID_METADATA`, `TOO_LARGE`, `PREVIEW` | The DNG writer's `Error` variants |
| 100 | `TR_STATUS_PANIC` | A bug: a Rust panic was caught at the boundary. Please report it |

Messages are the Rust errors' `Display` text, for example
`truncated TIFF u16 at byte 3570`. A classic (non-HE) NEF fails with `TR_STATUS_UNSUPPORTED`.

### Memory, threads and lifetimes

- Handles are created by the library and released only with their `tr_*_free` function.
  Freeing `NULL` is a no-op. Pointers returned by accessors (`tr_raw_pixels`,
  `tr_raw_model`, `tr_buffer_data`, ...) stay valid until the handle is freed.
- Input buffers are only read during the call; the library keeps no reference to them.
- On failure, `*out` handle arguments are set to `NULL`, and the outputs of the
  `tr_raw_*(raw, out)` accessors are left untouched.
- Every function is thread-safe. A `TrRaw` is immutable, so several threads may read one
  handle at once; it must not be freed while another thread uses it.
- `tr_last_error_message()` is per thread and valid until the next call into the library
  on the same thread.
- No Rust panic unwinds into the caller: every function catches panics and returns
  `TR_STATUS_PANIC`. Running out of memory aborts the process, as in any Rust program.

## ABI stability

- `TR_ABI_VERSION` (and `tr_abi_version()`) increase whenever a function, status code,
  constant or struct field is added. Nothing is removed or changes meaning.
- `TrDngOptions` starts with `uint32_t struct_size`. Fields are only appended, and a new
  field's zero value always means the earlier behaviour, so a zero-initialised struct with
  `struct_size = sizeof(TrDngOptions)` gives the defaults on every version. A library
  rejects a `struct_size` smaller than its version-1 struct and ignores fields beyond the
  ones it knows.
- Enumerations cross the boundary as fixed-width integers (`TrStatus` is `int32_t`,
  option values `uint32_t`); treat unknown status codes as generic failures.

## Regenerating the header

The header is generated with [cbindgen](https://github.com/mozilla/cbindgen) as a
development tool; building the library does not need it. After changing the API:

```sh
cargo install cbindgen --version 0.29.4 --locked
crates/ticoraw-capi/scripts/gen-header.sh          # rewrite include/ticoraw.h
crates/ticoraw-capi/scripts/gen-header.sh --check  # what CI runs: fail if stale
```

Commit the regenerated header with the change.

## Tests

```sh
cargo test -p ticoraw-capi --release                     # Rust tests of the extern functions
crates/ticoraw-capi/scripts/c-test.sh                    # C test + convert example, static library
SANITIZE=address CC=clang crates/ticoraw-capi/scripts/c-test.sh   # under ASan and UBSan
TICORAW_CORPUS=/path/to/nefs crates/ticoraw-capi/scripts/c-test.sh  # also real NEFs (read only)
```

The C test ([`tests/c/test_capi.c`](tests/c/test_capi.c)) runs on a synthetic NEF that
`cargo run --example synthetic_nef` writes, and checks its pixels against the hash that
helper prints. CI runs it on Linux, macOS and Windows, and under ASan, UBSan and LSan on
Linux.

## Safety

The crate's own lint table denies `unsafe_code`; it is allowed only in `src/ffi.rs`, where
every `unsafe` block carries a `// SAFETY:` comment (enforced by clippy's
`undocumented_unsafe_blocks`) and `unsafe_op_in_unsafe_fn` is denied. Everything else
(option parsing, status mapping, the JSON writer) is safe Rust in `src/lib.rs`.

## Licence

MIT OR Apache-2.0, like the rest of the workspace. The DNG writer's preview encoder
dependency, [`jpeg-encoder`](https://crates.io/crates/jpeg-encoder), is
`(MIT OR Apache-2.0) AND IJG`: binary distributions must state that the software is based in
part on the work of the Independent JPEG Group.
