// Unit tests of saveInto and existsIn in src/files.js, against an in-memory stand-in for the
// File System Access API's directory handles (the parts files.js uses, with the same error
// names). Every call yields to the event loop first, as the real, IPC-backed API does.

import assert from "node:assert/strict";
import { test } from "node:test";
import { existsIn, saveInto } from "../../src/files.js";

const tick = () => new Promise((r) => setTimeout(r, 0));
const domError = (name) => Object.assign(new Error(name), { name });

class MemFile {
  constructor(dir, name) {
    this.kind = "file";
    this.dir = dir;
    this.name = name;
  }

  async createWritable() {
    await tick();
    const chunks = [];
    return {
      write: async (blob) => {
        await tick();
        if (this.dir.fs.failWrite) throw domError(this.dir.fs.failWrite);
        chunks.push(new Uint8Array(await blob.arrayBuffer()));
      },
      close: async () => {
        await tick();
        // Like Chromium: the data reaches the file only on close (via a swap file).
        this.dir.entries.set(this.name, Buffer.concat(chunks));
      },
      abort: async () => {
        await tick();
      },
    };
  }

  async move(newName) {
    await tick();
    if (this.dir.fs.failMove) throw domError(this.dir.fs.failMove);
    this.dir.fs.moves++;
    const data = this.dir.entries.get(this.name);
    this.dir.entries.delete(this.name);
    this.dir.entries.set(newName, data); // replaces an existing file, as the spec allows
    this.name = newName;
  }
}

class MemDir {
  constructor(fs, name) {
    this.kind = "directory";
    this.fs = fs;
    this.name = name;
    this.entries = new Map(); // name -> Buffer | MemDir
  }

  async getDirectoryHandle(name, { create = false } = {}) {
    await tick();
    const e = this.entries.get(name);
    if (e instanceof MemDir) return e;
    if (e) throw domError("TypeMismatchError");
    if (!create) throw domError("NotFoundError");
    const d = new MemDir(this.fs, name);
    this.entries.set(name, d);
    return d;
  }

  async getFileHandle(name, { create = false } = {}) {
    await tick();
    const e = this.entries.get(name);
    if (e instanceof MemDir) throw domError("TypeMismatchError");
    if (!e) {
      if (!create) throw domError("NotFoundError");
      this.entries.set(name, Buffer.alloc(0)); // created empty at once, as in browsers
    }
    return new MemFile(this, name);
  }

  async removeEntry(name) {
    await tick();
    if (!this.entries.delete(name)) throw domError("NotFoundError");
  }

  /** `{ name: size }` for the files in this folder, `{ name: {...} }` for folders. */
  list() {
    return Object.fromEntries(
      [...this.entries].map(([n, e]) => [n, e instanceof MemDir ? e.list() : e.length]),
    );
  }
}

function memRoot() {
  return new MemDir({ failWrite: null, failMove: null, moves: 0 }, "root");
}

const blob = (n) => new Blob([new Uint8Array(n).fill(7)]);

/** Runs `fn` with `FileSystemFileHandle.prototype.move` present, as in Chromium. */
async function withMove(fn) {
  globalThis.FileSystemFileHandle = MemFile;
  try {
    await fn();
  } finally {
    delete globalThis.FileSystemFileHandle;
  }
}

test("writes through a temporary name, creating folders", () =>
  withMove(async () => {
    const root = memRoot();
    assert.equal(await saveInto(root, "Card/DCIM/DSC_0001.dng", blob(10)), "written");
    assert.deepEqual(root.list(), { Card: { DCIM: { "DSC_0001.dng": 10 } } });
    assert.equal(root.fs.moves, 1);
  }));

test("never overwrites: an existing file or folder is reported, not replaced", () =>
  withMove(async () => {
    const root = memRoot();
    await saveInto(root, "DSC_0001.dng", blob(10));
    assert.equal(await saveInto(root, "DSC_0001.dng", blob(99)), "exists");
    await root.getDirectoryHandle("folder.dng", { create: true });
    assert.equal(await saveInto(root, "folder.dng", blob(5)), "exists");
    assert.deepEqual(root.list(), { "DSC_0001.dng": 10, "folder.dng": {} });
  }));

test("a failed write leaves nothing behind", () =>
  withMove(async () => {
    const root = memRoot();
    root.fs.failWrite = "QuotaExceededError";
    await assert.rejects(saveInto(root, "a/DSC_0001.dng", blob(10)), { name: "QuotaExceededError" });
    assert.deepEqual(root.list(), { a: {} });
    // A retry gets the real name, not "DSC_0001 (2).dng".
    root.fs.failWrite = null;
    assert.equal(await saveInto(root, "a/DSC_0001.dng", blob(10)), "written");
    assert.deepEqual(root.list(), { a: { "DSC_0001.dng": 10 } });
  }));

test("a file that appears during the write is not replaced", () =>
  withMove(async () => {
    const root = memRoot();
    const saving = saveInto(root, "DSC_0001.dng", blob(10));
    // Another program creates the file while the temporary one is being written.
    for (let i = 0; i < 3; i++) await tick();
    root.entries.set("DSC_0001.dng", Buffer.alloc(3));
    assert.equal(await saving, "exists");
    assert.deepEqual(root.list(), { "DSC_0001.dng": 3 });
  }));

test("a refused rename is reported and cleaned up", () =>
  withMove(async () => {
    const root = memRoot();
    root.fs.failMove = "NotAllowedError";
    await assert.rejects(saveInto(root, "DSC_0001.dng", blob(10)), { name: "NotAllowedError" });
    assert.deepEqual(root.list(), {});
  }));

test("without move(), writes the final name and removes it on failure", async () => {
  const root = memRoot();
  root.fs.failWrite = "NotAllowedError";
  await assert.rejects(saveInto(root, "DSC_0001.dng", blob(10)), { name: "NotAllowedError" });
  assert.deepEqual(root.list(), {});
  root.fs.failWrite = null;
  assert.equal(await saveInto(root, "DSC_0001.dng", blob(10)), "written");
  assert.deepEqual(root.list(), { "DSC_0001.dng": 10 });
  assert.equal(root.fs.moves, 0);
});

// Last: a rename that fails for another reason switches this module to direct writes.
test("a rename that fails falls back to writing the final name", () =>
  withMove(async () => {
    const root = memRoot();
    root.fs.failMove = "NotSupportedError";
    assert.equal(await saveInto(root, "DSC_0001.dng", blob(10)), "written");
    assert.deepEqual(root.list(), { "DSC_0001.dng": 10 });
  }));

test("existsIn looks without creating anything", async () => {
  const root = memRoot();
  assert.equal(await existsIn(root, "Card/DCIM/DSC_0001.dng"), false);
  assert.deepEqual(root.list(), {});
  await saveInto(root, "Card/DSC_0001.dng", blob(1));
  assert.equal(await existsIn(root, "Card/DSC_0001.dng"), true);
  assert.equal(await existsIn(root, "Card"), true);
  assert.equal(await existsIn(root, "Card/DSC_0002.dng"), false);
});
