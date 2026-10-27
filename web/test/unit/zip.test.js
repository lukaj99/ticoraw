// Unit tests of src/zip.js. Archives are checked by an independent reader, Python's
// zipfile module (which verifies every CRC and understands ZIP64), when python3 is on PATH.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { crc32, dosDateTime, zipParts } from "../../src/zip.js";

const python = spawnSync("python3", ["-c", "import zipfile"]).status === 0;

test("crc32 known values", () => {
  const enc = new TextEncoder();
  assert.equal(crc32(new Uint8Array(0)), 0);
  assert.equal(crc32(enc.encode("123456789")), 0xcbf43926);
  assert.equal(crc32(enc.encode("The quick brown fox jumps over the lazy dog")), 0x414fa339);
  // Continuing from a previous value equals one pass over the concatenation.
  assert.equal(crc32(enc.encode("6789"), crc32(enc.encode("12345"))), 0xcbf43926);
});

test("DOS date and time", () => {
  const { date, time } = dosDateTime(new Date(2026, 9, 27, 13, 45, 31));
  assert.equal(date, ((2026 - 1980) << 9) | (10 << 5) | 27);
  assert.equal(time, (13 << 11) | (45 << 5) | 15);
  assert.equal(dosDateTime(new Date(1970, 0, 1)).date >> 9, 0); // clamped to 1980
});

function entries() {
  const enc = new TextEncoder();
  const date = new Date(2026, 9, 27, 12, 0, 0);
  const files = [
    ["DSC_0001.dng", enc.encode("first file\n")],
    ["Card/DCIM/DSC_0002.dng", new Uint8Array(70_000).map((_, i) => (i * 31) & 0xff)],
    ["Ünïcödé/empty.dng", new Uint8Array(0)],
  ];
  return files.map(([name, bytes]) => ({
    name,
    data: new Blob([bytes]),
    size: bytes.length,
    crc: crc32(bytes),
    date,
    bytes,
  }));
}

async function check(parts, list) {
  const zip = new Uint8Array(await new Blob(parts).arrayBuffer());
  const dir = mkdtempSync(join(tmpdir(), "ticoraw-zip-"));
  try {
    const path = join(dir, "t.zip");
    writeFileSync(path, zip);
    const script = [
      "import json, sys, zipfile",
      "z = zipfile.ZipFile(sys.argv[1])",
      "bad = z.testzip()",
      "print(json.dumps({'bad': bad, 'files': [[i.filename, i.file_size, i.compress_type,",
      "  list(i.date_time), z.read(i).hex()] for i in z.infolist()]}))",
    ].join("\n");
    const r = spawnSync("python3", ["-I", "-c", script, path], { encoding: "utf8" });
    assert.equal(r.status, 0, r.stderr);
    const out = JSON.parse(r.stdout);
    assert.equal(out.bad, null);
    assert.deepEqual(
      out.files,
      list.map((e) => [e.name, e.size, 0, [2026, 10, 27, 12, 0, 0], Buffer.from(e.bytes).toString("hex")]),
    );
    return zip;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("a plain archive reads back", { skip: !python && "python3 not found" }, async () => {
  const list = entries();
  const zip = await check(zipParts(list), list);
  // No ZIP64 records when nothing needs them.
  assert.equal(Buffer.from(zip).indexOf(Buffer.from([0x50, 0x4b, 0x06, 0x06])), -1);
});

test("a ZIP64 archive reads back", { skip: !python && "python3 not found" }, async () => {
  const list = entries();
  const zip = await check(zipParts(list, { forceZip64: true }), list);
  assert.notEqual(Buffer.from(zip).indexOf(Buffer.from([0x50, 0x4b, 0x06, 0x06])), -1);
});

test("an empty archive is valid", { skip: !python && "python3 not found" }, async () => {
  await check(zipParts([]), []);
});

test("parts pass the file data through without copying", () => {
  const list = entries();
  const parts = zipParts(list);
  for (const e of list) assert.ok(parts.includes(e.data));
});

test("offsets past 4 GiB switch to ZIP64 without reading the data", () => {
  // Fake entries: only their sizes matter for the headers.
  const big = { name: "big.dng", data: null, size: 3_000_000_000, crc: 0, date: new Date(2026, 0, 1) };
  const parts = zipParts([big, { ...big, name: "big2.dng" }, { ...big, name: "small.dng", size: 1 }]);
  const headers = parts.filter((p) => p instanceof Uint8Array);
  const tail = Buffer.concat(headers.slice(-3).map((p) => Buffer.from(p)));
  // ZIP64 end record, locator and classic end record, in that order.
  assert.equal(tail.readUInt32LE(0), 0x06064b50);
  assert.equal(tail.readUInt32LE(56), 0x07064b50);
  assert.equal(tail.readUInt32LE(76), 0x06054b50);
  const cdStart = tail.readBigUInt64LE(48);
  assert.ok(cdStart > 0xffffffffn);
  // The third entry starts past 4 GiB, so its central header carries a ZIP64 offset.
  const thirdCentral = headers.find(
    (p) => p.length === 46 && Buffer.from(p).readUInt32LE(0) === 0x02014b50 && Buffer.from(p).readUInt32LE(42) === 0xffffffff,
  );
  assert.ok(thirdCentral, "no central header with a ZIP64 offset");
});
