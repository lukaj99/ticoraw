// Unit tests of src/core.js: plain inputs and outputs, no browser.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import {
  CAMERA_SUPPORT_URL,
  PREFIX_TAIL,
  VERIFIED_MODELS,
  classify,
  classifyFailure,
  dngName,
  dngOptions,
  failure,
  formatBytes,
  formatSeconds,
  isRawName,
  poolSize,
  summary,
  uniqueName,
} from "../../src/core.js";

test("raw file names", () => {
  for (const n of ["DSC_0001.NEF", "a.nef", "b.Nrw", "x.y.NEF"]) assert.ok(isRawName(n), n);
  for (const n of ["DSC_0001.JPG", "nef", "a.nef.xmp", "a.dng", ".NEFx"]) {
    assert.ok(!isRawName(n), n);
  }
});

test("DNG names keep the folder and swap the extension", () => {
  assert.equal(dngName("DSC_0001.NEF"), "DSC_0001.dng");
  assert.equal(dngName("Card/DCIM/100NZ6_3/DSC_0001.NEF"), "Card/DCIM/100NZ6_3/DSC_0001.dng");
  assert.equal(dngName("a.b.nef"), "a.b.dng");
  assert.equal(dngName("noext"), "noext.dng");
  assert.equal(dngName("dir.x/.hidden"), "dir.x/.hidden.dng");
});

test("unique names ignore case and count up", () => {
  const taken = new Set();
  assert.equal(uniqueName("a/DSC.dng", taken), "a/DSC.dng");
  assert.equal(uniqueName("a/dsc.dng", taken), "a/dsc (2).dng");
  assert.equal(uniqueName("a/DSC.dng", taken), "a/DSC (3).dng");
  assert.equal(uniqueName("b/DSC.dng", taken), "b/DSC.dng");
  assert.equal(uniqueName("README", taken), "README");
  assert.equal(uniqueName("README", taken), "README (2)");
});

test("pool size follows cores and memory, within bounds", () => {
  assert.equal(poolSize({ cores: 1, memoryGb: 8 }), 1);
  assert.equal(poolSize({ cores: 2, memoryGb: 8 }), 1);
  assert.equal(poolSize({ cores: 4, memoryGb: 8 }), 3);
  assert.equal(poolSize({ cores: 16, memoryGb: 8 }), 4);
  assert.equal(poolSize({ cores: 16, memoryGb: undefined }), 4);
  assert.equal(poolSize({ cores: 64, memoryGb: 64 }), 6);
  assert.equal(poolSize({ cores: 8, memoryGb: 2 }), 1);
  assert.equal(poolSize({ cores: 8, memoryGb: 0.5 }), 1);
  assert.equal(poolSize({ cores: undefined, memoryGb: undefined }), 1);
});

test("classify: classic, unsupported, ready", () => {
  assert.deepEqual(classify({ model: "NIKON Z 7", codec: "classic", codestream: null }), {
    state: "classic",
    model: "NIKON Z 7",
  });
  assert.deepEqual(
    classify({ model: "NIKON Z 9", codec: "ticoraw", codestream: { unsupported: "NLx = 9" } }),
    { state: "unsupported", model: "NIKON Z 9", detail: "NLx = 9" },
  );
  assert.deepEqual(
    classify({ model: "NIKON Z6_3", codec: "ticoraw", codestream: { unsupported: null } }),
    { state: "ready", model: "NIKON Z6_3", verified: true },
  );
  assert.equal(
    classify({ model: "NIKON Z 99", codec: "ticoraw", codestream: { unsupported: null } }).verified,
    false,
  );
});

test("a short prefix asks for a longer one, then defers to the conversion", () => {
  const truncated = (offset) =>
    Object.assign(new Error("truncated"), { name: "TicoError", kind: "Truncated", what: "raw strip", offset });
  const MB = 1024 * 1024;
  // The raw strip starts at 4 MB: read up to just past its header.
  assert.deepEqual(classifyFailure(truncated(4 * MB), { prefixBytes: 2 * MB, fileBytes: 13 * MB }), {
    state: "retry",
    prefixBytes: 4 * MB + PREFIX_TAIL,
  });
  // Never past the end of the file.
  assert.deepEqual(classifyFailure(truncated(4 * MB), { prefixBytes: 2 * MB, fileBytes: 4 * MB + 10 }), {
    state: "retry",
    prefixBytes: 4 * MB + 10,
  });
  // No progress possible, or no offset: let the conversion decide.
  assert.equal(classifyFailure(truncated(1000), { prefixBytes: 2 * MB, fileBytes: 13 * MB }).state, "ready");
  assert.deepEqual(classifyFailure(truncated(undefined), { prefixBytes: 10, fileBytes: 20 }), {
    state: "retry",
    prefixBytes: 20,
  });
  // The whole file was read: it really is incomplete.
  const whole = classifyFailure(truncated(5), { prefixBytes: 20, fileBytes: 20 });
  assert.equal(whole.state, "failed");
  assert.match(whole.message, /incomplete/);
});

