// A browser runs a checkpoint after every event it dispatches, so a handler's promise jobs (Vue's
// patch among them) run before the next event's handlers.
import { createApp, h } from "@skathi-native/runtime";

const dom = document as unknown as SkDomDocument;

const log: string[] = [];
(globalThis as { __SK_TEST_ORDER__?: unknown }).__SK_TEST_ORDER__ = {
  log,
  clear() {
    log.length = 0;
  },
};

function rec(name: string, cancel = false, stop = false) {
  return (e: Event) => {
    if (cancel) e.preventDefault();
    if (stop) e.stopPropagation();
    log.push(name);
    queueMicrotask(() => log.push(`${name}:mt`));
  };
}

// The pad's copy is cancelled with nothing in its store, so the system clipboard is never written.
// Added before mount(), as module-scope code does.
dom.addEventListener("copy", rec("copy", true));

const box = { width: "120px", height: "40px", background: "#2a3440", margin: "4px" };
const scroller = { height: "60px", width: "140px", overflow: "auto", background: "#202020" };

createApp({
  setup() {
    return () =>
      h("div", { id: "root", style: { padding: "8px" } }, [
        h("div", { style: { display: "flex" } }, [
          h("div", { id: "h1", style: box, onMouseleave: rec("leave-h1") }, "h1"),
          h("div", { id: "h2", style: box, onMouseenter: rec("enter-h2") }, "h2"),
        ]),
        h("div", { style: { display: "flex" } }, [
          h("button", { id: "f1", onBlur: rec("blur-f1") }, "f1"),
          h("button", { id: "f2", onFocus: rec("focus-f2") }, "f2"),
        ]),
        h(
          "div",
          {
            id: "pad",
            tabindex: 0,
            style: box,
            onKeydown: rec("keydown"),
            onKeyup: rec("keyup"),
            onContextmenu: rec("contextmenu", true),
          },
          "pad",
        ),
        h("textarea", { id: "ta", rows: 2, onKeydown: rec("keydown-ta"), onCopy: rec("copy-ta", true, true) }),
        // 6. a keyboard click: Enter clicks on the press (before its keyup), Space on the release
        // (after its keyup)
        h("div", { style: { display: "flex" } }, [
          h("button", { id: "btn", onKeyup: rec("keyup-btn"), onClick: rec("click-btn") }, "btn"),
          h(
            "button",
            {
              id: "btn-x",
              onKeydown: (e: KeyboardEvent) => e.key === "Enter" && rec("keydown-x", true)(e),
              onKeyup: (e: KeyboardEvent) => e.key === " " && rec("keyup-x", true)(e),
              onClick: rec("click-x"),
            },
            "btn-x",
          ),
          h(
            "div",
            {
              id: "role-btn",
              role: "button",
              tabindex: 0,
              onKeydown: (e: KeyboardEvent) => (e.key === "Enter" || e.key === " ") && rec("act-role")(e),
              onClick: rec("click-role"),
            },
            "role",
          ),
        ]),
        h("div", { style: { display: "flex", gap: "4px" } }, [
          h(
            "div",
            {
              id: "scroller",
              style: scroller,
              onScroll: rec("scroll"),
              onWheel: rec("wheel"),
            },
            [h("div", { style: { height: "400px" } }, "tall")],
          ),
          h(
            "div",
            {
              id: "scroller-x",
              style: scroller,
              onScroll: rec("scroll-x"),
              onWheel: rec("wheel-x", true),
            },
            [h("div", { style: { height: "400px" } }, "tall")],
          ),
          h("div", { id: "guard", onWheel: rec("wheel-guard", true) }, [
            h("div", { id: "scroller-in", style: scroller, onScroll: rec("scroll-in") }, [
              h("div", { style: { height: "400px" } }, "tall"),
            ]),
          ]),
        ]),
      ]);
  },
}).mount();
