// End-to-end tests: the built page in headless Chromium, driven like a user would drive it.

import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { copyFileSync, mkdirSync, readFileSync, utimesSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { dngCfaSha256, manifest, readZip, sample, sha256, ticoraw } from "./helpers.js";

const MANIFEST = manifest();
const needSamples = (...ids) => {
  const missing = ids.filter((id) => sample(id) === null);
  if (missing.length > 0 && process.env.CI) {
    throw new Error(`TICORAW_SAMPLES lacks ${missing.join(", ")}`);
  }
  test.skip(missing.length > 0, `set TICORAW_SAMPLES to a folder with ${missing.join(", ")}`);
};

// Every request must go to the page's own server. Anything else is blocked and recorded,
// for the page and its workers alike.
let foreign;
test.beforeEach(async ({ context, baseURL }) => {
  foreign = [];
  const origin = new URL(baseURL).origin;
  context.on("request", (r) => {
    const u = new URL(r.url());
    if (u.protocol !== "blob:" && u.protocol !== "data:" && u.origin !== origin) {
      foreign.push(r.url());
    }
  });
  await context.route("**/*", (route) => {
    const u = new URL(route.request().url());
    return u.origin === origin ? route.continue() : route.abort("blockedbyclient");
  });
});

test.afterEach(() => {
  expect(foreign, "requests to other sites").toEqual([]);
});

async function open(page) {
  await page.goto("./");
  // The footer shows the decoder's version once the wasm module is running.
  await expect(page.locator("#version")).toHaveText(/^ticoraw \d+\.\d+\.\d+/);
}

const row = (page, path) => page.locator(`#rows tr[data-path="${path}"]`);

/** Converts the files in the list and waits until none is left to do. */
async function convertAll(page) {
  await page.locator("#convert").click();
  await expect(page.locator("#stop")).toBeHidden({ timeout: 200_000 });
  await expect(page.locator('#rows tr[data-state="converting"], #rows tr[data-state="ready"]')).toHaveCount(0);
}

test("converts HE samples bit-exactly, one by one and as a ZIP", async ({ page }, info) => {
  const ids = ["z6iii_he", "z5ii_he_dx"];
  needSamples(...ids);
  await open(page);
  await page.locator("#pick-files").setInputFiles(ids.map(sample));
  for (const id of ids) await expect(row(page, `${id}.nef`)).toHaveAttribute("data-state", "ready");
  await expect(page.locator("#convert")).toHaveText("Convert 2 files");
  await convertAll(page);
  await expect(page.locator("#summary")).toHaveText("2 files converted.");

  const singles = {};
  for (const id of ids) {
    const r = row(page, `${id}.nef`);
    await expect(r).toHaveAttribute("data-state", "done");
    const want = MANIFEST[id];
    const [download] = await Promise.all([
      page.waitForEvent("download"),
      r.getByRole("link", { name: `Download ${id}.dng` }).click(),
    ]);
    expect(download.suggestedFilename()).toBe(`${id}.dng`);
    const file = info.outputPath(`${id}.dng`);
    await download.saveAs(file);
    const dng = readFileSync(file);
    const got = await dngCfaSha256(dng);
    expect(got).toEqual({ sha: want.cfa_sha256, width: want.width, height: want.height });
    singles[`${id}.dng`] = sha256(dng);
    console.log(`${id}: ${(want.width * want.height / 1e6).toFixed(1)} MP, ` +
      `${await r.getAttribute("data-ms")} ms in the browser (2 files in parallel, cold workers)`);
  }

  const [zip] = await Promise.all([page.waitForEvent("download"), page.locator("#zip").click()]);
  expect(zip.suggestedFilename()).toBe("ticoraw-dng.zip");
  const zipFile = info.outputPath("all.zip");
  await zip.saveAs(zipFile);
  const entries = readZip(readFileSync(zipFile));
  expect(Object.fromEntries(entries.map((e) => [e.name, sha256(e.data)]))).toEqual(singles);

  // The finished list, with its download links, is accessible too.
  const results = await new AxeBuilder({ page }).analyze();
  const bad = results.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  expect(bad.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
});

test("the options reach the converter", async ({ page }, info) => {
  needSamples("z5ii_he_dx");
  await open(page);
  await page.getByRole("radio", { name: /Uncompressed/ }).check();
  await page.getByRole("radio", { name: /Small thumbnail only/ }).check();
  await page.getByRole("checkbox", { name: /XMP metadata/ }).uncheck();
  await page.locator("#pick-files").setInputFiles(sample("z5ii_he_dx"));
  await convertAll(page);
  const r = row(page, "z5ii_he_dx.nef");
  await expect(r).toHaveAttribute("data-state", "done");
  const [download] = await Promise.all([page.waitForEvent("download"), r.getByRole("link").click()]);
  const file = info.outputPath("u.dng");
  await download.saveAs(file);
  const dng = readFileSync(file);
  const want = MANIFEST.z5ii_he_dx;
  // Uncompressed: one 16-bit strip, so at least 2 bytes per photosite.
  expect(dng.length).toBeGreaterThan(want.width * want.height * 2);
  expect((await dngCfaSha256(dng)).sha).toBe(want.cfa_sha256);
  expect(dng.includes(Buffer.from("<x:xmpmeta"))).toBe(false);
});

test("folders: NEFs are found, other files left out, classic NEFs explained", async ({ page }, info) => {
  needSamples("z5ii_he_dx");
  const dir = info.outputPath("Card");
  mkdirSync(join(dir, "DCIM/100NZ5_2"), { recursive: true });
  copyFileSync(sample("z5ii_he_dx"), join(dir, "DCIM/100NZ5_2/DSC_0001.NEF"));
  writeFileSync(join(dir, "DCIM/100NZ5_2/DSC_0001.JPG"), "not a raw file");
  writeFileSync(join(dir, "notes.txt"), "hello");
  // A classic NEF: the same file with the TicoRAW magic at the start of its raw strip
  // overwritten, which is exactly what tells the two apart.
  const nef = readFileSync(sample("z5ii_he_dx"));
  const { offset } = (await ticoraw()).inspect(nef).strip;
  nef.fill(0, offset, offset + 4);
  writeFileSync(join(dir, "DCIM/100NZ5_2/DSC_0002.NEF"), nef);

  await open(page);
  await page.locator("#pick-folder").setInputFiles(dir);
  await expect(row(page, "Card/DCIM/100NZ5_2/DSC_0001.NEF")).toHaveAttribute("data-state", "ready");
  const classic = row(page, "Card/DCIM/100NZ5_2/DSC_0002.NEF");
  await expect(classic).toHaveAttribute("data-state", "classic");
  await expect(classic).toContainText("nothing to convert");
  await expect(page.locator("#ignored")).toHaveText(
    "2 files were left out because they are not NEF or NRW files.",
  );
  await expect(page.locator("#rows tr")).toHaveCount(2);
  await expect(page.locator("#convert")).toHaveText("Convert 1 file");
  await expect(page.locator("#summary")).toHaveText(
    "1 file ready to convert. 1 file needs no conversion.",
  );
});

test("a file that is not a NEF gets a friendly message", async ({ page }) => {
  await open(page);
  await page.locator("#pick-files").setInputFiles([
    { name: "broken.NEF", mimeType: "application/octet-stream", buffer: Buffer.from("not a raw file at all") },
    { name: "holiday.jpg", mimeType: "image/jpeg", buffer: Buffer.from([0xff, 0xd8, 0xff]) },
  ]);
  const r = row(page, "broken.NEF");
  await expect(r).toHaveAttribute("data-state", "failed");
  await expect(r).toContainText("This is not a Nikon raw file ticoraw can read.");
  await expect(page.locator("#ignored")).toHaveText(
    "1 file was left out because it is not a NEF or NRW file.",
  );
  await expect(page.locator("#convert")).toBeDisabled();
});

test("dropping files adds them", async ({ page }) => {
  await open(page);
  const dt = await page.evaluateHandle(() => {
    const d = new DataTransfer();
    d.items.add(new File(["x"], "dropped.NEF"));
    d.items.add(new File(["y"], "skip.txt"));
    return d;
  });
  await page.locator("#drop").dispatchEvent("dragenter", { dataTransfer: dt });
  await page.locator("#drop").dispatchEvent("drop", { dataTransfer: dt });
  await expect(row(page, "dropped.NEF")).toHaveAttribute("data-state", "failed");
  await expect(page.locator("#ignored")).toBeVisible();
});

test("the Content-Security-Policy blocks other sites", async ({ page }) => {
  await open(page);
  const csp = await page.locator('meta[http-equiv="Content-Security-Policy"]').getAttribute("content");
  for (const d of ["default-src 'self'", "script-src 'self' 'wasm-unsafe-eval'", "connect-src 'self'"]) {
    expect(csp).toContain(d);
  }
  const violations = await page.evaluate(async () => {
    const seen = [];
    document.addEventListener("securitypolicyviolation", (e) => seen.push(e.violatedDirective));
    const fetched = await fetch("https://example.com/upload", { method: "POST", body: "x" }).then(
      () => "sent",
      () => "blocked",
    );
    const img = new Image();
    img.src = "https://example.com/pixel.png";
    const s = document.createElement("script");
    s.src = "https://example.com/analytics.js";
    document.head.append(s);
    await new Promise((r) => setTimeout(r, 500));
    return { fetched, seen };
  });
  expect(violations.fetched).toBe("blocked");
  expect(violations.seen).toEqual(expect.arrayContaining(["connect-src", "img-src", "script-src-elem"]));
  // Playwright still reports the image and script requests that the policy blocked (and
  // the route above would abort them anyway). They were made on purpose here; every other
  // test requires that no request leaves the page's own server.
  foreign = foreign.filter((u) => !u.startsWith("https://example.com/"));
});

test("keyboard: skip link first, file picker from the keyboard", async ({ page }) => {
  await open(page);
  await page.keyboard.press("Tab");
  await expect(page.locator(".skip")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.locator("#main")).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.locator("#pick-files-button")).toBeFocused();
  // Enter on the button opens the file input. (Recorded on the input itself: headless
  // Chromium's filechooser event proved flaky.)
  await page.locator("#pick-files").evaluate((input) => {
    input.addEventListener("click", (ev) => {
      ev.preventDefault();
      input.dataset.opened = "yes";
    });
  });
  await page.keyboard.press("Enter");
  await expect(page.locator("#pick-files")).toHaveAttribute("data-opened", "yes");
  await expect(page.locator("#pick-files")).toHaveAttribute("multiple", "");
});

for (const scheme of ["light", "dark"]) {
  test(`accessibility (${scheme}): no serious or critical axe violations`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme, reducedMotion: "reduce" });
    await open(page);
    // Fill the list too, with a failed and an ignored file, so the table is checked.
    await page.locator("#pick-files").setInputFiles([
      { name: "broken.NEF", mimeType: "application/octet-stream", buffer: Buffer.from("x") },
      { name: "a.txt", mimeType: "text/plain", buffer: Buffer.from("x") },
    ]);
    await expect(row(page, "broken.NEF")).toHaveAttribute("data-state", "failed");
    await page.locator("details").first().evaluate((d) => {
      d.open = true;
    });
    for (const width of [1280, 375]) {
      await page.setViewportSize({ width, height: 900 });
      const results = await new AxeBuilder({ page }).analyze();
      const bad = results.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
      expect(bad.map((v) => `${v.id}: ${v.help} (${v.nodes.length})`), `width ${width}`).toEqual([]);
      // No horizontal scrolling at phone width.
      const overflow = await page.evaluate(() => document.documentElement.scrollWidth - innerWidth);
      expect(overflow).toBeLessThanOrEqual(0);
    }
  });
}

