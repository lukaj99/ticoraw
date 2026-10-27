// The page: wires the DOM to the decoder. The decisions (what a file is, what an error
// means, how many workers to run) live in core.js; the ZIP format in zip.js; folder walking
// and saving in files.js.
//
// Files go through these states:
//   checking -> ready -> converting -> done | failed | exists
//            -> classic | unsupported | failed        (decided from the file's headers)
// "exists": folder mode found a DNG of that name already there, so nothing was written.
// "Stop", and a folder the browser no longer lets the page write to, return converting
// files to ready.

import init, { inspectPrefix, version } from "../pkg/web/ticoraw.js";
import { TicorawWorker } from "../pkg/worker.js";
import {
  CAMERA_SUPPORT_URL,
  PREFIX_BYTES,
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
} from "./core.js";
import { canSaveToFolder, existsIn, fromDrop, fromInput, removeLeftovers, saveInto } from "./files.js";
import { crc32, safeEntryName, zipParts } from "./zip.js";

const $ = (id) => document.getElementById(id);
const ui = {
  fatal: $("fatal"),
  drop: $("drop"),
  pickFiles: $("pick-files"),
  pickFolder: $("pick-folder"),
  options: $("options"),
  folderOption: $("folder-option"),
  folderNote: $("folder-note"),
  folderChosen: $("folder-chosen"),
  workers: $("workers"),
  convert: $("convert"),
  stop: $("stop"),
  zip: $("zip"),
  clear: $("clear"),
  progress: $("progress"),
  summary: $("summary"),
  ignored: $("ignored"),
  queue: $("queue"),
  rows: $("rows"),
  version: $("version"),
};

const workerCount = poolSize({
  cores: navigator.hardwareConcurrency,
  memoryGb: navigator.deviceMemory,
});

/** Every file in the list, in the order it was added. */
const items = [];
const seen = new Set();
let nextId = 1;
let ignoredCount = 0;
/** Resolves when every file added so far has been classified. */
let checking = Promise.resolve();
let pool = null;
let running = false;
let stopping = false;
/**
 * The folder DNGs are saved into, or null: `{ dir, claimed }`. `dir` is the directory handle;
 * `claimed` holds the lower-cased paths given to files of this list, so that no two of them
 * get the same name, whatever the order their conversions finish in.
 */
let folder = null;
/** Set when the browser refused to write into `folder`; the run then stops. */
let folderDenied = false;
/** Lower-cased names given to converted files so far, so that ZIP entries are unique. */
const taken = new Set();

const wasm = init().then(
  () => {
    ui.version.textContent = `ticoraw ${version()}`;
  },
  (e) => {
    showFatal(
      "The converter could not start in this browser. Please use a recent version of " +
        `Chrome, Edge, Firefox or Safari. (${e?.message ?? e})`,
    );
    throw e;
  },
);

setUp();

function setUp() {
  ui.workers.textContent =
    workerCount === 1
      ? "This device converts one file at a time."
      : `This device converts up to ${workerCount} files at a time.`;

  if (!canSaveToFolder()) {
    ui.folderOption.querySelector("input").disabled = true;
    ui.folderOption.classList.add("disabled");
    ui.folderNote.hidden = false;
  }

  $("pick-files-button").addEventListener("click", () => ui.pickFiles.click());
  $("pick-folder-button").addEventListener("click", () => ui.pickFolder.click());
  for (const input of [ui.pickFiles, ui.pickFolder]) {
    input.addEventListener("change", () => {
      addFiles(fromInput(input));
      input.value = "";
    });
  }

  // Dropping anywhere on the page adds files; without this, a missed drop would make the
  // browser leave the page to open the NEF.
  let depth = 0;
  window.addEventListener("dragenter", (ev) => {
    if (!hasFiles(ev)) return;
    depth++;
    ui.drop.classList.add("over");
  });
  window.addEventListener("dragleave", () => {
    depth = Math.max(0, depth - 1);
    if (depth === 0) ui.drop.classList.remove("over");
  });
  window.addEventListener("dragover", (ev) => {
    if (!hasFiles(ev)) return;
    ev.preventDefault();
    ev.dataTransfer.dropEffect = "copy";
  });
  window.addEventListener("drop", (ev) => {
    if (!hasFiles(ev)) return;
    ev.preventDefault();
    depth = 0;
    ui.drop.classList.remove("over");
    fromDrop(ev.dataTransfer).then(addFiles, (e) =>
      showFatal(`Could not read what was dropped: ${e?.message ?? e}`),
    );
  });

  // In-page links ("#main", "#patent-notice") stay in the page even when a <base> element
  // (build.sh BASE_PATH) would resolve them to another URL and lose the list.
  document.addEventListener("click", (ev) => {
    const a = ev.target.closest?.('a[href^="#"]');
    const target = a && document.getElementById(a.getAttribute("href").slice(1));
    if (!target) return;
    ev.preventDefault();
    if (!target.hasAttribute("tabindex")) target.setAttribute("tabindex", "-1");
    target.focus();
    target.scrollIntoView();
  });

  ui.convert.addEventListener("click", convert);
  ui.stop.addEventListener("click", stop);
  ui.zip.addEventListener("click", downloadZip);
  ui.clear.addEventListener("click", clearList);
  ui.options.addEventListener("change", (ev) => {
    if (ev.target.name !== "output") return;
    folder = null;
    ui.folderChosen.hidden = true;
  });
  ui.options.addEventListener("submit", (ev) => ev.preventDefault());

  window.addEventListener("beforeunload", (ev) => {
    if (running || items.some((i) => i.state === "done" && i.out && !i.downloaded)) {
      ev.preventDefault();
    }
  });
}

