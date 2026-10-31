// Web fonts' object model (src/fonts.ts): `FontFace`, `document.fonts` and its
// `ready` / `check` / `load` / set methods / loading events, over a fake of the
// host's loader (crates/vn-tree/src/fonts.rs, whose parsing, selection and
// registration are its own Rust tests).

import { describe, expect, test } from "bun:test";
import { installFonts } from "../src/fonts";

type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any

interface Rec {
  id: number;
  css: boolean;
  family: string;
  style: string;
  weight: string;
  stretch: string;
  unicodeRange: string;
  display: string;
  status: string;
  error: string;
  inSet: boolean;
  /** The PUA-only range of a face like Alcarin Tengwar. */
  range?: [number, number];
}

/** A fake host: a face list, the calls made on it, and the listener to drive. */
function fakeHost(css: Partial<Rec>[] = []) {
  const faces: Rec[] = [];
  const calls: Any[] = [];
  let listener: ((id: number, status: string, message: string) => void) | null = null;
  let next = 0;
  const rec = (r: Partial<Rec>): Rec => ({
    id: ++next,
    css: false,
    family: "",
    style: "normal",
    weight: "normal",
    stretch: "normal",
    unicodeRange: "U+0-10FFFF",
    display: "auto",
    status: "unloaded",
    error: "",
    inSet: false,
    ...r,
  });
  for (const c of css) faces.push(rec({ css: true, inSet: true, status: "loading", ...c }));
  const blobs = new Map<string, unknown>();
  const g: Any = {
    EventTarget,
    Event,
    DOMException,
    Blob,
    URL: {
      createObjectURL(b: unknown) {
        const u = `blob:fake/${blobs.size + 1}`;
        blobs.set(u, b);
        return u;
      },
      revokeObjectURL(u: string) {
        blobs.delete(u);
      },
    },
    __vn_fonts_set_listener(f: Any) {
      listener = f;
    },
    __vn_fonts_list() {
      return JSON.stringify(faces);
    },
    __vn_font_create(family: string, source: string, packed: string, now: boolean) {
      calls.push(["create", family, source, packed, now]);
      if (source.startsWith("nonsense")) return `FontFace: the source "${source}" could not be parsed`;
      const r = rec({ family, status: now ? "loading" : "unloaded" });
      faces.push(r);
      return r.id;
    },
    __vn_font_load(id: number) {
      calls.push(["load", id]);
      faces.find((f) => f.id === id)!.status = "loading";
    },
    __vn_font_set(id: number, on: boolean) {
      calls.push(["set", id, on]);
      faces.find((f) => f.id === id)!.inSet = on;
    },
    __vn_fonts_match(font: string, text: string) {
      const m = /\d+px\s+(.*)$/.exec(font);
      if (!m) return null;
      const fams = m[1]!.split(",").map((s) => s.trim().replace(/^["']|["']$/g, "").toLowerCase());
      return faces
        .filter((f) => f.inSet && fams.includes(f.family.toLowerCase()))
        .filter((f) => !f.range || [...text].some((c) => c.codePointAt(0)! >= f.range![0] && c.codePointAt(0)! <= f.range![1]))
        .map((f) => f.id)
        .join(",");
    },
  };
  const doc: Any = {};
  const set: Any = installFonts(g, doc);
  /** What the host's frame tick delivers. */
  const emit = (id: number, status: string, message = "") => {
    const f = faces.find((x) => x.id === id);
    if (f && status !== "list") f.status = status;
    listener!(id, status, message);
  };
  return { g, doc, set, faces, calls, emit, blobs, declare: (r: Partial<Rec>) => faces.push(rec({ css: true, inSet: true, status: "loading", ...r })) };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

describe("web fonts (src/fonts.ts)", () => {
  test("nothing is installed without the host's loader", () => {
    const g: Any = {};
    expect(installFonts(g, {})).toBeNull();
    expect(g.FontFace).toBeUndefined();
  });

  test("the sheet's faces are document.fonts, and ready waits for them", async () => {
    const h = fakeHost([{ family: "Lato", weight: "700" }, { family: "Alcarin Tengwar", unicodeRange: "U+E000-E0FF", range: [0xe000, 0xe0ff] }]);
    expect(h.doc.fonts).toBe(h.set);
    expect(String(h.set)).toBe("[object FontFaceSet]");
    expect(h.set instanceof EventTarget).toBe(true);
    expect(h.set.size).toBe(2);
    const [lato, tengwar] = [...h.set];
    expect(lato instanceof h.g.FontFace).toBe(true);
    expect([lato.family, lato.weight, lato.status]).toEqual(["Lato", "700", "loading"]);
    expect(tengwar.unicodeRange).toBe("U+E000-E0FF");
    expect(h.set.status).toBe("loading");

    let ready = false;
    h.set.ready.then((s: Any) => {
      expect(s).toBe(h.set);
      ready = true;
    });
    const done: Any[] = [];
    h.set.addEventListener("loadingdone", (e: Any) => done.push(e.fontfaces.map((f: Any) => f.family)));
    h.emit(1, "loaded");
    await tick();
    expect(ready).toBe(false);
    expect(lato.status).toBe("loaded");
    h.emit(2, "loaded");
    await tick();
    expect(ready).toBe(true);
    expect(h.set.status).toBe("loaded");
    expect(done).toEqual([["Lato", "Alcarin Tengwar"]]);
    expect(await lato.loaded).toBe(lato);
  });

  test("check and load select by family and text, and a bad font string is a SyntaxError", async () => {
    const h = fakeHost([{ family: "Lato" }, { family: "Alcarin Tengwar", range: [0xe000, 0xe0ff] }]);
    expect(h.set.check("16px Lato")).toBe(false);
    expect(h.set.check("16px Inter")).toBe(true);
    // Latin text does not select the PUA face.
    h.emit(1, "loaded");
    expect(h.set.check("16px 'Alcarin Tengwar', Lato", "abc")).toBe(true);
    expect(h.set.check("16px 'Alcarin Tengwar', Lato", "\u{E010}")).toBe(false);
    expect(() => h.set.check("Lato")).toThrow(/Could not resolve/);
    await expect(h.set.load("Lato")).rejects.toMatchObject({ name: "SyntaxError" });
    const p = h.set.load("16px 'Alcarin Tengwar'", "\u{E010}");
    h.emit(2, "loaded");
    const faces = await p;
    expect(faces.map((f: Any) => f.family)).toEqual(["Alcarin Tengwar"]);
  });

  test("a script face loads on load(), joins the set on add(), and reports its failure", async () => {
    const h = fakeHost();
    const F = h.g.FontFace;
    const face = new F("Brand", "url(https://cdn.example/brand.woff2)", { weight: "300 700", style: "italic" });
    expect(face.status).toBe("unloaded");
    expect(h.calls[0]).toEqual(["create", "Brand", "url(https://cdn.example/brand.woff2)", "italic|300 700|normal||auto", false]);
    expect(h.set.has(face)).toBe(false);
    const loaded = face.load();
    expect(face.status).toBe("loading");
    expect(h.calls[1]).toEqual(["load", 1]);
    // The textComposition.ts idiom: load, then add.
    const added = loaded.then((f: Any) => h.set.add(f));
    h.emit(1, "loaded");
    expect(await added).toBe(h.set);
    expect(h.set.has(face)).toBe(true);
    expect(h.calls.at(-1)).toEqual(["set", 1, true]);
    expect(h.set.delete(face)).toBe(true);
    expect(h.calls.at(-1)).toEqual(["set", 1, false]);

    const bad = new F("Gone", "url(https://cdn.example/404.woff2)");
    h.set.add(bad);
    const p = bad.load();
    expect(h.set.status).toBe("loading");
    const errors: Any[] = [];
    h.set.onloadingerror = (e: Any) => errors.push(e.fontfaces.length);
    h.emit(2, "error", "https://cdn.example/404.woff2: HTTP 404 Not Found");
    await expect(p).rejects.toMatchObject({ name: "NetworkError", message: expect.stringContaining("404") });
    expect(bad.status).toBe("error");
    expect(errors).toEqual([1]);
  });

  test("a binary source goes over as a blob: URL, loads at once, and the URL is revoked after", async () => {
    const h = fakeHost();
    const face = new h.g.FontFace("Bytes", new Uint8Array([0, 1, 0, 0]));
    expect(face.status).toBe("loading");
    const [, family, source, , now] = h.calls[0];
    expect([family, now]).toEqual(["Bytes", true]);
    expect(source).toMatch(/^url\("blob:fake\/1"\)$/);
    expect(h.blobs.size).toBe(1);
    h.emit(1, "loaded");
    await face.loaded;
    expect(h.blobs.size).toBe(0);
  });

  test("a source that does not parse is a face in error, not a throw", async () => {
    const h = fakeHost();
    const face = new h.g.FontFace("X", "nonsense(");
    expect(face.status).toBe("error");
    await expect(face.loaded).rejects.toMatchObject({ name: "SyntaxError" });
    expect(() => new h.g.FontFace("only one")).toThrow(TypeError);
  });

  test("a sheet installed since the last event is the set's at once, and ready waits for it", async () => {
    // The mount installs the sheet; its `list` event comes with the next frame.
    const h = fakeHost();
    let ready = false;
    h.set.ready.then(() => (ready = true));
    await tick();
    expect(ready).toBe(true);
    h.declare({ family: "Lato" });
    expect(h.set.size).toBe(1);
    expect(h.set.status).toBe("loading");
    let again = false;
    h.set.ready.then(() => (again = true));
    await tick();
    expect(again).toBe(false);
    h.emit(0, "list");
    h.emit(1, "loaded");
    await tick();
    expect(again).toBe(true);
  });

  test("a sheet's CSS-connected face cannot be added or deleted, and a new sheet's faces appear", () => {
    const h = fakeHost([{ family: "Lato" }]);
    const [lato] = [...h.set];
    expect(() => h.set.add(lato)).toThrow(/CSS-connected/);
    expect(h.set.delete(lato)).toBe(false);
    h.declare({ family: "Roboto" });
    h.emit(0, "list");
    expect([...h.set].map((f: Any) => f.family)).toEqual(["Lato", "Roboto"]);
    expect([...h.set][0]).toBe(lato);
  });
});
