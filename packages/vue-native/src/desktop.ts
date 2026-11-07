// File System Access, `FileReader` and the file pickers — docs/DESKTOP.md
// ("Pickers and files"); research/reports/m25/desktop-apis-design.md §4.
//
// TIER 1 only, and spec-exact where script can tell: the WICG File System
// Access classes (`showOpenFilePicker`, `showSaveFilePicker`,
// `showDirectoryPicker`, `FileSystemHandle` and its two kinds,
// `FileSystemWritableFileStream`), the File API's `FileReader` and
// `ProgressEvent`, and the request half of `<input type=file>` and
// `<a download>`. runtime.ts owns the elements; this file owns what they ask
// the host for.
//
// ## Handles hold an id, never a path
//
// A handle is a number into vn-desktop's handle table (crates/vn-desktop/src/
// fs/mod.rs) plus the two fields a browser shows: `name` and `kind`. The path
// stays in Rust. A child is reached only through its parent's id and the name
// fence the host applies, so nothing here can name a file the user did not
// pick, and nothing here can leak one: `JSON.stringify`, the console and a
// devtools eval all see `{}`-shaped objects with no path anywhere. A
// FinalizationRegistry releases the id when the handle is collected.
//
// ## Handles are [Serializable]; permission is the path's
//
// As in Chromium, `structuredClone`, `postMessage` and IndexedDB keep a
// handle: redjs writes the bytes the host gives it (tag 0x39), and the clone
// is a new handle on the same path. Permission follows the path, not the
// object. A handle on something the user picked, dropped or launched with
// in this session, or on something inside it, is "granted". A handle read
// back in a later session is "prompt" until `requestPermission()` runs on a
// user activation. Chromium shows a prompt there; this host grants with no
// prompt, and says so once (D-F22).
//
// ## Why this is a separate module with injected dependencies
//
// It is re-exported by runtime.ts (like vnwindow.ts) and installed by it. It
// takes the host functions, the settle channel and the transient-activation
// state as ARGUMENTS rather than reading `__vn_*` globals, which is what lets
// the bun harness run it against a fake host and bun's own `WritableStream`,
// `DOMException` and `TextDecoder`.
//
// ## FileReader lives here, not in redjs
//
// It is pure event and promise plumbing over `Blob.arrayBuffer()` (which is
// redjs's, and does the path-backed read off the UI thread). RUNTIME.md §5's
// "missing globals" rule puts that kind of thing in the runtime, as it did
// `ResizeObserver` and `MutationObserver`; and a bug in it is then fixed
// without a redjs push and a submodule bump.

/** A rejection as the host serialises it (`Fail::to_json` / `to_sync_json`). */
export interface DesktopFailure {
  name: string;
  message?: string;
  error?: string;
  code?: string;
  osError?: number;
}

/** What a settled host call hands over: its JSON payload and its value. */
export interface DesktopSettle {
  payload: Record<string, unknown>;
  /** A JS value the host made on delivery (a `File`, a `FileList`), or undefined. */
  value: unknown;
}

/** The `{h, name, kind}` a handle crosses the host boundary as. */
export interface HandleWire {
  h: number;
  name: string;
  kind: "file" | "directory";
}

type Json = string;

/** vn-desktop's File System Access and dialog host fns (protocol.d.ts). */
export interface DesktopFsHost {
  setHandleFactory(fn: (h: number, name: string, kind: string) => unknown): void;
  release(h: number): void;
  getFile(h: number): Json;
  child(dir: number, name: string, isDirectory: boolean, create: boolean): Json;
  remove(dir: number, name: string, recursive: boolean): Json;
  same(a: number, b: number): Json;
  resolve(root: number, other: number): Json;
  dirOpen(dir: number): Json;
  dirNext(cursor: number): Json;
  dirClose(cursor: number): void;
  writableOpen(h: number, keepExistingData: boolean): Json;
  writableWrite(w: number, position: number, data: unknown): Json;
  writableSeek(w: number, position: number): Json;
  writableTruncate(w: number, size: number): Json;
  writableClose(w: number): Json;
  writableAbort(w: number): Json;
  refuseClone(proto: object, interfaceName: string): void;
  setSerializer(proto: object, idOf: (handle: object) => number | undefined): void;
  permission(h: number, request: boolean): "granted" | "prompt";
  dialogOpen(requestJson: Json, currentFiles: unknown): Json;
  download(url: string, downloadAttribute: string): Json;
}

/** The engine classes this file builds on, injected (see the header). */
export interface DesktopEngine {
  Blob: abstract new (...args: never[]) => BlobLike;
  DOMException: new (message?: string, name?: string) => Error;
  TextDecoder: new (label?: string, options?: { ignoreBOM?: boolean }) => {
    readonly encoding: string;
    decode(input?: ArrayBuffer | ArrayBufferView): string;
  };
  WritableStream: new (sink?: WritableSink) => WritableStreamLike;
  btoa(binary: string): string;
}

