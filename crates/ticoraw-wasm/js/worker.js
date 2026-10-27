// ticoraw/worker: run the decoder off the main thread.
//
// This one module plays both parts:
// - On the main thread it exports `TicorawWorker`, a small pool of module Web Workers.
// - Loaded as a module worker (which `TicorawWorker` does with this file's own URL), it
//   instantiates ../web/ticoraw.js and serves requests.
//
// Input bytes are copied once into a buffer the pool owns and transferred to the worker, so
// the caller's buffers are never detached; results (DNG bytes, decoded pixels) are
// transferred back without a copy.
//
// The wasm module is single-threaded. Run one worker per core for parallel conversions:
// `new TicorawWorker({ size: navigator.hardwareConcurrency })`.

const IN_WORKER =
  typeof WorkerGlobalScope !== "undefined" && globalThis instanceof WorkerGlobalScope;

if (IN_WORKER) {
  // Start loading at once, but register the handler synchronously so that no message
  // sent while the module is loading is lost.
  const ready = import("./web/ticoraw.js").then(async (m) => {
    await m.default();
    return m;
  });
  globalThis.onmessage = async ({ data }) => {
    const { id, op, bytes, options } = data;
    try {
      const t = await ready;
      const input = new Uint8Array(bytes);
      let result;
      let transfer = [];
      switch (op) {
        case "inspect":
          result = t.inspect(input);
          break;
        case "decode":
          result = t.decode(input);
          transfer = [result.pixels.buffer];
          break;
        case "toDng":
          result = t.toDng(input, options);
          transfer = [result.buffer];
          break;
        default:
          throw new TypeError(`ticoraw worker: unknown operation ${op}`);
      }
      globalThis.postMessage({ id, ok: true, result }, transfer);
    } catch (e) {
      // A RuntimeError means the instance trapped (a bug, never an input error): this
      // instance is unusable, so the pool replaces the worker.
      const trapped = e instanceof WebAssembly.RuntimeError;
      globalThis.postMessage({ id, ok: false, trapped, error: plainError(e) });
      if (trapped) globalThis.close();
    }
  };
  // A request that could not be deserialized has no readable id; the pool sends one job at
  // a time, so `id: null` answers the current one.
  globalThis.onmessageerror = () => {
    const error = { name: "Error", message: "ticoraw worker: the request could not be received" };
    globalThis.postMessage({ id: null, ok: false, trapped: false, error });
  };
}

function plainError(e) {
  if (!(e instanceof Error)) return { name: "Error", message: String(e) };
  const p = { name: e.name, message: e.message };
  for (const k of ["kind", "what", "offset"]) if (k in e) p[k] = e[k];
  return p;
}

function reviveError(p) {
  const e = p.name === "TypeError" ? new TypeError(p.message) : new Error(p.message);
  Object.assign(e, p);
  return e;
}

// Options reach the worker by structured clone. A shallow copy of the own enumerable
// properties, which are all the synchronous toDng reads, lets a Proxy (a Vue reactive object,
// say) through as it does there; anything that is not an object goes as is, for toDng to
// reject.
function plainOptions(options) {
  if (options === null || typeof options !== "object" || Array.isArray(options)) return options;
  return { ...options };
}

// postMessage throws a DataCloneError for options it cannot clone, such as a function: a
// bad argument, so reject with the TypeError the synchronous API throws for one.
function cloneError(e) {
  if (e?.name !== "DataCloneError") return e;
  return new TypeError(`ticoraw: toDng options cannot be sent to a worker: ${e.message}`, {
    cause: e,
  });
}

async function ownedBuffer(input) {
  if (typeof Blob !== "undefined" && input instanceof Blob) return input.arrayBuffer();
  if (input instanceof ArrayBuffer) return input.slice(0);
  if (ArrayBuffer.isView(input)) {
    return new Uint8Array(input.buffer, input.byteOffset, input.byteLength).slice().buffer;
  }
  throw new TypeError("ticoraw: input must be a Blob, an ArrayBuffer or a typed array");
}

/** A pool of module Web Workers running ticoraw. See index.d.ts. */
export class TicorawWorker {
  #size;
  #url;
  #slots = new Set();
  #idle = [];
  #queue = [];
  #closed = false;

  constructor({ size = 1, url = new URL(import.meta.url) } = {}) {
    if (!Number.isInteger(size) || size < 1) {
      throw new TypeError("TicorawWorker: size must be a positive integer");
    }
    this.#size = size;
    this.#url = url;
  }

  inspect(input) {
    return this.#run("inspect", input);
  }

  decode(input) {
    return this.#run("decode", input);
  }

  toDng(input, options) {
    return this.#run("toDng", input, options);
  }

  terminate() {
    this.#closed = true;
    const gone = new Error("TicorawWorker was terminated");
    for (const slot of this.#slots) {
      slot.worker.terminate();
      slot.job?.reject(gone);
    }
    for (const job of this.#queue.splice(0)) job.reject(gone);
    this.#slots.clear();
    this.#idle = [];
  }

  #run(op, input, options) {
    if (this.#closed) return Promise.reject(new Error("TicorawWorker was terminated"));
    return new Promise((resolve, reject) => {
      this.#queue.push({ op, input, options, resolve, reject });
      this.#pump();
    });
  }

  #pump() {
    while (this.#queue.length > 0) {
      const slot = this.#idle.pop() ?? (this.#slots.size < this.#size ? this.#spawn() : null);
      if (slot === null) return;
      const job = this.#queue.shift();
      slot.job = job;
      slot.id += 1;
      const id = slot.id;
      ownedBuffer(job.input)
        .then((bytes) => {
          if (slot.job === job && slot.id === id) {
            const options = plainOptions(job.options);
            slot.worker.postMessage({ id, op: job.op, bytes, options }, [bytes]);
          }
        })
        // A rejected input, a throwing getter or options that cannot be cloned (a
        // DataCloneError from postMessage) fail this job and free the slot.
        .catch((e) => this.#settle(slot, id, () => job.reject(cloneError(e))));
    }
  }

  #spawn() {
    const worker = new Worker(this.#url, { type: "module" });
    const slot = { worker, job: null, id: 0, answered: false };
    worker.onmessage = ({ data }) => {
      slot.answered = true;
      const job = slot.job;
      if (data.trapped) this.#drop(slot);
      this.#settle(slot, data.id ?? slot.id, () =>
        data.ok ? job.resolve(data.result) : job.reject(reviveError(data.error)),
      );
    };
    // The worker's answer could not be deserialized: the job fails, the worker stays usable.
    worker.onmessageerror = () => {
      const job = slot.job;
      this.#settle(slot, slot.id, () =>
        job.reject(new Error("ticoraw worker: the result could not be received")),
      );
    };
    worker.onerror = (ev) => {
      ev.preventDefault?.();
      const err = new Error(`ticoraw worker failed: ${ev.message ?? "unknown error"}`);
      const job = slot.job;
      this.#drop(slot);
      slot.job = null;
      job?.reject(err);
      // A worker that never answered could not even load (wrong URL, no wasm): fail the
      // queue instead of respawning forever.
      if (!slot.answered) for (const j of this.#queue.splice(0)) j.reject(err);
      this.#pump();
    };
    this.#slots.add(slot);
    return slot;
  }

  #drop(slot) {
    slot.worker.terminate();
    this.#slots.delete(slot);
    this.#idle = this.#idle.filter((s) => s !== slot);
  }

  #settle(slot, id, finish) {
    if (slot.id !== id || slot.job === null) return;
    slot.job = null;
    finish();
    if (this.#slots.has(slot)) this.#idle.push(slot);
    this.#pump();
  }
}
