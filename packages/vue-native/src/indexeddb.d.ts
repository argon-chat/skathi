// IndexedDB — what `indexedDB` and `navigator.storage` are in a vue-native
// app. vendor/redjs/redjs/src/webapi/indexeddb/ (m25 P1 the API, P2 the disk).
//
// SHIPPED with the runtime, like engine.d.ts and protocol.d.ts: an app's
// tsconfig lists it (`vn create` writes it in; apps/tsconfig.json does). It
// lived in apps/env.d.ts while IndexedDB was memory-only; it moved here when
// the databases went on disk, because an app that keeps its data in
// IndexedDB is the ordinary case, not a demo's.
//
// IndexedDB 3.0, the whole API, ON in vn-app by default (`indexeddb` in
// crates/vn-app/Cargo.toml). Dexie 4, idb 8, idb-keyval 6 and localForage run
// on it unmodified (redjs tests/idb_libs.rs, on both backends). The databases
// are SQLite files under the app's storage folder (`<folder>\indexeddb\`),
// encrypted at rest when the storage is — `appStorage.setStorageFolder(path,
// { encrypted: true })`, the same sticky rule as `localStorage`
// (docs/STORAGE.md "IndexedDB"). A host built without storage runs them in
// memory and says so once (`[redjs] IndexedDB: … IN MEMORY ONLY`).
//
// Typed here at the depth an app calls it directly; a wrapper library's own
// typings are its business. Keys are `IDBValidKey` in the spec: numbers,
// strings, Dates, ArrayBuffers/views and arrays of those. Merges onto
// engine.d.ts's `VnEvent` / `VnEventTarget` / `VnNavigator`.

export {};

