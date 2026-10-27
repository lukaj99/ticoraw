// Turns what the user picked or dropped into `{ file, path }` pairs, walking folders.
// `path` is relative to what was picked, with `/` between folders: `"DSC_0001.NEF"` for a
// loose file, `"Card/DCIM/100NZ6_3/DSC_0001.NEF"` for a file inside a dropped folder.

/** Files from an `<input type="file">`, with or without `webkitdirectory`. */
export function fromInput(input) {
  return Array.from(input.files, (file) => ({ file, path: file.webkitRelativePath || file.name }));
}

/**
 * Files from a drop event's DataTransfer. Folders are walked recursively. Each item is
 * read through `getAsFileSystemHandle` where it exists (Chromium), else `webkitGetAsEntry`
 * (every current browser), else as a plain File, which cannot be a folder.
 *
 * Both entry APIs must be called synchronously inside the drop handler, before any
 * `await`: the DataTransfer is emptied once the handler returns. So this function grabs
 * every item's handle or entry first and only then starts walking.
 */
export function fromDrop(dataTransfer) {
  const items = Array.from(dataTransfer.items ?? []).filter((i) => i.kind === "file");
  if (items.length === 0) {
    return Promise.resolve(
      Array.from(dataTransfer.files ?? [], (file) => ({ file, path: file.name })),
    );
  }
  // An item without a handle or entry (a file dragged from some apps, or built by a
  // script) still has its File: fall back to that.
  const grabbed = items.map((i) => ({
    file: i.getAsFile(),
    handle: typeof i.getAsFileSystemHandle === "function" ? i.getAsFileSystemHandle() : null,
    entry: typeof i.webkitGetAsEntry === "function" ? i.webkitGetAsEntry() : null,
  }));
  return collect(
    grabbed.map(async ({ file, handle, entry }) => {
      const h = await handle?.catch(() => null);
      if (h) return walkHandle(h, "");
      if (entry) return walkEntry(entry, "");
      return file ? [{ file, path: file.name }] : [];
    }),
  );
}

async function collect(walks) {
  return (await Promise.all(walks)).flat();
}

async function walkHandle(handle, prefix) {
  const path = prefix + handle.name;
  if (handle.kind === "file") return [{ file: await handle.getFile(), path }];
  const children = [];
  for await (const child of handle.values()) children.push(walkHandle(child, `${path}/`));
  return collect(children);
}

async function walkEntry(entry, prefix) {
  const path = prefix + entry.name;
  if (entry.isFile) {
    const file = await new Promise((resolve, reject) => entry.file(resolve, reject));
    return [{ file, path }];
  }
  const reader = entry.createReader();
  const children = [];
  // readEntries returns at most 100 entries per call; an empty batch means the end.
  for (;;) {
    const batch = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
    if (batch.length === 0) break;
    for (const child of batch) children.push(walkEntry(child, `${path}/`));
  }
  return collect(children);
}

/**
 * Whether this browser can write files into a folder the user picks (the File System
 * Access API: Chromium-based browsers on desktop).
 */
export function canSaveToFolder() {
  return typeof globalThis.showDirectoryPicker === "function";
}

/** Whether `path` (folders separated by `/`) names a file or folder inside `root`. */
export async function existsIn(root, path) {
  const parts = path.split("/");
  const name = parts.pop();
  let dir = root;
  for (const p of parts) {
    try {
      dir = await dir.getDirectoryHandle(p);
    } catch (e) {
      if (e?.name === "NotFoundError") return false;
      throw e;
    }
  }
  return exists(dir, name);
}

/** Set once a rename has failed, so later files are written under their final name. */
let moveFails = false;

/**
 * Writes `blob` as `path` (folders separated by `/`) inside the directory handle `root`,
 * creating folders as needed. Returns `"written"`, or `"exists"` without writing anything
 * when a file or folder of that name is already there: it never overwrites.
 *
 * The data goes to `<name>.partial` first and is renamed to `<name>` once it is complete
 * (`FileSystemHandle.move`), so a failed write never leaves a partial DNG under the final
 * name. Where `move` is missing or fails, the file is written under its final name, and
 * removed again if writing fails.
 *
 * Two calls must not write the same `path` at the same time; the caller keeps names unique.
 */
export async function saveInto(root, path, blob) {
  const parts = path.split("/");
  const name = parts.pop();
  let dir = root;
  for (const p of parts) dir = await dir.getDirectoryHandle(p, { create: true });
  if (await exists(dir, name)) return "exists";
  if (!moveFails && typeof globalThis.FileSystemFileHandle?.prototype.move === "function") {
    const temp = `${name}.partial`;
    const entry = { dir, name: temp };
    temporaries.add(entry);
    const handle = await write(dir, temp, blob);
    try {
      // Another program may have created the file meanwhile; move() would replace it.
      if (await exists(dir, name)) {
        await remove(dir, temp);
        return "exists";
      }
      await handle.move(name);
      temporaries.delete(entry);
      return "written";
    } catch (e) {
      await remove(dir, temp);
      if (e?.name === "NotAllowedError") throw e;
      moveFails = true;
    }
  }
  await write(dir, name, blob);
  return "written";
}

/** Writes `blob` as the file `name` in `dir`; on failure removes it and rethrows. */
async function write(dir, name, blob) {
  const handle = await dir.getFileHandle(name, { create: true });
  try {
    const out = await handle.createWritable();
    try {
      await out.write(blob);
      await out.close();
    } catch (e) {
      await out.abort?.().catch(() => {});
      throw e;
    }
  } catch (e) {
    await remove(dir, name);
    throw e;
  }
  return handle;
}

/** Temporary files this page created and has not yet removed or renamed. */
const temporaries = new Set();

/**
 * Removes `name` from `dir`. Chromium can refuse while an aborted writable still holds its
 * lock, so a refusal is retried briefly; whatever still fails is left to
 * `removeLeftovers()`, which the page calls when a run ends.
 */
async function remove(dir, name) {
  for (let attempt = 0; attempt < 5; attempt++) {
    try {
      await dir.removeEntry(name);
      temporaries.forEach((t) => t.dir === dir && t.name === name && temporaries.delete(t));
      return true;
    } catch (e) {
      if (e?.name === "NotFoundError") return true;
      await new Promise((r) => setTimeout(r, 50 * (attempt + 1)));
    }
  }
  return false;
}

/** Removes every temporary file a failed or stopped save left behind (best effort). */
export async function removeLeftovers() {
  for (const t of [...temporaries]) {
    if (await remove(t.dir, t.name)) temporaries.delete(t);
  }
}

async function exists(dir, name) {
  try {
    await dir.getFileHandle(name);
    return true;
  } catch (e) {
    if (e?.name === "NotFoundError") return false;
    if (e?.name === "TypeMismatchError") return true; // a folder of that name
    throw e;
  }
}
