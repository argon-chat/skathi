// The DOM surface reka-ui (and @floating-ui/dom) expects, implemented in APP
// SPACE so this trial can measure "what would we unlock if the runtime had it"
// without editing packages/vue-native/src/runtime.js — which another track owns
// this round. Every function here is a PROTOTYPE of a runtime patch; the exact
// diffs are in research/reports/m10/shadcn.md.
//
// Rules this file follows, and any real runtime version must too:
//   * ADD ONLY. Nothing here replaces or wraps an existing behaviour, so
//     turning it off (`globalThis.__VN_NO_DOM_SHIM__ = true`) restores the
//     stock host exactly.
//   * NEVER LIE ABOUT LAYOUT. Everything geometric answers zero, because the
//     host has no way to report gpui's computed bounds to JS yet. Consumers get
//     a degraded-but-consistent answer instead of an exception. Each such stub
//     logs ONE advisory naming the feature, so a run log shows what a component
//     actually reached for.
//   * PER-NODE STATE lives on the node (lazily installed own properties), never
//     on the shared prototype.
//
// WHY THIS IS NOT "MODIFYING THE COMPONENTS": nothing under src/components/ui
// changes. This is the ENVIRONMENT growing the surface a browser already has.
// The compat question is "do browser components run unmodified", and a missing
// `Element.prototype.closest` is a hole in our browser, not in shadcn-vue.

if (!globalThis.__VN_NO_DOM_SHIM__ && !globalThis.__VN_DOM_SHIM__) {
  globalThis.__VN_DOM_SHIM__ = true;
  install();
}