declare global {
  type VnIDBValidKey = number | string | Date | ArrayBuffer | ArrayBufferView | VnIDBValidKey[];
  type VnIDBMode = "readonly" | "readwrite" | "versionchange";
  type VnIDBCursorDirection = "next" | "nextunique" | "prev" | "prevunique";
  type VnIDBHandler = ((ev: VnEvent) => void) | null;

  /** vendor/redjs/redjs/src/webapi/domexception.rs (m25 P0) — what every
   *  IndexedDB `error` is: `ConstraintError`, `QuotaExceededError`,
   *  `VersionError`, `AbortError`, … */
  interface VnDOMException {
    readonly name: string;
    readonly message: string;
    readonly code: number;
  }
  var DOMException: { new (message?: string, name?: string): VnDOMException; prototype: VnDOMException };

  interface VnDOMStringList {
    readonly length: number;
    item(index: number): string | null;
    contains(name: string): boolean;
    [index: number]: string;
    [Symbol.iterator](): IterableIterator<string>;
  }

  interface VnIDBRequest<T = unknown> extends VnEventTarget {
    /** Throws `InvalidStateError` until the request is done, as in a browser. */
    readonly result: T;
    readonly error: VnDOMException | null;
    readonly readyState: "pending" | "done";
    readonly source: VnIDBObjectStore | VnIDBIndex | VnIDBCursor | null;
    readonly transaction: VnIDBTransaction | null;
    onsuccess: VnIDBHandler;
    onerror: VnIDBHandler;
  }
  interface VnIDBOpenDBRequest extends VnIDBRequest<VnIDBDatabase> {
    onupgradeneeded: ((ev: VnIDBVersionChangeEvent) => void) | null;
    onblocked: ((ev: VnIDBVersionChangeEvent) => void) | null;
  }
  interface VnIDBVersionChangeEvent extends VnEvent {
    readonly oldVersion: number;
    readonly newVersion: number | null;
  }

  interface VnIDBFactory {
    open(name: string, version?: number): VnIDBOpenDBRequest;
    deleteDatabase(name: string): VnIDBOpenDBRequest;
    databases(): Promise<{ name: string; version: number }[]>;
    cmp(a: unknown, b: unknown): -1 | 0 | 1;
  }

  interface VnIDBDatabase extends VnEventTarget {
    readonly name: string;
    readonly version: number;
    readonly objectStoreNames: VnDOMStringList;
    transaction(
      stores: string | string[],
      mode?: "readonly" | "readwrite",
      options?: { durability?: "default" | "strict" | "relaxed" },
    ): VnIDBTransaction;
    /** Only inside `upgradeneeded`. */
    createObjectStore(name: string, options?: { keyPath?: string | string[] | null; autoIncrement?: boolean }): VnIDBObjectStore;
    deleteObjectStore(name: string): void;
    close(): void;
    onversionchange: ((ev: VnIDBVersionChangeEvent) => void) | null;
    onclose: VnIDBHandler;
    onabort: VnIDBHandler;
    onerror: VnIDBHandler;
  }

  interface VnIDBTransaction extends VnEventTarget {
    readonly db: VnIDBDatabase;
    readonly mode: VnIDBMode;
    readonly durability: "default" | "strict" | "relaxed";
    readonly objectStoreNames: VnDOMStringList;
    readonly error: VnDOMException | null;
    objectStore(name: string): VnIDBObjectStore;
    commit(): void;
    abort(): void;
    oncomplete: VnIDBHandler;
    onabort: VnIDBHandler;
    onerror: VnIDBHandler;
  }

  /** What an object store and an index both read with. */
  interface VnIDBReadable {
    readonly name: string;
    readonly keyPath: string | string[] | null;
    get(query: VnIDBValidKey | VnIDBKeyRange): VnIDBRequest<unknown>;
    getKey(query: VnIDBValidKey | VnIDBKeyRange): VnIDBRequest<VnIDBValidKey | undefined>;
    getAll(query?: VnIDBValidKey | VnIDBKeyRange | null, count?: number): VnIDBRequest<unknown[]>;
    getAllKeys(query?: VnIDBValidKey | VnIDBKeyRange | null, count?: number): VnIDBRequest<VnIDBValidKey[]>;
    count(query?: VnIDBValidKey | VnIDBKeyRange): VnIDBRequest<number>;
    openCursor(query?: VnIDBValidKey | VnIDBKeyRange | null, direction?: VnIDBCursorDirection): VnIDBRequest<VnIDBCursorWithValue | null>;
    openKeyCursor(query?: VnIDBValidKey | VnIDBKeyRange | null, direction?: VnIDBCursorDirection): VnIDBRequest<VnIDBCursor | null>;
  }
  interface VnIDBObjectStore extends VnIDBReadable {
    readonly autoIncrement: boolean;
    readonly indexNames: VnDOMStringList;
    readonly transaction: VnIDBTransaction;
    put(value: unknown, key?: VnIDBValidKey): VnIDBRequest<VnIDBValidKey>;
    add(value: unknown, key?: VnIDBValidKey): VnIDBRequest<VnIDBValidKey>;
    delete(query: VnIDBValidKey | VnIDBKeyRange): VnIDBRequest<undefined>;
    clear(): VnIDBRequest<undefined>;
    index(name: string): VnIDBIndex;
    /** Only inside `upgradeneeded`. */
    createIndex(name: string, keyPath: string | string[], options?: { unique?: boolean; multiEntry?: boolean }): VnIDBIndex;
    deleteIndex(name: string): void;
  }
  interface VnIDBIndex extends VnIDBReadable {
    readonly objectStore: VnIDBObjectStore;
    readonly unique: boolean;
    readonly multiEntry: boolean;
  }

  interface VnIDBCursor {
    readonly source: VnIDBObjectStore | VnIDBIndex;
    readonly direction: VnIDBCursorDirection;
    readonly key: VnIDBValidKey;
    readonly primaryKey: VnIDBValidKey;
    readonly request: VnIDBRequest<VnIDBCursor | null>;
    advance(count: number): void;
    continue(key?: VnIDBValidKey): void;
    continuePrimaryKey(key: VnIDBValidKey, primaryKey: VnIDBValidKey): void;
    update(value: unknown): VnIDBRequest<VnIDBValidKey>;
    delete(): VnIDBRequest<undefined>;
  }
  interface VnIDBCursorWithValue extends VnIDBCursor {
    readonly value: unknown;
  }

  interface VnIDBKeyRange {
    readonly lower: VnIDBValidKey | undefined;
    readonly upper: VnIDBValidKey | undefined;
    readonly lowerOpen: boolean;
    readonly upperOpen: boolean;
    includes(key: VnIDBValidKey): boolean;
  }

  var indexedDB: VnIDBFactory;
  var IDBKeyRange: {
    only(value: VnIDBValidKey): VnIDBKeyRange;
    lowerBound(lower: VnIDBValidKey, open?: boolean): VnIDBKeyRange;
    upperBound(upper: VnIDBValidKey, open?: boolean): VnIDBKeyRange;
    bound(lower: VnIDBValidKey, upper: VnIDBValidKey, lowerOpen?: boolean, upperOpen?: boolean): VnIDBKeyRange;
    prototype: VnIDBKeyRange;
  };

  /** The Storage API's `StorageManager` — the same module. `estimate()`
   *  reports the IndexedDB bytes against the quota (1 GiB by default);
   *  `persist()` and `persisted()` answer `true`, because this runtime never
   *  evicts an app's data (IDB-D7). */
  interface VnStorageManager {
    estimate(): Promise<{ usage: number; quota: number }>;
    persist(): Promise<boolean>;
    persisted(): Promise<boolean>;
  }
  interface VnNavigator {
    storage: VnStorageManager;
  }
}
