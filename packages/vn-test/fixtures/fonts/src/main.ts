// A fixture for packages/vn-test/specs/fonts.spec.ts: web fonts (m26 client
// sweep #29, docs/RUNTIME.md §5.15).
//
// Probes, each "WWWWiiii" at 16px in its own shrink-wrapped box, so a probe's
// width says which font drew it (Press Start 2P: 128px exactly): `#pixel` (the
// sheet's face), `#stack` (behind a PUA-only family), `#inline` (the face named
// by an inline style), `#script` (a face a script adds, by
// `__VN_TEST_FONTS__.run("script")`), `#plain` (serif, never a web font) and
// `#late` (rendered by the `late` scenario: its face loads only then). An
// empty `.unused` box names `VN Unused` and draws no text with it.
//
// `__VN_TEST_FONTS__.events` records the set's loading events; `run(name)`
// settles into `results[name]`, because a promise does not survive the eval
// round trip. Render functions rather than an SFC, as fixtures/layout says why.
import { createApp, h, nextTick, ref } from "vue-native";
import "./fonts.css";

const TEXT = "WWWWiiii";
type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any
const fonts = (document as Any).fonts;

const events: string[] = [];
const results: Record<string, unknown> = {};
const showLate = ref(false);
const face = (family: string) => [...fonts].find((f: Any) => f.family === family);
for (const type of ["loading", "loadingdone", "loadingerror"]) {
  fonts.addEventListener(type, (e: Any) => events.push(`${type}:${[...e.fontfaces].map((f: Any) => f.family).join(",")}`));
}

const scenarios: Record<string, () => Promise<unknown>> = {
  /** A FontFace from an ArrayBuffer, added to the set: `#script` takes it. */
  async script() {
    const buf = await (await fetch("/src/fonts/PressStart2P-Regular.ttf")).arrayBuffer();
    const face = new (globalThis as Any).FontFace("VN Script", buf);
    const first = face.status;
    await face.loaded;
    fonts.add(face);
    return { first, status: face.status, has: fonts.has(face), size: fonts.size, check: fonts.check("16px 'VN Script'") };
  },
  /** check() and load() on the sheet's faces. */
  async check() {
    const tengwar = await fonts.load("16px 'VN Tengwar'", "");
    return {
      pixel: fonts.check("16px 'VN Pixel'"),
      tengwarPua: fonts.check("16px 'VN Tengwar'", ""),
      loaded: tengwar.map((f: Any) => `${f.family}:${f.status}:${f.unicodeRange}`),
      system: fonts.check("16px serif"),
      bad: (() => {
        try {
          fonts.check("VN Pixel");
          return "no throw";
        } catch (e) {
          return (e as Error).name;
        }
      })(),
    };
  },
  /** Lazy loading: `VN Late` waits, unloaded, until `#late` draws text with
   *  it; that frame draws the fallback, and the face then loads. */
  async late() {
    const late = face("VN Late");
    const before = late.status;
    showLate.value = true;
    await nextTick();
    const fallback = document.getElementById("late")!.getBoundingClientRect().width;
    await late.loaded;
    return { before, fallback, status: late.status, unused: face("VN Unused").status };
  },
  /** A face whose file is not there: `error`, and `loaded` rejects. */
  async missing() {
    const face = new (globalThis as Any).FontFace("VN Missing", "url(/src/fonts/nope.ttf)");
    fonts.add(face);
    try {
      await face.load();
      return "loaded?";
    } catch (e) {
      return { name: (e as Error).name, status: face.status };
    }
  },
};
const run = (name: string) => {
  results[name] = "pending";
  scenarios[name]!().then(
    (v) => (results[name] = v),
    (e) => (results[name] = `threw ${(e as Error).name}: ${(e as Error).message}`),
  );
  return "started";
};
(globalThis as unknown as { __VN_TEST_FONTS__: unknown }).__VN_TEST_FONTS__ = { events, results, run };

const probe = (id: string, style?: Record<string, string>) =>
  h("div", { class: "row" }, [h("div", { id, class: "probe", style }, TEXT)]);

createApp({
  render: () =>
    h("div", { id: "root" }, [
      probe("pixel"),
      probe("stack"),
      probe("inline", { fontFamily: "'VN Pixel', serif" }),
      probe("script", { fontFamily: "'VN Script', serif" }),
      probe("plain", { fontFamily: "serif" }),
      showLate.value ? probe("late") : null,
      h("div", { class: "unused" }),
    ]),
}).mount();

// Right after mount, which installs the sheet: what the set and `ready` say
// before any text has asked for a face. (Before mount there is no sheet: a
// vue-native app's CSS joins the stylesheet on mount.)
results.boot = {
  status: fonts.status,
  size: fonts.size,
  faces: [...fonts].map((f: Any) => `${f.family}:${f.status}`),
};
fonts.ready.then((s: Any) => {
  results.ready = { same: s === fonts, status: s.status, faces: [...s].map((f: Any) => `${f.family}:${f.status}`) };
});