interface BlobLike {
  readonly size: number;
  readonly type: string;
  arrayBuffer(): Promise<ArrayBuffer>;
}

interface WritableSink {
  write?(chunk: unknown): unknown;
  close?(): unknown;
  abort?(reason: unknown): unknown;
}

interface WritableStreamLike {
  readonly locked: boolean;
  getWriter(): { write(chunk: unknown): Promise<void>; releaseLock(): void };
  close(): Promise<void>;
  abort(reason?: unknown): Promise<void>;
}

export interface DesktopDeps {
  /** The host fns, or null in a host without vn-desktop. */
  host: DesktopFsHost | null;
  engine: DesktopEngine;
  /** One host answer → its settle; rejects with the raw failure. */
  call(json: Json): Promise<DesktopSettle>;
  warnCompat(key: string, message: string): void;
  /** HTML transient activation, as the runtime tracks it. */
  activation: { isActive(): boolean; consume(): void };
  /**
   * The runtime's same-realm reload bookkeeping (runtime.ts `REALM`). Is a
   * global missing or a previous runtime's (`vacant`), and record one as
   * this runtime's (`mark`). A class kept from before a reload would run
   * against the old runtime's state.
   */
  realm: { vacant(obj: object, name: PropertyKey): boolean; mark<T>(v: T): T };
}

/** What `<input type=file>` asks for (runtime.ts). */
export interface InputPickRequest {
  accept: string;
  multiple: boolean;
  directory: boolean;
  /** The input's current FileList: picking the same files again is a `cancel`. */
  current: unknown;
}

/** What `installDesktop` hands back to runtime.ts. */
export interface DesktopApi {
  /** A handle for a launch path or a drop the host already granted. */
  handleFrom(wire: HandleWire): FileSystemHandleLike;
  /**
   * `<input type=file>`'s picker. Resolves the new FileList, or null for a
   * dismissal (or the same selection again); `undefined` when no dialog
   * could be shown (another one is open on this window — the request is
   * ignored, as HTML allows).
   */
  pickForInput(req: InputPickRequest): Promise<unknown | null | undefined>;
  /** The color picker: a selected #rrggbb, null on dismissal, undefined if busy. */
  pickColorForInput(value: string): Promise<string | null | undefined>;
  /** `<a download>` for a `blob:` / `data:` URL: Save As, then the bytes. */
  download(url: string, downloadAttribute: string): void;
}

/** The common surface of the two handle kinds, for runtime.ts's types. */
export interface FileSystemHandleLike {
  readonly kind: "file" | "directory";
  readonly name: string;
}

/** Chromium's prefix on every exception a method throws. */
function failed(method: string, iface: string, message: string): string {
  return `Failed to execute '${method}' on '${iface}': ${message}`;
}

/** FSA's `id`: at most 32 of `[A-Za-z0-9_-]`. */
const ID_RE = /^[A-Za-z0-9_-]*$/;
/** An extension's code points after the dot (spec: "valid suffix code point"). */
const EXT_RE = /^\.[A-Za-z0-9+.]+$/;
/** An RFC 9110 token, which is what both halves of a MIME essence are. */
const TOKEN_RE = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;
const WELL_KNOWN = ["desktop", "documents", "downloads", "music", "pictures", "videos"] as const;

