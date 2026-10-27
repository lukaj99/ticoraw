#!/usr/bin/env node
// Tests js/worker.js (the `ticoraw/worker` pool) in Node.js, without a browser: a small
// shim gives Node's worker_threads the parts of the Web Worker API that worker.js uses
// (`Worker`, `WorkerGlobalScope`, `postMessage` / `onmessage`, `fetch` of the .wasm file).
//
//   node crates/ticoraw-wasm/scripts/worker-test.mjs <package dir> <nef file>
//
// <package dir> is the output of scripts/build.sh. Any HE/HE* NEF works, e.g. a sample
// from conformance/fetch.sh.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { Worker as NodeWorker, isMainThread, parentPort, workerData } from "node:worker_threads";

/** The main-thread half of the shim: a `Worker` backed by worker_threads. */
class ShimWorker {
  #w;
  onmessage = null;
  onmessageerror = null;
  onerror = null;

  constructor(url, options) {
    assert.equal(options?.type, "module");
    this.#w = new NodeWorker(new URL(import.meta.url), { workerData: { url: String(url) } });
    this.#w.on("message", (data) => this.onmessage?.({ data }));
    this.#w.on("messageerror", (data) => this.onmessageerror?.({ data }));
    this.#w.on("error", (e) => this.onerror?.({ message: e.message, preventDefault() {} }));
  }

  postMessage(msg, transfer) {
    this.#w.postMessage(msg, transfer);
  }

  terminate() {
    this.#w.terminate();
  }
}

async function main() {
  const [pkgDir, nefPath] = process.argv.slice(2);
  if (!pkgDir || !nefPath) {
    console.error("usage: worker-test.mjs <package dir> <nef file>");
    process.exit(2);
  }
  globalThis.Worker = ShimWorker;
  const pkg = resolve(pkgDir);
  const direct = createRequire(import.meta.url)(resolve(pkg, "node/ticoraw.js"));
  const { TicorawWorker } = await import(pathToFileURL(resolve(pkg, "worker.js")).href);

  const nef = new Uint8Array(await readFile(nefPath));
  const want = direct.toDng(nef);
  const wantPixels = direct.decode(nef).pixels;

  const pool = new TicorawWorker({ size: 2 });
  try {
    // Every input type, four jobs on two workers.
    const asArray = nef.slice();
    const results = await Promise.all([
      pool.toDng(new Blob([nef])),
      pool.toDng(asArray),
      pool.toDng(nef.buffer.slice(nef.byteOffset, nef.byteOffset + nef.byteLength)),
      pool.decode(nef),
    ]);
    assert.equal(asArray.byteLength, nef.byteLength, "the caller's buffer is not detached");
    for (const dng of results.slice(0, 3)) {
      assert.ok(dng instanceof Uint8Array);
      assert.deepEqual(dng, want);
    }
    assert.deepEqual(results[3].pixels, wantPixels);
    assert.equal(results[3].cfaPattern, "RGGB");

    const info = await pool.inspect(nef.subarray(0, nef.length));
    assert.equal(info.codec, "ticoraw");

    const opts = { compression: "uncompressed", preview: "none" };
    assert.deepEqual(await pool.toDng(nef, opts), direct.toDng(nef, opts));

    // Errors keep their name, kind and message across the thread boundary.
    await assert.rejects(pool.decode(new TextEncoder().encode("not a raw file")), (e) => {
      assert.equal(e.name, "TicoError");
      assert.equal(e.kind, "Container");
      assert.match(e.message, /^container: /);
      return true;
    });
    await assert.rejects(pool.toDng(nef, { compresion: "uncompressed" }), TypeError);
    await assert.rejects(pool.toDng(42), TypeError);
    // ...and the pool still works afterwards.
    assert.deepEqual(await pool.toDng(nef), want);

    // terminate() rejects what is still pending.
    const pending = pool.toDng(nef);
    pool.terminate();
    await assert.rejects(pending, /terminated/);
    await assert.rejects(pool.inspect(nef), /terminated/);
  } finally {
    pool.terminate();
  }

  // Options that cannot be structured-cloned reject with a TypeError, as the synchronous
  // toDng throws one, and free the worker: with one worker, the next job still runs.
  const single = new TicorawWorker({ size: 1 });
  try {
    for (const bad of [{ originalRawFileName: () => "a.NEF" }, { x: Symbol("x") }, () => {}]) {
      assert.throws(() => direct.toDng(nef, bad), TypeError);
      await assert.rejects(within(single.toDng(nef, bad)), TypeError);
    }
    assert.equal((await within(single.inspect(nef))).codec, "ticoraw");
    // A Proxy (a Vue reactive object, say) is read like a plain object, as toDng reads it.
    const opts = { compression: "uncompressed", preview: "none" };
    const proxy = new Proxy({ ...opts }, {});
    assert.throws(() => structuredClone(proxy));
    assert.deepEqual(await within(single.toDng(nef, proxy)), direct.toDng(nef, opts));
  } finally {
    single.terminate();
  }
  console.log("worker-test: ok");
}

/** Fails instead of hanging when a job never settles. */
function within(promise, ms = 30_000) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`worker-test: no answer in ${ms} ms`)), ms);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

// Last, so that the class above is initialised before main() uses it.
if (!isMainThread) {
  // Inside the worker thread: make it look like a module Web Worker, then load worker.js.
  globalThis.WorkerGlobalScope = class WorkerGlobalScope {
    static [Symbol.hasInstance](x) {
      return x === globalThis;
    }
  };
  globalThis.postMessage = (msg, transfer) => parentPort.postMessage(msg, transfer);
  globalThis.close = () => process.exit(0);
  parentPort.on("message", (data) => globalThis.onmessage?.({ data }));
  parentPort.on("messageerror", (data) => globalThis.onmessageerror?.({ data }));
  const webFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    const u = new URL(url);
    if (u.protocol !== "file:") return webFetch(url, init);
    const body = await readFile(fileURLToPath(u));
    return new Response(body, { headers: { "content-type": "application/wasm" } });
  };
  await import(workerData.url);
} else {
  await main();
}