test("timing: 24 MP and 45 MP frames", async ({ page }) => {
  const ids = ["z6iii_he", "z9_he"];
  needSamples(...ids);
  await open(page);
  const workers = await page.locator("#workers").textContent();
  const times = {};
  // One file at a time, so each time is one worker's conversion (file read, transfer and
  // toDng). The page keeps its workers, so the first round starts cold (a fresh worker for
  // the 24 MP frame, wasm memory still growing for the 45 MP one) and the second is warm.
  for (const round of ["cold", "warm"]) {
    for (const id of ids) {
      await page.locator("#pick-files").setInputFiles(sample(id));
      await convertAll(page);
      const r = row(page, `${id}.nef`);
      await expect(r).toHaveAttribute("data-state", "done");
      (times[id] ??= {})[round] = Number(await r.getAttribute("data-ms"));
      await page.locator("#clear").click();
    }
  }
  for (const id of ids) {
    const mp = (MANIFEST[id].width * MANIFEST[id].height) / 1e6;
    console.log(`${id} (${mp.toFixed(1)} MP): ${times[id].cold} ms cold, ${times[id].warm} ms warm`);
  }
  console.log(`${workers.trim()} (navigator.hardwareConcurrency in this browser: ${await page.evaluate(() => navigator.hardwareConcurrency)})`);
});