export function installDesktop(target: Record<string, unknown>, deps: DesktopDeps): DesktopApi {
  const { host, engine, warnCompat, activation } = deps;
  const { DOMException, TextDecoder } = engine;

  const domError = (name: string, message: string): Error =>
    name === "TypeError" ? new TypeError(message) : new DOMException(message, name);
  const fromFailure = (f: DesktopFailure): Error => domError(f.name || "InvalidStateError", f.message ?? f.error ?? f.name);
  /** A host answer → its settle, with the failure as the DOMException the spec names. */
  const settle = (json: Json): Promise<DesktopSettle> =>
    deps.call(json).catch((f: unknown) => {
      throw f instanceof Error ? f : fromFailure(f as DesktopFailure);
    });

  // ---- FileReader and ProgressEvent (File API) — host or no host -----------

  /** `ProgressEvent`, for an engine without one. redjs ships its own (with
   * `XMLHttpRequest`, since m26), and `PE` below prefers it. */
  class ProgressEvent extends Event {
    declare readonly lengthComputable: boolean;
    declare readonly loaded: number;
    declare readonly total: number;
    constructor(type: string, init: VnEventInit & { lengthComputable?: boolean; loaded?: number; total?: number } = {}) {
      super(type, init);
      Object.defineProperties(this, {
        lengthComputable: { value: !!init.lengthComputable, enumerable: true },
        loaded: { value: Number(init.loaded ?? 0), enumerable: true },
        total: { value: Number(init.total ?? 0), enumerable: true },
      });
    }
  }
  const PE = (
    typeof target.ProgressEvent === "function" && !deps.realm.vacant(target, "ProgressEvent") ? target.ProgressEvent : ProgressEvent
  ) as typeof ProgressEvent;

  // Does this engine's EventTarget call `on<type>` itself? redjs does (as an
  // own listener after the others, HTML's event handler attribute); bun's
  // does not. Probed once, so FileReader never calls a handler twice or never.
  const nativeOnHandlers = (() => {
    const t = new EventTarget() as VnEventTarget & { onvnprobe?: () => void };
    let hit = false;
    t.onvnprobe = () => (hit = true);
    t.dispatchEvent(new Event("vnprobe"));
    return hit;
  })();

  type ReadKind = "ArrayBuffer" | "BinaryString" | "Text" | "DataURL";

  /** WHATWG Encoding "get an encoding": a label TextDecoder knows, or null. */
  const encodingFor = (label: string | undefined): string | null => {
    if (label === undefined) return null;
    try {
      return new TextDecoder(String(label)).encoding;
    } catch {
      return null;
    }
  };

  /** File API "package data". */
  const packageData = (bytes: Uint8Array, kind: ReadKind, mime: string, encoding: string | undefined): unknown => {
    switch (kind) {
      case "ArrayBuffer":
        return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
      case "BinaryString": {
        let s = "";
        for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
        return s;
      }
      case "DataURL": {
        let s = "";
        for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
        // Chromium writes application/octet-stream for an untyped Blob.
        return `data:${mime || "application/octet-stream"};base64,${engine.btoa(s)}`;
      }
      case "Text": {
        // The argument, else the Blob type's charset, else UTF-8 — then the
        // Encoding standard's `decode`, whose BOM sniff overrides all three.
        let enc = encodingFor(encoding);
        if (!enc) {
          const m = /;\s*charset\s*=\s*"?([^";\s]+)/i.exec(mime);
          enc = encodingFor(m?.[1]) ?? "utf-8";
        }
        let start = 0;
        if (bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) [enc, start] = ["utf-8", 3];
        else if (bytes[0] === 0xfe && bytes[1] === 0xff) [enc, start] = ["utf-16be", 2];
        else if (bytes[0] === 0xff && bytes[1] === 0xfe) [enc, start] = ["utf-16le", 2];
        return new TextDecoder(enc, { ignoreBOM: true }).decode(bytes.subarray(start));
      }
    }
  };

  class FileReader extends EventTarget {
    static readonly EMPTY = 0;
    static readonly LOADING = 1;
    static readonly DONE = 2;
    declare readonly EMPTY: 0;
    declare readonly LOADING: 1;
    declare readonly DONE: 2;
    #state: 0 | 1 | 2 = 0;
    #result: unknown = null;
    #error: Error | null = null;
    /** Bumped by `abort()` and by every new read, so a stale read's tasks stop. */
    #generation = 0;
    #handlers: Record<string, ((ev: VnEvent) => unknown) | null> = {};

    get readyState(): 0 | 1 | 2 {
      return this.#state;
    }
    get result(): unknown {
      return this.#result;
    }
    get error(): Error | null {
      return this.#error;
    }

    readAsArrayBuffer(blob: BlobLike): void {
      this.#read(blob, "ArrayBuffer", "readAsArrayBuffer");
    }
    readAsBinaryString(blob: BlobLike): void {
      this.#read(blob, "BinaryString", "readAsBinaryString");
    }
    readAsText(blob: BlobLike, encoding?: string): void {
      this.#read(blob, "Text", "readAsText", encoding);
    }
    readAsDataURL(blob: BlobLike): void {
      this.#read(blob, "DataURL", "readAsDataURL");
    }

    abort(): void {
      if (this.#state !== 1) {
        // "empty" or "done": the result goes, nothing fires.
        this.#result = null;
        return;
      }
      this.#generation++;
      this.#state = 2;
      this.#result = null;
      this.#fire("abort");
      if (!this.#loading()) this.#fire("loadend");
    }

    /** File API "read operation". */
    #read(blob: BlobLike, kind: ReadKind, method: string, encoding?: string): void {
      if (!(blob instanceof engine.Blob)) {
        throw new TypeError(failed(method, "FileReader", "parameter 1 is not of type 'Blob'."));
      }
      if (this.#state === 1) {
        throw new DOMException(failed(method, "FileReader", "The object is already busy reading Blobs."), "InvalidStateError");
      }
      this.#state = 1;
      this.#result = null;
      this.#error = null;
      const gen = ++this.#generation;
      const live = () => gen === this.#generation;
      // The read starts now; its events are tasks, in the spec's order.
      const bytes = blob.arrayBuffer();
      bytes.catch(() => {});
      setTimeout(() => {
        if (!live()) return;
        this.#fire("loadstart");
        bytes.then(
          (buf) =>
            setTimeout(() => {
              if (!live()) return;
              const u8 = new Uint8Array(buf);
              this.#fire("progress", u8.length);
              if (!live()) return;
              let value: unknown;
              try {
                value = packageData(u8, kind, blob.type, encoding);
              } catch (e) {
                return this.#fail(e as Error);
              }
              this.#state = 2;
              this.#result = value;
              this.#fire("load", u8.length);
              if (!this.#loading()) this.#fire("loadend", u8.length);
            }, 0),
          (e: Error) =>
            setTimeout(() => {
              if (live()) this.#fail(e);
            }, 0),
        );
      }, 0);
    }

    /**
     * Read AFTER an event: a handler may have started a new read, in which
     * case the old one's `loadend` is not fired (File API).
     */
    #loading(): boolean {
      return this.#state === 1;
    }

    #fail(e: Error): void {
      this.#state = 2;
      this.#result = null;
      this.#error = e;
      this.#fire("error");
      if (!this.#loading()) this.#fire("loadend");
    }

    #fire(type: string, loaded = 0): void {
      this.dispatchEvent(new PE(type, { lengthComputable: loaded > 0, loaded, total: loaded }));
    }

    // The event handler IDL attributes.
    get onloadstart() { return this.#on("loadstart"); }
    set onloadstart(fn: unknown) { this.#setOn("loadstart", fn); }
    get onprogress() { return this.#on("progress"); }
    set onprogress(fn: unknown) { this.#setOn("progress", fn); }
    get onload() { return this.#on("load"); }
    set onload(fn: unknown) { this.#setOn("load", fn); }
    get onabort() { return this.#on("abort"); }
    set onabort(fn: unknown) { this.#setOn("abort", fn); }
    get onerror() { return this.#on("error"); }
    set onerror(fn: unknown) { this.#setOn("error", fn); }
    get onloadend() { return this.#on("loadend"); }
    set onloadend(fn: unknown) { this.#setOn("loadend", fn); }

    #on(type: string): ((ev: VnEvent) => unknown) | null {
      return this.#handlers[type] ?? null;
    }
    #setOn(type: string, fn: unknown): void {
      const had = type in this.#handlers;
      this.#handlers[type] = typeof fn === "function" ? (fn as (ev: VnEvent) => unknown) : null;
      // An engine that does not call `on<type>` itself gets ONE listener per
      // type, added the first time, which is where HTML orders the handler.
      if (!nativeOnHandlers && !had) {
        this.addEventListener(type, (ev) => void this.#handlers[type]?.call(this, ev));
      }
    }
  }
  for (const [k, v] of [
    ["EMPTY", 0],
    ["LOADING", 1],
    ["DONE", 2],
  ] as const) {
    Object.defineProperty(FileReader.prototype, k, { value: v, enumerable: true });
  }

  if (deps.realm.vacant(target, "FileReader")) target.FileReader = deps.realm.mark(FileReader);
  if (deps.realm.vacant(target, "ProgressEvent")) target.ProgressEvent = deps.realm.mark(ProgressEvent);
  host?.refuseClone(FileReader.prototype, "FileReader");

  // ---- File System Access --------------------------------------------------

  if (!host) {
    // No vn-desktop: no pickers, so feature detection takes the
    // `<input type=file>` path, which says why it cannot open either.
    return {
      handleFrom: () => {
        throw new TypeError("no desktop host");
      },
      pickForInput: () => {
        warnCompat("input-file:nohost", "<input type=file>: this host has no vn-desktop, so there is no file picker");
        return Promise.resolve(undefined);
      },
      pickColorForInput: () => {
        warnCompat("input-color:nohost", "<input type=color>: this host has no vn-desktop, so there is no color picker");
        return Promise.resolve(undefined);
      },
      download: () => warnCompat("download:nohost", "<a download>: this host has no vn-desktop; nothing was saved"),
    };
  }
  const fs = host;

  /** The id behind each live handle, and the registry that releases it. */
  const ids = new WeakMap<object, number>();
  const released = new FinalizationRegistry<number>((h) => fs.release(h));
  const INTERNAL = Symbol("vn.fs.internal");

  const idOf = (h: object, method: string, iface: string): number => {
    const id = ids.get(h);
    if (id === undefined) throw new TypeError(failed(method, iface, "Illegal invocation"));
    return id;
  };

  class FileSystemHandle {
    readonly #name: string;
    readonly #kind: "file" | "directory";
    constructor(key?: symbol, id?: number, name?: string, kind?: "file" | "directory") {
      if (key !== INTERNAL) throw new TypeError("Illegal constructor");
      this.#name = name!;
      this.#kind = kind!;
      ids.set(this, id!);
      released.register(this, id!);
    }
    get kind(): "file" | "directory" {
      idOf(this, "kind", "FileSystemHandle");
      return this.#kind;
    }
    get name(): string {
      idOf(this, "name", "FileSystemHandle");
      return this.#name;
    }
    isSameEntry(other: unknown): Promise<boolean> {
      try {
        const a = idOf(this, "isSameEntry", "FileSystemHandle");
        const b = other instanceof FileSystemHandle ? ids.get(other) : undefined;
        if (b === undefined) {
          throw new TypeError(failed("isSameEntry", "FileSystemHandle", "parameter 1 is not of type 'FileSystemHandle'."));
        }
        if ((other as FileSystemHandle).#kind !== this.#kind) return Promise.resolve(false);
        return settle(fs.same(a, b)).then((s) => s.payload.same === true);
      } catch (e) {
        return Promise.reject(e);
      }
    }
    /**
     * The path's state: "granted" for what the user chose this session (both
     * modes alike, D-F2), "prompt" for a handle restored from storage.
     */
    queryPermission(descriptor?: { mode?: string }): Promise<"granted" | "prompt"> {
      return permission(this, "queryPermission", descriptor, false);
    }
    /** "prompt" becomes "granted" on a user activation, with no prompt (D-F22). */
    requestPermission(descriptor?: { mode?: string }): Promise<"granted" | "prompt"> {
      return permission(this, "requestPermission", descriptor, true);
    }
  }

  let advisedGrant = false;
  /** `requestPermission` on a "prompt" handle: Chromium asks; this host grants on the gesture. */
  const grantOnGesture = (id: number): void => {
    fs.permission(id, true);
    if (advisedGrant) return;
    advisedGrant = true;
    warnCompat(
      "fsa:grant",
      "FileSystemHandle (D-F22): a handle restored from storage was granted on a user activation with no " +
        "prompt. Chromium asks the user first (\"Allow this site to view/edit …?\"); this host has no such prompt yet.",
    );
  };

  const permission = (h: object, method: string, d: { mode?: string } | undefined, request: boolean): Promise<"granted" | "prompt"> => {
    try {
      const id = idOf(h, method, "FileSystemHandle");
      const mode = d?.mode ?? "read";
      if (mode !== "read" && mode !== "readwrite") {
        throw new TypeError(
          failed(method, "FileSystemHandle", `The provided value '${mode}' is not a valid enum value of type FileSystemPermissionMode.`),
        );
      }
      const state = fs.permission(id, false);
      if (!request || state === "granted") return Promise.resolve(state);
      // Chromium's browser-side words: no "Failed to execute" prefix.
      if (!activation.isActive()) throw new DOMException("User activation is required to request permissions.", "SecurityError");
      grantOnGesture(id);
      return Promise.resolve(fs.permission(id, false));
    } catch (e) {
      return Promise.reject(e);
    }
  };

  /**
   * What Chromium's `RunWithWritePermission` does in front of createWritable,
   * a `create` lookup and removeEntry: a "prompt" handle is asked, which needs
   * a user activation; without one the operation is NotAllowedError (the host
   * refuses it, so nothing more is needed here).
   */
  const writeAccess = (id: number): void => {
    if (fs.permission(id, false) === "prompt" && activation.isActive()) grantOnGesture(id);
  };

  class FileSystemFileHandle extends FileSystemHandle {
    getFile(): Promise<unknown> {
      try {
        return settle(fs.getFile(idOf(this, "getFile", "FileSystemFileHandle"))).then((s) => s.value);
      } catch (e) {
        return Promise.reject(e);
      }
    }
    createWritable(options?: { keepExistingData?: boolean }): Promise<FileSystemWritableFileStream> {
      try {
        const id = idOf(this, "createWritable", "FileSystemFileHandle");
        writeAccess(id);
        return settle(fs.writableOpen(id, !!options?.keepExistingData)).then(
          (s) => new FileSystemWritableFileStream(INTERNAL, Number(s.payload.w)),
        );
      } catch (e) {
        return Promise.reject(e);
      }
    }
  }

  const makeHandle = (w: HandleWire): FileSystemHandle =>
    w.kind === "directory"
      ? new FileSystemDirectoryHandle(INTERNAL, w.h, w.name, "directory")
      : new FileSystemFileHandle(INTERNAL, w.h, w.name, "file");

  class FileSystemDirectoryHandle extends FileSystemHandle {
    getFileHandle(name: string, options?: { create?: boolean }): Promise<FileSystemFileHandle> {
      return this.#child("getFileHandle", name, false, options) as Promise<FileSystemFileHandle>;
    }
    getDirectoryHandle(name: string, options?: { create?: boolean }): Promise<FileSystemDirectoryHandle> {
      return this.#child("getDirectoryHandle", name, true, options) as Promise<FileSystemDirectoryHandle>;
    }
    #child(method: string, name: string, dir: boolean, options?: { create?: boolean }): Promise<FileSystemHandle> {
      try {
        const id = idOf(this, method, "FileSystemDirectoryHandle");
        if (options?.create) writeAccess(id);
        return settle(fs.child(id, String(name), dir, !!options?.create)).then((s) => makeHandle(s.payload as unknown as HandleWire));
      } catch (e) {
        return Promise.reject(e);
      }
    }
    removeEntry(name: string, options?: { recursive?: boolean }): Promise<void> {
      try {
        const id = idOf(this, "removeEntry", "FileSystemDirectoryHandle");
        writeAccess(id);
        return settle(fs.remove(id, String(name), !!options?.recursive)).then(() => undefined);
      } catch (e) {
        return Promise.reject(e);
      }
    }
    resolve(possibleDescendant: unknown): Promise<string[] | null> {
      try {
        const id = idOf(this, "resolve", "FileSystemDirectoryHandle");
        const other = possibleDescendant instanceof FileSystemHandle ? ids.get(possibleDescendant) : undefined;
        if (other === undefined) {
          throw new TypeError(failed("resolve", "FileSystemDirectoryHandle", "parameter 1 is not of type 'FileSystemHandle'."));
        }
        return settle(fs.resolve(id, other)).then((s) => (Array.isArray(s.payload.path) ? (s.payload.path as string[]) : null));
      } catch (e) {
        return Promise.reject(e);
      }
    }
    /** `async iterable<USVString, FileSystemHandle>`, over the host's cursor. */
    async *entries(): AsyncGenerator<[string, FileSystemHandle], void, undefined> {
      const id = idOf(this, "entries", "FileSystemDirectoryHandle");
      const cursor = Number((await settle(fs.dirOpen(id))).payload.cursor);
      try {
        for (;;) {
          const { payload } = await settle(fs.dirNext(cursor));
          for (const w of (payload.entries as HandleWire[]) ?? []) yield [w.name, makeHandle(w)];
          if (payload.done) return;
        }
      } finally {
        // A `break` out of `for await` lands here too.
        fs.dirClose(cursor);
      }
    }
    async *keys(): AsyncGenerator<string, void, undefined> {
      for await (const [k] of this.entries()) yield k;
    }
    async *values(): AsyncGenerator<FileSystemHandle, void, undefined> {
      for await (const [, v] of this.entries()) yield v;
    }
    [Symbol.asyncIterator](): AsyncGenerator<[string, FileSystemHandle], void, undefined> {
      return this.entries();
    }
  }

  /** WriteParams / a chunk → what `__vn_fs_writable_*` takes. */
  const asData = (data: unknown, method: string): unknown => {
    if (typeof data === "string" || data instanceof engine.Blob) return data;
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    throw new TypeError(
      failed(method, "FileSystemWritableFileStream", "The provided value is not of type '(ArrayBuffer or ArrayBufferView or Blob or USVString or WriteParams)'."),
    );
  };

  const closedStreams = new FinalizationRegistry<number>((w) => void fs.writableAbort(w));

  class FileSystemWritableFileStream extends engine.WritableStream {
    constructor(key?: symbol, w?: number) {
      if (key !== INTERNAL) throw new TypeError("Illegal constructor");
      const id = w!;
      let token: object | null = null;
      const done = () => {
        if (token) closedStreams.unregister(token);
      };
      super({
        write(chunk: unknown) {
          const c = chunk as { type?: unknown; data?: unknown; position?: unknown; size?: unknown } | null;
          if (c !== null && typeof c === "object" && !(c instanceof engine.Blob) && !ArrayBuffer.isView(c) && !(c instanceof ArrayBuffer)) {
            const pos = (v: unknown) => (v === undefined || v === null ? -1 : Number(v));
            switch (c.type) {
              case "write":
                if (c.data === undefined) throw new DOMException(failed("write", "FileSystemWritableFileStream", "write requires a data argument"), "SyntaxError");
                return settle(fs.writableWrite(id, pos(c.position), asData(c.data, "write"))).then(() => undefined);
              case "seek":
                if (typeof c.position !== "number") throw new DOMException(failed("write", "FileSystemWritableFileStream", "seek requires a position argument"), "SyntaxError");
                return settle(fs.writableSeek(id, c.position)).then(() => undefined);
              case "truncate":
                if (typeof c.size !== "number") throw new DOMException(failed("write", "FileSystemWritableFileStream", "truncate requires a size argument"), "SyntaxError");
                return settle(fs.writableTruncate(id, c.size)).then(() => undefined);
              default:
                throw new TypeError(failed("write", "FileSystemWritableFileStream", `The provided value '${String(c.type)}' is not a valid enum value of type WriteCommandType.`));
            }
          }
          return settle(fs.writableWrite(id, -1, asData(chunk, "write"))).then(() => undefined);
        },
        close() {
          done();
          return settle(fs.writableClose(id)).then(() => undefined);
        },
        abort() {
          done();
          return settle(fs.writableAbort(id)).then(() => undefined);
        },
      });
      // A stream collected without close() or abort() deletes its swap file
      // (design §4.3, "GC-less drop").
      token = {};
      closedStreams.register(this, id, token);
    }
    /** Spec: get a writer, write, release it. */
    write(data: unknown): Promise<void> {
      try {
        const writer = this.getWriter();
        const p = writer.write(data);
        writer.releaseLock();
        return p;
      } catch (e) {
        return Promise.reject(e);
      }
    }
    seek(position: number): Promise<void> {
      return this.write({ type: "seek", position });
    }
    truncate(size: number): Promise<void> {
      return this.write({ type: "truncate", size });
    }
  }

  for (const [cls, name] of [
    [FileSystemFileHandle, "FileSystemFileHandle"],
    [FileSystemDirectoryHandle, "FileSystemDirectoryHandle"],
    [FileSystemHandle, "FileSystemHandle"],
    [FileSystemWritableFileStream, "FileSystemWritableFileStream"],
  ] as const) {
    Object.defineProperty(cls.prototype, Symbol.toStringTag, { value: name, configurable: true });
  }
  // Both kinds are [Serializable] through their base (the clone's kind is in
  // the host's bytes); a writable stream is not, as in Chromium.
  fs.setSerializer(FileSystemHandle.prototype, (h) => ids.get(h));
  fs.refuseClone(FileSystemWritableFileStream.prototype, "FileSystemWritableFileStream");

  fs.setHandleFactory((h, name, kind) => makeHandle({ h, name, kind: kind === "directory" ? "directory" : "file" }));

  // ---- the pickers ----------------------------------------------------------

  interface AcceptOption {
    description?: string;
    accept?: Record<string, string | string[]>;
  }
  interface PickerOptions {
    id?: string;
    startIn?: unknown;
    types?: AcceptOption[];
    excludeAcceptAllOption?: boolean;
    multiple?: boolean;
    suggestedName?: string;
    mode?: string;
  }

  /** The spec's option checks, each a TypeError with Chromium's wording. */
  const validate = (method: string, o: PickerOptions): Record<string, unknown> => {
    const fail = (m: string) => new TypeError(failed(method, "Window", m));
    const req: Record<string, unknown> = {};
    if (o.id !== undefined) {
      const id = String(o.id);
      if (id.length > 32) throw fail("ID too long");
      if (!ID_RE.test(id)) throw fail("ID contains invalid characters");
      req.id = id;
    }
    if (o.startIn !== undefined) {
      if (o.startIn instanceof FileSystemHandle) req.startIn = { h: ids.get(o.startIn) };
      else if ((WELL_KNOWN as readonly string[]).includes(String(o.startIn))) req.startIn = String(o.startIn);
      else throw fail(`The provided value '${String(o.startIn)}' is not a valid enum value of type WellKnownDirectory.`);
    }
    if (o.types !== undefined) {
      const types: { description: string; accept: Record<string, string[]> }[] = [];
      for (const t of o.types) {
        const accept: Record<string, string[]> = {};
        for (const [mime, raw] of Object.entries(t?.accept ?? {})) {
          const [type, sub, ...rest] = mime.trim().split("/");
          if (rest.length || !type || !sub || !TOKEN_RE.test(type) || !TOKEN_RE.test(sub)) throw fail(`Invalid type: ${mime}`);
          const exts = typeof raw === "string" ? [raw] : Array.from(raw ?? [], String);
          for (const e of exts) {
            if (!e.startsWith(".")) throw fail(`Extension '${e}' must start with '.'.`);
            if (!EXT_RE.test(e)) throw fail(`Extension '${e}' contains invalid characters.`);
            if (e.endsWith(".")) throw fail(`Extension '${e}' must not end with '.'.`);
            if ([...e].length > 16) throw fail(`Extension '${e}' cannot be longer than 16 characters.`);
          }
          accept[mime.trim().toLowerCase()] = exts;
        }
        types.push({ description: String(t?.description ?? ""), accept });
      }
      req.types = types;
      if (!types.length && o.excludeAcceptAllOption) throw fail("Need at least one accepted type");
    } else if (o.excludeAcceptAllOption) {
      throw fail("Need at least one accepted type");
    } else {
      req.types = [];
    }
    req.excludeAcceptAllOption = !!o.excludeAcceptAllOption;
    if (o.mode !== undefined && o.mode !== "read" && o.mode !== "readwrite") {
      throw fail(`The provided value '${String(o.mode)}' is not a valid enum value of type FileSystemPermissionMode.`);
    }
    return req;
  };

  /** One FSA picker: validate, require (and consume) activation, ask. */
  const picker = (method: string, kind: "open" | "save" | "folder", o: PickerOptions = {}): Promise<HandleWire[]> => {
    try {
      const req = validate(method, o ?? {});
      req.kind = kind;
      req.result = "handles";
      if (kind === "open") req.multiple = !!o?.multiple;
      if (kind === "save" && o?.suggestedName !== undefined) req.suggestedName = String(o.suggestedName);
      // HTML/FSA: a picker needs transient activation, and showing one
      // consumes it (Chromium's ConsumeTransientUserActivation).
      if (!activation.isActive()) {
        throw new DOMException(failed(method, "Window", "Must be handling a user gesture to show a file picker."), "SecurityError");
      }
      activation.consume();
      return deps.call(fs.dialogOpen(JSON.stringify(req), null)).then(
        (s) => {
          if (s.payload.cancelled) throw new DOMException("The user aborted a request.", "AbortError");
          return (s.payload.handles as HandleWire[]) ?? [];
        },
        (f: DesktopFailure) => {
          if (f.name === "NotAllowedError" && f.code === "EBUSY") {
            throw new DOMException(failed(method, "Window", "File picker already active."), "NotAllowedError");
          }
          throw fromFailure(f);
        },
      );
    } catch (e) {
      return Promise.reject(e);
    }
  };

  const pickers = {
    showOpenFilePicker(options?: PickerOptions): Promise<FileSystemFileHandle[]> {
      return picker("showOpenFilePicker", "open", options).then((ws) => ws.map((w) => makeHandle(w) as FileSystemFileHandle));
    },
    showSaveFilePicker(options?: PickerOptions): Promise<FileSystemFileHandle> {
      return picker("showSaveFilePicker", "save", options).then((ws) => makeHandle(ws[0]) as FileSystemFileHandle);
    },
    showDirectoryPicker(options?: PickerOptions): Promise<FileSystemDirectoryHandle> {
      return picker("showDirectoryPicker", "folder", options).then((ws) => makeHandle(ws[0]) as FileSystemDirectoryHandle);
    },
  };
  for (const [k, v] of Object.entries(pickers)) {
    if (target[k] === undefined) target[k] = v;
  }
  for (const [k, v] of Object.entries({
    FileSystemHandle,
    FileSystemFileHandle,
    FileSystemDirectoryHandle,
    FileSystemWritableFileStream,
  })) {
    if (target[k] === undefined) target[k] = v;
  }

  // ---- <input type=file> and <a download> -----------------------------------

  /**
   * HTML `accept`: trimmed, lowercased tokens; anything else advised (A-F8)
   * and dropped. A `type/*` other than the three HTML names is kept, as
   * Chromium keeps it (`*` is a token character) and maps it to every
   * extension of that type.
   */
  const acceptTokens = (accept: string): string[] => {
    const out: string[] = [];
    for (const raw of accept.split(",")) {
      const t = raw.trim().toLowerCase();
      if (!t) continue;
      const [type, sub, ...rest] = t.split("/");
      const ok =
        (t.startsWith(".") && t.length > 1) ||
        (!rest.length && !!type && !!sub && TOKEN_RE.test(type) && TOKEN_RE.test(sub));
      if (ok) out.push(t);
      else warnCompat(`accept:${t}`, `<input accept>: the token "${t}" is not an extension or MIME type and was ignored (A-F8)`);
    }
    return out;
  };

  return {
    handleFrom: (w) => makeHandle(w),
    pickForInput(req) {
      const json = JSON.stringify({
        kind: req.directory ? "folder" : "open",
        multiple: req.multiple,
        accept: acceptTokens(req.accept),
        result: "files",
      });
      return deps.call(fs.dialogOpen(json, req.current)).then(
        (s) => (s.payload.cancelled ? null : s.value),
        (f: DesktopFailure) => {
          // A dialog already open on this window: HTML lets the request wait
          // or be dropped; Chromium drops it.
          if (f.name === "NotAllowedError" && f.code === "EBUSY") return undefined;
          console.error(`[vue-native] <input type=file>: ${f.message ?? f.error ?? f.name}`);
          return undefined;
        },
      );
    },
    pickColorForInput(value) {
      return deps.call(fs.dialogOpen(JSON.stringify({ kind: "color", value }), null)).then(
        (s) => s.payload.cancelled ? null : String(s.payload.color),
        (f: DesktopFailure) => {
          if (f.name === "NotAllowedError" && f.code === "EBUSY") return undefined;
          console.error(`[vue-native] <input type=color>: ${f.message ?? f.error ?? f.name}`);
          return undefined;
        },
      );
    },
    download(url, attr) {
      warnCompat(
        "download:saveas",
        "<a download> (D-F6): the file is saved through a Save As dialog, starting in Downloads. Chromium " +
          "saves to Downloads without asking unless \"Ask where to save each file\" is on",
      );
      deps.call(fs.download(url, attr)).then(
        () => {},
        (f: DesktopFailure) =>
          warnCompat(`download:${f.name}`, `<a download>: nothing was saved — ${f.message ?? f.error ?? f.name}`),
      );
    },
  };
}
