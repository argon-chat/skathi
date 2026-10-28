// The TIER-1 surface the app host installs — what an APP may assume exists.
//
// `packages/vue-native/src/engine.d.ts` is deliberately not this file. It is
// scoped to what `runtime.ts` and `livekit.ts` themselves touch ("as much of it
// as the voice shim touches and no more"), and the runtime package exports no
// ambient types for its consumers at all. So an app that writes the ordinary
// browser code the compat policy promises — `localStorage.setItem`,
// `matchMedia`, `navigator.clipboard`, `new AudioContext()` — had nothing
// telling the compiler those names exist.
//
// This file is that, for the four demo apps in this directory. Two rules kept
// while writing it:
//
//   1. NOTHING IS DECLARED THAT THE HOST DOES NOT PROVIDE. Each block names the
//      Rust or JS that installs it. A name that type-checks and is absent at
//      runtime is the exact bug the no-lib.dom decision exists to catch, so
//      adding to this file means reading the host first — never reaching for
//      lib.dom because a symbol was missing.
//   2. WHERE THE RUNTIME PACKAGE ALREADY HAS THE TYPE, THIS FILE MERGES INTO IT
//      rather than restating it. `VnNavigator`, `VnDocument`, `VnAudioContext`,
//      `VnFetchResponse` and friends below are interface merges onto
//      engine.d.ts's own declarations, so there is one definition per concept
//      and the two files cannot drift into disagreeing.
//
// This is a stopgap in the right place, not the final home: most of it belongs
// in the runtime package, shipped to apps as part of `vue-native`. See the
// conversion report.

export {};

// ---------------------------------------------------------------------------
// the window, as a global EventTarget — crates/vn-app/src/window.prelude.js
// ---------------------------------------------------------------------------
//
// The prelude installs these on `globalThis` before the app bundle evaluates.
// `resize` / `focus` / `blur` are the STANDARD window events; `appWindow`
// deliberately carries no `resize` of its own, so this is the only spelling.

declare global {
  function addEventListener(
    type: string,
    listener: (ev: VnEvent) => void,
    options?: boolean | { capture?: boolean; once?: boolean },
  ): void;
  function removeEventListener(
    type: string,
    listener: (ev: VnEvent) => void,
    options?: boolean | { capture?: boolean },
  ): void;
  function dispatchEvent(ev: VnEvent): boolean;

  // Geometry. `innerWidth`/`innerHeight`/`devicePixelRatio` come from redjs
  // itself; the prelude defines the `outer*` / `screen*` pair over the host's
  // per-frame snapshot (window.prelude.js §"ro(globalThis, …)").
  var innerWidth: number;
  var innerHeight: number;
  var outerWidth: number;
  var outerHeight: number;
  var screenX: number;
  var screenY: number;
  var devicePixelRatio: number;

  /** `resizeBy(dw, dh)` — window.prelude.js. There is no `moveBy`; nudging the
   *  window is `appWindow.setBounds()`, which is what WindowPage.vue does. */
  function resizeBy(dw: number, dh: number): void;

  /**
   * `window.open(url)` — runtime.ts over window.prelude.js's A-22. An
   * absolute http/https/mailto URL opens in the SYSTEM browser / mail client
   * and the call returns `null` (noopener semantics, D-F7); any other scheme
   * is refused with an advisory naming `appShell.openExternal`. No url, a
   * relative one or `about:blank` is A-22: "a second document" is not
   * supported, and the call returns `null`. docs/SHELL.md §1.
   */
  function open(url?: string, target?: string, features?: string): null;

  /**
   * The Launch Handler API (runtime.ts; docs/LAUNCH.md). This process's own
   * launch is one `LaunchParams`, buffered until `setConsumer`; with
   * `"singleInstance": true`, every later launch of the app is forwarded here
   * and is another. `targetURL` is the raw deep link (or null) — UNTRUSTED
   * input — and `files` is empty until FileSystemHandle exists (read
   * `appLaunch.paths` meanwhile; D-F8).
   */
  interface VnLaunchParams {
    readonly targetURL: string | null;
    readonly files: readonly unknown[];
  }
  var launchQueue: {
    setConsumer(consumer: (params: VnLaunchParams) => unknown): void;
  };

  /** `screen`, single-display in v1 (`getScreenDetails()` is not implemented
   *  and advises). window.prelude.js §"ro(screenObj, …)". */
  var screen: {
    readonly width: number;
    readonly height: number;
    readonly availWidth: number;
    readonly availHeight: number;
    readonly availLeft: number;
    readonly availTop: number;
    readonly isExtended: boolean;
  };

  /**
   * `matchMedia` — window.prelude.js §1.6, bridged to the SAME vn-css
   * evaluator the cascade runs, so a JS query and an `@media` block cannot
   * disagree. A query the evaluator cannot run advises rather than answering
   * false forever.
   */
  function matchMedia(query: string): VnMediaQueryList;

  interface VnMediaQueryList extends VnEventTarget {
    readonly matches: boolean;
    readonly media: string;
  }

  /** redjs's `performance`. Only `now()` is used here; it is a monotonic
   *  millisecond clock, as in a browser. */
  var performance: { now(): number };
}