function hasFiles(ev) {
  return Array.from(ev.dataTransfer?.types ?? []).includes("Files");
}

/** Adds `{ file, path }` pairs to the list and starts classifying them. */
function addFiles(pairs) {
  let added = 0;
  for (const { file, path } of pairs) {
    if (!isRawName(file.name)) {
      ignoredCount++;
      continue;
    }
    const key = `${path}\u0000${file.size}\u0000${file.lastModified}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const item = { id: nextId++, file, path, key, state: "checking", model: null };
    items.push(item);
    ui.rows.append(makeRow(item));
    added++;
    checking = checking.then(() => check(item));
  }
  ui.ignored.hidden = ignoredCount === 0;
  ui.ignored.textContent =
    ignoredCount === 1
      ? "1 file was left out because it is not a NEF or NRW file."
      : `${ignoredCount} files were left out because they are not NEF or NRW files.`;
  if (added > 0) ui.queue.hidden = false;
  refresh();
  return checking;
}

/** Reads the start of a file and decides what to do with it. */
async function check(item) {
  try {
    await wasm;
    const size = item.file.size;
    let verdict = { state: "retry", prefixBytes: Math.min(PREFIX_BYTES, size) };
    // At most three reads: each retry asks for a strictly longer prefix.
    for (let tries = 0; verdict.state === "retry" && tries < 3; tries++) {
      const len = verdict.prefixBytes;
      const prefix = new Uint8Array(await item.file.slice(0, len).arrayBuffer());
      try {
        verdict = classify(inspectPrefix(prefix, size));
      } catch (e) {
        verdict = classifyFailure(e, { prefixBytes: len, fileBytes: size });
      }
    }
    if (verdict.state === "retry") verdict = { state: "ready", model: null, verified: true };
    if (item.state === "checking") Object.assign(item, verdict);
  } catch (e) {
    // The file could not be read (moved, deleted, permission revoked) or wasm failed.
    if (item.state === "checking") Object.assign(item, failure(e));
  }
  renderRow(item);
  refresh();
}

async function convert() {
  if (running) return;
  const choice = readOptions();
  if (choice.output === "folder" && folder === null) {
    try {
      const dir = await showDirectoryPicker({ id: "ticoraw-dng", mode: "readwrite" });
      folder = { dir, claimed: new Set() };
    } catch (e) {
      if (e?.name === "AbortError") return; // the user closed the picker
      showFatal(`Could not open that folder: ${e?.message ?? e}`);
      return;
    }
    ui.folderChosen.hidden = false;
    ui.folderChosen.textContent = `Saving into the folder “${folder.dir.name}”.`;
  }
  const out = choice.output === "folder" ? folder : null;

  running = true;
  stopping = false;
  folderDenied = false;
  setRunning(true);
  pool ??= new TicorawWorker({ size: workerCount, url: new URL("../pkg/worker.js", import.meta.url) });
  const lane = async () => {
    for (;;) {
      await checking;
      if (stopping || folderDenied) return;
      const item = items.find((i) => i.state === "ready");
      if (!item) return;
      await convertOne(item, choice, out);
    }
  };
  await Promise.all(Array.from({ length: workerCount }, lane));
  // A save that failed part-way may not have been able to remove its temporary file yet.
  if (out) await removeLeftovers();
  running = false;
  if (folderDenied) {
    // Ask for a folder again next time.
    folder = null;
    ui.folderChosen.hidden = false;
    ui.folderChosen.textContent =
      "The browser did not allow saving into that folder. Press Convert to choose a folder again.";
  }
  refresh();
  setRunning(false);
}

const STOPPED = "Stopped. Convert again to resume.";
const NOT_SAVED =
  "Not saved: the browser did not allow saving into the folder. Convert again to choose a folder.";

/** Converts one file, into the folder `out` (`{ dir, claimed }`) or, if null, the page. */
async function convertOne(item, choice, out) {
  item.state = "converting";
  item.note = null;
  renderRow(item);
  refresh();
  const path = safeEntryName(dngName(item.path));
  // Claimed before the first await, so that no other lane can be given the same name.
  const target = out ? uniqueName(path, out.claimed) : null;
  let keepClaim = false;
  try {
    if (out && (await existsIn(out.dir, target))) {
      // Already converted into this folder, or another file has that name: never overwrite.
      Object.assign(item, { state: "exists", existing: target });
      keepClaim = true;
    } else {
      const t0 = performance.now();
      const dng = await pool.toDng(item.file, dngOptions(choice, item.file.name));
      item.ms = performance.now() - t0;
      if (!out) {
        const crc = crc32(dng);
        const blob = new Blob([dng], { type: "image/x-adobe-dng" });
        item.out = { name: uniqueName(path, taken), blob, size: dng.length, crc, url: null };
        item.state = "done";
      } else if (folderDenied) {
        Object.assign(item, { state: "ready", note: NOT_SAVED });
      } else if ((await saveInto(out.dir, target, new Blob([dng]))) === "exists") {
        Object.assign(item, { state: "exists", existing: target });
        keepClaim = true;
      } else {
        Object.assign(item, { state: "done", savedAs: target, out: { size: dng.length } });
        keepClaim = true;
      }
    }
  } catch (e) {
    if (stopping) {
      Object.assign(item, { state: "ready", note: STOPPED });
    } else if (out && e?.name === "NotAllowedError") {
      // Permission to the folder was refused or revoked: stop taking files, keep this one.
      folderDenied = true;
      Object.assign(item, { state: "ready", note: NOT_SAVED });
    } else {
      Object.assign(item, failure(e));
    }
  }
  if (target !== null && !keepClaim) out.claimed.delete(target.toLowerCase());
  renderRow(item);
  refresh();
}

function stop() {
  if (!running) return;
  stopping = true;
  pool?.terminate();
  pool = null;
}

function downloadZip() {
  const done = items.filter((i) => i.state === "done" && i.out?.blob);
  if (done.length === 0) return;
  const now = new Date();
  const entries = done.map((i) => ({
    name: i.out.name,
    data: i.out.blob,
    size: i.out.size,
    crc: i.out.crc,
    date: i.file.lastModified ? new Date(i.file.lastModified) : now,
  }));
  const blob = new Blob(zipParts(entries), { type: "application/zip" });
  save(blob, "ticoraw-dng.zip");
  for (const i of done) i.downloaded = true;
}

function clearList() {
  if (running) return;
  for (const i of items) if (i.out?.url) URL.revokeObjectURL(i.out.url);
  items.length = 0;
  seen.clear();
  taken.clear();
  folder?.claimed.clear();
  ignoredCount = 0;
  ui.ignored.hidden = true;
  ui.rows.replaceChildren();
  ui.queue.hidden = true;
  refresh();
}

/** Starts a download of `blob` as `name`. */
function save(blob, name) {
  const url = URL.createObjectURL(blob);
  const a = Object.assign(document.createElement("a"), { href: url, download: name });
  document.body.append(a);
  a.click();
  a.remove();
  // Revoking at once can cancel the download in some browsers.
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
}

function readOptions() {
  const f = new FormData(ui.options);
  return {
    compression: f.get("compression"),
    preview: f.get("preview"),
    keepXmp: f.get("keepXmp") === "on",
    output: f.get("output") === "folder" && canSaveToFolder() ? "folder" : "page",
  };
}

/** Swaps Convert and Stop. Call `refresh()` first, so that Convert is enabled if it can be. */
function setRunning(on) {
  // Hiding Stop or disabling the options drops their focus to <body>: note it first.
  const lost = document.activeElement === document.body || document.activeElement === ui.stop;
  ui.convert.hidden = on;
  ui.stop.hidden = !on;
  for (const el of ui.options.elements) {
    if (el.closest("#folder-option") && !canSaveToFolder()) continue;
    el.disabled = on;
  }
  if (on) ui.stop.focus();
  // After a run: Convert if there is more to convert (after Stop), else the summary of the
  // run, from where Tab reaches the results.
  else if (lost) (ui.convert.disabled ? ui.summary : ui.convert).focus();
}

/** Updates the buttons, the progress bar and the summary from the list. */
function refresh() {
  const n = (s) => items.filter((i) => i.state === s).length;
  const counts = {
    total: items.length,
    checking: n("checking"),
    ready: n("ready"),
    converting: n("converting"),
    done: n("done"),
    failed: n("failed") + n("unsupported"),
    skipped: n("classic"),
    exists: n("exists"),
  };
  const text = summary({ ...counts, running });
  if (ui.summary.textContent !== text) ui.summary.textContent = text;
  const finished = counts.done + counts.failed + counts.exists;
  ui.progress.max = Math.max(1, finished + counts.ready + counts.converting);
  ui.progress.value = finished;
  const toConvert = counts.ready + counts.checking;
  ui.convert.disabled = running || toConvert === 0;
  ui.convert.textContent =
    toConvert > 0 ? `Convert ${toConvert === 1 ? "1 file" : `${toConvert} files`}` : "Convert";
  const zippable = items.filter((i) => i.state === "done" && i.out?.blob);
  ui.zip.disabled = zippable.length === 0;
  ui.zip.textContent =
    zippable.length > 0
      ? `Download all as ZIP (${formatBytes(zippable.reduce((a, i) => a + i.out.size, 0))})`
      : "Download all as ZIP";
  ui.clear.disabled = running || items.length === 0;
}

function makeRow(item) {
  const tr = document.createElement("tr");
  tr.id = `file-${item.id}`;
  tr.dataset.path = item.path;
  for (const label of ["File", "Camera", "Status", "DNG"]) {
    const td = document.createElement("td");
    td.dataset.label = label;
    tr.append(td);
  }
  tr.cells[0].textContent = item.path;
  item.row = tr;
  renderRow(item);
  return tr;
}

const STATUS = {
  checking: "Checking…",
  ready: "Ready to convert",
  converting: "Converting…",
  classic:
    "Classic NEF: your editor already opens this file, so there is nothing to convert.",
};

function renderRow(item) {
  const tr = item.row;
  if (!tr) return;
  tr.dataset.state = item.state;
  const [, camera, status, result] = tr.cells;
  camera.textContent = item.model ? cameraName(item.model) : "";
  status.replaceChildren();
  result.replaceChildren();
  if (item.ms !== undefined) tr.dataset.ms = String(Math.round(item.ms));

  if (item.state === "done") {
    status.textContent = `Converted in ${formatSeconds(item.ms)}`;
    if (item.verified === false) {
      status.append(
        document.createElement("br"),
        "This camera is not verified yet: colours may be off. ",
        link(CAMERA_SUPPORT_URL, "Tell us about your camera"),
      );
    }
    if (item.savedAs) {
      result.textContent = `Saved as ${item.savedAs} (${formatBytes(item.out.size)})`;
    } else {
      const base = item.out.name.slice(item.out.name.lastIndexOf("/") + 1);
      item.out.url ??= URL.createObjectURL(item.out.blob);
      const a = Object.assign(document.createElement("a"), {
        href: item.out.url,
        download: base,
        className: "button",
      });
      a.textContent = `Download ${base}`;
      a.append(hidden(` (${formatBytes(item.out.size)})`));
      a.addEventListener("click", () => {
        item.downloaded = true;
      });
      const size = document.createElement("span");
      size.className = "small";
      size.setAttribute("aria-hidden", "true");
      size.textContent = ` ${formatBytes(item.out.size)}`;
      result.append(a, size);
    }
  } else if (item.state === "exists") {
    status.textContent = `Skipped: ${item.existing} already exists in the folder.`;
  } else if (item.state === "failed" || item.state === "unsupported") {
    status.append(item.message, " ");
    if (item.link) status.append(link(item.link.href, item.link.text));
    if (item.detail) {
      const d = document.createElement("details");
      const s = document.createElement("summary");
      s.textContent = "Details";
      const code = document.createElement("code");
      code.textContent = item.detail;
      d.append(s, code);
      status.append(d);
    }
  } else {
    status.textContent = (item.state === "ready" && item.note) || STATUS[item.state];
  }
}

/** `"NIKON Z6_3"` as `"Nikon Z6_3"`: the TIFF model, less shouty. */
function cameraName(model) {
  return model.replace(/^NIKON\b/, "Nikon");
}

function link(href, text) {
  const a = Object.assign(document.createElement("a"), { href, target: "_blank", rel: "noopener" });
  a.append(text, hidden(" (opens in a new tab)"));
  return a;
}

function hidden(text) {
  const s = document.createElement("span");
  s.className = "visually-hidden";
  s.textContent = text;
  return s;
}

function showFatal(message) {
  ui.fatal.textContent = message;
  ui.fatal.hidden = false;
}