test("errors become beginner messages", () => {
  const tico = (kind, message) => Object.assign(new Error(message), { name: "TicoError", kind });
  const u = failure(tico("Unsupported", "unsupported: classic NEF"));
  assert.equal(u.state, "unsupported");
  assert.equal(u.link.href, CAMERA_SUPPORT_URL);
  assert.equal(u.detail, "unsupported: classic NEF");
  assert.match(failure(tico("Container", "container: not TIFF")).message, /not a Nikon raw/);
  assert.match(failure(tico("Header", "x")).message, /not a Nikon raw/);
  assert.match(failure({ name: "RuntimeError", message: "unreachable" }).message, /memory/);
  assert.match(failure(new DOMException("gone", "NotReadableError")).message, /could not be read/);
  assert.match(failure(new DOMException("no", "NotAllowedError")).message, /saving into that folder/);
  const other = failure(tico("Entropy", "entropy: bad"));
  assert.equal(other.state, "failed");
  assert.equal(other.detail, "Entropy: entropy: bad");
  assert.match(other.link.href, /bug_report/);
  assert.equal(failure("plain string").detail, "plain string");
});

test("form choices map to toDng options", () => {
  assert.deepEqual(dngOptions({ compression: "lossless-jpeg", preview: "auto", keepXmp: true }, "A.NEF"), {
    compression: "lossless-jpeg",
    preview: "auto",
    carryXmp: true,
    originalRawFileName: "A.NEF",
  });
  assert.deepEqual(dngOptions({ compression: "uncompressed", preview: "none", keepXmp: false }, "B"), {
    compression: "uncompressed",
    preview: "none",
    carryXmp: false,
    originalRawFileName: "B",
  });
});

test("formatting", () => {
  assert.equal(formatBytes(999), "999 B");
  assert.equal(formatBytes(1500), "1.5 KB");
  assert.equal(formatBytes(28_400_000), "28 MB");
  assert.equal(formatBytes(4_300_000_000), "4.3 GB");
  assert.equal(formatSeconds(850), "0.85 s");
  assert.equal(formatSeconds(12_500), "12.5 s");
});

test("summary lines", () => {
  const zero = { total: 0, checking: 0, ready: 0, converting: 0, done: 0, failed: 0, skipped: 0 };
  assert.equal(summary({ ...zero, running: false }), "No files yet.");
  assert.equal(summary({ ...zero, total: 3, ready: 3, running: false }), "3 files ready to convert.");
  assert.equal(
    summary({ ...zero, total: 5, ready: 2, converting: 1, done: 1, failed: 1, running: true }),
    "Converting: 2 of 5 finished. 1 file could not be converted.",
  );
  assert.equal(
    summary({ ...zero, total: 4, done: 2, skipped: 2, running: false }),
    "2 files converted. 2 files need no conversion.",
  );
  assert.equal(summary({ ...zero, total: 1, skipped: 1, running: false }), "1 file needs no conversion.");
  assert.equal(
    summary({ ...zero, total: 3, done: 1, exists: 2, running: false }),
    "1 file converted. 2 files skipped: their DNGs already exist in the folder.",
  );
  assert.equal(
    summary({ ...zero, total: 3, ready: 1, done: 1, exists: 1, running: true }),
    "Converting: 2 of 3 finished. 1 file skipped: its DNG already exists in the folder.",
  );
});

test("VERIFIED_MODELS matches the library's camera profiles", () => {
  const lib = readFileSync(new URL("../../../crates/ticoraw/src/lib.rs", import.meta.url), "utf8");
  const start = lib.indexOf("fn adobe_camera_name");
  assert.ok(start > 0, "adobe_camera_name not found in crates/ticoraw/src/lib.rs");
  const body = lib.slice(start, lib.indexOf("\n}\n", start));
  const models = [...body.matchAll(/^\s*"([^"]+)" =>/gm)].map((m) => m[1]);
  assert.deepEqual([...models].sort(), [...VERIFIED_MODELS].sort());
});