// ---------------------------------------------------------------------------
// navigator — merged onto engine.d.ts's `VnNavigator`
// ---------------------------------------------------------------------------

declare global {
  interface VnNavigator {
    /**
     * Window Controls Overlay (docs/WINDOWING.md §2.3, window.prelude.js).
     * Standards-track, not an extension. The rect NARROWS when the host draws
     * its own caption cluster (`setTitleBarOverlay`), which is how
     * Titlebar.vue knows to drop its own buttons — reading the rect rather
     * than assuming a side is also what keeps it correct on macOS and RTL.
     */
    windowControlsOverlay: VnWindowControlsOverlay;

    /**
     * The async Clipboard API — vendor/redjs/redjs/src/webapi/clipboard.rs,
     * over the host's own clipboard (the same one the native `<input>` uses
     * for Ctrl+C: one owner, not two).
     *
     * `read()` / `write()` (the image forms) are NOT implemented in v1 and
     * REJECT; they are declared because rejecting is their contract and
     * StoragePage.vue demonstrates exactly that.
     */
    clipboard: {
      readText(): Promise<string>;
      writeText(text: string): Promise<void>;
      read(): Promise<unknown[]>;
      write(items: unknown[]): Promise<void>;
    };
  }

  interface VnWindowControlsOverlay extends VnEventTarget {
    readonly visible: boolean;
    getTitlebarAreaRect(): VnWindowRect;
  }

  /**
   * `appWindow.chrome` — the per-frame chrome snapshot the prelude publishes
   * (`ro(appWindow, "chrome", …)`, window.prelude.js). `VnAppWindow` types it
   * as a bare `Record<string, unknown>`, so `chrome.client.width` is `unknown`
   * and does not compile; the shape is fixed and small, so it is written down
   * here and the two pages that read it cast once. Reported.
   */
  interface VnWindowChrome {
    titleBarStyle: string;
    displayMode: string;
    fallbackTitleBar: boolean;
    overlayVisible: boolean;
    hostControls: boolean;
    /** The client area, in CSS pixels. */
    client: { width: number; height: number };
    /** Zero on all four sides once the app owns the whole client area. */
    contentInset: { left: number; top: number; right: number; bottom: number };
  }
}

// ---------------------------------------------------------------------------
// document — merged onto engine.d.ts's `VnDocument`
// ---------------------------------------------------------------------------
//
// NOTE: `document` itself stays `VnDocument | undefined`, because that is what
// engine.d.ts declares and a global `var` cannot be narrowed by a merge. Under
// vn-app it is always present (runtime.ts force-installs the entry points
// before the app bundle evaluates), so app code asserts it once per file and
// says so on the spot.

declare global {
  interface VnDocument {
    /** The Fullscreen API — window.prelude.js §"fsStack". Deliberately the
     *  STANDARD surface rather than an `appWindow` verb: fullscreen state is
     *  observed through `fullscreenchange`, which is why docs/WINDOWING.md
     *  §3.4 carries no `appWindow` event for it. */
    readonly fullscreenElement: VnElement | null;
    exitFullscreen(): Promise<void>;
    // NOT `documentElement`: engine.d.ts already declares it as
    // `documentElement?: unknown`, and an interface merge cannot narrow a
    // property that is already there. WindowPage.vue casts at the one place it
    // needs `.requestFullscreen()` off it, and says so.

    /** Inert, and returns `false` — exactly as a browser does when a command
     *  cannot be executed. clipboard.rs advises once naming
     *  `navigator.clipboard`. */
    execCommand(command: string): boolean;
  }
}

