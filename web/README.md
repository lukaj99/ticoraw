# ticoraw web converter

A single web page that converts Nikon **High Efficiency** (HE and HE\*) NEF files to DNG,
inside the browser. You drop in files or a whole folder, press Convert, and download the
DNGs one by one, as one ZIP file, or straight into a folder.

It is built on [`ticoraw-wasm`](../crates/ticoraw-wasm), the ticoraw decoder and DNG
writer compiled to WebAssembly. The page itself is plain HTML, CSS and ES modules: no
framework, no bundler and no runtime dependencies.

> **Not hosted yet (pending patent-licensing review).** JPEG XS, the codec inside HE/HE\*
> files, is licensed per decoder copy through a patent pool, and a hosted copy of this page
> would distribute a compiled decoder to every visitor. Until that review is done the
> project does not deploy it anywhere, and CI builds and tests it without uploading it. You
> can build it and run it on your own computer, as below. See the patent notice in the
> [top-level README](../README.md#patent-notice). This is not legal advice.

## Privacy

Everything happens on your device. The page reads the files you give it, converts them in
Web Workers and hands the DNGs back to you; it has no server side and sends nothing
anywhere. A Content-Security-Policy in `index.html` enforces that: the page may load only
its own files (`default-src 'self'`) and connect only to the site it came from
(`connect-src 'self'`). It loads no fonts, analytics or scripts from other sites. The
end-to-end tests check that no request leaves the page's own server.

## Build it and run it

You need:

- Rust 1.85 or later with the WebAssembly target: `rustup target add wasm32-unknown-unknown`
- [wasm-pack](https://github.com/drager/wasm-pack) 0.15: `cargo install wasm-pack`
- Node.js 22 or later, only to serve the page with the script below and to run the tests

From the repository root:

```sh
sh web/build.sh                          # writes web/dist
node web/scripts/serve.mjs web/dist 8080
```

Then open <http://127.0.0.1:8080/>. The server listens on your computer only.

`web/dist` is a complete static site. Any static web server works, as long as it serves
`.wasm` files as `application/wasm`. The page needs no special headers: it does not use
`SharedArrayBuffer`, so no COOP/COEP.

### Build settings

`build.sh` takes an optional output directory (default `web/dist`) and these environment
variables:

| Variable | Default | Meaning |
|---|---|---|
| `WASM_PACK` | `wasm-pack` | the wasm-pack binary |
| `BASE_PATH` | `./` | where the site's files live, ending in `/`. Every URL in the page is relative, so the default (no `<base>` element) works under any path that ends in `/`. Set an absolute path, such as `/ticoraw/convert/`, only if the page is also served from a URL without the trailing slash; `build.sh` then adds `<base href="/ticoraw/convert/">` |
| `DOCS_URL` | `SUPPORT.md` on GitHub | where "More questions and help" points; an `https://` URL |
| `BUILD_ID` | the short git commit, or `local` | shown in the footer |

`build.sh` only ever replaces a directory it created itself (it leaves a `.ticoraw-web`
marker file there).

### Hosting notes for later

When distribution is allowed, three things are worth knowing:

- Send the Content-Security-Policy as an HTTP header too, if the host allows it. A
  `<meta>` policy covers the page but not its Web Workers. The workers run only the
  decoder, which does no I/O, but a header covers them as well.
- A host must not inject scripts (analytics, banners): the policy blocks them.
- Deploying must stay behind the patent gate: the repository variable
  `ALLOW_BINARY_DISTRIBUTION` set to `'true'` and a manual run.

## What the page does

- **Adding files.** "Choose files", "Choose a folder" (`webkitdirectory`), or drag and drop
  files and folders anywhere on the page. Dropped folders are walked with
  `DataTransferItem.getAsFileSystemHandle` where it exists (Chromium) and
  `webkitGetAsEntry` elsewhere. Only `.nef` and `.nrw` files are kept; the page says how
  many other files it left out.
- **Checking each file.** Before converting, the page reads the start of each file (2 MB,
  more if the raw data starts further in) and asks the decoder what it is. Classic NEFs
  (lossless, lossy or uncompressed) are marked "nothing to convert", because every raw
  editor already opens them. HE/HE\* files the decoder cannot handle get a short
  explanation and a link to the
  [camera support form](https://github.com/lukaj99/ticoraw/issues/new?template=camera_support.yml).
- **Converting.** Files are converted in a pool of Web Workers (`ticoraw/worker`), one
  file per worker. The pool has one worker per CPU core, minus one for the page, at most one
  per 2 GB of device memory (`navigator.deviceMemory`, where the browser reports it) and at
  most 6. A 45 MP frame needs about 0.5 GB in its worker, and a worker keeps that memory
  for its next file. Each row shows how long its file took. "Stop" ends the workers; the
  remaining files can be converted again later.
- **Options.** Lossless JPEG or uncompressed raw data, full-size preview or thumbnail only,
  and whether to copy the NEF's XMP packet. These are the `toDng` options of
  `ticoraw-wasm`. The NEF's name is always written as `OriginalRawFileName`.
- **Output.** By default each DNG stays in the page as a `Blob` (browsers keep large Blobs
  on disk when memory runs short) with its own download link, and "Download all as ZIP"
  joins them into one archive. The ZIP writer (`src/zip.js`) stores the DNGs as they are,
  since their raw data is already compressed, writes ZIP64 records when the archive passes
  4 GiB, and never copies the file data: the archive is a `Blob` made of the small headers
  and the DNG Blobs. Where the File System Access API exists (Chrome and Edge on
  desktop), "Save each one straight into a folder" writes every DNG to disk as soon as it
  is ready, keeps nothing in memory, recreates the subfolders of a dropped folder and never
  overwrites a file.
- **Accessibility.** Everything works from the keyboard, focus is always visible, progress
  is announced through a polite live region, and the page follows the system's light or
  dark mode and reduced-motion setting. Below 40 rem the file table becomes one card per
  file.

| File | What it is |
|---|---|
| `index.html` | The page, its copy and the Content-Security-Policy |
| `src/app.js` | Wires the page to the decoder: the list, the workers, downloads |
| `src/core.js` | The decisions, as pure functions: what a file is, what an error means, how many workers to run |
| `src/files.js` | Reading picked and dropped files and folders; saving into a folder |
| `src/zip.js` | The ZIP writer, with CRC-32 and ZIP64 |
| `src/style.css` | Styles, light and dark |
| `build.sh` | Builds `dist/` |
| `scripts/serve.mjs` | A small static server for local use and the tests |

## Tests

```sh
cd web
npm ci                                   # Playwright and axe-core, for the tests only
npm run test:unit                        # src/core.js and src/zip.js in Node.js
sh build.sh
npx playwright install chromium          # once
sh ../conformance/fetch.sh ~/.cache/ticoraw-conformance
TICORAW_SAMPLES=~/.cache/ticoraw-conformance npx playwright test
```

The unit tests check the ZIP writer against Python's `zipfile` module when `python3` is
installed, ZIP64 included. The end-to-end tests run the built page in headless Chromium and:

- convert the public `z6iii_he` and `z5ii_he_dx` samples through the page, download each
  DNG and the ZIP, and check every DNG's CFA SHA-256 against
  [`conformance/manifest.toml`](../conformance/manifest.toml) (the DNGs are decoded in
  Node.js with the same wasm module);
- check that the options reach the converter, that folders are walked, that classic NEFs
  and broken files are explained, and that drag and drop works;
- check that no request leaves the page's server, and that the Content-Security-Policy
  blocks requests to other sites;
- run [axe-core](https://github.com/dequelabs/axe-core) in light and dark mode, at desktop
  and phone width, and require zero serious or critical violations, plus no horizontal
  scrolling on a phone;
- time the conversion of a 24 MP and a 45 MP frame.

Without `TICORAW_SAMPLES` the tests that need samples are skipped (in CI they fail
instead). `.github/workflows/web.yml` runs all of this on every change to `web/` or to the
crates it builds on. It uploads nothing: no build output and no Playwright traces, which
would contain the compiled decoder.

## Performance

Chrome for Testing 156, headless, Apple M5 Pro; one file at a time, file read and transfer
to the worker included (the `timing` test, two runs):

| Frame | First conversion | Again, same worker |
|---|---|---|
| Z6III HE, 24.5 MP (`z6iii_he`) | 0.88-0.91 s | 0.52-0.53 s |
| Z 9 HE, 45.7 MP (`z9_he`) | 0.98 s | 0.93-0.97 s |

The first conversion in a worker is slower while the wasm memory grows and the engine
optimises the code. With several workers, files convert in parallel. On the same machine
the page converts with 6 workers.

## Licence

MIT OR Apache-2.0, like the rest of ticoraw. The DNG preview encoder in the wasm module is
based in part on the work of the Independent JPEG Group; `build.sh` copies
`THIRD-PARTY-NOTICES.md` into the built site.
