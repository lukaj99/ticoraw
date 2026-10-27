// Node-side helpers for the end-to-end tests: the conformance manifest, a DNG decoder (the
// same wasm the page runs, loaded in Node) and a reader for the STORE-only ZIPs the page
// writes.

import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const web = fileURLToPath(new URL("../../", import.meta.url));
export const DIST = join(web, "dist");
export const SAMPLES = process.env.TICORAW_SAMPLES ?? "";

/** The path of conformance sample `id`, or null when TICORAW_SAMPLES does not have it. */
export function sample(id) {
  const p = SAMPLES && join(SAMPLES, `${id}.nef`);
  return p && existsSync(p) ? p : null;
}

/** The `[[sample]]` tables of conformance/manifest.toml, by id (see its TOML subset). */
export function manifest() {
  const text = readFileSync(join(web, "../conformance/manifest.toml"), "utf8");
  const samples = {};
  let cur = null;
  for (const raw of text.split("\n")) {
    const line = raw.replace(/#[^"]*$/, "").trim();
    if (line === "[[sample]]") {
      cur = {};
      continue;
    }
    const m = /^([a-z0-9_]+)\s*=\s*(.+)$/.exec(line);
    if (!m || !cur) continue;
    const v = m[2].trim();
    cur[m[1]] = v.startsWith('"') ? v.slice(1, -1) : v.startsWith("[") ? JSON.parse(v) : Number(v);
    if (m[1] === "id") samples[cur.id] = cur;
  }
  return samples;
}

let wasm = null;

/** The built web package, instantiated in Node (decodeDng, inspect, ...). */
export async function ticoraw() {
  if (wasm === null) {
    wasm = await import(join(DIST, "pkg/web/ticoraw.js"));
    wasm.initSync({ module: readFileSync(join(DIST, "pkg/web/ticoraw_bg.wasm")) });
  }
  return wasm;
}

export function sha256(bytes) {
  const u8 = new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  return createHash("sha256").update(u8).digest("hex");
}

/** The SHA-256 of a DNG's CFA mosaic, hashed as the manifest's cfa_sha256 is. */
export async function dngCfaSha256(dng) {
  const d = (await ticoraw()).decodeDng(dng);
  return { sha: sha256(d.pixels), width: d.width, height: d.height };
}

/**
 * The entries of a ZIP written with method 0 (STORE), from its central directory, ZIP64
 * included: `[{ name, data }]`. Only what the page writes is supported.
 */
export function readZip(buf) {
  const b = Buffer.from(buf.buffer, buf.byteOffset, buf.byteLength);
  const eocd = b.lastIndexOf(Buffer.from([0x50, 0x4b, 0x05, 0x06]));
  if (eocd < 0) throw new Error("no end of central directory record");
  let count = b.readUInt16LE(eocd + 10);
  let cd = b.readUInt32LE(eocd + 16);
  if (cd === 0xffffffff) {
    const rec = Number(b.readBigUInt64LE(eocd - 20 + 8));
    count = Number(b.readBigUInt64LE(rec + 32));
    cd = Number(b.readBigUInt64LE(rec + 48));
  }
  const out = [];
  for (let i = 0, p = cd; i < count; i++) {
    if (b.readUInt32LE(p) !== 0x02014b50) throw new Error(`bad central header at ${p}`);
    const method = b.readUInt16LE(p + 10);
    let size = b.readUInt32LE(p + 24);
    const nameLen = b.readUInt16LE(p + 28);
    const extraLen = b.readUInt16LE(p + 30);
    const commentLen = b.readUInt16LE(p + 32);
    let local = b.readUInt32LE(p + 42);
    const name = b.toString("utf8", p + 46, p + 46 + nameLen);
    // ZIP64 extra field: the 64-bit values of the fields set to 0xFFFFFFFF, in order.
    const extra = b.subarray(p + 46 + nameLen, p + 46 + nameLen + extraLen);
    for (let q = 0; q + 4 <= extra.length; ) {
      const id = extra.readUInt16LE(q);
      const len = extra.readUInt16LE(q + 2);
      if (id === 1) {
        let f = q + 4;
        if (size === 0xffffffff) {
          size = Number(extra.readBigUInt64LE(f));
          f += 16; // uncompressed, then compressed
        }
        if (local === 0xffffffff) local = Number(extra.readBigUInt64LE(f));
      }
      q += 4 + len;
    }
    if (method !== 0) throw new Error(`${name}: method ${method}, not STORE`);
    const start = local + 30 + b.readUInt16LE(local + 26) + b.readUInt16LE(local + 28);
    out.push({ name, data: b.subarray(start, start + size) });
    p += 46 + nameLen + extraLen + commentLen;
  }
  return out;
}
