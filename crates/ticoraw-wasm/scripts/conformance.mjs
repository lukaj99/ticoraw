#!/usr/bin/env node
// Runs the public conformance samples through the built npm package in Node.js and checks
// every decode against conformance/manifest.toml: the CFA SHA-256 and the metadata. Each
// sample is also converted with toDng (default options: lossless JPEG, preview) and the
// DNG's CFA image must hash the same. Prints per-frame timings (single-threaded wasm).
//
//   node crates/ticoraw-wasm/scripts/conformance.mjs <package dir> <samples dir> [manifest]
//
// <package dir> is the output of scripts/build.sh; <samples dir> holds <id>.nef files as
// conformance/fetch.sh writes them. Exits non-zero if any sample is missing or differs.

import { createHash } from "node:crypto";
import { readFileSync, existsSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { performance } from "node:perf_hooks";

const [pkgDir, samplesDir, manifestArg] = process.argv.slice(2);
if (!pkgDir || !samplesDir) {
  console.error("usage: conformance.mjs <package dir> <samples dir> [manifest]");
  process.exit(2);
}
const here = dirname(fileURLToPath(import.meta.url));
const manifestPath = manifestArg ?? join(here, "../../../conformance/manifest.toml");
const ticoraw = createRequire(import.meta.url)(resolve(pkgDir, "node/ticoraw.js"));

// The manifest hashes little-endian samples; typed arrays use the host's byte order.
if (new Uint8Array(new Uint16Array([1]).buffer)[0] !== 1) {
  throw new Error("conformance.mjs assumes a little-endian host");
}

/** Reads the manifest's TOML subset (see conformance/README.md). */
function parseManifest(text) {
  const samples = [];
  let cur = null;
  for (const [n, raw] of text.split("\n").entries()) {
    const line = stripComment(raw).trim();
    if (line === "") continue;
    if (line === "[[sample]]") {
      cur = {};
      samples.push(cur);
      continue;
    }
    const m = /^([a-z0-9_]+)\s*=\s*(.+)$/.exec(line);
    if (!m) throw new Error(`manifest line ${n + 1}: cannot parse ${JSON.stringify(raw)}`);
    if (cur) cur[m[1]] = parseValue(m[2], n + 1);
  }
  return samples;
}

function stripComment(line) {
  let inString = false;
  for (let i = 0; i < line.length; i++) {
    if (line[i] === '"') inString = !inString;
    else if (line[i] === "#" && !inString) return line.slice(0, i);
  }
  return line;
}

function parseValue(v, n) {
  if (v.startsWith('"') && v.endsWith('"')) return v.slice(1, -1);
  if (v.startsWith("[") && v.endsWith("]")) {
    return v.slice(1, -1).split(",").map((x) => parseValue(x.trim(), n));
  }
  const x = Number(v);
  if (Number.isNaN(x)) throw new Error(`manifest line ${n}: bad value ${v}`);
  return x;
}

const sha256 = (u16) =>
  createHash("sha256")
    .update(new Uint8Array(u16.buffer, u16.byteOffset, u16.byteLength))
    .digest("hex");

function check(s) {
  const file = join(samplesDir, `${s.id}.nef`);
  if (!existsSync(file)) return { id: s.id, failures: [`missing ${file}`] };
  const nef = readFileSync(file);
  const failures = [];
  const expect = (what, got, want) => {
    if (JSON.stringify(got) !== JSON.stringify(want)) {
      failures.push(`${what}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
    }
  };

  let t0 = performance.now();
  const d = ticoraw.decode(nef);
  const decodeMs = performance.now() - t0;
  const cfa = sha256(d.pixels);
  expect("width", d.width, s.width);
  expect("height", d.height, s.height);
  expect("cfa_sha256", cfa, s.cfa_sha256);
  expect("cfa_pattern", d.cfaPattern, s.cfa_pattern);
  expect("black_level", d.blackLevel, s.black_level);
  expect("white_level", d.whiteLevel, s.white_level);
  expect("orientation", d.orientation, s.orientation);
  expect("unique_camera_model", d.uniqueCameraModel, s.unique_camera_model);
  expect("crop", d.crop && [d.crop.x, d.crop.y, d.crop.width, d.crop.height], s.crop);
  if (Math.abs((d.baselineExposure ?? 0) - s.baseline_exposure) > 1e-9) {
    failures.push(`baseline_exposure: got ${d.baselineExposure}, want ${s.baseline_exposure}`);
  }

  t0 = performance.now();
  const dng = ticoraw.toDng(nef, { originalRawFileName: `${s.id}.nef` });
  const toDngMs = performance.now() - t0;
  const back = ticoraw.decodeDng(dng);
  expect("dng cfa_sha256", sha256(back.pixels), s.cfa_sha256);
  return {
    id: s.id,
    mp: (d.width * d.height) / 1e6,
    nefMb: nef.length / 1e6,
    dngMb: dng.length / 1e6,
    decodeMs,
    toDngMs,
    cfa,
    failures,
  };
}

const samples = parseManifest(readFileSync(manifestPath, "utf8"));
console.log(`ticoraw ${ticoraw.version()} (wasm, Node.js ${process.version}), ${samples.length} samples`);
console.log(
  `${"id".padEnd(22)} ${"MP".padStart(5)} ${"NEF MB".padStart(7)} ${"decode ms".padStart(10)}` +
    ` ${"toDng ms".padStart(9)} ${"DNG MB".padStart(7)}  ${"cfa sha256".padEnd(16)}  result`,
);
const rows = [];
for (const s of samples) {
  let r;
  try {
    r = check(s);
  } catch (e) {
    r = { id: s.id, failures: [`${e.name}${e.kind ? ` (${e.kind})` : ""}: ${e.message}`] };
  }
  rows.push(r);
  const f = (x, d) => (x === undefined ? "-" : x.toFixed(d));
  console.log(
    `${r.id.padEnd(22)} ${f(r.mp, 1).padStart(5)} ${f(r.nefMb, 1).padStart(7)}` +
      ` ${f(r.decodeMs, 0).padStart(10)} ${f(r.toDngMs, 0).padStart(9)} ${f(r.dngMb, 1).padStart(7)}` +
      `  ${(r.cfa ?? "-").slice(0, 16).padEnd(16)}  ${r.failures.length ? "FAIL" : "ok"}`,
  );
  for (const msg of r.failures) console.log(`    ${msg}`);
}

const ok = rows.filter((r) => r.failures.length === 0);
const per = (key) => ok.reduce((a, r) => a + r[key] / r.mp, 0) / Math.max(ok.length, 1);
console.log(
  `\n${ok.length}/${rows.length} ok; mean ${per("decodeMs").toFixed(1)} ms/MP decode,` +
    ` ${per("toDngMs").toFixed(1)} ms/MP toDng (decode included);` +
    ` peak RSS ${(process.resourceUsage().maxRSS / 1024).toFixed(0)} MB`,
);
process.exit(ok.length === rows.length ? 0 : 1);
