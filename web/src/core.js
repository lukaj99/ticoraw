// The converter's pure logic: no DOM, no I/O, no clock. app.js is the imperative shell
// around it; test/unit/core.test.js tests it in Node.

/** Where people ask for a camera, firmware or mode to be supported. */
export const CAMERA_SUPPORT_URL =
  "https://github.com/lukaj99/ticoraw/issues/new?template=camera_support.yml";
/** Where people report something that does not work. */
export const BUG_REPORT_URL =
  "https://github.com/lukaj99/ticoraw/issues/new?template=bug_report.yml";

/**
 * TIFF `Model` values of the bodies ticoraw has a verified colour profile for. Keep in
 * sync with `adobe_camera_name` in crates/ticoraw/src/lib.rs (a unit test checks it).
 * Other bodies still convert, but their DNG gets an identity colour matrix.
 */
export const VERIFIED_MODELS = Object.freeze([
  "NIKON Z6_3",
  "NIKON Z 8",
  "NIKON Z 9",
  "NIKON Z f",
  "NIKON Z5_2",
  "NIKON Z50_2",
  "NIKON ZR",
]);

/** How much of a file is read first to classify it with `inspectPrefix`. */
export const PREFIX_BYTES = 2 * 1024 * 1024;
/**
 * How far past the start of a structure a longer prefix reaches when the first one was
 * too short: enough for the codestream's main header at the start of the raw strip.
 */
export const PREFIX_TAIL = 256 * 1024;

/** Upper bound on parallel conversions, whatever the machine reports. */
export const MAX_WORKERS = 6;

/** True for the raw file names a folder may hold: `.nef` and `.nrw`, any case. */
export function isRawName(name) {
  return /\.(nef|nrw)$/i.test(name);
}

/**
 * The DNG name for a NEF path: the same path with a `.dng` extension.
 * `"2026/DSC_0001.NEF"` becomes `"2026/DSC_0001.dng"`.
 */
export function dngName(path) {
  const slash = path.lastIndexOf("/") + 1;
  const base = path.slice(slash);
  const dot = base.lastIndexOf(".");
  const stem = dot > 0 ? base.slice(0, dot) : base;
  return `${path.slice(0, slash)}${stem}.dng`;
}

/**
 * Makes `name` unique among `taken` (a Set, updated) by adding " (2)", " (3)", ... before
 * the extension. Comparison ignores case, because most desktop file systems do.
 */
export function uniqueName(name, taken) {
  const dot = name.lastIndexOf(".");
  const stem = dot > 0 ? name.slice(0, dot) : name;
  const ext = dot > 0 ? name.slice(dot) : "";
  let candidate = name;
  for (let n = 2; taken.has(candidate.toLowerCase()); n++) candidate = `${stem} (${n})${ext}`;
  taken.add(candidate.toLowerCase());
  return candidate;
}

/**
 * How many files to convert at once. Each conversion runs in its own Web Worker, uses one
 * core, and needs about 0.5 GB of memory for a 45 MP frame, which the worker keeps after
 * the conversion (wasm memory is never handed back). So: one worker per core, leaving one
 * for the page, at most one per 2 GB of device memory, and never more than MAX_WORKERS.
 *
 * `cores` is `navigator.hardwareConcurrency`; `memoryGb` is `navigator.deviceMemory`
 * (Chromium only, rounded and capped at 8), `undefined` elsewhere, which counts as 8.
 */
export function poolSize({ cores, memoryGb }) {
  const c = Number.isInteger(cores) && cores > 0 ? cores : 2;
  const mem = typeof memoryGb === "number" && memoryGb > 0 ? memoryGb : 8;
  return Math.max(1, Math.min(c - 1, Math.floor(mem / 2), MAX_WORKERS));
}

/**
 * What to do with a file, from `inspectPrefix`'s result.
 *
 * @returns one of
 *   `{ state: "ready", model, verified }`: a HE/HE* NEF to convert;
 *   `{ state: "classic", model }`: a classic NEF, which editors already open;
 *   `{ state: "unsupported", model, detail }`: a HE/HE* file outside the decoder's profile.
 */
export function classify(inspection) {
  const model = inspection.model;
  if (inspection.codec === "classic") return { state: "classic", model };
  const unsupported = inspection.codestream?.unsupported;
  if (unsupported) return { state: "unsupported", model, detail: unsupported };
  return { state: "ready", model, verified: VERIFIED_MODELS.includes(model) };
}

/**
 * What to do with a file whose prefix `inspectPrefix` rejected. A `Truncated` error on a
 * prefix shorter than the file only means that the structure it names starts further in
 * (the raw strip usually follows a preview JPEG of several megabytes). Then:
 * `{ state: "retry", prefixBytes }` asks for a longer prefix that covers `offset` plus
 * PREFIX_TAIL, if that is longer than what was tried; otherwise the full conversion decides
 * (`ready`). Anything else is final.
 */
