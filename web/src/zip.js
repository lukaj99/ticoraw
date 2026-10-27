// A small ZIP writer for files that are already compressed (DNG tiles are lossless JPEG),
// so every entry is STOREd. It never copies file data: `zipParts` returns the headers as
// small Uint8Arrays interleaved with the entries' own Blobs, and `new Blob(parts)` then
// joins them without reading anything. The browser keeps large Blobs on disk, so a ZIP of
// many gigabytes does not have to fit in memory.
//
// ZIP64 records are written when they are needed: an entry or the archive past 4 GiB, or
// more than 65,534 entries. `forceZip64` writes them anyway, which the tests use.
//
// Format: PKWARE APPNOTE.TXT 6.3.10, sections 4.3 (structure), 4.4 (fields) and 4.5.3
// (the ZIP64 extra field).

const MAX32 = 0xffffffff;
const MAX16 = 0xffff;

let table = null;

/** CRC-32 (IEEE 802.3, as ZIP uses it) of `bytes`, continuing from `crc`. */
export function crc32(bytes, crc = 0) {
  if (table === null) {
    table = new Int32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      table[n] = c;
    }
  }
  let c = ~crc;
  for (let i = 0; i < bytes.length; i++) c = table[(c ^ bytes[i]) & 0xff] ^ (c >>> 8);
  return ~c >>> 0;
}

