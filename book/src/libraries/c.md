# C and C++

The `ticoraw-capi` crate builds ticoraw as a C library with a stable ABI and one header,
[`ticoraw.h`](https://github.com/lukaj99/ticoraw/blob/main/crates/ticoraw-capi/include/ticoraw.h).
Use it from C, C++, or any language with a C foreign-function interface (Go, Swift, Zig,
C#, ...).

> [!NOTE]
> Prebuilt libraries are not published yet, pending a patent-licensing review (see
> [Patents and downloads](../patents.md)). Build the library from source, as shown below:
> it takes one `cargo` command.

The rest of this page is the crate's README.

{{#include ../../../crates/ticoraw-capi/README.md:2:}}

## The complete example

[`examples/convert.c`](https://github.com/lukaj99/ticoraw/blob/main/crates/ticoraw-capi/examples/convert.c)
converts one NEF to DNG with the C API:

```c
{{#include ../../../crates/ticoraw-capi/examples/convert.c}}
```