// ---------------------------------------------------------------------------
// elements
// ---------------------------------------------------------------------------
//
// The retained tree's mirror node. `runtime.ts` has the full and accurate
// interface for this (`MirrorElement`, ~150 lines of it) and does not EXPORT
// it, so an app holding a template ref to a native element has no name for
// what it is holding. Declared here as the subset these apps touch; every
// member is one `runtime.ts` really installs.

declare global {
  interface VnDomRect {
    readonly x: number;
    readonly y: number;
    readonly top: number;
    readonly left: number;
    readonly right: number;
    readonly bottom: number;
    readonly width: number;
    readonly height: number;
  }

  /**
   * `scrollTo` / `scrollBy`'s option bag. Spelled here rather than taken from
   * `lib.dom` for the reason at the top of this file: nothing is declared that
   * the host does not provide, and `behavior: "smooth"` is honoured as an
   * instant jump with one advisory (research/reports/m23/scroll.md).
   */
  interface VnScrollToOptions {
    left?: number;
    top?: number;
    behavior?: "auto" | "instant" | "smooth";
  }

  /** `scrollIntoView`'s option bag; `block` is vertical, `inline` horizontal. */
  interface VnScrollIntoViewOptions {
    block?: "start" | "center" | "end" | "nearest";
    inline?: "start" | "center" | "end" | "nearest";
    behavior?: "auto" | "instant" | "smooth";
  }

  interface VnElement extends VnEventTarget {
    /**
     * The RETAINED-TREE node id — `crates/vn-tree`'s arena index, which
     * `runtime.ts` puts on every mirror node it makes (`mk()`), NOT the `id`
     * ATTRIBUTE. That divergence is a known and reported one (m19 §5, m20
     * polish §7); it is declared here because it is also the addressing seam
     * the multi-window host fns are specified against — `__vn_window_open`
     * takes exactly this number (docs/MULTIWINDOW.md §5.2 invariant 1), which
     * is what lets `apps/popout` pop a subtree out with no runtime change.
     * When `element.id` is fixed to answer the attribute, the id a host fn
     * wants will need its own spelling and this line moves with it.
     */
    readonly id: VnNodeId;
    getBoundingClientRect(): VnDomRect;
    /**
     * The scroll position, over the host's real `ScrollHandle` — the same one
     * the wheel and the scrollbar thumb move. WRITABLE since m23: the value is
     * clamped to what the box can cover and the getter answers the clamped
     * number on the next line, with no frame in between. `0` on an element
     * that does not scroll, and a write to one does nothing, exactly as in a
     * browser.
     */
    scrollTop: number;
    scrollLeft: number;
    /** The CONTENT box of a scroll container (viewport + overflow). */
    readonly scrollWidth: number;
    readonly scrollHeight: number;
    /**
     * `scrollTo(x, y)` / `scrollTo({ left, top, behavior })`. `behavior:
     * "smooth"` is performed INSTANTLY and advises once — see
     * research/reports/m23/scroll.md for why it is not animated.
     */
    scrollTo(x?: number | VnScrollToOptions, y?: number): void;
    scrollBy(x?: number | VnScrollToOptions, y?: number): void;
    /**
     * Scrolls every scrollable ancestor so this element lands where `block` /
     * `inline` ask (`start` | `center` | `end` | `nearest`). Off-screen
     * elements work: the whole document is laid out, so a row far below the
     * fold has an exact rectangle to aim at.
     */
    scrollIntoView(arg?: boolean | VnScrollIntoViewOptions): void;
    focus(): void;
    blur(): void;
    /**
     * HTML's synthetic click (runtime.ts): a `MouseEvent("click")` through the
     * same capture/bubble walk a real click takes, then the activation
     * behaviour if nobody cancelled it — so `a.click()` on an `<a href>`
     * follows the link (http/https/mailto to the system, docs/SHELL.md §1).
     * Does nothing on a disabled form control.
     */
    click(): void;
    /** Patched onto `Element.prototype` by window.prelude.js; makes the
     *  WINDOW fullscreen and advises that per-element fullscreen is not a
     *  thing here. */
    requestFullscreen(): Promise<void>;
  }

  /**
   * `<video>` / `<audio>`. Every member is read LIVE off `__vn_media_state`
   * (crates/vn-video CONTRACT.md §6) rather than mirrored, so none of it is
   * reactive — the EVENTS are what drive a UI, exactly as in a browser.
   */
  interface VnMediaElement extends VnElement {
    currentTime: number;
    readonly duration: number;
    readonly paused: boolean;
    readonly ended: boolean;
    readonly seeking: boolean;
    readonly readyState: number;
    volume: number;
    muted: boolean;
    playbackRate: number;
    loop: boolean;
    readonly videoWidth: number;
    readonly videoHeight: number;
    /** A real `MediaError`: the message is the advisory naming what THIS
     *  machine cannot decode. `null` when there is none. */
    readonly error: VnMediaError | null;
    play(): Promise<void>;
    pause(): void;
    canPlayType(type: string): string;
  }

  interface VnMediaError {
    readonly code: number;
    readonly message: string;
  }

  /** What a media/lottie/transition event carries once `synthesize()` has
   *  spread the payload onto it (runtime.ts §"mediaDetail"/"lottieDetail"). */
  interface VnMediaEvent extends VnEvent {
    readonly currentTime: number;
    readonly duration: number;
    readonly readyState: number;
    readonly playbackRate: number;
  }
  interface VnLottieLoopEvent extends VnEvent {
    readonly currentTime: number;
    readonly totalTime: number;
    readonly currentLoop: number;
  }
}