export function classifyFailure(error, { prefixBytes, fileBytes }) {
  if (error?.kind === "Truncated" && prefixBytes < fileBytes) {
    const offset = Number.isSafeInteger(error.offset) ? error.offset : fileBytes;
    const want = Math.min(fileBytes, offset + PREFIX_TAIL);
    if (want > prefixBytes) return { state: "retry", prefixBytes: want };
    return { state: "ready", model: null, verified: true };
  }
  return failure(error);
}

/**
 * A failed file as the queue shows it: `{ state: "unsupported" | "failed", message, link }`.
 * `link` is `{ href, text }` or null. `message` is plain text for beginners; the original
 * error text goes in `detail`.
 */
export function failure(error) {
  const kind = error?.kind ?? error?.name ?? "Error";
  const detail = error?.message ? String(error.message) : String(error);
  const report = { href: BUG_REPORT_URL, text: "Report a problem" };
  const support = { href: CAMERA_SUPPORT_URL, text: "Tell us about your camera" };
  switch (kind) {
    case "Unsupported":
      return {
        state: "unsupported",
        message: "This file uses a variant ticoraw cannot read yet.",
        detail,
        link: support,
      };
    case "Truncated":
      return {
        state: "failed",
        message: "This file is incomplete: it ends too early. Copy it from the card again.",
        detail,
        link: null,
      };
    case "Container":
    case "Header":
      return {
        state: "failed",
        message: "This is not a Nikon raw file ticoraw can read.",
        detail,
        link: null,
      };
    case "TooLarge":
      return {
        state: "failed",
        message: "This image is larger than ticoraw accepts.",
        detail,
        link: report,
      };
    case "RangeError":
    case "RuntimeError":
      // RangeError: an allocation in JavaScript failed. RuntimeError: the wasm instance
      // trapped, which in practice means it ran out of memory (the decoder is fuzzed and
      // does not panic on bad input).
      return {
        state: "failed",
        message:
          "The converter ran out of memory. Close other tabs or convert fewer files at once, " +
          "then try again.",
        detail,
        link: report,
      };
    case "NotAllowedError":
      return {
        state: "failed",
        message: "The browser did not allow saving into that folder. Choose another one.",
        detail,
        link: null,
      };
    case "NotFoundError":
    case "NotReadableError":
      return {
        state: "failed",
        message: "This file could not be read. It may have been moved or deleted.",
        detail,
        link: null,
      };
    default:
      return {
        state: "failed",
        message: "Something went wrong while converting this file.",
        detail: kind === "Error" ? detail : `${kind}: ${detail}`,
        link: report,
      };
  }
}

/** The `toDng` options for the form's choices and a file name. */
export function dngOptions({ compression, preview, keepXmp }, fileName) {
  return {
    compression: compression === "uncompressed" ? "uncompressed" : "lossless-jpeg",
    preview: preview === "none" ? "none" : "auto",
    carryXmp: Boolean(keepXmp),
    originalRawFileName: fileName,
  };
}

/** `1536` as `"1.5 KB"`; decimal units, as file managers on macOS and Linux show them. */
export function formatBytes(n) {
  if (n < 1000) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n;
  let i = -1;
  do {
    v /= 1000;
    i++;
  } while (v >= 1000 && i < units.length - 1);
  return `${v.toFixed(v < 10 ? 1 : 0)} ${units[i]}`;
}

/** `850` as `"0.85 s"`, `12500` as `"12.5 s"`. */
export function formatSeconds(ms) {
  const s = ms / 1000;
  return `${s.toFixed(s < 10 ? 2 : 1)} s`;
}

/**
 * A one-line summary of the list, for the progress bar's label and the live region.
 * `counts` has `total`, `checking`, `ready`, `converting`, `done`, `failed` (unsupported
 * files included), `skipped` (classic NEFs) and `running`.
 */
export function summary(counts) {
  const { total, checking, ready, converting, done, failed, skipped, running } = counts;
  if (total === 0) return "No files yet.";
  const parts = [];
  if (running) {
    const finished = done + failed;
    parts.push(`Converting: ${finished} of ${finished + converting + ready + checking} finished.`);
  } else {
    if (checking > 0) parts.push(`Checking ${plural(checking, "file")}.`);
    if (done > 0) parts.push(`${plural(done, "file")} converted.`);
    if (ready > 0) parts.push(`${plural(ready, "file")} ready to convert.`);
  }
  if (failed > 0) parts.push(`${plural(failed, "file")} could not be converted.`);
  if (skipped > 0) {
    parts.push(skipped === 1 ? "1 file needs no conversion." : `${skipped} files need no conversion.`);
  }
  return parts.join(" ");
}

function plural(n, word) {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}