function install() {
  const seen = new Set();
  /** One line per missing capability that was actually reached for. */
  const note = (key, msg) => {
    if (seen.has(key)) return;
    seen.add(key);
    console.warn(`[dom-shim] ${msg}`);
  };

  const ElementProto = typeof Element === "function" ? Element.prototype : null;
  if (!ElementProto) {
    console.warn("[dom-shim] no Element class in this engine — nothing installed");
    return;
  }

  const def = (target, name, value) => {
    if (name in target) return false;
    Object.defineProperty(target, name, { value, configurable: true, writable: true });
    return true;
  };
  const defGet = (target, name, get) => {
    if (name in target) return false;
    Object.defineProperty(target, name, { get, configurable: true });
    return true;
  };

  // --- interning, so the style shim can write through to the real host -------
  const interned = new Map();
  const I = (s) => {
    let v = interned.get(s);
    if (v === undefined) interned.set(s, (v = __vn_intern(s)));
    return v;
  };
  const hyphenate = (s) => s.replace(/\B([A-Z])/g, "-$1").toLowerCase();

  // --- 1. the attribute channel ---------------------------------------------
  // patchProp() pushes props straight through `__vn_set_prop` and keeps no copy
  // on the mirror, so there is nothing here to read back: every getter answers
  // "absent". Enough to stop the TypeErrors; NOT enough for reka-ui's
  // roving-focus, which navigates by reading `data-*` off candidate nodes.
  // The real fix is one line in patchProp — see the report.
  def(ElementProto, "getAttribute", function (name) {
    note("getAttribute", `getAttribute(${JSON.stringify(name)}) always returns null — the mirror keeps no attribute map`);
    return null;
  });
  def(ElementProto, "hasAttribute", function () {
    note("hasAttribute", "hasAttribute() always returns false — the mirror keeps no attribute map");
    return false;
  });
  def(ElementProto, "getAttributeNames", function () {
    return [];
  });
  def(ElementProto, "setAttribute", function (name, value) {
    // This direction CAN be honest: it is exactly what patchProp does.
    __vn_set_prop(this.id, I(String(name)), String(value));
  });
  def(ElementProto, "removeAttribute", function (name) {
    __vn_set_prop(this.id, I(String(name)), undefined);
  });
  defGet(ElementProto, "dataset", function () {
    const d = {};
    Object.defineProperty(this, "dataset", { value: d, configurable: true });
    return d;
  });

  // --- 2. tree queries -------------------------------------------------------
  // `contains` is exact (the mirror IS the tree). `closest`/`matches`/
  // `querySelector*` need a selector engine, which lives in Rust; they answer
  // "no match" rather than throwing.
  def(ElementProto, "contains", function (other) {
    for (let n = other; n; n = n.parentNode) if (n === this) return true;
    return false;
  });
  def(ElementProto, "closest", function (selector) {
    note("closest", `closest(${JSON.stringify(selector)}) always returns null — no JS-side selector matcher`);
    return null;
  });
  def(ElementProto, "matches", function () {
    note("matches", "matches() always returns false — no JS-side selector matcher");
    return false;
  });
  def(ElementProto, "querySelector", function () {
    note("querySelector", "element.querySelector() always returns null — no JS-side selector matcher");
    return null;
  });
  def(ElementProto, "querySelectorAll", function () {
    note("querySelectorAll", "element.querySelectorAll() always returns [] — no JS-side selector matcher");
    return [];
  });
  defGet(ElementProto, "firstElementChild", function () {
    return (this.children || []).find((c) => c.kind === 1) ?? null;
  });
  defGet(ElementProto, "nextElementSibling", function () {
    const p = this.parentNode;
    if (!p) return null;
    const i = p.children.indexOf(this);
    for (let j = i + 1; j < p.children.length; j++) if (p.children[j].kind === 1) return p.children[j];
    return null;
  });
  defGet(ElementProto, "ownerDocument", () => globalThis.document);
  defGet(ElementProto, "nodeType", function () {
    return this.kind === 2 ? 3 : this.kind === 3 ? 8 : 1;
  });
  defGet(ElementProto, "nodeName", function () {
    return this.tagName ?? "#text";
  });
  defGet(ElementProto, "textContent", function () {
    let out = "";
    const walk = (n) => {
      if (n.kind === 2) out += n.text ?? "";
      for (const c of n.children || []) walk(c);
    };
    walk(this);
    return out;
  });

  // --- 3. geometry: honest zeros --------------------------------------------
  // gpui computes layout in Rust and never publishes bounds to JS. Anything
  // that positions itself from measurements (floating-ui, CollapsibleContent's
  // height animation, Select's item-aligned positioning) gets zeros and lands
  // at the origin. This is the single biggest missing HOST capability.
  const ZERO_RECT = () => ({
    x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0,
    toJSON() { return this; },
  });
  def(ElementProto, "getBoundingClientRect", function () {
    note("getBoundingClientRect", "getBoundingClientRect() returns an all-zero rect — the host does not publish gpui layout bounds to JS");
    return ZERO_RECT();
  });
  def(ElementProto, "getClientRects", function () {
    return [];
  });
  for (const p of ["offsetWidth", "offsetHeight", "offsetTop", "offsetLeft", "clientWidth", "clientHeight", "scrollWidth", "scrollHeight"]) {
    defGet(ElementProto, p, function () {
      note("offsets", "offset*/client*/scroll* dimensions read 0 — the host does not publish gpui layout bounds to JS");
      return 0;
    });
  }
  defGet(ElementProto, "offsetParent", () => null);

  // --- 4. inline style -------------------------------------------------------
  // Writes go through to the real host (same call patchProp makes for `:style`),
  // so this is genuinely functional — except for custom properties, which
  // vn-css has nowhere to store. Reads return only what was written here.
  defGet(ElementProto, "style", function () {
    const node = this;
    const own = Object.create(null);
    const write = (name, value) => {
      const k = String(name);
      if (k.startsWith("--")) {
        note("customProps", `style.setProperty(${JSON.stringify(k)}) — custom properties are not stored by vn-css; the write is inert`);
        own[k] = value;
        return;
      }
      own[k] = value;
      __vn_set_style(node.id, I(hyphenate(k)), value == null ? "" : String(value));
    };
    const style = new Proxy(
      {
        setProperty: (name, value) => write(name, value),
        removeProperty: (name) => write(name, ""),
        getPropertyValue: (name) => own[String(name)] ?? "",
        get cssText() { return ""; },
        set cssText(_v) { note("cssText", "style.cssText assignment is ignored"); },
      },
      {
        get: (t, k) => (k in t ? t[k] : (own[k] ?? "")),
        set: (t, k, v) => {
          if (k in t) return true;
          write(k, v);
          return true;
        },
      },
    );
    Object.defineProperty(this, "style", { value: style, configurable: true });
    return style;
  });

  // --- 5. focus --------------------------------------------------------------
  // gpui owns focus; the renderer only exposes it for <input>. A no-op keeps
  // focus traps and roving focus from throwing, at the cost of the focus never
  // actually moving — which is why keyboard navigation stays a documented gap.
  def(ElementProto, "focus", function () {
    note("focus", "element.focus() is a no-op — gpui owns focus and the host exposes no programmatic focus call");
  });
  def(ElementProto, "blur", function () {});
  def(ElementProto, "scrollIntoView", function () {
    note("scrollIntoView", "element.scrollIntoView() is a no-op");
  });
  def(ElementProto, "insertAdjacentElement", function (_where, el) {
    note("insertAdjacentElement", "insertAdjacentElement() is a no-op — sentinel nodes injected outside Vue are not rendered");
    return el;
  });
  def(ElementProto, "append", function () {});
  def(ElementProto, "remove", function () {
    note("remove", "element.remove() is a no-op outside the Vue renderer");
  });
  defGet(ElementProto, "isConnected", function () {
    return !!this.parentNode;
  });

  // --- 6. document surface ---------------------------------------------------
  const doc = globalThis.document;
  if (doc) {
    def(doc, "querySelectorAll", () => {
      note("doc.querySelectorAll", "document.querySelectorAll() always returns [] — no JS-side selector matcher");
      return [];
    });
    def(doc, "createTextNode", (text) => ({ nodeType: 3, textContent: String(text) }));
    def(doc, "createDocumentFragment", () => new globalThis.DocumentFragment());
    def(doc, "createTreeWalker", () => ({ nextNode: () => null, currentNode: null }));
    defGet(doc, "defaultView", () => globalThis);
    defGet(doc, "activeElement", () => null);
    // `document.body`/`documentElement` are plain objects in redjs: three
    // libraries (aria-hidden, reka's scroll lock, floating-ui) reach straight
    // for `.style` on them.
    for (const el of [doc.body, doc.documentElement, doc.head]) {
      if (!el) continue;
      if (!el.style) el.style = {};
      def(el, "setProperty", function () {});
      def(el, "querySelectorAll", () => []);
      def(el, "getAttribute", () => null);
      def(el, "setAttribute", () => {});
      def(el, "hasAttribute", () => false);
      def(el, "contains", () => false);
      def(el, "insertAdjacentElement", (_w, e) => e);
      def(el, "appendChild", (e) => e);
      def(el, "removeChild", (e) => e);
      def(el, "getBoundingClientRect", () => ZERO_RECT());
      defGet(el, "ownerDocument", () => doc);
      defGet(el, "parentNode", () => null);
      if (!("clientWidth" in el)) el.clientWidth = 0;
      if (!("clientHeight" in el)) el.clientHeight = 0;
      if (!("scrollLeft" in el)) el.scrollLeft = 0;
      if (!("scrollTop" in el)) el.scrollTop = 0;
      if (!("children" in el)) el.children = [];
    }
  }

  // --- 7. missing constructors ----------------------------------------------
  if (typeof globalThis.DocumentFragment !== "function") {
    globalThis.DocumentFragment = class DocumentFragment {
      constructor() {
        this.nodeType = 11;
        this.children = [];
        this.childNodes = [];
      }
      appendChild(n) { return n; }
      removeChild(n) { return n; }
      querySelector() { return null; }
      querySelectorAll() { return []; }
    };
    note("DocumentFragment", "DocumentFragment is shimmed as an inert object — a Teleport target that is one cannot render");
  }
  // Tag-specific element classes. redjs only ships Element/HTMLElement/
  // SVGElement, and reka-ui brands nodes with `instanceof HTMLInputElement`
  // (focus scope's isSelectableInput) — a MISSING class is a ReferenceError,
  // which is strictly worse than a false `instanceof`. Our mirrors are all
  // plain HTMLElement, so every check below answers false; the point is only
  // that it answers.
  for (const name of [
    "HTMLInputElement", "HTMLTextAreaElement", "HTMLSelectElement",
    "HTMLButtonElement", "HTMLAnchorElement", "HTMLFormElement",
    "HTMLLabelElement", "HTMLDivElement", "HTMLSpanElement", "HTMLIFrameElement",
  ]) {
    if (typeof globalThis[name] === "undefined") {
      globalThis[name] = class extends HTMLElement {};
      Object.defineProperty(globalThis[name], "name", { value: name });
    }
  }

  // Focus traps walk the tree with document.createTreeWalker(root, NodeFilter…);
  // the missing GLOBAL is what throws first, before the walker is even built.
  if (typeof globalThis.NodeFilter === "undefined") {
    globalThis.NodeFilter = {
      SHOW_ALL: 0xffffffff, SHOW_ELEMENT: 1, SHOW_TEXT: 4, SHOW_COMMENT: 128,
      FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3,
    };
  }
  if (typeof globalThis.MutationObserver !== "function") {
    globalThis.MutationObserver = class MutationObserver {
      constructor(cb) { this._cb = cb; }
      observe() { note("MutationObserver", "MutationObserver is inert — focus traps never see DOM mutations"); }
      disconnect() {}
      takeRecords() { return []; }
    };
  }
  if (typeof globalThis.ResizeObserver !== "function") {
    globalThis.ResizeObserver = class ResizeObserver {
      constructor(cb) { this._cb = cb; }
      observe() { note("ResizeObserver", "ResizeObserver is inert — nothing is ever measured or re-measured"); }
      unobserve() {}
      disconnect() {}
    };
  }
  if (typeof globalThis.getComputedStyle !== "function") {
    const COMPUTED = {
      animationName: "none", animationDuration: "0s", transitionDuration: "0s",
      transitionProperty: "none", display: "block", visibility: "visible",
      overflow: "visible", overflowX: "visible", overflowY: "visible",
      position: "static", zIndex: "auto", direction: "ltr", pointerEvents: "auto",
      paddingLeft: "0px", paddingRight: "0px", paddingTop: "0px", paddingBottom: "0px",
      borderLeftWidth: "0px", borderRightWidth: "0px", getPropertyValue: (k) => COMPUTED[k] ?? "",
    };
    globalThis.getComputedStyle = () => {
      note("getComputedStyle", "getComputedStyle() returns fixed defaults — vn-css does not expose computed values to JS");
      return COMPUTED;
    };
  }

  console.log("[dom-shim] installed (app-space prototype of the runtime patch)");
}