// ---------------------------------------------------------------------------
// Web Storage — crates/vn-storage/src/lib.rs (docs/STORAGE.md tier 1)
// ---------------------------------------------------------------------------

declare global {
  interface VnStorage {
    readonly length: number;
    key(index: number): string | null;
    getItem(key: string): string | null;
    /** `value` is a STRING. A browser coerces whatever it is handed; so does
     *  this. Typed strictly so the coercion is written down at the call site
     *  rather than being invisible. */
    setItem(key: string, value: string): void;
    removeItem(key: string): void;
    clear(): void;
  }
  /** Backed by a file under the app's storage folder; survives a restart. */
  var localStorage: VnStorage;
  /** Per-run, never written anywhere. */
  var sessionStorage: VnStorage;
}

// ---------------------------------------------------------------------------
// IndexedDB — vendor/redjs/redjs/src/webapi/indexeddb/ (m25 P1)
// ---------------------------------------------------------------------------
//
// IndexedDB 3.0, the whole API, ON in vn-app by default (`indexeddb` in
// crates/vn-app/Cargo.toml). Dexie 4, idb 8, idb-keyval 6 and localForage run
// on it unmodified (redjs tests/idb_libs.rs). In P1 the databases live in
// process memory: nothing survives a restart, and redjs says so once
// (`[redjs] IndexedDB: … IN MEMORY ONLY`). The SQLite backend under the app's
// storage folder is P2; the API does not change when it lands.
//
// Typed here at the depth an app calls it directly; a wrapper library's own
// typings are its business. Keys are `IDBValidKey` in the spec: numbers,
// strings, Dates, ArrayBuffers/views and arrays of those.

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

// ---------------------------------------------------------------------------
// fetch — merged onto engine.d.ts's `VnFetchResponse`
// ---------------------------------------------------------------------------

declare global {
  interface VnFetchResponse {
    /** vendor/redjs/redjs/src/webapi/fetch.rs. AudioPage.vue feeds it
     *  straight to `decodeAudioData`, which is the browser's own idiom. */
    arrayBuffer(): Promise<ArrayBuffer>;
  }

  /**
   * `AbortController` / `AbortSignal` — vendor/redjs/redjs/src/webapi/abort.rs,
   * the real WHATWG pair, and `webapi::install` calls `abort::install`
   * UNCONDITIONALLY (no cargo feature, no host opt-in). So unlike `fetch` and
   * `AudioContext` next door this one is not `| undefined`: a redjs that lacks
   * it does not exist, and apps/devtools' request timeout depends on it.
   *
   * `fetch` reads `init.signal` (fetch.rs `signal_is_aborted`) and rejects with
   * an `AbortError`, which is what makes a wedged target a message rather than
   * a pane that waits forever.
   */
  interface VnAbortSignal extends VnEventTarget {
    readonly aborted: boolean;
    readonly reason: unknown;
    throwIfAborted(): void;
  }
  interface VnAbortController {
    readonly signal: VnAbortSignal;
    abort(reason?: unknown): void;
  }
  var AbortController: { new (): VnAbortController; prototype: VnAbortController };
}

// ---------------------------------------------------------------------------
// WebSocket / Blob — vendor/redjs/redjs/src/webapi/{websocket,blob}.rs
// ---------------------------------------------------------------------------
//
// redjs's own RFC 6455 client: a real EventTarget with real CloseEvent and
// real binary frames. Nothing about the Lab page's use of it is native-specific.

