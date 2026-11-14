// IndexedDB 3.0 as `indexedDB` and `navigator.storage` in a Skathi app. Databases are SQLite
// files under the app's storage folder (`<folder>\indexeddb\`), encrypted at rest when the storage
// is; a host built without storage keeps them in memory. Typed at the depth an app calls it
// directly. Merges onto engine.d.ts's `Event` / `EventTarget` / `Navigator`.

export {};

declare global {
  type IDBValidKey = number | string | Date | ArrayBuffer | ArrayBufferView | IDBValidKey[];
  type IDBTransactionMode = "readonly" | "readwrite" | "versionchange";
  type IDBCursorDirection = "next" | "nextunique" | "prev" | "prevunique";

  interface DOMException {
    readonly name: string;
    readonly message: string;
    readonly code: number;
  }
  var DOMException: { new (message?: string, name?: string): DOMException; prototype: DOMException };

  interface DOMStringList {
    readonly length: number;
    item(index: number): string | null;
    contains(name: string): boolean;
    [index: number]: string;
    [Symbol.iterator](): IterableIterator<string>;
  }

  interface IDBRequest<T = unknown> extends EventTarget {
    readonly result: T;
    readonly error: DOMException | null;
    readonly readyState: "pending" | "done";
    readonly source: IDBObjectStore | IDBIndex | IDBCursor | null;
    readonly transaction: IDBTransaction | null;
    onsuccess: ((ev: Event) => unknown) | null;
    onerror: ((ev: Event) => unknown) | null;
  }
  interface IDBOpenDBRequest extends IDBRequest<IDBDatabase> {
    onupgradeneeded: ((ev: IDBVersionChangeEvent) => void) | null;
    onblocked: ((ev: IDBVersionChangeEvent) => void) | null;
  }
  interface IDBVersionChangeEvent extends Event {
    readonly oldVersion: number;
    readonly newVersion: number | null;
  }

  interface IDBFactory {
    open(name: string, version?: number): IDBOpenDBRequest;
    deleteDatabase(name: string): IDBOpenDBRequest;
    databases(): Promise<{ name: string; version: number }[]>;
    cmp(a: unknown, b: unknown): -1 | 0 | 1;
  }

  interface IDBDatabase extends EventTarget {
    readonly name: string;
    readonly version: number;
    readonly objectStoreNames: DOMStringList;
    transaction(
      stores: string | string[],
      mode?: "readonly" | "readwrite",
      options?: { durability?: "default" | "strict" | "relaxed" },
    ): IDBTransaction;
    createObjectStore(
      name: string,
      options?: { keyPath?: string | string[] | null; autoIncrement?: boolean },
    ): IDBObjectStore;
    deleteObjectStore(name: string): void;
    close(): void;
    onversionchange: ((ev: IDBVersionChangeEvent) => void) | null;
    onclose: ((ev: Event) => unknown) | null;
    onabort: ((ev: Event) => unknown) | null;
    onerror: ((ev: Event) => unknown) | null;
  }

  interface IDBTransaction extends EventTarget {
    readonly db: IDBDatabase;
    readonly mode: IDBTransactionMode;
    readonly durability: "default" | "strict" | "relaxed";
    readonly objectStoreNames: DOMStringList;
    readonly error: DOMException | null;
    objectStore(name: string): IDBObjectStore;
    commit(): void;
    abort(): void;
    oncomplete: ((ev: Event) => unknown) | null;
    onabort: ((ev: Event) => unknown) | null;
    onerror: ((ev: Event) => unknown) | null;
  }

  interface SkIDBReadable {
    readonly name: string;
    readonly keyPath: string | string[] | null;
    get(query: IDBValidKey | IDBKeyRange): IDBRequest<unknown>;
    getKey(query: IDBValidKey | IDBKeyRange): IDBRequest<IDBValidKey | undefined>;
    getAll(query?: IDBValidKey | IDBKeyRange | null, count?: number): IDBRequest<unknown[]>;
    getAllKeys(query?: IDBValidKey | IDBKeyRange | null, count?: number): IDBRequest<IDBValidKey[]>;
    count(query?: IDBValidKey | IDBKeyRange): IDBRequest<number>;
    openCursor(
      query?: IDBValidKey | IDBKeyRange | null,
      direction?: IDBCursorDirection,
    ): IDBRequest<IDBCursorWithValue | null>;
    openKeyCursor(
      query?: IDBValidKey | IDBKeyRange | null,
      direction?: IDBCursorDirection,
    ): IDBRequest<IDBCursor | null>;
  }
  interface IDBObjectStore extends SkIDBReadable {
    readonly autoIncrement: boolean;
    readonly indexNames: DOMStringList;
    readonly transaction: IDBTransaction;
    put(value: unknown, key?: IDBValidKey): IDBRequest<IDBValidKey>;
    add(value: unknown, key?: IDBValidKey): IDBRequest<IDBValidKey>;
    delete(query: IDBValidKey | IDBKeyRange): IDBRequest<undefined>;
    clear(): IDBRequest<undefined>;
    index(name: string): IDBIndex;
    createIndex(
      name: string,
      keyPath: string | string[],
      options?: { unique?: boolean; multiEntry?: boolean },
    ): IDBIndex;
    deleteIndex(name: string): void;
  }
  interface IDBIndex extends SkIDBReadable {
    readonly objectStore: IDBObjectStore;
    readonly unique: boolean;
    readonly multiEntry: boolean;
  }

  interface IDBCursor {
    readonly source: IDBObjectStore | IDBIndex;
    readonly direction: IDBCursorDirection;
    readonly key: IDBValidKey;
    readonly primaryKey: IDBValidKey;
    readonly request: IDBRequest<IDBCursor | null>;
    advance(count: number): void;
    continue(key?: IDBValidKey): void;
    continuePrimaryKey(key: IDBValidKey, primaryKey: IDBValidKey): void;
    update(value: unknown): IDBRequest<IDBValidKey>;
    delete(): IDBRequest<undefined>;
  }
  interface IDBCursorWithValue extends IDBCursor {
    readonly value: unknown;
  }

  interface IDBKeyRange {
    readonly lower: IDBValidKey | undefined;
    readonly upper: IDBValidKey | undefined;
    readonly lowerOpen: boolean;
    readonly upperOpen: boolean;
    includes(key: IDBValidKey): boolean;
  }

  var indexedDB: IDBFactory;
  var IDBKeyRange: {
    only(value: IDBValidKey): IDBKeyRange;
    lowerBound(lower: IDBValidKey, open?: boolean): IDBKeyRange;
    upperBound(upper: IDBValidKey, open?: boolean): IDBKeyRange;
    bound(lower: IDBValidKey, upper: IDBValidKey, lowerOpen?: boolean, upperOpen?: boolean): IDBKeyRange;
    prototype: IDBKeyRange;
  };

  /** `estimate()` reports IndexedDB bytes against the quota (1 GiB by default); `persist()` and `persisted()` answer `true` because data is never evicted. */
  interface StorageManager {
    estimate(): Promise<{ usage: number; quota: number }>;
    persist(): Promise<boolean>;
    persisted(): Promise<boolean>;
  }
  interface Navigator {
    storage: StorageManager;
  }
}
