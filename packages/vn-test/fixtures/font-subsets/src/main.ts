// A fixture for packages/vn-test/specs/font-subsets.spec.ts: one family as
// `unicode-range` subsets, drawn per character by the subset that covers it
// (docs/RUNTIME.md §5.15, CSS Fonts 4 §4.5 and §5.3).
//
// Pairs of probes, each one shrink-wrapped line at 20px: a `Mix` probe (the
// subset family) and its reference in the single-file `Ref *` families, which
// are the same files. The same text in the same face rasterizes the same, so
// the spec compares their pixels; a glyph drawn with another subset's
// outlines does not. `#mixed` and `#stack` mix scripts in one run; their
// pieces are the references `#mixed-lat`, `#mixed-cyr`, `#mixed-num` and
// `#stack-lat`, `#stack-pua`.
//
// `__VN_TEST_SUBSETS__.faces()` lists `document.fonts` as `family:status:range`.
// Render functions rather than an SFC, as fixtures/layout says why.
import { createApp, h } from "vue-native";
import "./fonts.css";

type Any = any; // eslint-disable-line @typescript-eslint/no-explicit-any
const fonts = (document as Any).fonts;

(globalThis as unknown as { __VN_TEST_SUBSETS__: unknown }).__VN_TEST_SUBSETS__ = {
  faces: () => [...fonts].map((f: Any) => `${f.family}:${f.status}:${f.unicodeRange.split(",")[0]}`),
};

const PUA = "";

const probe = (id: string, cls: string, text: string) =>
  h("div", { class: "row" }, [h("div", { id, class: `probe ${cls}` }, text)]);

createApp({
  render: () =>
    h("div", { id: "root" }, [
      // Latin only, in the subset family and in its file.
      probe("lat", "mix", "EXOBIO OVERLAY LAB"),
      probe("lat-ref", "lat", "EXOBIO OVERLAY LAB"),
      // The fallback, for scale: not Manrope.
      probe("plain", "plain", "EXOBIO OVERLAY LAB"),
      // Bold Latin and digits: the variable font's 700.
      probe("bold", "mix bold", "FSD 1–0 2026"),
      probe("bold-ref", "lat bold", "FSD 1–0 2026"),
      // Cyrillic only (no space: the latin subset has that).
      probe("cyr", "mix", "Жукбиолог"),
      probe("cyr-ref", "cyr", "Жукбиолог"),
      // One run of three scripts' worth of subsets, and its pieces.
      probe("mixed", "mix", "EXOBIO Жук 1–0"),
      probe("mixed-lat", "lat", "EXOBIO "),
      probe("mixed-cyr", "cyr", "Жук"),
      probe("mixed-num", "lat", " 1–0"),
      // A PUA-only family leads the stack: its glyphs, then `Mix` for the rest.
      probe("stack", "pua", `LAB ${PUA}`),
      probe("stack-lat", "lat", "LAB "),
      probe("stack-pua", "refpua", PUA),
    ]),
}).mount();
