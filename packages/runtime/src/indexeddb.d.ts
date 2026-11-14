// IndexedDB 3.0 as `indexedDB` and `navigator.storage` in a Skathi app. Databases are SQLite
// files under the app's storage folder (`<folder>\indexeddb\`), encrypted at rest when the storage
// is; a host built without storage keeps them in memory. Typed at the depth an app calls it
// directly. Merges onto engine.d.ts's `SkEvent` / `SkEventTarget` / `SkNavigator`.

export {};

declare global {
  type SkIDBValidKey = number | string | Date | ArrayBuffer | ArrayBufferView | SkIDBValidKey[];
  type SkIDBMode = "readonly" | "readwrite" | "versionchange";
  type SkIDBCursorDirection = "next" | "nextunique" | "prev" | "prevunique";
  type SkIDBHandler = ((ev: SkEvent) => void) | null;

  interface SkDOMException {
    readonly name: string;
    readonly message: string;
    readonly code: number;
  }
  var DOMException: { new (message?: string, name?: string): SkDOMException; prototype: SkDOMException };

  interface SkDOMStringList {
    readonly length: number;
    item(index: number): string | null;
    contains(name: string): boolean;
    [index: number]: string;
    [Symbol.iterator](): IterableIterator<string>;
  }

  interface SkIDBRequest<T = unknown> extends SkEventTarget {
    readonly result: T;
    readonly error: SkDOMException | null;
    readonly readyState: "pending" | "done";
    readonly source: SkIDBObjectStore | SkIDBIndex | SkIDBCursor | null;
    readonly transaction: SkIDBTransaction | null;
    onsuccess: SkIDBHandler;
    onerror: SkIDBHandler;
  }
  interface SkIDBOpenDBRequest extends SkIDBRequest<SkIDBDatabase> {
    onupgradeneeded: ((ev: SkIDBVersionChangeEvent) => void) | null;
    onblocked: ((ev: SkIDBVersionChangeEvent) => void) | null;
  }
  interface SkIDBVersionChangeEvent extends SkEvent {
    readonly oldVersion: number;
    readonly newVersion: number | null;
  }

  interface SkIDBFactory {
    open(name: string, version?: number): SkIDBOpenDBRequest;
    deleteDatabase(name: string): SkIDBOpenDBRequest;
    databases(): Promise<{ name: string; version: number }[]>;
    cmp(a: unknown, b: unknown): -1 | 0 | 1;
  }

  interface SkIDBDatabase extends SkEventTarget {
    readonly name: string;
    readonly version: number;
    readonly objectStoreNames: SkDOMStringList;
    transaction(
      stores: string | string[],
      mode?: "readonly" | "readwrite",
      options?: { durability?: "default" | "strict" | "relaxed" },
    ): SkIDBTransaction;
    createObjectStore(
      name: string,
      options?: { keyPath?: string | string[] | null; autoIncrement?: boolean },
    ): SkIDBObjectStore;
    deleteObjectStore(name: string): void;
    close(): void;
    onversionchange: ((ev: SkIDBVersionChangeEvent) => void) | null;
    onclose: SkIDBHandler;
    onabort: SkIDBHandler;
    onerror: SkIDBHandler;
  }

  interface SkIDBTransaction extends SkEventTarget {
    readonly db: SkIDBDatabase;
    readonly mode: SkIDBMode;
    readonly durability: "default" | "strict" | "relaxed";
    readonly objectStoreNames: SkDOMStringList;
    readonly error: SkDOMException | null;
    objectStore(name: string): SkIDBObjectStore;
    commit(): void;
    abort(): void;
    oncomplete: SkIDBHandler;
    onabort: SkIDBHandler;
    onerror: SkIDBHandler;
  }

  interface SkIDBReadable {
    readonly name: string;
    readonly keyPath: string | string[] | null;
    get(query: SkIDBValidKey | SkIDBKeyRange): SkIDBRequest<unknown>;
    getKey(query: SkIDBValidKey | SkIDBKeyRange): SkIDBRequest<SkIDBValidKey | undefined>;
    getAll(query?: SkIDBValidKey | SkIDBKeyRange | null, count?: number): SkIDBRequest<unknown[]>;
    getAllKeys(query?: SkIDBValidKey | SkIDBKeyRange | null, count?: number): SkIDBRequest<SkIDBValidKey[]>;
    count(query?: SkIDBValidKey | SkIDBKeyRange): SkIDBRequest<number>;
    openCursor(
      query?: SkIDBValidKey | SkIDBKeyRange | null,
      direction?: SkIDBCursorDirection,
    ): SkIDBRequest<SkIDBCursorWithValue | null>;
    openKeyCursor(
      query?: SkIDBValidKey | SkIDBKeyRange | null,
      direction?: SkIDBCursorDirection,
    ): SkIDBRequest<SkIDBCursor | null>;
  }
  interface SkIDBObjectStore extends SkIDBReadable {
    readonly autoIncrement: boolean;
    readonly indexNames: SkDOMStringList;
    readonly transaction: SkIDBTransaction;
    put(value: unknown, key?: SkIDBValidKey): SkIDBRequest<SkIDBValidKey>;
    add(value: unknown, key?: SkIDBValidKey): SkIDBRequest<SkIDBValidKey>;
    delete(query: SkIDBValidKey | SkIDBKeyRange): SkIDBRequest<undefined>;
    clear(): SkIDBRequest<undefined>;
    index(name: string): SkIDBIndex;
    createIndex(
      name: string,
      keyPath: string | string[],
      options?: { unique?: boolean; multiEntry?: boolean },
    ): SkIDBIndex;
    deleteIndex(name: string): void;
  }
  interface SkIDBIndex extends SkIDBReadable {
    readonly objectStore: SkIDBObjectStore;
    readonly unique: boolean;
    readonly multiEntry: boolean;
  }

  interface SkIDBCursor {
    readonly source: SkIDBObjectStore | SkIDBIndex;
    readonly direction: SkIDBCursorDirection;
    readonly key: SkIDBValidKey;
    readonly primaryKey: SkIDBValidKey;
    readonly request: SkIDBRequest<SkIDBCursor | null>;
    advance(count: number): void;
    continue(key?: SkIDBValidKey): void;
    continuePrimaryKey(key: SkIDBValidKey, primaryKey: SkIDBValidKey): void;
    update(value: unknown): SkIDBRequest<SkIDBValidKey>;
    delete(): SkIDBRequest<undefined>;
  }
  interface SkIDBCursorWithValue extends SkIDBCursor {
    readonly value: unknown;
  }

  interface SkIDBKeyRange {
    readonly lower: SkIDBValidKey | undefined;
    readonly upper: SkIDBValidKey | undefined;
    readonly lowerOpen: boolean;
    readonly upperOpen: boolean;
    includes(key: SkIDBValidKey): boolean;
  }

  var indexedDB: SkIDBFactory;
  var IDBKeyRange: {
    only(value: SkIDBValidKey): SkIDBKeyRange;
    lowerBound(lower: SkIDBValidKey, open?: boolean): SkIDBKeyRange;
    upperBound(upper: SkIDBValidKey, open?: boolean): SkIDBKeyRange;
    bound(lower: SkIDBValidKey, upper: SkIDBValidKey, lowerOpen?: boolean, upperOpen?: boolean): SkIDBKeyRange;
    prototype: SkIDBKeyRange;
  };

  /** `estimate()` reports IndexedDB bytes against the quota (1 GiB by default); `persist()` and `persisted()` answer `true` because data is never evicted. */
  interface SkStorageManager {
    estimate(): Promise<{ usage: number; quota: number }>;
    persist(): Promise<boolean>;
    persisted(): Promise<boolean>;
  }
  interface SkNavigator {
    storage: SkStorageManager;
  }
}
