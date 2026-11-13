import { createApp, h, nextTick, ref } from "vue-native";
import "./fonts.css";

const dom = document as unknown as VnDomDocument;

const TEXT = "WWWWiiii";
const fonts = dom.fonts;

const events: string[] = [];
const results: Record<string, unknown> = {};
const showLate = ref(false);
const face = (family: string) => [...fonts].find((f) => f.family === family);
for (const type of ["loading", "loadingdone", "loadingerror"]) {
  fonts.addEventListener(type, (e: VnEvent) =>
    events.push(`${type}:${[...(e as VnFontFaceSetEvent).fontfaces].map((f) => f.family).join(",")}`),
  );
}

const scenarios: Record<string, () => Promise<unknown>> = {
  async script() {
    const buf = await (await fetch!("/src/fonts/PressStart2P-Regular.ttf")).arrayBuffer();
    const face = new FontFace("VN Script", buf);
    const first = face.status;
    await face.loaded;
    fonts.add(face);
    return {
      first,
      status: face.status,
      has: fonts.has(face),
      size: fonts.size,
      check: fonts.check("16px 'VN Script'"),
    };
  },
  async check() {
    const tengwar = await fonts.load("16px 'VN Tengwar'", "");
    return {
      pixel: fonts.check("16px 'VN Pixel'"),
      tengwarPua: fonts.check("16px 'VN Tengwar'", ""),
      loaded: tengwar.map((f) => `${f.family}:${f.status}:${f.unicodeRange}`),
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
  async late() {
    const late = face("VN Late")!;
    const before = late.status;
    showLate.value = true;
    await nextTick();
    const fallback = dom.getElementById("late")!.getBoundingClientRect().width;
    await late.loaded;
    return { before, fallback, status: late.status, unused: face("VN Unused")!.status };
  },
  async missing() {
    const face = new FontFace("VN Missing", "url(/src/fonts/nope.ttf)");
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

// Right after mount, which installs the sheet: what the set and `ready` say before any text has
// asked for a face.
results.boot = {
  status: fonts.status,
  size: fonts.size,
  faces: [...fonts].map((f) => `${f.family}:${f.status}`),
};
fonts.ready.then((s) => {
  results.ready = { same: s === fonts, status: s.status, faces: [...s].map((f) => `${f.family}:${f.status}`) };
});