declare global {
  interface VnBlob {
    readonly size: number;
    readonly type: string;
    arrayBuffer(): Promise<ArrayBuffer>;
    text(): Promise<string>;
  }
  var Blob: { new (parts?: unknown[], options?: { type?: string }): VnBlob; prototype: VnBlob };

  interface VnCloseEvent extends VnEvent {
    readonly code: number;
    readonly reason: string;
    readonly wasClean: boolean;
  }
  interface VnMessageEvent extends VnEvent {
    /** `string` for a text frame; a `Blob` or an `ArrayBuffer` for a binary
     *  one, depending on `binaryType`. */
    readonly data: string | VnBlob | ArrayBuffer;
  }

  interface VnWebSocket extends VnEventTarget {
    readonly readyState: number;
    readonly bufferedAmount: number;
    readonly protocol: string;
    readonly url: string;
    binaryType: "blob" | "arraybuffer";
    onopen: ((ev: VnEvent) => void) | null;
    onerror: ((ev: VnEvent) => void) | null;
    onclose: ((ev: VnCloseEvent) => void) | null;
    onmessage: ((ev: VnMessageEvent) => void) | null;
    send(data: string | ArrayBuffer | ArrayBufferView): void;
    close(code?: number, reason?: string): void;
  }
  var WebSocket: {
    new (url: string, protocols?: string | string[]): VnWebSocket;
    prototype: VnWebSocket;
    readonly CONNECTING: 0;
    readonly OPEN: 1;
    readonly CLOSING: 2;
    readonly CLOSED: 3;
  };
}

// ---------------------------------------------------------------------------
// Web Audio — vendor/redjs/redjs/src/webapi/audio.rs, behind vn-app's `audio`
// cargo feature
// ---------------------------------------------------------------------------
//
// `AudioContext` is declared `| undefined` ON PURPOSE, and engine.d.ts's own
// comment is the reason: `vn-app --no-default-features` is a legitimate host
// with no Web Audio at all, so the implementation is ABSENT rather than
// stubbed and `typeof AudioContext !== "undefined"` is the honest question.
// Declaring it non-optional would make AudioPage.vue's feature probe look
// redundant when it is the whole story.

