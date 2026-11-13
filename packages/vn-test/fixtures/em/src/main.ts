// `em` in `font-size` is of the parent's computed size, and everywhere else of the element's own.
import { createApp, h } from "vue-native";
import "./em.css";

document.documentElement.style.fontSize = "14px";

createApp({
  render: () =>
    h("div", { class: "page" }, [
      h("div", { id: "chip", class: "chip" }, [h("i", { id: "chip-a" }), h("i", { id: "chip-b" })]),
      h("div", { id: "l1", class: "l1" }, [
        h("div", { id: "l2", class: "l2" }, [h("div", { id: "l3", class: "l3" })]),
      ]),
      h("div", { id: "rem", class: "rem" }),
      h("div", { class: "lh-num" }, [h("div", { id: "lh-num" }, "number")]),
      h("div", { class: "lh-em" }, [h("div", { id: "lh-em" }, "em")]),
      h("div", { id: "hov", class: "hov" }, [h("i")]),
      h("div", { id: "ch", class: "ch" }),
    ]),
}).mount();

(globalThis as Record<string, unknown>).__VN_TEST_EM__ = {
  boxes() {
    const out: Record<string, { x: number; y: number; w: number; h: number; fontSize: string }> = {};
    for (const el of Array.from(document.querySelectorAll("[id]"))) {
      const r = el.getBoundingClientRect();
      out[el.id] = { x: r.x, y: r.y, w: r.width, h: r.height, fontSize: getComputedStyle(el).fontSize };
    }
    return out;
  },
};
