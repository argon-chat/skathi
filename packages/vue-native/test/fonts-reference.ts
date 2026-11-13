// TEST ONLY: the pre-native Font Loading model, retained as a behavioral oracle. Never imported by src or
// included in production bundles; native behavior is tested against actual JSC in
// crates/vn-tree/src/font_api.rs.
//
// Web fonts: `FontFace`, `FontFaceSet` and `document.fonts`. The host does the work
// (crates/vn-tree/src/fonts.rs); this module is the object model over it: a `FontFace` per face (the
// stylesheet's included) and `document.fonts`, the set, with `ready`, `status`, `check()`, `load()`, the
// set methods and the `loading` / `loadingdone` / `loadingerror` events. Statuses move by the host's events
// (`__vn_fonts_set_listener`), delivered from the frame tick.
//
// `installFonts(globalThis, document)` must stay importable on its own: host functions are read off `g` at
// install time, and without them nothing is installed and `FontFace` stays undefined.

/** The host's half (crates/vn-tree/src/lib.rs, "web fonts"). */
export interface FontNatives {
  /** Status changes: `(id, "loading" | "loaded" | "error", message)`; id 0 + "list" = the sheet's faces changed. */
  __vn_fonts_set_listener(fn: (id: number, status: string, message: string) => void): void;
  __vn_fonts_list(): string;
  /** A script face: its id, or why it is a SyntaxError. */
  __vn_font_create(family: string, source: string, packed: string, now: boolean): number | string;
  __vn_font_load(id: number): void;
  __vn_font_set(id: number, inSet: boolean): void;
  /** The ids `font` + `text` select in the set, comma-joined; null = `font` does not parse. */
  __vn_fonts_match(font: string, text: string): string | null;
}

interface FaceRecord {
  id: number;
  css: boolean;
  family: string;
  style: string;
  weight: string;
  stretch: string;
  unicodeRange: string;
  display: string;
  status: FontFaceLoadStatus;
  error: string;
}

type FontFaceLoadStatus = "unloaded" | "loading" | "loaded" | "error";

interface Descriptors {
  style?: string;
  weight?: string;
  stretch?: string;
  unicodeRange?: string;
  variant?: string;
  featureSettings?: string;
  variationSettings?: string;
  display?: string;
  ascentOverride?: string;
  descentOverride?: string;
  lineGapOverride?: string;
}

const DEFAULTS: Required<Descriptors> = {
  style: "normal",
  weight: "normal",
  stretch: "normal",
  unicodeRange: "U+0-10FFFF",
  variant: "normal",
  featureSettings: "normal",
  variationSettings: "normal",
  display: "auto",
  ascentOverride: "normal",
  descentOverride: "normal",
  lineGapOverride: "normal",
};

/** The descriptors the host parses, packed (`fonts::descriptors`): `|` occurs in none of them. */
const PACKED = ["style", "weight", "stretch", "unicodeRange", "display"] as const;

type Listener = (ev: object) => unknown;

interface Engine {
  EventTarget: { new (): VnEventTarget };
  Event: { new (type: string): VnEvent };
  DOMException?: { new (message: string, name: string): Error };
  Blob?: { new (parts: unknown[], options?: { type?: string }): object };
  URL?: { createObjectURL?(blob: object): string; revokeObjectURL?(url: string): void };
}