declare global {
  /** Merged onto engine.d.ts's `VnAudioContext`, which types only the two
   *  members the LiveKit shim uses. */
  interface VnAudioContext {
    readonly state: "suspended" | "running" | "closed";
    readonly sampleRate: number;
    readonly currentTime: number;
    readonly audioWorklet?: unknown;
    createOscillator(): VnOscillatorNode;
    createAnalyser(): VnAnalyserNode;
    createBuffer(channels: number, frames: number, sampleRate: number): VnAudioBuffer;
    createBufferSource(): VnAudioBufferSourceNode;
    decodeAudioData(bytes: ArrayBuffer): Promise<VnAudioBuffer>;
    resume(): Promise<void>;
    suspend(): Promise<void>;
    /**
     * OVERLOAD, deliberately added here: engine.d.ts's
     * `VnRedStreamSourceOptions` declares all four fields REQUIRED, but
     * `webapi/audio.rs` (`ctx_create_red_stream_source`, and its own doc
     * comment at §"`options`: `{ sampleRate?, channelCount?, latencySeconds?,
     * capacitySeconds? }`") defaults every one of them — `createRedStreamSource()`
     * with no argument at all is legal and its Rust test covers it. Reported;
     * the fix belongs in engine.d.ts.
     */
    createRedStreamSource(options?: Partial<VnRedStreamSourceOptions>): VnRedStreamSourceNode;
  }

  /** The PUSH seam. engine.d.ts types `streamId` and `close?` — what the voice
   *  shim needs — and not the producer-side surface a JS feeder uses, which is
   *  the rest of this. redjs::webapi::pcm. */
  interface VnRedStreamSourceNode {
    /** Frames actually accepted; less than `block.length` means the channel
     *  was full and the rest was dropped rather than blocking. */
    push(block: Float32Array): number;
    /** How deep the channel is right now. Reads `latencySeconds` the instant
     *  the node exists — `fixed_resample` PRE-FILLS with silence. */
    readonly queuedSeconds?: number;
    /** Is the device pulling from this stream at all? */
    readonly ready: boolean;
    /** Read-and-clear on the host side, which is what makes a rare glitch
     *  visible after the fact. */
    readonly underflowed: boolean;
    readonly overflowed: boolean;
  }

  /**
   * A `GainNode` whose `gain` is the WHOLE AudioParam.
   *
   * NOT a merge onto engine.d.ts's `VnGainNode`, which already declares
   * `readonly gain: { value: number }` — the `.value` half, which is all the
   * LiveKit shim writes — and a merge cannot narrow a property that is already
   * there. The timeline half is just as real: `webapi/audio.rs` implements
   * `setValueAtTime` / `linearRampToValueAtTime` /
   * `exponentialRampToValueAtTime` / `cancelScheduledValues`, and AudioPage.vue
   * exercises them on purpose ("so the AudioParam timeline is exercised, not
   * just `.value`"). Reported; the fix belongs in engine.d.ts.
   */
  interface VnGainNodeWithParam extends VnAudioNode {
    readonly gain: VnAudioParam;
  }

  interface VnAudioParam {
    value: number;
    cancelScheduledValues(when: number): void;
    setValueAtTime(value: number, when: number): void;
    linearRampToValueAtTime(value: number, when: number): void;
    exponentialRampToValueAtTime(value: number, when: number): void;
  }

  interface VnOscillatorNode extends VnAudioNode {
    type: string;
    readonly frequency: VnAudioParam;
    readonly detune: VnAudioParam;
    onended: (() => void) | null;
    start(when?: number): void;
    stop(when?: number): void;
  }

  interface VnAnalyserNode extends VnAudioNode {
    fftSize: number;
    smoothingTimeConstant: number;
    readonly frequencyBinCount: number;
    getByteFrequencyData(into: Uint8Array): void;
    getFloatTimeDomainData(into: Float32Array): void;
  }

  interface VnAudioBuffer {
    readonly numberOfChannels: number;
    readonly length: number;
    readonly sampleRate: number;
    readonly duration: number;
    getChannelData(channel: number): Float32Array;
    copyToChannel(source: Float32Array, channel: number): void;
  }

  interface VnAudioBufferSourceNode extends VnAudioNode {
    buffer: VnAudioBuffer | null;
    onended: (() => void) | null;
    start(when?: number): void;
    stop(when?: number): void;
  }

  var AudioContext: { new (): VnAudioContext; prototype: VnAudioContext } | undefined;
  /** Probed by AudioPage's capability matrix and genuinely absent — there is
   *  no offline rendering. Declared so the probe compiles; `undefined` is the
   *  honest type. */
  var OfflineAudioContext: undefined;
}

// ---------------------------------------------------------------------------
// host-published globals
// ---------------------------------------------------------------------------
//
// None of these is a web API and none pretends to be. They are values the HOST
// writes onto the global before the bundle evaluates, so an app that does not
// look for one never notices it is missing — which is why every one of them is
// optional here.

declare global {
  /**
   * `VN_SCENARIO=<name>` — pins the app to one screen at boot, so a soak or a
   * frame-time measurement is ONE process, ONE screen and no input.
   * `crates/vn-app/src/main.rs` publishes it (`globalThis.__VN_SCENARIO__ = …`).
   */
  var __VN_SCENARIO__: string | undefined;

  /** `VN_HTTP_ORIGIN` — where MediaPage points its http(s) `src` demos, so the
   *  page can be aimed at a real server without an internet dependency. */
  var __VN_HTTP_ORIGIN__: string | undefined;

  /** `VN_VOICE_IDENTITY` — which participant this PROCESS is, so two instances
   *  in two VN_DIST slots never collide on the SFU. */
  var __VN_VOICE_IDENTITY__: string | undefined;

  /**
   * The dev host's local JWT minter, COMPILED OUT of a production build — a
   * browser app fetches this from its own backend instead, and
   * `TokenSource.endpoint()` is the same shape. The `typeof !== "function"`
   * guard in VoicePage.vue is what makes the absence a message rather than a
   * crash, so this must stay optional.
   */
  var __vn_dev_voice_token: ((identity: string, room: string) => string) | undefined;

  /** Flushes the host profiler's window and relabels the following seconds, so
   *  `VN_PERF=1`'s log reads as one block per imgperf scenario. Installed only
   *  under `VN_PERF`. */
  var __vn_perf_mark: ((name: string) => void) | undefined;

  /** The scenario driver reaches VoicePage's verbs through `ctl.ts eval`, so
   *  the page publishes them under a known name. */
  var __voice: Record<string, (...args: never[]) => unknown> | undefined;
}