// Folder mode, with showDirectoryPicker answering with the origin private file system: the
// same FileSystemDirectoryHandle API that a folder on disk gets.
async function openWithFolder(page, setUp = () => {}) {
  await page.addInitScript(() => {
    window.showDirectoryPicker = async () => navigator.storage.getDirectory();
  });
  await page.addInitScript(setUp);
  await open(page);
  await page.getByRole("radio", { name: /straight into a folder/ }).check();
}

/** `{ name: size }` for the files in the picked folder (its top level). */
const folderFiles = (page) =>
  page.evaluate(async () => {
    const out = {};
    for await (const [name, h] of (await navigator.storage.getDirectory()).entries()) {
      out[name] = h.kind === "file" ? (await h.getFile()).size : "folder";
    }
    return out;
  });

/** Copies of sample `id` as `names` (relative paths) under a fresh folder; their paths. */
function copies(info, id, names) {
  return names.map((name, i) => {
    const p = info.outputPath("in", name);
    mkdirSync(dirname(p), { recursive: true });
    copyFileSync(sample(id), p);
    // Distinct times, so that the page does not take two same-named copies for one file.
    utimesSync(p, 1_700_000_000 + i, 1_700_000_000 + i);
    return p;
  });
}

test("folder: names never collide, an existing DNG is skipped, never replaced", async ({ page }, info) => {
  needSamples("z5ii_he_dx");
  await openWithFolder(page);
  await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const w = await (await root.getFileHandle("DSC_0002.dng", { create: true })).createWritable();
    await w.write("old");
    await w.close();
  });
  // Three files whose DNG names are the same (two differ only in case), and one whose DNG
  // is already there.
  const names = ["a/DSC_0001.NEF", "b/DSC_0001.NEF", "c/dsc_0001.nef", "d/DSC_0002.NEF"];
  await page.locator("#pick-files").setInputFiles(copies(info, "z5ii_he_dx", names));
  await expect(page.locator('#rows tr[data-state="ready"]')).toHaveCount(4);
  await convertAll(page);

  await expect(page.locator('#rows tr[data-state="done"]')).toHaveCount(3);
  const skipped = page.locator('#rows tr[data-state="exists"]');
  await expect(skipped).toHaveCount(1);
  await expect(skipped).toContainText("Skipped: DSC_0002.dng already exists in the folder.");
  await expect(page.locator("#summary")).toHaveText(
    "3 files converted. 1 file skipped: its DNG already exists in the folder.",
  );
  const cells = await page.locator('#rows tr[data-state="done"] td[data-label="DNG"]').allTextContents();
  const saved = cells.map((t) => /^Saved as (.+) \(/.exec(t)[1]).sort();
  expect(saved).toEqual(["DSC_0001 (2).dng", "DSC_0001.dng", "dsc_0001 (3).dng"]);
  // Every row that says "Saved as" has its own file, and nothing else was written.
  const onDisk = await folderFiles(page);
  expect(Object.keys(onDisk).sort()).toEqual([...saved, "DSC_0002.dng"].sort());
  expect(onDisk["DSC_0002.dng"]).toBe(3);
  for (const n of saved) expect(onDisk[n]).toBeGreaterThan(1_000_000);
  const b64 = await page.evaluate(async () => {
    const root = await navigator.storage.getDirectory();
    const file = await (await root.getFileHandle("dsc_0001 (3).dng")).getFile();
    const url = await new Promise((resolve) => {
      const fr = new FileReader();
      fr.onload = () => resolve(fr.result);
      fr.readAsDataURL(file);
    });
    return url.slice(url.indexOf(",") + 1);
  });
  expect((await dngCfaSha256(Buffer.from(b64, "base64"))).sha).toBe(MANIFEST.z5ii_he_dx.cfa_sha256);
  // Nothing is left to convert: focus goes to the summary, from where Tab reaches the list.
  await expect(page.locator("#summary")).toBeFocused();
});

