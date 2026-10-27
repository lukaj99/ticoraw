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

/**
 * Writes `blob` as `path` (folders separated by `/`) inside the directory handle `root`,
 * creating folders as needed. Never overwrites: an existing name gets " (2)", " (3)", ...
 * Returns the path it wrote.
 */
export async function saveInto(root, path, blob) {
  const parts = path.split("/");
  const fileName = parts.pop();
  let dir = root;
  for (const p of parts) dir = await dir.getDirectoryHandle(p, { create: true });
  const dot = fileName.lastIndexOf(".");
  const stem = dot > 0 ? fileName.slice(0, dot) : fileName;
  const ext = dot > 0 ? fileName.slice(dot) : "";
  let name = fileName;
  for (let n = 2; await exists(dir, name); n++) name = `${stem} (${n})${ext}`;
  const handle = await dir.getFileHandle(name, { create: true });
  const out = await handle.createWritable();
  try {
    await out.write(blob);
    await out.close();
  } catch (e) {
    await out.abort?.().catch(() => {});
    throw e;
  }
  return [...parts, name].join("/");
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
