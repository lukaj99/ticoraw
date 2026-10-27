// Types of `ticoraw/worker` (worker.js): ticoraw in a pool of module Web Workers.

import type { DecodedRaw, DngOptions, Inspection } from "./web/ticoraw.js";

export type {
  CodestreamInfo,
  Crop,
  DecodedRaw,
  DngCfa,
  DngOptions,
  Inspection,
  TicoError,
  TicoErrorKind,
} from "./web/ticoraw.js";

/**
 * What the pool accepts as a file. It is copied once into a buffer the pool transfers to a
 * worker, so the caller's buffer is never detached (a `Blob` or `File` is read directly).
 */
export type Input = Blob | ArrayBuffer | ArrayBufferView;

export interface TicorawWorkerOptions {
  /** Number of workers, each converting one file at a time (default 1). */
  size?: number;
  /**
   * URL of worker.js. Defaults to the module's own URL, which is right when the package is
   * served as is. With a bundler, pass `new URL("ticoraw/worker", import.meta.url)` or
   * wherever the bundler emits the file.
   */
  url?: string | URL;
}

/**
 * Runs ticoraw in module Web Workers, off the main thread. Workers start on first use and
 * each loads its own wasm instance.
 *
 * Promises reject with the same errors the synchronous API throws: an `Error` named
 * `"TicoError"` with a `kind` (see `TicoError`), or a `TypeError` for bad arguments. A
 * worker that traps is replaced, and its job rejects with a `RuntimeError` description.
 */
export class TicorawWorker {
  constructor(options?: TicorawWorkerOptions);
  /** Container and codestream facts without decoding pixels. */
  inspect(input: Input): Promise<Inspection>;
  /** The CFA mosaic and colour metadata; `pixels` is transferred from the worker. */
  decode(input: Input): Promise<DecodedRaw>;
  /** A complete DNG; its buffer is transferred from the worker. */
  toDng(input: Input, options?: DngOptions): Promise<Uint8Array>;
  /** Stops every worker and rejects pending jobs. */
  terminate(): void;
}