test("folder: a failed write leaves no file behind", async ({ page }, info) => {
  needSamples("z5ii_he_dx");
  await openWithFolder(page, () => {
    FileSystemWritableFileStream.prototype.write = () =>
      Promise.reject(new DOMException("The disk is full.", "QuotaExceededError"));
  });
  await page.locator("#pick-files").setInputFiles(copies(info, "z5ii_he_dx", ["DSC_0001.NEF"]));
  await convertAll(page);
  await expect(page.locator('#rows tr[data-state="failed"]')).toHaveCount(1);
  expect(await folderFiles(page)).toEqual({});
});

test("folder: when saving is no longer allowed, the run stops cleanly", async ({ page }, info) => {
  needSamples("z5ii_he_dx");
  // The first file is saved; then the browser refuses every write, as when the permission
  // is revoked during a run.
  await openWithFolder(page, () => {
    const write = FileSystemWritableFileStream.prototype.write;
    let calls = 0;
    FileSystemWritableFileStream.prototype.write = function (...args) {
      if (calls++ === 0) return write.apply(this, args);
      return Promise.reject(new DOMException("Permission revoked.", "NotAllowedError"));
    };
  });
  const names = Array.from({ length: 8 }, (_, i) => `DSC_000${i + 1}.NEF`);
  await page.locator("#pick-files").setInputFiles(copies(info, "z5ii_he_dx", names));
  await expect(page.locator('#rows tr[data-state="ready"]')).toHaveCount(8);
  await page.locator("#convert").click();
  await expect(page.locator("#stop")).toBeHidden({ timeout: 200_000 });

  await expect(page.locator('#rows tr[data-state="done"]')).toHaveCount(1);
  // The others wait for a folder that can be written: no failures, no "Report a problem".
  await expect(page.locator('#rows tr[data-state="ready"]')).toHaveCount(7);
  await expect(page.locator("#rows")).toContainText("Not saved: the browser did not allow saving");
  await expect(page.locator("#rows")).not.toContainText("Report a problem");
  await expect(page.locator("#rows")).not.toContainText("TypeError");
  await expect(page.locator("#folder-chosen")).toHaveText(/did not allow saving into that folder/);
  const files = await folderFiles(page);
  expect(Object.keys(files)).toHaveLength(1);
  expect(Object.values(files)[0]).toBeGreaterThan(1_000_000);
  // Files are left to convert, so focus goes back to Convert.
  await expect(page.locator("#convert")).toHaveText("Convert 7 files");
  await expect(page.locator("#convert")).toBeFocused();
});

