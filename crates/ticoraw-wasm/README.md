# ticoraw for JavaScript (WebAssembly)

Decode Nikon **High Efficiency (HE)** and **High Efficiency\* (HE\*)** NEF raw files, and
convert them to complete DNGs, in the browser or in Node.js. This is the
[`ticoraw`](https://github.com/lukaj99/ticoraw) decoder and the `ticoraw-dng` writer
compiled to WebAssembly:

- **Bit-exact:** the mosaic equals the CFA image Adobe's converters write, on every public
  sample of every HE/HE\* body (Z 9, Z 8, Z f, Z6III, Z 5 II, Z 50 II, ZR).
- **Complete DNGs:** lossless JPEG tiles, EXIF, GPS, XMP, the Nikon MakerNote and a
  preview.
- **Private:** everything runs on the user's device. Apart from loading its own `.wasm`
  file, the module does no I/O and makes no network requests.
- **Small:** 226 KB of wasm (95 KB gzipped), no dependencies.

> **Not published (pending patent-licensing review).** JPEG XS, the codec inside HE/HE\*
> files, is licensed per decoder copy through a patent pool, and compiled decoders count as
> distribution. Until that review is done the npm package is built and tested in CI but
> **not published**: `package.json` is marked `"private"`, and the publish job in
> `.github/workflows/wasm.yml` runs only when started by hand **and** the repository
> variable `ALLOW_BINARY_DISTRIBUTION` is `'true'`. Build it from source, as below. This is
> not legal advice.

## Building

You need Rust with the `wasm32-unknown-unknown` target and
[wasm-pack](https://github.com/drager/wasm-pack) 0.15 (`cargo install wasm-pack`).

```sh
rustup target add wasm32-unknown-unknown
crates/ticoraw-wasm/scripts/build.sh          # writes crates/ticoraw-wasm/pkg
npm install ./crates/ticoraw-wasm/pkg         # in your project
```

The package, named `ticoraw`, contains:

| Path | What |
|---|---|
| `web/` | ESM build for browsers, Web Workers, Deno and bundlers (`import "ticoraw"` outside Node) |
| `node/` | CommonJS build for Node.js 18+ (`import "ticoraw"` or `require("ticoraw")` in Node) |
| `worker.js`, `index.d.ts` | `ticoraw/worker`: a pool of module Web Workers |
| `LICENSE-MIT`, `LICENSE-APACHE`, `THIRD-PARTY-NOTICES.md` | licences, and the notices of every bundled crate |

## API

| Function | Returns |
|---|---|
| `toDng(bytes, options?)` | a complete DNG as a `Uint8Array` |
| `decode(bytes)` | `{ width, height, pixels: Uint16Array, cfaPattern, blackLevel, whiteLevel, colorMatrix, asShotNeutral, orientation, crop, baselineExposure, make, model, uniqueCameraModel }` |
| `inspect(bytes)` | container and codestream facts without decoding pixels |
| `inspectPrefix(prefix, fileLength)` | the same from the first megabyte or two of a file |
| `decodeDng(bytes)` | the CFA mosaic of a DNG, e.g. to check a conversion |
| `version()` | the library version |

`bytes` is a `Uint8Array` (a Node `Buffer` works), an `ArrayBuffer` such as
`await file.arrayBuffer()`, or any other typed array or `DataView`; anything else throws a
`TypeError`. Every call is synchronous and runs on
the calling thread, so in a page use the worker pool below. Full types are in
`web/ticoraw.d.ts`.

`pixels` holds the 14-bit values the camera recorded, row-major, `width * height` of them:
no crop, black subtraction or scaling. `cfaPattern` is a string such as `"RGGB"`.

`toDng` options (all optional; an unknown key throws a `TypeError`):

| Key | Default | Meaning |
|---|---|---|
| `compression` | `"lossless-jpeg"` | `"uncompressed"` writes one 16-bit strip, 2 bytes per photosite |
| `preview` | `"auto"` | `"none"` skips the preview; the thumbnail is always written |
| `carryXmp` | `true` | copy the NEF's XMP packet |
| `extraXmp` | none | an XMP packet (`Uint8Array` or string) that replaces the NEF's; empty means none |
| `originalRawFileName` | none | written as `OriginalRawFileName`, usually the NEF's file name |

### Errors

Every decoding or conversion failure throws an `Error` whose `name` is `"TicoError"`:

```js
try {
  toDng(bytes);
} catch (e) {
  if (e.name === "TicoError") console.log(e.kind, e.message); // "Unsupported", "unsupported: ..."
  else throw e;
}
```

`kind` is the Rust error variant: `Truncated` (with `what` and `offset`), `Container`,
`Header`, `Unsupported` (for example a classic, non-HE NEF), `Entropy`, `Jpeg`,
`InvalidImage`, `InvalidMetadata`, `TooLarge` or `Preview`. `message` is the same text the
Rust library and the `ticoraw` CLI print. Bad arguments throw a `TypeError`.

The decoder does not panic on any input: it is fuzzed, and the tests feed this module
truncated and corrupted files. Should a bug panic anyway, the message goes to
`console.error` and the call throws a `WebAssembly.RuntimeError`; that instance is then
unusable (the worker pool replaces its worker). A panic never unwinds into JavaScript.

## In the browser, with Web Workers

Conversion takes about half a second per 24 MP frame, so run it off the main thread.
`ticoraw/worker` starts module workers on first use, sends each file to an idle one and
transfers the result back without a copy. Each worker converts one file at a time; use one
per core:

```js
import { TicorawWorker } from "ticoraw/worker";

const pool = new TicorawWorker({ size: navigator.hardwareConcurrency || 4 });

input.addEventListener("change", async () => {
  for (const file of input.files) {
    try {
      const dng = await pool.toDng(file, { originalRawFileName: file.name });
      const url = URL.createObjectURL(new Blob([dng], { type: "image/x-adobe-dng" }));
      Object.assign(document.createElement("a"), {
        href: url,
        download: file.name.replace(/\.nef$/i, ".dng"),
      }).click();
      URL.revokeObjectURL(url);
    } catch (e) {
      console.error(`${file.name}: ${e.kind ?? e.name}: ${e.message}`);
    }
  }
});
```

To convert a folder in parallel, start all the jobs and await them together
(`Promise.all(files.map((f) => pool.toDng(f)))`); the pool queues what it cannot start yet.
`pool.decode(file)` and `pool.inspect(file)` work the same way, and `pool.terminate()`
stops the workers.

The pool accepts a `File`/`Blob`, an `ArrayBuffer` or a typed array, and never detaches the
caller's buffer. It needs no `SharedArrayBuffer`, so no COOP/COEP headers: a static host
such as GitHub Pages works. Served as is, `worker.js` finds itself and `web/ticoraw_bg.wasm`
through `import.meta.url`. A bundler may move them; then pass the URL it gives the worker
file as `new TicorawWorker({ url })` (in Vite, for example, from a `?worker&url` import).

On the main thread, without workers, initialise the module once:

```js
import init, { inspect } from "ticoraw";
await init();
console.log(inspect(await file.arrayBuffer()).model);
```

## In Node.js

```js
import { readFileSync, writeFileSync } from "node:fs";
import { decode, toDng } from "ticoraw";

const nef = readFileSync("DSC_0001.NEF");
writeFileSync("DSC_0001.dng", toDng(nef, { originalRawFileName: "DSC_0001.NEF" }));

const raw = decode(nef);
console.log(raw.model, raw.width, raw.height, raw.cfaPattern, raw.blackLevel);
```

The Node build loads its wasm synchronously on import; there is nothing to initialise. It
is single-threaded: for batch conversion use
[`worker_threads`](https://nodejs.org/api/worker_threads.html), one module instance per
thread, or the native `ticoraw` CLI, which uses every core.

## Performance and size

Node.js 26, one thread, Apple M5 Pro, on the 13 public conformance samples
(`scripts/conformance.mjs`; repeated runs vary by about 10%):

| Frame | `decode` | `toDng` (decode included) |
|---|---|---|
| Z6III HE, 24.5 MP | 363 ms | 578 ms |
| Z 9 HE\*, 45.7 MP | 703 ms | 1001 ms |
| Mean | 15.8 ms/MP | 23.2 ms/MP |

In Chrome 156 on the same machine, a cold `toDng` of the Z6III frame took 0.88 s on the
main thread, and two fresh pool workers converted it twice and decoded it once in 0.86 s.

That is close to the native library on one thread (the Z6III frame takes 0.36 s with
`RAYON_NUM_THREADS=1 ticoraw decode`, file I/O included); natively it also scales across
cores. The first call in a fresh instance is up to 1.8 times slower while the wasm memory
grows and the engine optimises the code. Converting a 45 MP frame peaks at about 0.5 GB of
process memory in Node; wasm memory is reused by the next call but never returned to the
system.

Build settings, measured on the same samples:

| Build | `.wasm` | gzipped | `decode` | `toDng` |
|---|---|---|---|---|
| **opt-level 3, wasm-opt -Oz (used)** | 226,482 B | 94,706 B | 15.0-15.8 ms/MP | 21.9-23.2 ms/MP |
| opt-level 3, wasm-opt -O3 | 226,597 B | 94,602 B | 15.0 ms/MP | 21.9 ms/MP |
| opt-level "s", wasm-opt -Oz | 206,018 B | 90,823 B | 16.9 ms/MP | 27.1 ms/MP |
| opt-level 3, fat LTO, wasm-opt -O3 | 237,687 B | 97,371 B | 15.8 ms/MP | 22.3 ms/MP |

Opt-level 3 is 7% faster to decode and 15% faster to convert than "s", for 20 KB more wasm
(4 KB gzipped). wasm-opt -Oz and -O3 run at the same speed, and -Oz is smaller. LTO is off:
cargo does not apply it to a crate that is also an rlib, and a fat-LTO build was larger and
no faster. Panics abort (the only stable option on `wasm32-unknown-unknown`;
`wasm-pack --panic-unwind` needs nightly Rust).

## Testing

```sh
cargo test -p ticoraw-wasm                    # option parsing and error mapping, natively
wasm-pack test --node crates/ticoraw-wasm     # the JS API in a real wasm instance
conformance/fetch.sh ~/.cache/ticoraw-conformance
node crates/ticoraw-wasm/scripts/conformance.mjs crates/ticoraw-wasm/pkg ~/.cache/ticoraw-conformance
node crates/ticoraw-wasm/scripts/worker-test.mjs crates/ticoraw-wasm/pkg ~/.cache/ticoraw-conformance/z5ii_he_dx.nef
```

The wasm-bindgen tests use a synthetic NEF from the Rust test encoder, so they need no
samples. `conformance.mjs` checks every sample's CFA SHA-256 and metadata against
`conformance/manifest.toml`, and the SHA-256 of the DNG `toDng` writes. `worker-test.mjs`
runs `ticoraw/worker` under Node with a small shim for the Web Worker API.

## Licence

MIT OR Apache-2.0, like the rest of ticoraw. The DNG preview encoder,
[`jpeg-encoder`](https://crates.io/crates/jpeg-encoder), is `(MIT OR Apache-2.0) AND IJG`,
so a distributed build must say: this software is based in part on the work of the
Independent JPEG Group. The package carries that statement, the IJG licence and the
licences of every bundled crate in `THIRD-PARTY-NOTICES.md`.
