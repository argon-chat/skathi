// The microtask checkpoint between two events that ONE platform event produces
// (m26 client sweep #8, docs/RUNTIME.md §5.8).
//
// A browser runs a checkpoint after every event it dispatches, so a handler's
// promise jobs (Vue's patch among them) run before the next event's handlers.
// Here a dispatch made under a gpui listener leaves its checkpoint owed and
// settles it after gpui's dispatch, unless another dispatch starts first, which
// then settles it before it runs. Every handler below logs its event and queues
// a microtask that logs `<event>:mt`, so a lost checkpoint shows up as
// `a, b, a:mt, b:mt` instead of `a, a:mt, b, b:mt`.
import { createApp, h } from "vue-native";

const log: string[] = [];
(globalThis as { __VN_TEST_ORDER__?: unknown }).__VN_TEST_ORDER__ = {
  log,
  clear() {
    log.length = 0;
  },
};

/**
 * A handler that logs `name`, and `name:mt` from its own microtask. `stop`
 * keeps the event off the document's listener: two listeners of ONE dispatch
 * share its checkpoint here (a browser runs one after each), which is not
 * what this fixture is about.
 */
function rec(name: string, cancel = false, stop = false) {
  return (e: Event) => {
    if (cancel) e.preventDefault();
    if (stop) e.stopPropagation();
    log.push(name);
    queueMicrotask(() => log.push(`${name}:mt`));
  };
}

// The pad's copy is cancelled with nothing in its store, so the system
// clipboard is never written. Added before mount(), as module-scope code does.
document.addEventListener("copy", rec("copy", true));

const box = { width: "120px", height: "40px", background: "#2a3440", margin: "4px" };
const scroller = { height: "60px", width: "140px", overflow: "auto", background: "#202020" };

createApp({
  setup() {
    return () =>
      h("div", { id: "root", style: { padding: "8px" } }, [
        // 1. one move: mouseleave on #h1, mouseenter on #h2
        h("div", { style: { display: "flex" } }, [
          h("div", { id: "h1", style: box, onMouseleave: rec("leave-h1") }, "h1"),
          h("div", { id: "h2", style: box, onMouseenter: rec("enter-h2") }, "h2"),
        ]),
        // 2. one focus move: blur on #f1, focus on #f2
        h("div", { style: { display: "flex" } }, [
          h("button", { id: "f1", onBlur: rec("blur-f1") }, "f1"),
          h("button", { id: "f2", onFocus: rec("focus-f2") }, "f2"),
        ]),
        // 3 and 5. a key on a focusable box: keydown then the root's copy
        // (Ctrl+C); keyup then the Menu key's contextmenu
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
        // 4. a text field: its keydown, then its own copy action
        h("textarea", { id: "ta", rows: 2, onKeydown: rec("keydown-ta"), onCopy: rec("copy-ta", true, true) }),
        // 6. a keyboard click: Enter clicks on the press (before its keyup),
        // Space on the release (after its keyup)
        h("div", { style: { display: "flex" } }, [
          h("button", { id: "btn", onKeyup: rec("keyup-btn"), onClick: rec("click-btn") }, "btn"),
          // …and a cancelled key clicks nothing: Enter's keydown, Space's keyup
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
        ]),
        // 7. one wheel: the wheel, then the container's scroll
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
          // A cancelled wheel scrolls nothing: on the container itself…
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
          // …or on a box around it.
          h("div", { id: "guard", onWheel: rec("wheel-guard", true) }, [
            h("div", { id: "scroller-in", style: scroller, onScroll: rec("scroll-in") }, [
              h("div", { style: { height: "400px" } }, "tall"),
            ]),
          ]),
        ]),
      ]);
  },
}).mount();