/** MS-DOS date and time fields (local time, 2-second resolution, years 1980-2107). */
export function dosDateTime(date) {
  const y = Math.min(Math.max(date.getFullYear(), 1980), 2107);
  return {
    time: (date.getHours() << 11) | (date.getMinutes() << 5) | (date.getSeconds() >> 1),
    date: ((y - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate(),
  };
}

/**
 * `name` as a safe relative entry name (APPNOTE 4.4.17.1: forward slashes only, no drive
 * letter, no leading slash), so that no extractor writes outside the folder it extracts
 * into. Backslashes become `/`; empty, `.` and `..` folders and a leading drive letter are
 * dropped; control characters become `_`. Applying it again changes nothing.
 * `"..\\..\\C:\\x.dng"` becomes `"x.dng"`.
 */
export function safeEntryName(name) {
  let parts = name.replace(/[\u0000-\u001f\u007f]/g, "_").replaceAll("\\", "/").split("/");
  for (;;) {
    parts = parts.filter((p) => p !== "" && p !== "." && p !== "..");
    if (!/^[A-Za-z]:/.test(parts[0] ?? "")) break;
    parts[0] = parts[0].slice(2);
  }
  return parts.join("/") || "_";
}

/**
 * The parts of a ZIP archive, in order, for `new Blob(parts)`.
 *
 * @param entries `{ name, data, size, crc, date }[]`: `name` uses `/` between folders and is
 *   written as `safeEntryName(name)`; `data` is a Blob (or Uint8Array) of `size` bytes whose
 *   CRC-32 is `crc`; `date` is a Date. Names must be unique after `safeEntryName`; this
 *   function does not check.
 * @param options `{ forceZip64 }`.
 */
export function zipParts(entries, { forceZip64 = false } = {}) {
  const enc = new TextEncoder();
  const parts = [];
  const central = [];
  let offset = 0;

  for (const e of entries) {
    const name = enc.encode(safeEntryName(e.name));
    const { time, date } = dosDateTime(e.date);
    const big = forceZip64 || e.size >= MAX32;
    // Local header: sizes go in the ZIP64 extra field when they do not fit in 32 bits.
    const localExtra = big ? zip64Extra([e.size, e.size]) : new Uint8Array(0);
    const local = new DataView(new ArrayBuffer(30));
    local.setUint32(0, 0x04034b50, true);
    local.setUint16(4, big ? 45 : 20, true); // version needed to extract
    local.setUint16(6, 0x0800, true); // bit 11: names are UTF-8
    local.setUint16(8, 0, true); // method 0: stored
    local.setUint16(10, time, true);
    local.setUint16(12, date, true);
    local.setUint32(14, e.crc, true);
    local.setUint32(18, big ? MAX32 : e.size, true); // compressed size
    local.setUint32(22, big ? MAX32 : e.size, true); // uncompressed size
    local.setUint16(26, name.length, true);
    local.setUint16(28, localExtra.length, true);
    parts.push(new Uint8Array(local.buffer), name, localExtra, e.data);
    central.push({ name, time, date, crc: e.crc, size: e.size, offset, big });
    offset += 30 + name.length + localExtra.length + e.size;
  }

  const cdStart = offset;
  for (const c of central) {
    const farOffset = forceZip64 || c.offset >= MAX32;
    const fields = [];
    if (c.big) fields.push(c.size, c.size); // uncompressed, then compressed
    if (farOffset) fields.push(c.offset);
    const extra = fields.length ? zip64Extra(fields) : new Uint8Array(0);
    const z64 = c.big || farOffset;
    const h = new DataView(new ArrayBuffer(46));
    h.setUint32(0, 0x02014b50, true);
    h.setUint16(4, z64 ? 45 : 20, true); // version made by (MS-DOS attributes)
    h.setUint16(6, z64 ? 45 : 20, true); // version needed to extract
    h.setUint16(8, 0x0800, true);
    h.setUint16(10, 0, true);
    h.setUint16(12, c.time, true);
    h.setUint16(14, c.date, true);
    h.setUint32(16, c.crc, true);
    h.setUint32(20, c.big ? MAX32 : c.size, true);
    h.setUint32(24, c.big ? MAX32 : c.size, true);
    h.setUint16(28, c.name.length, true);
    h.setUint16(30, extra.length, true);
    // 32: comment length, 34: disk number, 36: internal and 38: external attributes, all 0.
    h.setUint32(42, farOffset ? MAX32 : c.offset, true);
    parts.push(new Uint8Array(h.buffer), c.name, extra);
    offset += 46 + c.name.length + extra.length;
  }

  const cdSize = offset - cdStart;
  const count = central.length;
  const z64End = forceZip64 || count >= MAX16 || cdSize >= MAX32 || cdStart >= MAX32;
  if (z64End) {
    // ZIP64 end of central directory record (4.3.14), then its locator (4.3.15).
    const r = new DataView(new ArrayBuffer(56));
    r.setUint32(0, 0x06064b50, true);
    r.setBigUint64(4, 44n, true); // size of the rest of this record
    r.setUint16(12, 45, true);
    r.setUint16(14, 45, true);
    // 16: this disk, 20: disk with the central directory, both 0.
    r.setBigUint64(24, BigInt(count), true);
    r.setBigUint64(32, BigInt(count), true);
    r.setBigUint64(40, BigInt(cdSize), true);
    r.setBigUint64(48, BigInt(cdStart), true);
    const l = new DataView(new ArrayBuffer(20));
    l.setUint32(0, 0x07064b50, true);
    l.setUint32(4, 0, true);
    l.setBigUint64(8, BigInt(offset), true); // where the record above starts
    l.setUint32(16, 1, true); // total number of disks
    parts.push(new Uint8Array(r.buffer), new Uint8Array(l.buffer));
  }
  const end = new DataView(new ArrayBuffer(22));
  end.setUint32(0, 0x06054b50, true);
  end.setUint16(8, z64End ? MAX16 : count, true);
  end.setUint16(10, z64End ? MAX16 : count, true);
  end.setUint32(12, z64End ? MAX32 : cdSize, true);
  end.setUint32(16, z64End ? MAX32 : cdStart, true);
  parts.push(new Uint8Array(end.buffer));
  return parts;
}

/** The ZIP64 extended information extra field (header 0x0001) with 64-bit `values`. */
function zip64Extra(values) {
  const v = new DataView(new ArrayBuffer(4 + 8 * values.length));
  v.setUint16(0, 0x0001, true);
  v.setUint16(2, 8 * values.length, true);
  values.forEach((x, i) => v.setBigUint64(4 + 8 * i, BigInt(x), true));
  return new Uint8Array(v.buffer);
}
