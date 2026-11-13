// File System Access (`showOpenFilePicker`, `showSaveFilePicker`, `showDirectoryPicker`, handles,
// `FileSystemWritableFileStream`) and the request half of `<input type=file>` / `<a download>`. runtime.ts owns the
// elements; this file asks the host (vn-desktop).
// Handles hold a numeric id into vn-desktop's handle table, never a path, so nothing here can name or leak a file the
// user did not pick; a FinalizationRegistry releases the id. Handles are structured-cloneable (a clone is a new handle
// on the same path). Permission follows the path: picked, dropped or launched paths are "granted"; a handle restored
// in a later session is "prompt" until requestPermission() on a user activation, which this host grants without
// Chromium's prompt (warned once).
// Dependencies are injected so the bun harness can run this against a fake host.

export interface DesktopFailure {
  name: string;
  message?: string;
  error?: string;
  code?: string;
  osError?: number;
}

export interface DesktopSettle {
  payload: Record<string, unknown>;
  value: unknown;
}

export interface HandleWire {
  h: number;
  name: string;
  kind: "file" | "directory";
}

type Json = string;

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

export interface DesktopEngine {
  Blob: abstract new (...args: never[]) => BlobLike;
  DOMException: new (message?: string, name?: string) => Error;
  WritableStream: new (sink?: WritableSink) => WritableStreamLike;
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
  host: DesktopFsHost | null;
  engine: DesktopEngine;
  call(json: Json): Promise<DesktopSettle>;
  warnCompat(key: string, message: string): void;
  activation: { isActive(): boolean; consume(): void };
  /** runtime.ts `REALM` bookkeeping: `vacant` finds a missing or stale global, `mark` records this runtime's. */
  realm: { vacant(obj: object, name: PropertyKey): boolean; mark<T>(v: T): T };
}

export interface InputPickRequest {
  accept: string;
  multiple: boolean;
  directory: boolean;
  current: unknown;
}

export interface DesktopApi {
  handleFrom(wire: HandleWire): FileSystemHandleLike;
  /** `<input type=file>` picker: the new FileList; null on dismissal or the same selection; undefined if no dialog could be shown (one already open). */
  pickForInput(req: InputPickRequest): Promise<unknown | null | undefined>;
  pickColorForInput(value: string): Promise<string | null | undefined>;
  download(url: string, downloadAttribute: string): void;
}

export interface FileSystemHandleLike {
  readonly kind: "file" | "directory";
  readonly name: string;
}

function failed(method: string, iface: string, message: string): string {
  return `Failed to execute '${method}' on '${iface}': ${message}`;
}

const ID_RE = /^[A-Za-z0-9_-]*$/;
const EXT_RE = /^\.[A-Za-z0-9+.]+$/;
const TOKEN_RE = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+$/;
const WELL_KNOWN = ["desktop", "documents", "downloads", "music", "pictures", "videos"] as const;

