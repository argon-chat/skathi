// The same text in the same face rasterizes the same, so the spec compares their pixels; a glyph
// drawn with another subset's outlines does not.
import { createApp, h } from "@skathi-native/runtime";
import "./fonts.css";

const fonts = (document as unknown as SkDomDocument).fonts;

(globalThis as unknown as { __SK_TEST_SUBSETS__: unknown }).__SK_TEST_SUBSETS__ = {
  faces: () => [...fonts].map((f) => `${f.family}:${f.status}:${f.unicodeRange.split(",")[0]}`),
};

const PUA = "";

const probe = (id: string, cls: string, text: string) =>
  h("div", { class: "row" }, [h("div", { id, class: `probe ${cls}` }, text)]);

createApp({
  render: () =>
    h("div", { id: "root" }, [
      probe("lat", "mix", "EXOBIO OVERLAY LAB"),
      probe("lat-ref", "lat", "EXOBIO OVERLAY LAB"),
      probe("plain", "plain", "EXOBIO OVERLAY LAB"),
      probe("bold", "mix bold", "FSD 1–0 2026"),
      probe("bold-ref", "lat bold", "FSD 1–0 2026"),
      probe("cyr", "mix", "Жукбиолог"),
      probe("cyr-ref", "cyr", "Жукбиолог"),
      probe("mixed", "mix", "EXOBIO Жук 1–0"),
      probe("mixed-lat", "lat", "EXOBIO "),
      probe("mixed-cyr", "cyr", "Жук"),
      probe("mixed-num", "lat", " 1–0"),
      probe("stack", "pua", `LAB ${PUA}`),
      probe("stack-lat", "lat", "LAB "),
      probe("stack-pua", "refpua", PUA),
    ]),
}).mount();
