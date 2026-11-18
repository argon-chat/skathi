// Every frame of a WKWebView, in its own content world: the CSS cursor under the pointer, as the
// host cannot read it from WebKit. `auto` is resolved the way the engine does for the common cases.
(() => {
  "use strict";
  const handler = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.skCursor;
  if (!handler) return;
  const TEXT_INPUTS = /^(text|search|url|tel|email|password|number|date|datetime-local|month|time|week)$/;
  let last = "";

  function overText(x, y) {
    const range = document.caretRangeFromPoint && document.caretRangeFromPoint(x, y);
    const node = range && range.startContainer;
    if (!node || node.nodeType !== Node.TEXT_NODE || !node.data.trim()) return false;
    const whole = document.createRange();
    whole.selectNodeContents(node);
    for (const r of whole.getClientRects()) {
      if (x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) return true;
    }
    return false;
  }

  function cursorAt(e) {
    const el = e.target instanceof Element ? e.target : null;
    if (!el) return "default";
    const css = getComputedStyle(el).cursor || "auto";
    const named = css.split(",").pop().trim();
    if (named !== "auto") return named;
    if (el.closest("a[href], area[href]")) return "pointer";
    if (el instanceof HTMLTextAreaElement || el.isContentEditable) return "text";
    if (el instanceof HTMLInputElement && TEXT_INPUTS.test(el.type)) return "text";
    return overText(e.clientX, e.clientY) ? "text" : "default";
  }

  addEventListener(
    "mouseover",
    () => {
      last = "";
    },
    true,
  );
  addEventListener(
    "mousemove",
    (e) => {
      const cursor = cursorAt(e);
      if (cursor === last) return;
      last = cursor;
      handler.postMessage(cursor);
    },
    true,
  );
})();
