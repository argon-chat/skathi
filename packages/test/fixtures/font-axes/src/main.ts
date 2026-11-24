import { createApp, h } from "@skathi-native/runtime";
import "./fonts.css";

const fonts = document.fonts;

(globalThis as unknown as { __SK_TEST_AXES__: unknown }).__SK_TEST_AXES__ = {
  loaded: () => [...fonts].filter((f) => f.status === "loaded").length,
};

const TEXT = "Hamburgefonstiv";
const probe = (id: string, cls: string) => h("div", { id, class: `probe ${cls}` }, TEXT);

createApp({
  render: () =>
    h("div", { id: "root" }, [
      probe("base", ""),
      probe("condensed", "condensed"),
      probe("expanded", "expanded"),
      probe("width", "width"),
      probe("settings", "settings"),
      h("div", { class: "settings" }, [h("span", { id: "inherited", class: "probe" }, TEXT)]),
      probe("override", "override"),
      probe("oblique", "oblique"),
      probe("italic", "italic"),
      probe("grad", "grad"),
      probe("narrow", "narrow"),
      probe("narrow-ref", "narrow-ref"),
    ]),
}).mount();
