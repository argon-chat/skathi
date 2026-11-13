import { createApp, h, ref } from "vue-native";

const dom = document as unknown as VnDomDocument;

const name = (e: unknown): string => {
  if (!e) return String(e);
  if (e === dom.body) return "BODY";
  const el = e as Element;
  return el.id || el.tagName || String(e);
};

const log: string[] = [];
const types = [
  "pointerdown",
  "mousedown",
  "pointerup",
  "mouseup",
  "click",
  "contextmenu",
  "focus",
  "blur",
  "focusin",
  "focusout",
];
for (const t of types) {
  dom.addEventListener(
    t,
    (e: Event) => {
      if (t === "contextmenu") e.preventDefault();
      const rel = t.startsWith("focus") || t === "blur" ? ` rel=${name((e as FocusEvent).relatedTarget)}` : "";
      log.push(`${t}:${name(e.target)}${rel} active=${name(dom.activeElement)}`);
    },
    true,
  );
}

const showGone = ref(true);
const showGone2 = ref(true);
const byId = (id: string) => dom.getElementById(id) as HTMLElement;

(globalThis as Record<string, unknown>).__VN_TEST_PRESS__ = {
  log,
  reset() {
    byId("field").focus();
    log.length = 0;
    return name(dom.activeElement);
  },
  active: () => name(dom.activeElement),
  restore() {
    showGone.value = true;
    showGone2.value = true;
  },
  programmatic() {
    const out: Record<string, string> = {};
    for (const id of ["tag", "keys", "minus", "plain", "nolink", "link", "ce", "sum", "role-btn", "lbl"]) {
      byId("field").focus();
      byId(id).focus();
      out[id] = name(dom.activeElement);
    }
    return JSON.stringify(out);
  },
};

const box = { padding: "3px 6px", background: "#e8e8f0", fontSize: "12px" };
const removeLater = (shown: typeof showGone, id: string) => () =>
  queueMicrotask(() => {
    shown.value = false;
    log.push(`removed:${id}`);
  });

const App = {
  render: () =>
    h(
      "div",
      {
        id: "page",
        style: {
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: "6px",
          padding: "8px",
          background: "#fff",
          alignContent: "start",
        },
      },
      [
        h("input", { id: "field", style: { width: "200px" } }),
        h("span", { id: "tag", style: box, onFocus: () => log.push("tag-listener") }, "tag with focus listener"),
        h("div", { id: "keys", style: box, onKeydown: () => log.push("keys-listener") }, "div with keydown listener"),
        h("div", { id: "minus", tabindex: -1, style: box }, "tabindex -1"),
        h("div", { id: "outer", tabindex: 0, style: box }, [
          "focusable outer ",
          h("span", { id: "inner" }, "inner span"),
        ]),
        h("button", { id: "btn" }, "button"),
        h("div", { id: "plain", style: box }, "plain div"),
        h("div", { id: "menu-row", style: box, onContextmenu: () => log.push("row-menu") }, "row with contextmenu"),
        h("button", { id: "prevent", onMousedown: (e: MouseEvent) => e.preventDefault() }, "mousedown cancelled"),
        h("button", { id: "steal", onMousedown: () => byId("field").focus() }, "mousedown focuses the field"),
        h("button", { id: "steal2", onMousedown: () => byId("btn").focus() }, "mousedown focuses btn"),
        showGone.value
          ? h("button", { id: "gone", onMousedown: removeLater(showGone, "gone") }, "removed by a microtask")
          : h("div", { style: box }, "-"),
        h("div", { id: "wrapper", tabindex: 0, style: box }, [
          "wrapper ",
          showGone2.value
            ? h("button", { id: "gone2", onMousedown: removeLater(showGone2, "gone2") }, "removed inside")
            : null,
        ]),
        h("a", { id: "link", href: "#x", style: box }, "link"),
        h("a", { id: "nolink", style: box }, "anchor without href"),
        h("div", { id: "ce", contenteditable: "true", style: box }, "editable"),
        h("details", { style: box }, [h("summary", { id: "sum" }, "summary"), "body"]),
        h("div", { id: "role-btn", role: "button", style: box }, "role=button, no tabindex"),
        h("label", { id: "lbl", for: "field", style: box }, "label for field"),
      ],
    ),
};

createApp(App).mount("#app");