/** Installs `FontFace`, `FontFaceSet`, `FontFaceSetLoadEvent` and `document.fonts`. Returns the set (or null). */
export function installFonts(g: Record<string, unknown>, doc: object | null | undefined): object | null {
  const n = g as unknown as Partial<FontNatives> & Partial<Engine>;
  if (typeof n.__vn_fonts_list !== "function" || typeof n.__vn_fonts_set_listener !== "function") return null;
  const host = n as FontNatives;
  // The world's own (the engine's global, or a test's), so `document.fonts`
  // is an EventTarget of the page's realm.
  const EventTargetBase = (n.EventTarget ?? class {}) as { new (): VnEventTarget };
  const domError = (message: string, name: string): Error => {
    if (typeof n.DOMException === "function") return new n.DOMException(message, name);
    const e = new Error(message);
    e.name = name;
    return e;
  };

  const byId = new Map<number, FontFaceImpl>();
  let cssFaces: FontFaceImpl[] = [];
  const added: FontFaceImpl[] = [];
  let adopting: FaceRecord | null = null;

  class FontFaceImpl {
    family = "";
    style = DEFAULTS.style;
    weight = DEFAULTS.weight;
    stretch = DEFAULTS.stretch;
    unicodeRange = DEFAULTS.unicodeRange;
    variant = DEFAULTS.variant;
    featureSettings = DEFAULTS.featureSettings;
    variationSettings = DEFAULTS.variationSettings;
    display = DEFAULTS.display;
    ascentOverride = DEFAULTS.ascentOverride;
    descentOverride = DEFAULTS.descentOverride;
    lineGapOverride = DEFAULTS.lineGapOverride;
    #id = 0;
    #css = false;
    #status: FontFaceLoadStatus = "unloaded";
    #loaded: Promise<FontFaceImpl>;
    #resolve!: (f: FontFaceImpl) => void;
    #reject!: (e: unknown) => void;
    /** The `blob:` URL a binary source was handed over as, revoked once it is read. */
    #blobUrl: string | null = null;

    constructor(family: unknown, source: unknown, descriptors?: Descriptors) {
      this.#loaded = new Promise<FontFaceImpl>((resolve, reject) => {
        this.#resolve = resolve;
        this.#reject = reject;
      });
      // A rejection nobody asked about is not an error the page made.
      this.#loaded.catch(() => {});
      const record = adopting;
      if (record) {
        this.#id = record.id;
        this.#css = record.css;
        this.family = record.family;
        this.style = record.style;
        this.weight = record.weight;
        this.stretch = record.stretch;
        this.unicodeRange = record.unicodeRange;
        this.display = record.display;
        this.settle(record.status, record.error);
        return;
      }
      if (arguments.length < 2) throw new TypeError(`Failed to construct 'FontFace': 2 arguments required, but only ${arguments.length} present.`);
      this.family = String(family);
      const d = descriptors ?? {};
      for (const k of Object.keys(DEFAULTS) as (keyof Descriptors)[]) {
        if (d[k] !== undefined) this[k] = String(d[k]);
      }
      let src: string;
      let binary = false;
      if (typeof source === "string") {
        src = source;
      } else if (source instanceof ArrayBuffer || ArrayBuffer.isView(source)) {
        // The bytes go over as a `blob:` URL: the host reads in-memory blobs by URL (crates/vn-tree/src/img.rs)
        // and a constructor with a binary source starts loading at once.
        const url = n.URL;
        if (typeof n.Blob !== "function" || typeof url?.createObjectURL !== "function") {
          this.#fail("SyntaxError", "FontFace: a binary source needs Blob and URL.createObjectURL");
          return;
        }
        this.#blobUrl = url.createObjectURL(new n.Blob([source]));
        src = `url("${this.#blobUrl}")`;
        binary = true;
      } else {
        src = String(source);
      }
      const packed = PACKED.map((k) => (k === "unicodeRange" && this[k] === DEFAULTS.unicodeRange ? "" : this[k].replace(/\|/g, ""))).join("|");
      const made = host.__vn_font_create(this.family, src, packed, binary);
      if (typeof made === "string") {
        this.#fail("SyntaxError", made);
        return;
      }
      this.#id = made;
      byId.set(made, this);
      if (binary) this.#status = "loading";
    }

    get status(): FontFaceLoadStatus {
      return this.#status;
    }
    get loaded(): Promise<FontFaceImpl> {
      return this.#loaded;
    }

    load(): Promise<FontFaceImpl> {
      if (this.#status === "unloaded" && this.#id) {
        this.#status = "loading";
        host.__vn_font_load(this.#id);
        if (inSet(this)) state();
      }
      return this.#loaded;
    }

    #fail(name: string, message: string): void {
      this.#status = "error";
      this.#reject(domError(message, name));
    }

    settle(status: string, message: string): void {
      if (status === "loading") {
        if (this.#status === "unloaded") this.#status = "loading";
        return;
      }
      if (status === "loaded" && this.#status !== "loaded") {
        this.#status = "loaded";
        this.#resolve(this);
        this.#dropBlob();
      } else if (status === "error" && this.#status !== "error") {
        this.#status = "error";
        this.#reject(domError(message || `A network error occurred loading the font '${this.family}'.`, "NetworkError"));
        this.#dropBlob();
      }
    }

    #dropBlob(): void {
      if (this.#blobUrl) n.URL?.revokeObjectURL?.(this.#blobUrl);
      this.#blobUrl = null;
    }

    get hostId(): number {
      return this.#id;
    }
    get cssConnected(): boolean {
      return this.#css;
    }
  }
  // The module-internal members stay off the page's view of the prototype.
  for (const k of ["settle", "hostId", "cssConnected"]) {
    const d = Object.getOwnPropertyDescriptor(FontFaceImpl.prototype, k)!;
    Object.defineProperty(FontFaceImpl.prototype, k, { ...d, enumerable: false });
  }
  Object.defineProperty(FontFaceImpl, "name", { value: "FontFace" });
  Object.defineProperty(FontFaceImpl.prototype, Symbol.toStringTag, { value: "FontFace", configurable: true });

  const adopt = (r: FaceRecord): FontFaceImpl => {
    adopting = r;
    try {
      return new FontFaceImpl("", "");
    } finally {
      adopting = null;
    }
  };

  /** A status from the host, applied; a face of the set that finished is noted for the next `loadingdone`/`loadingerror`. */
  const apply = (f: FontFaceImpl, status: string, message: string): void => {
    const before = f.status;
    f.settle(status, message);
    if (inSet(f) && before !== f.status) {
      if (f.status === "loaded") loadedSince.push(f);
      if (f.status === "error") failedSince.push(f);
    }
  };

  /** The sheet's faces, again from the host's list (a recompile can add or drop rules); read on every use as well as on the `list` event, since a `mount()` sheet is the host's at once and its event arrives next frame. */
  let lastList = "";
  const syncCss = (): void => {
    const raw = host.__vn_fonts_list();
    if (raw === lastList) return;
    lastList = raw;
    const records = JSON.parse(raw) as FaceRecord[];
    const next: FontFaceImpl[] = [];
    for (const r of records) {
      const known = byId.get(r.id);
      if (known) apply(known, r.status, r.error);
      if (!r.css) continue;
      let f = known;
      if (!f) {
        f = adopt(r);
        byId.set(r.id, f);
      }
      next.push(f);
    }
    for (const f of cssFaces) if (!next.includes(f)) byId.delete(f.hostId);
    cssFaces = next;
  };
  const fresh = (): void => {
    syncCss();
    state();
  };

  const members = (): FontFaceImpl[] => [...cssFaces, ...added];
  const inSet = (f: FontFaceImpl): boolean => cssFaces.includes(f) || added.includes(f);

  let setStatus: "loading" | "loaded" = "loaded";
  let ready!: Promise<FontFaceSetImpl>;
  let resolveReady: ((s: FontFaceSetImpl) => void) | null = null;
  const freshReady = () => {
    ready = new Promise<FontFaceSetImpl>((resolve) => (resolveReady = resolve));
  };
  let loadedSince: FontFaceImpl[] = [];
  let failedSince: FontFaceImpl[] = [];

  const fire = (type: string, faces: FontFaceImpl[]): void => {
    const Ev = n.Event;
    if (typeof Ev !== "function") return;
    const ev = new Ev(type);
    Object.defineProperty(ev, "fontfaces", { value: Object.freeze([...faces]), enumerable: true });
    set.dispatchEvent?.(ev);
    const h = (set as unknown as Record<string, unknown>)[`on${type}`];
    if (typeof h === "function") {
      try {
        (h as Listener).call(set, ev);
      } catch (e) {
        console.error(e);
      }
    }
  };

  /** The host laid out a frame with nothing loading or newly asked for. Faces load lazily when drawn text asks, so `ready` waits for this, as a browser's waits for pending layout. */
  let hostQuiet = false;

  const state = (): void => {
    const loading = members().some((f) => f.status === "loading");
    if (loading && setStatus === "loaded") {
      setStatus = "loading";
      if (!resolveReady) freshReady();
      fire("loading", members().filter((f) => f.status === "loading"));
    } else if (!loading && setStatus === "loading") {
      setStatus = "loaded";
      const [ok, bad] = [loadedSince, failedSince];
      loadedSince = [];
      failedSince = [];
      fire("loadingdone", ok);
      if (bad.length) fire("loadingerror", bad);
    }
    if (!loading && hostQuiet && resolveReady) {
      resolveReady(set);
      resolveReady = null;
    }
  };

  class FontFaceSetImpl extends EventTargetBase {
    onloading: Listener | null = null;
    onloadingdone: Listener | null = null;
    onloadingerror: Listener | null = null;

    get status(): "loading" | "loaded" {
      fresh();
      return setStatus;
    }
    get ready(): Promise<FontFaceSetImpl> {
      fresh();
      return ready;
    }
    get size(): number {
      fresh();
      return members().length;
    }

    add(face: unknown): this {
      const f = face as FontFaceImpl;
      if (!(face instanceof FontFaceImpl)) throw new TypeError("Failed to execute 'add' on 'FontFaceSet': parameter 1 is not of type 'FontFace'.");
      if (f.cssConnected) throw domError("Failed to execute 'add' on 'FontFaceSet': Cannot add a CSS-connected FontFace.", "InvalidModificationError");
      if (!added.includes(f)) {
        added.push(f);
        if (f.hostId) host.__vn_font_set(f.hostId, true);
        state();
      }
      return this;
    }

    delete(face: unknown): boolean {
      const f = face as FontFaceImpl;
      if (!(face instanceof FontFaceImpl) || f.cssConnected) return false;
      const i = added.indexOf(f);
      if (i < 0) return false;
      added.splice(i, 1);
      if (f.hostId) host.__vn_font_set(f.hostId, false);
      state();
      return true;
    }

    clear(): void {
      for (const f of [...added]) this.delete(f);
    }

    has(face: unknown): boolean {
      fresh();
      return face instanceof FontFaceImpl && inSet(face);
    }

    forEach(cb: (value: FontFaceImpl, key: FontFaceImpl, set: FontFaceSetImpl) => void, thisArg?: unknown): void {
      fresh();
      for (const f of members()) cb.call(thisArg, f, f, this);
    }
    values(): IterableIterator<FontFaceImpl> {
      fresh();
      return members()[Symbol.iterator]();
    }
    keys(): IterableIterator<FontFaceImpl> {
      return this.values();
    }
    entries(): IterableIterator<[FontFaceImpl, FontFaceImpl]> {
      fresh();
      return members().map((f) => [f, f] as [FontFaceImpl, FontFaceImpl])[Symbol.iterator]();
    }
    [Symbol.iterator](): IterableIterator<FontFaceImpl> {
      return this.values();
    }

    /** The faces `font` and `text` select; throws SyntaxError for a `font` that does not parse. */
    #matching(method: string, font: unknown, text: unknown): FontFaceImpl[] {
      fresh();
      const ids = host.__vn_fonts_match(String(font), text === undefined ? " " : String(text));
      if (ids == null) throw domError(`Failed to execute '${method}' on 'FontFaceSet': Could not resolve '${String(font)}' as a font.`, "SyntaxError");
      return ids
        .split(",")
        .filter(Boolean)
        .map((id) => byId.get(Number(id)))
        .filter((f): f is FontFaceImpl => !!f && inSet(f));
    }

    check(font: unknown, text?: unknown): boolean {
      if (arguments.length < 1) throw new TypeError("Failed to execute 'check' on 'FontFaceSet': 1 argument required, but only 0 present.");
      return this.#matching("check", font, text).every((f) => f.status === "loaded");
    }

    load(font: unknown, text?: unknown): Promise<FontFaceImpl[]> {
      let faces: FontFaceImpl[];
      try {
        faces = this.#matching("load", font, text);
      } catch (e) {
        return Promise.reject(e);
      }
      return Promise.all(faces.map((f) => f.load())).then(() => faces);
    }
  }
  Object.defineProperty(FontFaceSetImpl, "name", { value: "FontFaceSet" });
  Object.defineProperty(FontFaceSetImpl.prototype, Symbol.toStringTag, { value: "FontFaceSet", configurable: true });

  const set = new FontFaceSetImpl();

  host.__vn_fonts_set_listener((id: number, status: string, message: string) => {
    if (id === 0) {
      if (status === "list") syncCss();
      else if (status === "quiet") hostQuiet = true;
      else if (status === "busy") {
        hostQuiet = false;
        if (!resolveReady) freshReady();
      }
    } else {
      const f = byId.get(id);
      if (f) apply(f, status, message);
    }
    state();
  });

  // Pending until the host's first quiet frame: the first frame's text has
  // not asked for its faces yet.
  syncCss();
  freshReady();
  state();

  const define = (target: object, name: string, value: unknown) =>
    Object.defineProperty(target, name, { value, configurable: true, writable: true });
  define(g, "FontFace", FontFaceImpl);
  define(g, "FontFaceSet", FontFaceSetImpl);
  if (doc) Object.defineProperty(doc, "fonts", { get: () => set, configurable: true });
  return set;
}
