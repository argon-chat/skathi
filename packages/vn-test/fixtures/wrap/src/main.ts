// Beside them, 120px boxes each set one property, and `.mc-row`s ask whether a flex item's text
// can shrink below its longest token (`anywhere` lowers min-content, `break-word` does not).
import { createApp, h, type VNode } from "vue-native";
import "./wrap.css";

const A = "a".repeat(160);
const URL = `https://example.com/files/${"b".repeat(40)}/${"c".repeat(60)}?token=${"d".repeat(48)}`;
const HASH = "3f9a0c4e".repeat(16);
const WORDS = "abcdefghij abcdefghij abcdefghij";
const CJK = "日本語の文章です日本語の文章です";
const SHY = "super­cali­fragilistic­expiali­docious";

const message = (id: string, kids: (string | VNode)[]) =>
  h("div", { class: "row" }, [
    h("div", { class: "av" }),
    h("div", { id: `body-${id}`, class: "body" }, [
      h("div", { id: `rb-${id}`, class: "row-body" }, [
        h("div", { id: `bw-${id}`, class: "bubble-wrap" }, [
          h("div", { id: `b-${id}`, class: "bubble" }, [h("div", { id: `t-${id}`, class: "text" }, kids)]),
        ]),
      ]),
    ]),
  ]);

const url = (id: string) => h("a", { id, class: "url", href: URL }, URL);

const box = (id: string, cls: string, text: string) => h("div", { id, class: `box ${cls}` }, text);

createApp({
  render: () =>
    h("div", { class: "page" }, [
      h("div", { class: "stack" }, [
        h("div", { id: "wide", class: "col wide" }, [
          message("plain", [`look: ${A}`]),
          message("url", ["see ", url("u-url"), " ok"]),
          message("hash", [`sha256 ${HASH}`]),
        ]),
        h("div", { id: "narrow", class: "col narrow" }, [
          message("n-url", ["see ", url("u-n-url"), " ok"]),
          message("n-plain", [`look: ${A}`]),
        ]),
      ]),
      h("div", { class: "stack" }, [
        box("x-normal", "normal", A.slice(0, 40)),
        box("x-break-word", "break-word", A.slice(0, 40)),
        box("x-anywhere", "anywhere", A.slice(0, 40)),
        box("x-break-all", "break-all", WORDS),
        box("x-normal-words", "normal", WORDS),
        box("x-keep-all", "keep-all", CJK),
        box("x-cjk", "normal", CJK),
        box("x-legacy", "word-break-legacy", A.slice(0, 40)),
        box("x-nowrap", "nowrap", `${A.slice(0, 20)} ${A.slice(0, 20)}`),
        box("x-pre", "pre", A.slice(0, 40)),
        box("x-pre-wrap", "pre-wrap", A.slice(0, 40)),
        box("x-shy", "normal", SHY),
        box("x-shy-none", "normal no-hyphens", SHY),
        h("div", { id: "x-wbr", class: "box normal" }, ["supercali", h("wbr"), "fragilistic", h("wbr"), "expialidocious"]),
        h("div", { class: "mc-row" }, [h("div", { id: "mc-break-word", class: "break-word" }, A.slice(0, 60))]),
        h("div", { class: "mc-row" }, [h("div", { id: "mc-anywhere", class: "anywhere" }, A.slice(0, 60))]),
      ]),
      h("div", { class: "stack" }, [
        h("div", { class: "fc" }, [h("a", { id: "fc-a", class: "url", href: URL }, URL)]),
        h("div", { class: "fc" }, [h("div", { id: "fc-text", class: "text" }, ["see ", url("u-fc-text"), " ok"])]),
        h("div", { class: "fc" }, [
          h("div", { id: "fc-bubble", class: "bubble" }, [h("div", { class: "text" }, ["see ", url("u-fc-bubble"), " ok"])]),
        ]),
      ]),
    ]),
}).mount();

(globalThis as Record<string, unknown>).__VN_TEST_WRAP__ = {
  boxes() {
    const out: Record<string, { x: number; y: number; w: number; h: number; right: number }> = {};
    for (const el of Array.from(document.querySelectorAll("[id]"))) {
      const r = el.getBoundingClientRect();
      out[el.id] = { x: r.x, y: r.y, w: r.width, h: r.height, right: r.right };
    }
    return out;
  },
};