test("keyboard: after Stop, focus is on Convert", async ({ page }, info) => {
  needSamples("z9_he");
  await open(page);
  const names = Array.from({ length: 8 }, (_, i) => `z9_${i + 1}.NEF`);
  await page.locator("#pick-files").setInputFiles(copies(info, "z9_he", names));
  await expect(page.locator('#rows tr[data-state="ready"]')).toHaveCount(8);
  await page.locator("#convert").focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#stop")).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.locator("#stop")).toBeHidden();
  await expect(page.locator("#rows")).toContainText("Stopped. Convert again to resume.");
  await expect(page.locator("#convert")).toBeEnabled();
  await expect(page.locator("#convert")).toBeFocused();
});

test("ZIP entry names cannot point outside the folder they are extracted into", async ({ page }, info) => {
  needSamples("z5ii_he_dx");
  await open(page);
  const nef = readFileSync(sample("z5ii_he_dx"));
  await page.locator("#pick-files").setInputFiles([
    { name: "..\\..\\evil.NEF", mimeType: "application/octet-stream", buffer: nef },
    { name: "a\\b.NEF", mimeType: "application/octet-stream", buffer: nef },
  ]);
  await convertAll(page);
  await expect(page.locator('#rows tr[data-state="done"]')).toHaveCount(2);
  const [zip] = await Promise.all([page.waitForEvent("download"), page.locator("#zip").click()]);
  const file = info.outputPath("names.zip");
  await zip.saveAs(file);
  expect(readZip(readFileSync(file)).map((e) => e.name).sort()).toEqual(["a/b.dng", "evil.dng"]);
});
