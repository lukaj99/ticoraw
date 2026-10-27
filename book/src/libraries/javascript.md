# JavaScript and WebAssembly

The `ticoraw` JavaScript package is the Rust decoder and DNG writer compiled to
WebAssembly, for browsers (with a Web Worker pool) and Node.js. It is built with
[wasm-pack](https://github.com/drager/wasm-pack) from the `crates/ticoraw-wasm` folder of the
repository.

> [!NOTE]
> `npm install ticoraw` from the npm registry does not work yet: the package is pending a
> patent-licensing review (see [Patents and downloads](../patents.md)). Build it from
> source, as shown below, and install the folder it writes.

The rest of this page is the package's README.

{{#include ../../../crates/ticoraw-wasm/README.md:2:}}