export function installDesktop(target: Record<string, unknown>, deps: DesktopDeps): DesktopApi {
  const { host, engine, warnCompat, activation } = deps;
  const { DOMException } = engine;

  const domError = (name: string, message: string): Error =>
    name === "TypeError" ? new TypeError(message) : new DOMException(message, name);
  const fromFailure = (f: DesktopFailure): Error =>
    domError(f.name || "InvalidStateError", f.message ?? f.error ?? f.name);
  const settle = (json: Json): Promise<DesktopSettle> =>
    deps.call(json).catch((f: unknown) => {
      throw f instanceof Error ? f : fromFailure(f as DesktopFailure);
    });

  if (!host) {
    return {
      handleFrom: () => {
        throw new TypeError("no desktop host");
      },
      pickForInput: () => {
        warnCompat("input-file:nohost", "<input type=file>: this host has no vn-desktop, so there is no file picker");
        return Promise.resolve(undefined);
      },
      pickColorForInput: () => {
        warnCompat(
          "input-color:nohost",
          "<input type=color>: this host has no vn-desktop, so there is no color picker",
        );
        return Promise.resolve(undefined);
      },
      download: () => warnCompat("download:nohost", "<a download>: this host has no vn-desktop; nothing was saved"),
    };
  }
  const fs = host;

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
          throw new TypeError(
            failed("isSameEntry", "FileSystemHandle", "parameter 1 is not of type 'FileSystemHandle'."),
          );
        }
        if ((other as FileSystemHandle).#kind !== this.#kind) return Promise.resolve(false);
        return settle(fs.same(a, b)).then((s) => s.payload.same === true);
      } catch (e) {
        return Promise.reject(e);
      }
    }
    queryPermission(descriptor?: { mode?: string }): Promise<"granted" | "prompt"> {
      return permission(this, "queryPermission", descriptor, false);
    }
    requestPermission(descriptor?: { mode?: string }): Promise<"granted" | "prompt"> {
      return permission(this, "requestPermission", descriptor, true);
    }
  }

  let advisedGrant = false;
  const grantOnGesture = (id: number): void => {
    fs.permission(id, true);
    if (advisedGrant) return;
    advisedGrant = true;
    warnCompat(
      "fsa:grant",
      "FileSystemHandle: a handle restored from storage was granted on a user activation with no " +
        'prompt. Chromium asks the user first ("Allow this site to view/edit …?"); this host has no such prompt yet.',
    );
  };

  const permission = (
    h: object,
    method: string,
    d: { mode?: string } | undefined,
    request: boolean,
  ): Promise<"granted" | "prompt"> => {
    try {
      const id = idOf(h, method, "FileSystemHandle");
      const mode = d?.mode ?? "read";
      if (mode !== "read" && mode !== "readwrite") {
        throw new TypeError(
          failed(
            method,
            "FileSystemHandle",
            `The provided value '${mode}' is not a valid enum value of type FileSystemPermissionMode.`,
          ),
        );
      }
      const state = fs.permission(id, false);
      if (!request || state === "granted") return Promise.resolve(state);
      // Chromium's browser-side words: no "Failed to execute" prefix.
      if (!activation.isActive())
        throw new DOMException("User activation is required to request permissions.", "SecurityError");
      grantOnGesture(id);
      return Promise.resolve(fs.permission(id, false));
    } catch (e) {
      return Promise.reject(e);
    }
  };

  // As Chromium: a "prompt" handle is granted on a user activation before a write; otherwise the host refuses (NotAllowedError).
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
        return settle(fs.child(id, String(name), dir, !!options?.create)).then((s) =>
          makeHandle(s.payload as unknown as HandleWire),
        );
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
          throw new TypeError(
            failed("resolve", "FileSystemDirectoryHandle", "parameter 1 is not of type 'FileSystemHandle'."),
          );
        }
        return settle(fs.resolve(id, other)).then((s) =>
          Array.isArray(s.payload.path) ? (s.payload.path as string[]) : null,
        );
      } catch (e) {
        return Promise.reject(e);
      }
    }
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

  const asData = (data: unknown, method: string): unknown => {
    if (typeof data === "string" || data instanceof engine.Blob) return data;
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (ArrayBuffer.isView(data)) return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
    throw new TypeError(
      failed(
        method,
        "FileSystemWritableFileStream",
        "The provided value is not of type '(ArrayBuffer or ArrayBufferView or Blob or USVString or WriteParams)'.",
      ),
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
          if (
            c !== null &&
            typeof c === "object" &&
            !(c instanceof engine.Blob) &&
            !ArrayBuffer.isView(c) &&
            !(c instanceof ArrayBuffer)
          ) {
            const pos = (v: unknown) => (v === undefined || v === null ? -1 : Number(v));
            switch (c.type) {
              case "write":
                if (c.data === undefined)
                  throw new DOMException(
                    failed("write", "FileSystemWritableFileStream", "write requires a data argument"),
                    "SyntaxError",
                  );
                return settle(fs.writableWrite(id, pos(c.position), asData(c.data, "write"))).then(() => undefined);
              case "seek":
                if (typeof c.position !== "number")
                  throw new DOMException(
                    failed("write", "FileSystemWritableFileStream", "seek requires a position argument"),
                    "SyntaxError",
                  );
                return settle(fs.writableSeek(id, c.position)).then(() => undefined);
              case "truncate":
                if (typeof c.size !== "number")
                  throw new DOMException(
                    failed("write", "FileSystemWritableFileStream", "truncate requires a size argument"),
                    "SyntaxError",
                  );
                return settle(fs.writableTruncate(id, c.size)).then(() => undefined);
              default:
                throw new TypeError(
                  failed(
                    "write",
                    "FileSystemWritableFileStream",
                    `The provided value '${String(c.type)}' is not a valid enum value of type WriteCommandType.`,
                  ),
                );
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
      // A stream collected without close() or abort() deletes its swap file.
      token = {};
      closedStreams.register(this, id, token);
    }
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
  // Both kinds clone through their base; a writable stream does not, as in Chromium.
  fs.setSerializer(FileSystemHandle.prototype, (h) => ids.get(h));
  fs.refuseClone(FileSystemWritableFileStream.prototype, "FileSystemWritableFileStream");

  fs.setHandleFactory((h, name, kind) => makeHandle({ h, name, kind: kind === "directory" ? "directory" : "file" }));

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
      else
        throw fail(`The provided value '${String(o.startIn)}' is not a valid enum value of type WellKnownDirectory.`);
    }
    if (o.types !== undefined) {
      const types: { description: string; accept: Record<string, string[]> }[] = [];
      for (const t of o.types) {
        const accept: Record<string, string[]> = {};
        for (const [mime, raw] of Object.entries(t?.accept ?? {})) {
          const [type, sub, ...rest] = mime.trim().split("/");
          if (rest.length || !type || !sub || !TOKEN_RE.test(type) || !TOKEN_RE.test(sub))
            throw fail(`Invalid type: ${mime}`);
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

  const picker = (method: string, kind: "open" | "save" | "folder", o: PickerOptions = {}): Promise<HandleWire[]> => {
    try {
      const req = validate(method, o ?? {});
      req.kind = kind;
      req.result = "handles";
      if (kind === "open") req.multiple = !!o?.multiple;
      if (kind === "save" && o?.suggestedName !== undefined) req.suggestedName = String(o.suggestedName);
      if (!activation.isActive()) {
        throw new DOMException(
          failed(method, "Window", "Must be handling a user gesture to show a file picker."),
          "SecurityError",
        );
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
      return picker("showOpenFilePicker", "open", options).then((ws) =>
        ws.map((w) => makeHandle(w) as FileSystemFileHandle),
      );
    },
    showSaveFilePicker(options?: PickerOptions): Promise<FileSystemFileHandle> {
      return picker("showSaveFilePicker", "save", options).then((ws) => makeHandle(ws[0]!) as FileSystemFileHandle);
    },
    showDirectoryPicker(options?: PickerOptions): Promise<FileSystemDirectoryHandle> {
      return picker("showDirectoryPicker", "folder", options).then(
        (ws) => makeHandle(ws[0]!) as FileSystemDirectoryHandle,
      );
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

  // `accept`: trimmed, lowercased tokens; others are advised and dropped. A `type/*` is kept, as Chromium does.
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
      else
        warnCompat(`accept:${t}`, `<input accept>: the token "${t}" is not an extension or MIME type and was ignored`);
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
          if (f.name === "NotAllowedError" && f.code === "EBUSY") return undefined;
          console.error(`[vue-native] <input type=file>: ${f.message ?? f.error ?? f.name}`);
          return undefined;
        },
      );
    },
    pickColorForInput(value) {
      return deps.call(fs.dialogOpen(JSON.stringify({ kind: "color", value }), null)).then(
        (s) => (s.payload.cancelled ? null : String(s.payload.color)),
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
        "<a download>: the file is saved through a Save As dialog, starting in Downloads. Chromium " +
          'saves to Downloads without asking unless "Ask where to save each file" is on',
      );
      deps.call(fs.download(url, attr)).then(
        () => {},
        (f: DesktopFailure) =>
          warnCompat(`download:${f.name}`, `<a download>: nothing was saved — ${f.message ?? f.error ?? f.name}`),
      );
    },
  };
}
