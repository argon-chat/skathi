// vue-native JS runtime: @vue/runtime-core custom renderer over the __vn_*
// host protocol (crates/vn-tree). Apps import `createApp` from "vue-native";
// compiled SFC render functions import their helpers from "vue" — both are
// aliased to this module by the bundler (packages/vue-native/build.ts).

import {
  BaseTransition,
  BaseTransitionPropsValidators,
  createRenderer,
  createVNode,
  defineComponent,
  Fragment,
  getCurrentInstance,
  getTransitionRawChildren,
  h,
  resolveTransitionHooks,
  setTransitionHooks,
  toRaw,
  useTransitionState,
  version,
} from "@vue/runtime-core";

// ---- interning: names cross the boundary once, ids ever after -------------
const interned = new Map();
const I = (s) => {
  let v = interned.get(s);
  if (v === undefined) {
    v = __vn_intern(s);
    interned.set(s, v);
  }
  return v;
};

// Lockstep with crates/vn-tree (tree.rs EV_*).
const EV_CODES = {
  click: 1,
  mousedown: 2,
  mouseup: 3,
  wheel: 4,
  keydown: 5,
  keyup: 6,
  focus: 7,
  blur: 8,
  input: 9,
  scroll: 10,
  // <lottie> playback (crates/vn-lottie/CONTRACT.md §2). The names are
  // lottie-web's, camelCase and all, so `e.type` reads the same as it does on a
  // browser <lottie-player>.
  complete: 11,
  loopComplete: 12,
  enterFrame: 13,
  // The pointer family. gpui sources exactly three things — "the mouse moved
  // while over me" and the two edges of "the mouse is over me" — so only those
  // three have host codes; everything else in the W3C pointer set is DERIVED
  // from them (and from mousedown/mouseup) below.
  mousemove: 14,
  mouseenter: 15,
  mouseleave: 16,
  // CSS animation/transition events. Unlike everything above, the SOURCE is
  // vn-css's own animator rather than gpui: it queues one per finished
  // transition track and per animation start/end/cancel, and the host drains
  // the queue once a frame (crates/vn-tree/src/render.rs::dispatch_anim_events).
  // The payload carries `elapsedTime` in `a` and the property / @keyframes
  // name in `key`.
  transitionend: 17,
  animationstart: 18,
  animationend: 19,
  animationcancel: 20,
};
// Events the host does not deliver under their own code, synthesized in JS from
// one that it does. `pre: true` means the derived event fires BEFORE its source,
// which is the W3C order for the pointer family (`pointerdown` precedes
// `mousedown`); `focusin`/`focusout` are the bubbling siblings of `focus`/`blur`
// and fire after them.
//
// This is a real implementation, not a stub: a `pointerdown` listener registers
// the host's `mousedown` bit, and the payload it receives is the same one the
// mouse event carries — which is what makes reka-ui's Select (`onPointerDown`)
// and Tooltip (`onPointerEnter`) work rather than merely not throw.
const DERIVED = {
  pointerdown: { from: "mousedown", pre: true },
  pointerup: { from: "mouseup", pre: true },
  pointermove: { from: "mousemove", pre: true },
  pointerenter: { from: "mouseenter", pre: true },
  pointerleave: { from: "mouseleave", pre: true },
  focusin: { from: "focus", pre: false },
  focusout: { from: "blur", pre: false },
};
/** What `__vn_set_listener` has to be told about for this event name. */
const hostCode = (name) => EV_CODES[name] ?? EV_CODES[DERIVED[name]?.from];
/** Every name that shares one host code — the flag is the OR over all of them. */
const NAMES_BY_CODE = (() => {
  const out = {};
  const add = (code, name) => (out[code] ??= []).push(name);
  for (const name in EV_CODES) add(EV_CODES[name], name);
  for (const name in DERIVED) add(EV_CODES[DERIVED[name].from], name);
  return out;
})();
/** The derived events one host event produces, in dispatch order. */
const DERIVED_OF = (() => {
  const out = {};
  for (const name in DERIVED) (out[DERIVED[name].from] ??= []).push(name);
  return out;
})();
const POINTER_EVENTS = new Set([
  "pointerdown",
  "pointerup",
  "pointermove",
  "pointerenter",
  "pointerleave",
]);
// Vue hyphenates an `@camelCase` listener prop (`onLoopComplete` becomes
// "loop-complete"), which is the well-known reason `@loopComplete` silently
// misses a web component's camelCase event in a browser. We accept BOTH
// spellings and fold them onto the canonical camelCase name, so `@loopComplete`
// works from a template and `addEventListener("loopComplete")` works from JS.
const EVENT_ALIASES = {
  "loop-complete": "loopComplete",
  loopcomplete: "loopComplete",
  "enter-frame": "enterFrame",
  enterframe: "enterFrame",
};
const canonicalEvent = (name) => EVENT_ALIASES[name] ?? name;
// Per the UI Events spec these fire at the target only. (`scroll` bubbles from
// the *document*, never from an element.) The Lottie events are CustomEvents
// dispatched on the element by <lottie-player>, i.e. `bubbles: false`.
const NO_BUBBLE = new Set([
  "focus",
  "blur",
  "scroll",
  "complete",
  "loopComplete",
  "enterFrame",
  // The enter/leave pair is the one part of the pointer family that does NOT
  // bubble — getting that wrong is what makes a menu close as soon as the
  // pointer crosses one of its own items.
  "mouseenter",
  "mouseleave",
  "pointerenter",
  "pointerleave",
]);
const nodesById = new Map();

// ---- JS mirror nodes ------------------------------------------------------
// Vue writes symbol props onto host nodes and needs parentNode/nextSibling
// synchronously — both served from this mirror, zero host calls.
// One-shot compat advisories: never silently drop behavior a browser has —
// tell the developer what happened and what to use instead.
const warned = new Set();
function warnCompat(key, msg) {
  if (warned.has(key)) return;
  warned.add(key);
  console.warn(`[vue-native compat] ${msg}`);
}

// Host nodes sit on HTMLElement.prototype (redjs provides the real class) so
// `el instanceof HTMLElement` / `Node` checks in reused browser components
// pass, and carry the DOM-ish surface cheap to keep (tagName, value,
// add/removeEventListener).
// (…and when the engine has no HTMLElement at all — the bun test harness — a
// private prototype stands in, so the DOM surface below never lands on
// Object.prototype.)
const ElementProto =
  typeof HTMLElement === "function" ? HTMLElement.prototype : Object.create(Object.prototype);
// Text and comment mirrors are NOT elements — no selector matching, no layout,
// no attributes — but they are Nodes, and reused components branch on
// `nodeType === 3` all the time. They get their own prototype carrying only the
// node-level surface (see `NODE_MEMBERS` below).
const TextProto = Object.create(Object.prototype);

// A mirror node must never become reactive state.
//
// In a browser this is automatic: `Object.prototype.toString.call(divElement)`
// is `[object HTMLDivElement]`, which Vue's `getTargetType` classifies as
// INVALID, so `reactive(node)` hands the node straight back. Our mirrors are
// `Object.create(HTMLElement.prototype)` and stringify as `[object Object]`, so
// Vue would wrap each one in a Proxy — and then `set.has(node)`,
// `array.indexOf(node)` and `a === b` all start answering NO for the same node
// reached two different ways. reka-ui's DismissableLayer stack, its Collection
// ordering and its focus guards are ALL identity comparisons; every one of them
// silently breaks.
//
// `__v_skip` (Vue's `markRaw` flag) on the prototypes restores the browser's
// behaviour for every node at once, at zero per-node cost.
for (const proto of [ElementProto, TextProto]) {
  Object.defineProperty(proto, "__v_skip", { value: true, configurable: true });
}

function mk(id, kind, tag) {
  const n = Object.create(kind === 1 ? ElementProto : TextProto);
  n.id = id;
  n.kind = kind;
  n.tag = tag;
  n.parentNode = null;
  n.childNodes = [];
  if (kind === 1) {
    n.tagName = String(tag).toUpperCase();
    n.value = "";
    // Browser-visible attribute state. `patchProp` is the only writer that Vue
    // drives; reka-ui's roving focus and its Collection read `data-*`/`aria-*`
    // back off candidate nodes, so the mirror has to remember what was written
    // rather than answering "absent" for everything.
    n.attrs = Object.create(null);
    // The `class` Vue assigned, kept separately from the transition classes
    // `<Transition>` layers on top (runtime-dom keeps the same split, for the
    // same reason: a re-render must not wipe an in-flight `v-enter-active`).
    n.className = "";
    // Scroll containers report their position through these, exactly like the
    // DOM: CSS convention, positive px, 0 before the first scroll.
    n.scrollLeft = 0;
    n.scrollTop = 0;
    // Own-property methods shadow any prototype ones: these route into the
    // same registry the renderer's props use, so propagation sees both.
    n.addEventListener = (name, fn, opts) => addExtraListener(n, name, fn, opts);
    n.removeEventListener = (name, fn, opts) => removeExtraListener(n, name, fn, opts);
  }
  nodesById.set(id, n);
  return n;
}

function forEachDeep(n, f) {
  f(n);
  for (const c of n.childNodes) forEachDeep(c, f);
}

const handlers = new Map(); // node id -> { click: fn }

const nodeOps = {
  createElement: (type) => mk(__vn_create_element(I(type)), 1, type),
  createText: (text) => {
    const n = mk(__vn_create_text(), 2, "#text");
    // The mirror keeps the string too — `textContent` and `Node.data` read it
    // back, and a host round trip for that would be absurd.
    n.text = String(text);
    __vn_set_text(n.id, n.text);
    return n;
  },
  createComment: () => mk(__vn_create_anchor(), 3, "#comment"),
  setText: (node, text) => {
    node.text = String(text);
    __vn_set_text(node.id, node.text);
  },
  setElementText: (node, text) => {
    for (const c of node.childNodes) {
      c.parentNode = null;
      forEachDeep(c, (n) => {
        handlers.delete(n.id);
        nodesById.delete(n.id);
      });
    }
    node.childNodes.length = 0;
    // The element's OWN text (exclusive with children, exactly as in the host
    // tree) — `textContent` reads it back.
    node.ownText = String(text);
    __vn_set_element_text(node.id, String(text));
    notifyMutation(node, "childList");
  },
  insert: (child, parent, anchor) => {
    // insert is also move: detach from the old mirror parent first.
    if (child.parentNode) {
      const cs = child.parentNode.childNodes;
      const i = cs.indexOf(child);
      if (i >= 0) cs.splice(i, 1);
    }
    let idx = parent.childNodes.length;
    if (anchor != null) {
      const i = parent.childNodes.indexOf(anchor);
      if (i >= 0) idx = i;
    }
    parent.childNodes.splice(idx, 0, child);
    child.parentNode = parent;
    __vn_insert(child.id, parent.id, anchor ? anchor.id : 0);
    notifyMutation(parent, "childList", { addedNodes: [child], removedNodes: [] });
  },
  remove: (child) => {
    const p = child.parentNode;
    if (p) {
      const i = p.childNodes.indexOf(child);
      if (i >= 0) p.childNodes.splice(i, 1);
      child.parentNode = null;
      notifyMutation(p, "childList", { addedNodes: [], removedNodes: [child] });
    }
    forEachDeep(child, (n) => {
      handlers.delete(n.id);
      nodesById.delete(n.id);
    });
    __vn_remove(child.id);
  },
  parentNode: (node) => node.parentNode,
  nextSibling: (node) => {
    const p = node.parentNode;
    if (!p) return null;
    const i = p.childNodes.indexOf(node);
    return i >= 0 ? (p.childNodes[i + 1] ?? null) : null;
  },
  // <style scoped>: Vue stamps every element a scoped component renders (and
  // child component roots) with its data-v id — the Rust matcher buckets on it.
  setScopeId: (el, id) => __vn_add_scope(el.id, id),
  // Vue's `<Teleport>` resolves a STRING `to` through this hook. Without it
  // `<Teleport to="body">` — reka-ui's default for EVERY portal — warns
  // "Current renderer does not support string target" and renders nothing.
  //
  // The document roots a browser component reaches for all mean "the app root"
  // here; anything else goes through the real matcher.
  querySelector: (sel) => {
    if (sel === "body" || sel === "html" || sel === ":root" || sel === "#app") return appRoot;
    return queryOne(sel, appRoot ? appRoot.id : 0);
  },
};

// @vue/shared's hyphenate, verbatim: `\B` so a leading capital is only
// lowercased, never prefixed. Style keys are camelCase either way, but
// `withKeys` runs it over `KeyboardEvent.key` — "Enter" must become "enter",
// not "-enter", or every `@keydown.enter` silently stops matching.
const hyphenate = (s) => s.replace(/\B([A-Z])/g, "-$1").toLowerCase();

/// A `:style` key as CSS spells it. Custom properties are already CSS-cased and
/// are CASE-SENSITIVE, so hyphenating `--rekaPopperWidth` would rename it.
const styleKey = (k) => (k.startsWith("--") ? k : hyphenate(k));

function patchProp(el, key, prev, next) {
  if (key === "style") {
    const p = prev || {};
    const n = next || {};
    for (const k in p) if (!(k in n)) __vn_set_style(el.id, I(styleKey(k)), "");
    for (const k in n) {
      if (p[k] === n[k]) continue;
      // runtime-dom's `setStyle` rule: a null/undefined value CLEARS the
      // property. Stringifying it instead writes the literal text "undefined",
      // which makes the whole declaration block unparseable — and
      // `:style="{ transform: maybeUndefined }"` is an extremely common shape
      // (it is what @floating-ui/vue's `floatingStyles` produces before its
      // first measurement).
      __vn_set_style(el.id, I(styleKey(k)), n[k] == null ? "" : String(n[k]));
    }
  } else if (key === "class") {
    if (prev !== next) {
      el.className = next == null ? "" : String(next);
      writeClass(el);
    }
  } else if (key.startsWith("on") && key.length > 2) {
    // runtime-dom's `parseName`, same order: strip the option suffixes, then
    // read the name.
    //   onClick / onClickCapture / onClickOnce / onClickCaptureOnce
    //   on:loopComplete — the case-PRESERVING spelling compiler-dom emits for
    //                     a custom element's camelCase event (`@loopComplete`
    //                     on <lottie>). Everything else is hyphenated.
    let rest = key.slice(2);
    let capture = false;
    let once = false;
    for (;;) {
      if (rest.endsWith("Once")) {
        once = true;
        rest = rest.slice(0, -4);
      } else if (rest.endsWith("Capture")) {
        capture = true;
        rest = rest.slice(0, -7);
      } else break;
    }
    const namespaced = rest.startsWith(":");
    // Hyphenate, don't lowercase — `\B` keeps a leading capital bare, so
    // "Click" -> "click" and "LoopComplete" -> "loop-complete", exactly as in a
    // browser. `canonicalEvent` folds that back onto the camelCase name the
    // host actually emits.
    const name = namespaced ? rest.slice(1) : canonicalEvent(hyphenate(rest));
    const code = hostCode(name);
    if (!code) {
      // Compat-first: a listener the host will never deliver must not vanish in
      // silence. `onUpdate:modelValue` & friends are excluded — a `:` inside
      // (rather than in front of) the name marks a component-emit prop that
      // leaked onto an element, which a browser also binds to an event that
      // never fires, also silently.
      if (namespaced || !rest.includes(":")) {
        warnCompat(
          `event:${name}`,
          `@${name} — this event type is not delivered by the native host yet; ` +
            `the handler will never fire`,
        );
      }
      return;
    }
    let m = handlers.get(el.id);
    if (!m) {
      m = {};
      handlers.set(el.id, m);
    }
    let buckets = ensureBuckets(m, name);
    if (capture) {
      buckets.capture = next || null;
      buckets.onceC = once;
    } else {
      buckets.bubble = next || null;
      buckets.onceB = once;
    }
    syncListenerFlag(el.id, name);
  } else if (key === "key" || key === "ref") {
    // stripped by the vnode layer; never reaches here — but be safe
  } else {
    // DOM-compat attribute channel: standard-ish props flow to the host as
    // strings (native widgets read what they understand); null clears.
    if (next == null) __vn_set_prop(el.id, I(key), undefined);
    else if (typeof next === "string" || typeof next === "number" || typeof next === "boolean")
      __vn_set_prop(el.id, I(key), String(next));
    else
      warnCompat(
        `prop:${key}`,
        `prop "${key}" with a non-primitive value is not supported by the native host yet; ` +
          `it was dropped — pass a string/number/boolean or handle it in the component`,
      );
    // …and stays readable from JS, which is what `getAttribute`/`dataset`/the
    // Collection ordering in reka-ui need.
    if (el.attrs) {
      const before = el.attrs[key];
      if (next == null) delete el.attrs[key];
      else if (typeof next !== "object" && typeof next !== "function") el.attrs[key] = String(next);
      if (before !== el.attrs[key]) {
        notifyMutation(el, "attributes", { attributeName: key, oldValue: before ?? null });
      }
    }
    if (key === "value") el.value = next == null ? "" : String(next);
  }
}

/// The element's class attribute = what Vue assigned + whatever `<Transition>`
/// has layered on top. runtime-dom does exactly this (its `patchClass` merges
/// `el._vtc`); without it a re-render during a transition drops the phase class
/// and the animation stops halfway.
function writeClass(el) {
  const base = el.className || "";
  const extra = el._vtc;
  const value = extra && extra.size ? [base, ...extra].filter(Boolean).join(" ") : base;
  __vn_set_class(el.id, value);
}

function ensureBuckets(m, name) {
  let buckets = m[name];
  if (!buckets)
    buckets = m[name] = {
      bubble: null,
      capture: null,
      onceB: false,
      onceC: false,
      extraB: [],
      extraC: [],
    };
  return buckets;
}

function bucketsAlive(b) {
  return !!(b.bubble || b.capture || b.extraB.length || b.extraC.length);
}

/// Several event NAMES can share one host code (`pointerdown` rides
/// `mousedown`), so the flag is the OR over every name that maps to it —
/// switching a `pointerdown` listener off must not silence a `mousedown` one.
/// A custom event name has no host code at all and needs no flag: it is
/// dispatched from JS.
function syncListenerFlag(id, name) {
  const code = hostCode(name);
  if (!code) return;
  const m = handlers.get(id);
  let on = NAMES_BY_CODE[code].some((n) => m?.[n] && bucketsAlive(m[n]));
  // Document- and window-level listeners ride the APP ROOT's bit: the host
  // delivers events to nodes, and the root is the node that stands in for the
  // document. See `globalHandlers` below.
  if (!on && appRoot && id === appRoot.id) {
    on = NAMES_BY_CODE[code].some((n) =>
      [...globalHandlers.values()].some((g) => g[n] && bucketsAlive(g[n])),
    );
  }
  __vn_set_listener(id, code, on);
}

// ---- document / window level listeners ------------------------------------
// reka-ui's DismissableLayer (outside click, Escape), its focus guards and
// every "click outside to close" implementation in the ecosystem bind on
// `document` or `window`, neither of which is a node in the mirror tree. They
// get their own registry, joined onto the SAME propagation walk one level above
// the app root — which is exactly where a browser's event path puts them.
//
// Without this, a Dialog opens and never closes, and a menu stays open forever.
const globalHandlers = new Map(); // target object -> { [event name]: buckets }
const isMirrorNode = (n) => !!n && typeof n.id === "number" && nodesById.get(n.id) === n;

function addGlobalListener(target, rawName, fn, opts) {
  if (!fn) return false;
  const name = canonicalEvent(rawName);
  if (!hostCode(name)) return false;
  let m = globalHandlers.get(target);
  if (!m) globalHandlers.set(target, (m = {}));
  const buckets = ensureBuckets(m, name);
  const capture = opts === true || (opts && opts.capture);
  (capture ? buckets.extraC : buckets.extraB).push({ fn, once: !!(opts && opts.once) });
  if (appRoot) syncListenerFlag(appRoot.id, name);
  return true;
}

function removeGlobalListener(target, rawName, fn, opts) {
  const name = canonicalEvent(rawName);
  const buckets = globalHandlers.get(target)?.[name];
  if (!buckets) return false;
  const capture = opts === true || (opts && opts.capture);
  const arr = capture ? buckets.extraC : buckets.extraB;
  const i = arr.findIndex((e) => e.fn === fn);
  if (i >= 0) arr.splice(i, 1);
  if (appRoot) syncListenerFlag(appRoot.id, name);
  return true;
}

/// Installs the interception on one EventTarget-ish global. Events the host CAN
/// deliver are taken over; everything else still reaches the engine's own
/// EventTarget, so redjs' web APIs keep working.
function interceptGlobalTarget(target) {
  if (!target) return;
  const nativeAdd = typeof target.addEventListener === "function" ? target.addEventListener.bind(target) : null;
  const nativeRemove =
    typeof target.removeEventListener === "function" ? target.removeEventListener.bind(target) : null;
  try {
    Object.defineProperty(target, "addEventListener", {
      value: (name, fn, opts) => {
        if (!addGlobalListener(target, name, fn, opts)) nativeAdd?.(name, fn, opts);
      },
      configurable: true,
      writable: true,
    });
    Object.defineProperty(target, "removeEventListener", {
      value: (name, fn, opts) => {
        if (!removeGlobalListener(target, name, fn, opts)) nativeRemove?.(name, fn, opts);
      },
      configurable: true,
      writable: true,
    });
  } catch (e) {
    warnCompat(
      "global-listeners",
      "document/window event listeners could not be intercepted on this engine — a component " +
        `that closes itself on an outside click or Escape will stay open (${e})`,
    );
  }
}

// EventTarget-shaped registration for reused browser components/directives
// (vModelText uses this path). Merged into the same propagation walk.
/// Does this name look like an event the PLATFORM sources? Those are the ones
/// worth an advisory when the host cannot deliver them — a listener for one will
/// simply never fire. A name with a dot or a capital in it is a component's own
/// custom event (`"select.select"`, `"focusScope.autoFocusOnMount"` — reka-ui
/// dispatches a dozen of them), and those work fine: they are dispatched from JS
/// through [`dispatchEvent`] and never touch the host at all.
const looksNative = (name) => /^[a-z]+$/.test(name);

function addExtraListener(node, rawName, fn, opts) {
  if (!fn) return;
  const name = canonicalEvent(rawName);
  const code = hostCode(name);
  if (!code && looksNative(name)) {
    warnCompat(
      `event:${name}`,
      `addEventListener("${rawName}") — this event type is not delivered by the native host yet; ` +
        `the listener will never fire`,
    );
    return;
  }
  let m = handlers.get(node.id);
  if (!m) {
    m = {};
    handlers.set(node.id, m);
  }
  const buckets = ensureBuckets(m, name);
  const capture = opts === true || (opts && opts.capture);
  const entry = { fn, once: !!(opts && opts.once) };
  (capture ? buckets.extraC : buckets.extraB).push(entry);
  syncListenerFlag(node.id, name);
}

function removeExtraListener(node, rawName, fn, opts) {
  const name = canonicalEvent(rawName);
  const buckets = handlers.get(node.id)?.[name];
  if (!buckets) return;
  const capture = opts === true || (opts && opts.capture);
  const arr = capture ? buckets.extraC : buckets.extraB;
  const i = arr.findIndex((e) => e.fn === fn);
  if (i >= 0) arr.splice(i, 1);
  syncListenerFlag(node.id, name);
}

// ---- the DOM surface on mirror nodes (compat roadmap G2) ------------------
// Reused browser components call element methods unconditionally, and redjs'
// HTMLElement.prototype carries none — so the absence is a TypeError rather
// than a wrong answer. Everything below either answers TRUTHFULLY (from the
// mirror tree, or from the host's real matcher / real layout bounds), or
// answers a degraded constant AND leaves exactly one advisory.
//
// The two questions that cannot be answered in JS go to the host:
//   __vn_query          selector matching, through the CASCADE'S OWN matcher —
//                       a second JS-side matcher would be a silent divergence
//   __vn_bounds         the rectangle gpui laid the node out at last frame
//   __vn_computed_style the folded computed values + custom properties

let appRoot = null; // set by createApp().mount()

function defineDom(name, value, protos = [ElementProto]) {
  for (const proto of protos) {
    Object.defineProperty(proto, name, { value, configurable: true, writable: true });
  }
}
function defineDomGet(name, get, set, protos = [ElementProto]) {
  for (const proto of protos) Object.defineProperty(proto, name, { get, set, configurable: true });
}
/// The members every Node has, elements and text/comment mirrors alike.
const NODE_MEMBERS = [ElementProto, TextProto];

/// `__vn_query`, decoded. The host answers `"!<advisory>"` for a selector the
/// engine cannot run (`:has()`, `::before`, `:visited`) — the same text a rule
/// using it would print — so an unanswerable query explains itself instead of
/// quietly reporting "no match".
function hostQuery(selector, rootId, mode) {
  if (rootId === 0) return [];
  const raw = __vn_query(String(selector), rootId, mode);
  if (raw === "") return [];
  if (raw.charCodeAt(0) === 33 /* ! */) {
    warnCompat(
      `selector:${selector}`,
      `querySelector/closest/matches(${JSON.stringify(String(selector))}) — ${raw.slice(1)}`,
    );
    return [];
  }
  const out = [];
  for (const part of raw.split(",")) {
    const node = nodesById.get(Number(part));
    if (node) out.push(node);
  }
  return out;
}
const queryOne = (selector, rootId, mode = 0) => hostQuery(selector, rootId, mode)[0] ?? null;

const ZERO_RECT = {
  x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0,
};

/// The viewport, as `document.documentElement` reports it.
///
/// The coordinate space every rect here lives in is the WINDOW'S CLIENT AREA
/// (that is what gpui hands each element during prepaint), and the app root is
/// inset within it by whatever chrome the host draws. So the viewport is the
/// rectangle from the client-area origin to the far corner of the app root —
/// NOT the root's own box. Reporting the root's size instead would leave
/// @floating-ui clipping against `(0,0,rootW,rootH)` while measuring elements in
/// a space offset by the chrome, and every popover would be pushed against the
/// left edge by the `shift` middleware.
function viewportRect() {
  if (!appRoot) return { ...ZERO_RECT };
  const r = rectOf(appRoot);
  const width = Math.round(r.x + r.width);
  const height = Math.round(r.y + r.height);
  return { x: 0, y: 0, left: 0, top: 0, right: width, bottom: height, width, height };
}
function rectOf(node) {
  const raw = node.id ? __vn_bounds(node.id) : "";
  // A node that was not painted (detached, `display: none`, parked in a
  // <KeepAlive> cache) reports nothing — and an all-zero rect is exactly what a
  // browser reports for those, so no advisory is warranted.
  if (!raw) return { ...ZERO_RECT, toJSON() { return this; } };
  const [x, y, w, h] = raw.split(",").map(Number);
  return {
    x, y, width: w, height: h,
    top: y, left: x, right: x + w, bottom: y + h,
    toJSON() { return this; },
  };
}

// -- truthful: the mirror IS the tree
defineDom(
  "contains",
  function (other) {
    for (let n = other; n; n = n.parentNode) if (n === this) return true;
    return false;
  },
  NODE_MEMBERS,
);
defineDomGet("ownerDocument", () => globalThis.document, undefined, NODE_MEMBERS);
defineDomGet(
  "nodeType",
  function () {
    return this.kind === 2 ? 3 : this.kind === 3 ? 8 : 1;
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "nodeName",
  function () {
    return this.tagName ?? (this.kind === 2 ? "#text" : "#comment");
  },
  undefined,
  NODE_MEMBERS,
);
defineDomGet(
  "isConnected",
  function () {
    for (let n = this; n; n = n.parentNode) if (n === appRoot) return true;
    return false;
  },
  undefined,
  NODE_MEMBERS,
);
// A text/comment mirror's content, under both DOM names.
for (const alias of ["data", "nodeValue"]) {
  defineDomGet(
    alias,
    function () {
      return this.text ?? "";
    },
    function (value) {
      nodeOps.setText(this, value == null ? "" : String(value));
    },
    [TextProto],
  );
}
defineDomGet(
  "textContent",
  function () {
    return this.text ?? "";
  },
  function (value) {
    nodeOps.setText(this, value == null ? "" : String(value));
  },
  [TextProto],
);
// `childNodes` is the renderer's own list: every child node, in order.
// `children` is the DOM's ELEMENT-only view of it, and the distinction is
// load-bearing — `aria-hidden` (which reka's Dialog uses to hide the rest of
// the page) walks `parent.children` and calls `getAttribute` on each, so
// handing it text and comment mirrors made it log one failure per node.
const elementChildren = (n) => (n.childNodes || []).filter((c) => c.kind === 1);
defineDomGet("children", function () {
  return elementChildren(this);
});
defineDomGet("firstElementChild", function () {
  return elementChildren(this)[0] ?? null;
});
defineDomGet("lastElementChild", function () {
  const kids = elementChildren(this);
  return kids[kids.length - 1] ?? null;
});
defineDomGet("childElementCount", function () {
  return elementChildren(this).length;
});
function elementSibling(node, step) {
  const p = node.parentNode;
  if (!p) return null;
  for (let i = p.childNodes.indexOf(node) + step; i >= 0 && i < p.childNodes.length; i += step) {
    if (p.childNodes[i].kind === 1) return p.childNodes[i];
  }
  return null;
}
defineDomGet("nextElementSibling", function () {
  return elementSibling(this, 1);
});
defineDomGet("previousElementSibling", function () {
  return elementSibling(this, -1);
});
defineDomGet(
  "textContent",
  function () {
    let out = "";
    const walk = (n) => {
      if (n.kind === 2) out += n.text ?? "";
      else if (n.kind === 1 && n.ownText) out += n.ownText;
      for (const c of n.childNodes || []) walk(c);
    };
    walk(this);
    return out;
  },
  function (value) {
    nodeOps.setElementText(this, value == null ? "" : String(value));
  },
);

// -- selector matching, through the real cascade matcher
defineDom("matches", function (selector) {
  return hostQuery(selector, this.id, 1).length > 0;
});
defineDom("closest", function (selector) {
  return queryOne(selector, this.id, 2);
});
defineDom("querySelector", function (selector) {
  return queryOne(selector, this.id, 0);
});
defineDom("querySelectorAll", function (selector) {
  return hostQuery(selector, this.id, 0);
});

// -- layout, off the bounds gpui recorded for the last painted frame
defineDom("getBoundingClientRect", function () {
  return rectOf(this);
});
defineDom("getClientRects", function () {
  const r = rectOf(this);
  return r.width || r.height ? [r] : [];
});
for (const [name, pick] of [
  ["offsetWidth", (r) => r.width],
  ["offsetHeight", (r) => r.height],
  ["clientWidth", (r) => r.width],
  ["clientHeight", (r) => r.height],
  ["scrollWidth", (r) => r.width],
  ["scrollHeight", (r) => r.height],
]) {
  defineDomGet(name, function () {
    return Math.round(pick(rectOf(this)));
  });
}
// `offset*` is measured from the offsetParent in the DOM; we have no positioned
// -ancestor chain to measure from, so these are window-relative.
for (const [name, pick] of [
  ["offsetTop", (r) => r.top],
  ["offsetLeft", (r) => r.left],
]) {
  defineDomGet(name, function () {
    warnCompat(
      "offsetParent",
      "offsetTop/offsetLeft are measured from the WINDOW, not from an offsetParent — " +
        "vue-native has no positioned-ancestor chain; use getBoundingClientRect(), which is exact",
    );
    return Math.round(pick(rectOf(this)));
  });
}
defineDomGet("offsetParent", () => null);
// The border widths a browser reports here. gpui's painted rectangle is the
// BORDER box and the cascade's border widths are not published, so 0 is the
// honest constant — @floating-ui adds these to every clipping rect, and
// `undefined` there turns the whole computation into NaN.
defineDomGet("clientTop", () => 0);
defineDomGet("clientLeft", () => 0);
defineDom("scrollIntoView", function () {
  warnCompat(
    "scrollIntoView",
    "element.scrollIntoView() is a no-op — vue-native cannot drive a scroll container from JS yet " +
      "(the host exposes scrolling to devtools only)",
  );
});

// -- the attribute channel (`patchProp` is the writer; see `mk`)
defineDom("getAttribute", function (name) {
  const v = this.attrs?.[String(name)];
  return v === undefined ? null : v;
});
defineDom("hasAttribute", function (name) {
  return this.attrs ? String(name) in this.attrs : false;
});
defineDom("getAttributeNames", function () {
  return this.attrs ? Object.keys(this.attrs) : [];
});
defineDom("setAttribute", function (name, value) {
  const key = String(name);
  // `style` is a declaration BODY as an attribute, not the object `patchProp`'s
  // `:style` path expects.
  if (key === "style") {
    this.style.cssText = value == null ? "" : String(value);
    return;
  }
  patchProp(this, key, undefined, value == null ? "" : String(value));
});
defineDom("removeAttribute", function (name) {
  patchProp(this, String(name), undefined, null);
});
defineDom("toggleAttribute", function (name, force) {
  const on = force === undefined ? !this.hasAttribute(name) : !!force;
  if (on) this.setAttribute(name, "");
  else this.removeAttribute(name);
  return on;
});
const camelToData = (k) => "data-" + hyphenate(k);
defineDomGet("dataset", function () {
  const node = this;
  const proxy = new Proxy(Object.create(null), {
    get: (_t, k) => (typeof k === "string" ? node.attrs?.[camelToData(k)] : undefined),
    has: (_t, k) => typeof k === "string" && camelToData(k) in (node.attrs ?? {}),
    set: (_t, k, v) => {
      node.setAttribute(camelToData(k), v);
      return true;
    },
    deleteProperty: (_t, k) => {
      node.removeAttribute(camelToData(k));
      return true;
    },
    ownKeys: () =>
      Object.keys(node.attrs ?? {})
        .filter((k) => k.startsWith("data-"))
        .map((k) => k.slice(5).replace(/-([a-z])/g, (_, c) => c.toUpperCase())),
    getOwnPropertyDescriptor: () => ({ enumerable: true, configurable: true }),
  });
  Object.defineProperty(this, "dataset", { value: proxy, configurable: true });
  return proxy;
});

// -- classList, over the class Vue assigned (transition classes ride `_vtc`)
defineDomGet("classList", function () {
  const node = this;
  const parts = () => (node.className ? node.className.split(/\s+/).filter(Boolean) : []);
  const commit = (list) => {
    node.className = list.join(" ");
    writeClass(node);
  };
  const list = {
    add(...names) {
      const cur = parts();
      for (const n of names) if (n && !cur.includes(n)) cur.push(n);
      commit(cur);
    },
    remove(...names) {
      commit(parts().filter((c) => !names.includes(c)));
    },
    toggle(name, force) {
      const on = force === undefined ? !parts().includes(name) : !!force;
      if (on) list.add(name);
      else list.remove(name);
      return on;
    },
    contains: (name) => parts().includes(name),
    get length() {
      return parts().length;
    },
    item: (i) => parts()[i] ?? null,
    toString: () => node.className,
    [Symbol.iterator]: () => parts()[Symbol.iterator](),
  };
  Object.defineProperty(this, "classList", { value: list, configurable: true });
  return list;
});

// -- inline style: writes go through the same host call `:style` makes, so this
//    is genuinely functional — CUSTOM PROPERTIES INCLUDED, which is how
//    reka-ui's `--reka-popper-*` sizing variables reach the cascade.
defineDomGet("style", function () {
  const node = this;
  const own = Object.create(null);
  const write = (k, v) => {
    const key = String(k);
    const css = key.startsWith("--") ? key : hyphenate(key);
    own[key] = v == null ? "" : String(v);
    __vn_set_style(node.id, I(css), v == null ? "" : String(v));
  };
  const style = new Proxy(
    {
      setProperty: (k, v) => write(k, v),
      removeProperty: (k) => {
        const prev = own[String(k)] ?? "";
        write(k, "");
        return prev;
      },
      getPropertyValue: (k) => own[String(k)] ?? "",
      get cssText() {
        return Object.keys(own)
          .filter((k) => own[k] !== "")
          .map((k) => `${k.startsWith("--") ? k : hyphenate(k)}: ${own[k]}`)
          .join("; ");
      },
      set cssText(v) {
        for (const decl of String(v).split(";")) {
          const i = decl.indexOf(":");
          if (i > 0) write(decl.slice(0, i).trim(), decl.slice(i + 1).trim());
        }
      },
    },
    {
      get: (t, k) => (k in t ? t[k] : (own[k] ?? "")),
      set: (t, k, v) => {
        if (!(k in t)) write(k, v);
        else t[k] = v;
        return true;
      },
    },
  );
  Object.defineProperty(this, "style", { value: style, configurable: true });
  return style;
});

// -- focus: gpui owns it and exposes no programmatic call to JS yet
defineDom("focus", function () {
  warnCompat(
    "focus",
    "element.focus() is a no-op — gpui owns focus and the host exposes no programmatic focus " +
      "call to JS yet, so focus traps and roving (arrow-key) focus are inert; clicking and Tab " +
      "still move focus for real",
  );
});
defineDom("blur", function () {});

// -- dispatchEvent: a component's own events, through the SAME capture/bubble
//    walk the host's events take. reka-ui builds its whole selection protocol
//    out of this (`select.select`, `menu.itemSelect`,
//    `dismissableLayer.pointerDownOutside`, `focusScope.autoFocusOnMount`): it
//    adds a one-shot listener for a custom name and immediately dispatches it,
//    so without this an option click reaches a handler that is never called.
defineDom(
  "dispatchEvent",
  function (ev) {
    if (!ev || typeof ev.type !== "string") return true;
    return propagate(this, canonicalEvent(ev.type), ev);
  },
  NODE_MEMBERS,
);

// -- pointer capture. gpui captures the mouse implicitly for the duration of a
//    press, which is what this API is FOR — so the capture itself is already
//    the platform's behaviour and only the browser BOOKKEEPING has to exist,
//    consistently. reka-ui's Select opens on `pointerdown` and immediately
//    releases the capture it would otherwise hold; a missing
//    `hasPointerCapture` is a TypeError that kills the open outright.
defineDom("setPointerCapture", function (pointerId) {
  (this._pointerCaptures || (this._pointerCaptures = new Set())).add(pointerId);
});
defineDom("hasPointerCapture", function (pointerId) {
  return !!this._pointerCaptures?.has(pointerId);
});
defineDom("releasePointerCapture", function (pointerId) {
  this._pointerCaptures?.delete(pointerId);
});

defineDom("insertAdjacentElement", function (_where, el) {
  warnCompat(
    "insertAdjacentElement",
    "insertAdjacentElement() is a no-op — nodes injected outside the Vue renderer are not part of " +
      "the retained tree and are never painted",
  );
  return el;
});
defineDom("append", function () {});
defineDom("remove", function () {
  warnCompat(
    "remove",
    "element.remove() is a no-op — the retained tree is owned by the Vue renderer; use v-if/v-for",
  );
});
defineDom("cloneNode", function () {
  warnCompat(
    "cloneNode",
    "element.cloneNode() is not implemented — the retained tree has no copy operation; render a " +
      "second element instead",
  );
  return null;
});

// ---- W3C event synthesis + propagation ------------------------------------
// The host sends ONE compact payload per input event targeted at the deepest
// hit node; the mirror tree provides the ancestor path. Events are real redjs
// native classes when available (KeyboardEvent & co), with a CustomEvent
// fallback so nothing hard-depends on the engine revision.
const mods = (d) => ({
  ctrlKey: !!(d & 1),
  shiftKey: !!(d & 2),
  altKey: !!(d & 4),
  metaKey: !!(d & 8),
});

// The one synthetic pointer: gpui reports a single OS cursor, so there is
// exactly one primary mouse pointer. (A pen or a touch point would have to be
// reported by the platform layer, which does not.)
const POINTER_INIT = {
  pointerId: 1,
  pointerType: "mouse",
  isPrimary: true,
  width: 1,
  height: 1,
};

function synthesize(name, a, b, c, d, key) {
  const common = { bubbles: !NO_BUBBLE.has(name), cancelable: !NO_BUBBLE.has(name) };
  const M = mods(d);
  try {
    if (POINTER_EVENTS.has(name)) {
      const init = {
        ...common,
        ...M,
        ...POINTER_INIT,
        clientX: a,
        clientY: b,
        button: c,
        buttons: c >= 0 ? 1 << c : 0,
        pressure: c >= 0 ? 0.5 : 0,
      };
      if (typeof PointerEvent === "function") return new PointerEvent(name, init);
      // No PointerEvent class in this engine: a MouseEvent carrying the pointer
      // fields reads identically to everything that consumes one.
      return withProps(new MouseEvent(name, init), { ...POINTER_INIT, pressure: init.pressure });
    }
    switch (name) {
      case "click":
      case "mousedown":
      case "mouseup":
      case "mousemove":
      case "mouseenter":
      case "mouseleave":
        return new MouseEvent(name, { ...common, ...M, clientX: a, clientY: b, button: c });
      case "focusin":
      case "focusout":
        if (typeof FocusEvent === "function") return new FocusEvent(name, common);
        break;
      case "wheel":
        if (typeof WheelEvent === "function")
          return new WheelEvent(name, { ...common, ...M, deltaX: a, deltaY: b });
        break;
      case "keydown":
      case "keyup":
        if (typeof KeyboardEvent === "function")
          return new KeyboardEvent(name, { ...common, ...M, key: key ?? "" });
        break;
      case "focus":
      case "blur":
        if (typeof FocusEvent === "function") return new FocusEvent(name, common);
        break;
      case "input":
        if (typeof InputEvent === "function")
          return new InputEvent(name, { ...common, data: key ?? null });
        break;
      case "scroll":
        // A plain Event in every browser; the position is read off e.target.
        if (typeof Event === "function") return new Event(name, common);
        break;
      case "transitionend":
        if (typeof TransitionEvent === "function")
          return new TransitionEvent(name, { ...common, propertyName: key ?? "", elapsedTime: a });
        break;
      case "animationstart":
      case "animationend":
      case "animationcancel":
        if (typeof AnimationEvent === "function")
          return new AnimationEvent(name, { ...common, animationName: key ?? "", elapsedTime: a });
        break;
      case "complete":
      case "loopComplete":
      case "enterFrame":
        // <lottie-player> dispatches CustomEvents; lottie-web hands its own
        // listeners a plain object with these fields. Both shapes are served:
        // the numbers sit on `detail` AND directly on the event.
        return withProps(new CustomEvent(name, { ...common, detail: lottieDetail(name, a, b, c, d) }),
                         lottieDetail(name, a, b, c, d));
    }
  } catch (e) {
    /* fall through to the generic event */
  }
  const ev = new CustomEvent(name, common);
  const extra =
    name === "wheel"
      ? { deltaX: a, deltaY: b, ...M }
      : name === "keydown" || name === "keyup"
        ? { key: key ?? "", ...M }
        : name === "input"
          ? { data: key ?? null }
          : name === "transitionend"
            ? { propertyName: key ?? "", elapsedTime: a, pseudoElement: "" }
          : name === "animationstart" || name === "animationend" || name === "animationcancel"
            ? { animationName: key ?? "", elapsedTime: a, pseudoElement: "" }
          : name === "focus" || name === "blur" || name === "focusin" || name === "focusout" || name === "scroll"
            ? {}
            : name === "complete" || name === "loopComplete" || name === "enterFrame"
              ? lottieDetail(name, a, b, c, d)
              : POINTER_EVENTS.has(name)
                ? { clientX: a, clientY: b, button: c, ...POINTER_INIT, ...M }
                : { clientX: a, clientY: b, button: c, ...M };
  for (const k in extra) Object.defineProperty(ev, k, { value: extra[k], configurable: true });
  return ev;
}

function withProps(ev, props) {
  for (const k in props) Object.defineProperty(ev, k, { value: props[k], configurable: true });
  return ev;
}

// The payload crates/vn-tree/src/lottie.rs sends, named the way lottie-web
// names it (`currentTime` / `totalTime` are frame counts there too).
function lottieDetail(name, a, b, c, d) {
  if (name === "enterFrame") return { currentTime: a, totalTime: b, elapsed: c, progress: d };
  if (name === "loopComplete") return { currentTime: a, totalTime: b, currentLoop: c };
  return { currentTime: a, totalTime: b, loops: c };
}

function invoke(buckets, phase, ev, node, name, state) {
  Object.defineProperty(ev, "currentTarget", { value: node, configurable: true });
  const fn = phase === "capture" ? buckets.capture : buckets.bubble;
  if (fn) {
    if (buckets[phase === "capture" ? "onceC" : "onceB"]) {
      if (phase === "capture") buckets.capture = null;
      else buckets.bubble = null;
    }
    if (Array.isArray(fn)) {
      for (const f of fn) {
        if (state.immediate) break;
        f(ev);
      }
    } else fn(ev);
  }
  const extra = phase === "capture" ? buckets.extraC : buckets.extraB;
  // snapshot: spec semantics — listeners added during dispatch don't fire now
  for (const entry of [...extra]) {
    if (state.immediate) break;
    if (entry.once) {
      const i = extra.indexOf(entry);
      if (i >= 0) extra.splice(i, 1);
    }
    entry.fn.call(node, ev);
  }
  if (isMirrorNode(node)) syncListenerFlag(node.id, name);
  else if (appRoot) syncListenerFlag(appRoot.id, name);
}

function propagate(target, name, ev) {
  const path = [];
  for (let n = target; n; n = n.parentNode) path.push(n);
  // …and above the root, the document and the window — where a browser's event
  // path puts them, and where every dismissable-layer implementation listens.
  if (globalHandlers.size) {
    if (globalThis.document) path.push(globalThis.document);
    path.push(globalThis);
  }
  Object.defineProperty(ev, "target", { value: target, configurable: true });
  const state = { stopped: false, immediate: false };
  const origStop = ev.stopPropagation?.bind(ev);
  const origStopIm = ev.stopImmediatePropagation?.bind(ev);
  ev.stopPropagation = () => {
    state.stopped = true;
    origStop?.();
  };
  ev.stopImmediatePropagation = () => {
    state.stopped = true;
    state.immediate = true;
    origStopIm?.();
  };
  const at = (node, phase) => {
    const buckets = isMirrorNode(node)
      ? handlers.get(node.id)?.[name]
      : globalHandlers.get(node)?.[name];
    if (buckets) invoke(buckets, phase, ev, node, name, state);
  };
  // capture: root -> target (exclusive)
  for (let i = path.length - 1; i > 0 && !state.stopped; i--) at(path[i], "capture");
  // at target: capture listeners, then bubble listeners
  if (!state.stopped) at(target, "capture");
  if (!state.stopped) at(target, "bubble");
  // bubble: target.parent -> root
  if (ev.bubbles) for (let i = 1; i < path.length && !state.stopped; i++) at(path[i], "bubble");
  return !ev.defaultPrevented;
}

const CODE_TO_NAME = Object.fromEntries(Object.entries(EV_CODES).map(([k, v]) => [v, k]));

__vn_set_dispatcher((nodeId, evCode, a, b, c, d, key) => {
  const name = CODE_TO_NAME[evCode];
  const target = nodesById.get(nodeId);
  if (!name || !target) return true;
  // Sticky activation (docs/WINDOWING.md §1.9). The windowing prelude arms it
  // from `keydown`/`mousedown` on the engine's own EventTarget, which gpui
  // events never reach — the host delivers them HERE. The prelude publishes
  // this hook for exactly that reason; without the call, `beforeunload` could
  // never veto a close because the window would never count as interacted with.
  if (name === "keydown" || name === "mousedown") globalThis.__vnStickyActivation?.();
  // input/change: the payload string is the new value — mirror it onto the
  // node BEFORE dispatch so `e.target.value` reads like the browser.
  if ((name === "input" || name === "change") && key != null) target.value = key;
  // scroll: same idea — the position lives on the element, not on the event.
  if (name === "scroll") {
    target.scrollLeft = a;
    target.scrollTop = b;
  }
  // The derived family (`pointerdown` from `mousedown`, `focusin` from `focus`,
  // …): each one is a full W3C event of its own with its own propagation, in
  // the order a browser fires them. `preventDefault()` on a derived pointer
  // event cancels the source, exactly as it does in a browser.
  const derived = DERIVED_OF[name];
  let proceed = true;
  if (derived) {
    for (const dn of derived) {
      if (!DERIVED[dn].pre) continue;
      if (!propagate(target, dn, synthesize(dn, a, b, c, d, key))) proceed = false;
    }
  }
  const ev = synthesize(name, a, b, c, d, key);
  if (!proceed && ev.preventDefault) ev.preventDefault();
  proceed = propagate(target, name, ev) && proceed;
  if (derived) {
    for (const dn of derived) {
      if (DERIVED[dn].pre) continue;
      propagate(target, dn, synthesize(dn, a, b, c, d, key));
    }
  }
  return proceed;
});

const renderer = createRenderer({ ...nodeOps, patchProp });

/// App entry point. No container argument — the window IS the root.
export function createApp(rootComponent, rootProps = null) {
  const app = renderer.createApp(rootComponent, rootProps);
  const mount = app.mount.bind(app);
  app.mount = (container) => {
    if (container != null)
      warnCompat(
        "mount:container",
        `app.mount(${JSON.stringify(container)}) — container arguments are accepted for ` +
          `browser parity but ignored: the window is the root in vue-native`,
      );
    // SFC styles collected at module-eval time (see build.ts) become the
    // stylesheet, installed before first mount so the first paint is styled.
    if (globalThis.__VN_CSS__?.length) {
      __vn_set_stylesheet(__VN_CSS__.join("\n"));
    }
    const root = mk(__vn_create_root(), 1, "root");
    // The document roots a reused browser component reaches for (`<Teleport
    // to="body">`, `document.querySelectorAll`, `document.body.contains`) all
    // mean this node.
    appRoot = root;
    const vm = mount(root);
    // Dev hot-restart: the host calls this before re-evaling a new bundle.
    globalThis.__VN_TEARDOWN__ = () => {
      try {
        app.unmount();
      } catch (e) {
        __vn_log("teardown: " + e);
      }
      __vn_remove(root.id);
      // Detached-but-live nodes: <KeepAlive> creates its cache container with
      // createElement and NEVER inserts or removes it (Vue moves subtrees in
      // and out of it), so nothing above ever tells us to free it. After
      // app.unmount() every node Vue owns is gone from the mirror, so whatever
      // is left is exactly that kind of orphan — free it or the host arena
      // grows by one node per KeepAlive on every hot restart.
      for (const id of Array.from(nodesById.keys())) __vn_remove(id);
      nodesById.clear();
      handlers.clear();
      globalThis.__VN_CSS__ = [];
      globalThis.__VN_TEARDOWN__ = undefined;
      appRoot = null;
      observers.clear();
      globalHandlers.clear();
    };
    __vn_log(`Vue ${version} app mounted (vue-native runtime)`);
    return vm;
  };
  return app;
}

// ---- missing globals (compat roadmap G5) ----------------------------------
// Each of these is a ReferenceError today, i.e. strictly worse than a wrong
// answer: reka-ui's FocusScope constructs a MutationObserver inside a
// SETUP-TIME watchEffect, so every modal (Dialog, Popover, Menu, Select) dies
// before it renders. They are installed on the global unconditionally when the
// engine does not already have them.

const defineGlobal = (name, value) => {
  if (typeof globalThis[name] === "undefined") {
    Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
  }
};

// -- the `CSS` namespace object.
//
// reka-ui's `Presence` matches the `animationend` it just received against the
// element's `animationName` with `CSS.escape(...)`. `CSS is not defined` there
// is a ReferenceError *inside an event handler*: the exit animation never
// completes and the element never unmounts. It only became reachable when
// `animationend` started firing at all (m12 item 3).
//
// `escape` is CSSOM §4.2's algorithm implemented rather than approximated — it
// feeds a string comparison, and an under-escaped identifier silently stops
// matching.
defineGlobal("CSS", {
  escape(value) {
    const s = String(value);
    let out = "";
    for (let i = 0; i < s.length; i++) {
      const c = s.charCodeAt(i);
      const ch = s[i];
      if (c === 0) {
        out += "�";
      } else if (
        (c >= 0x1 && c <= 0x1f) ||
        c === 0x7f ||
        (i === 0 && c >= 0x30 && c <= 0x39) ||
        (i === 1 && c >= 0x30 && c <= 0x39 && s.charCodeAt(0) === 0x2d)
      ) {
        out += "\\" + c.toString(16) + " ";
      } else if (i === 0 && c === 0x2d && s.length === 1) {
        out += "\\" + ch;
      } else if (
        c >= 0x80 ||
        c === 0x2d ||
        c === 0x5f ||
        (c >= 0x30 && c <= 0x39) ||
        (c >= 0x41 && c <= 0x5a) ||
        (c >= 0x61 && c <= 0x7a)
      ) {
        out += ch;
      } else {
        out += "\\" + ch;
      }
    }
    return out;
  },
  // A feature PROBE. Answering `false` for everything would be a lie in the
  // other direction (it is how `@supports`-guarded fallbacks get chosen), so
  // this says what vue-native can express: the properties the cascade maps.
  supports(a, b) {
    const body = b === undefined ? String(a) : `${String(a)}:${String(b)}`;
    const name = (body.split(":")[0] || "").trim().toLowerCase();
    if (!name) return false;
    if (name.startsWith("--")) return true;
    return CSS_SUPPORTED.has(name);
  },
  registerProperty() {
    warnCompat(
      "CSS.registerProperty",
      "CSS.registerProperty() is a no-op — vue-native's custom properties are untyped strings, " +
        "so a registered syntax/inherits/initialValue has nothing to enforce",
    );
  },
});

/// The property names `CSS.supports` answers `true` for: the ones vn-css maps
/// onto a real computed value. Deliberately conservative — a probe that says
/// yes and then does nothing is worse than one that says no.
const CSS_SUPPORTED = new Set([
  "animation", "animation-delay", "animation-direction", "animation-duration",
  "animation-fill-mode", "animation-iteration-count", "animation-name",
  "animation-play-state", "animation-timing-function",
  "transition", "transition-delay", "transition-duration", "transition-property",
  "transition-timing-function",
  "align-items", "align-self", "background", "background-color", "border", "border-color",
  "border-radius", "border-width", "bottom", "box-shadow", "color", "column-gap", "display",
  "flex", "flex-basis", "flex-direction", "flex-grow", "flex-shrink", "flex-wrap", "font-size",
  "font-style", "font-weight", "gap", "height", "inset", "justify-content", "left",
  "line-height", "margin", "margin-bottom", "margin-left", "margin-right", "margin-top",
  "max-height", "max-width", "min-height", "min-width", "opacity", "overflow", "overflow-x",
  "overflow-y", "padding", "padding-bottom", "padding-left", "padding-right", "padding-top",
  "pointer-events", "position", "right", "row-gap", "text-align", "top", "transform",
  "visibility", "white-space", "width", "z-index",
]);

// -- MutationObserver, wired to the REAL mirror-tree mutations.
// Records are queued at the nodeOps/patchProp call sites and delivered on a
// microtask, which is exactly a browser's timing (the observer callback runs at
// the end of the current task, before rendering).
const observers = new Set();
let mutationFlushQueued = false;

function notifyMutation(target, type, extra) {
  if (observers.size === 0) return;
  for (const ob of observers) {
    for (const entry of ob._targets) {
      const inScope = entry.node === target || (entry.subtree && isAncestor(entry.node, target));
      if (!inScope) continue;
      if (type === "childList" && !entry.options.childList) continue;
      if (type === "attributes") {
        if (!entry.options.attributes) continue;
        if (entry.options.attributeFilter && !entry.options.attributeFilter.includes(extra.attributeName)) {
          continue;
        }
      }
      ob._records.push({
        type,
        target,
        addedNodes: extra?.addedNodes ?? [],
        removedNodes: extra?.removedNodes ?? [],
        attributeName: extra?.attributeName ?? null,
        attributeNamespace: null,
        oldValue: entry.options.attributeOldValue ? (extra?.oldValue ?? null) : null,
        previousSibling: null,
        nextSibling: null,
      });
      break; // one record per observer per mutation, as in the DOM
    }
  }
  if (!mutationFlushQueued) {
    mutationFlushQueued = true;
    Promise.resolve().then(flushMutations);
  }
}

function isAncestor(ancestor, node) {
  for (let n = node?.parentNode; n; n = n.parentNode) if (n === ancestor) return true;
  return false;
}

function flushMutations() {
  mutationFlushQueued = false;
  for (const ob of [...observers]) {
    if (!ob._records.length) continue;
    const records = ob.takeRecords();
    try {
      ob._callback(records, ob);
    } catch (e) {
      console.error(`[vue-native] MutationObserver callback threw: ${e}`);
    }
  }
}

defineGlobal(
  "MutationObserver",
  class MutationObserver {
    constructor(callback) {
      this._callback = callback;
      this._targets = [];
      this._records = [];
    }
    observe(node, options = {}) {
      if (!node || node.kind !== 1) {
        warnCompat(
          "MutationObserver:target",
          "MutationObserver.observe() was given something that is not a rendered element — " +
            "document/body are not real nodes in vue-native, so nothing under them is observed",
        );
        return;
      }
      if (options.characterData) {
        warnCompat(
          "MutationObserver:characterData",
          "MutationObserver characterData records are not delivered — the retained tree carries " +
            "text as node content, not as a mutation; childList and attributes are",
        );
      }
      this._targets.push({
        node,
        subtree: !!options.subtree,
        options: {
          childList: !!options.childList,
          attributes: !!options.attributes || !!options.attributeFilter,
          attributeOldValue: !!options.attributeOldValue,
          attributeFilter: options.attributeFilter ?? null,
        },
      });
      observers.add(this);
    }
    disconnect() {
      this._targets.length = 0;
      this._records.length = 0;
      observers.delete(this);
    }
    takeRecords() {
      return this._records.splice(0, this._records.length);
    }
  },
);

// -- ResizeObserver, sampled from the host's real layout.
//
// gpui recomputes layout every frame and publishes no per-element resize
// SIGNAL, but it does publish the resulting rectangle (`__vn_bounds`) — so the
// observer polls its targets once per animation frame and reports the frames on
// which a size actually changed. That is a frame of latency against a browser's
// notification, and it costs one host call per observed element per frame while
// anything is observed (nothing is, until something floats).
//
// It has to be real: @floating-ui's `autoUpdate` positions an element ONCE and
// then waits for a ResizeObserver to tell it the element has been laid out. An
// inert observer means every popover, tooltip, dropdown and select keeps the
// position computed from its pre-layout 0×0 box — which is to say none of them
// is ever placed at all.
const resizeObservers = new Set();
let resizePollQueued = false;

function scheduleResizePoll() {
  if (resizePollQueued) return;
  const raf = globalThis.requestAnimationFrame;
  if (typeof raf !== "function") return;
  resizePollQueued = true;
  raf(pollResizeObservers);
}

function pollResizeObservers() {
  resizePollQueued = false;
  let observing = false;
  for (const ob of [...resizeObservers]) {
    if (!ob._targets.size) continue;
    observing = true;
    const entries = [];
    for (const [node, last] of ob._targets) {
      const r = rectOf(node);
      if (last && last.w === r.width && last.h === r.height) continue;
      ob._targets.set(node, { w: r.width, h: r.height });
      const box = [{ inlineSize: r.width, blockSize: r.height }];
      entries.push({ target: node, contentRect: r, borderBoxSize: box, contentBoxSize: box, devicePixelContentBoxSize: box });
    }
    if (entries.length) {
      try {
        ob._callback(entries, ob);
      } catch (e) {
        console.error(`[vue-native] ResizeObserver callback threw: ${e}`);
      }
    }
  }
  if (observing) scheduleResizePoll();
}

defineGlobal(
  "ResizeObserver",
  class ResizeObserver {
    constructor(callback) {
      this._callback = callback;
      this._targets = new Map(); // node -> last reported {w, h}
    }
    observe(node) {
      if (!node || typeof node.id !== "number") return;
      // `null` = "no size reported yet", so the first poll always fires —
      // which is what a browser does on `observe()` too.
      this._targets.set(node, null);
      resizeObservers.add(this);
      scheduleResizePoll();
    }
    unobserve(node) {
      this._targets.delete(node);
    }
    disconnect() {
      this._targets.clear();
      resizeObservers.delete(this);
    }
  },
);

// -- DocumentFragment. Vue's Teleport accepts one as a target, and reka-ui's
//    Select CONSTRUCTS one even while closed — a missing class kills it at
//    setup. Ours is a detached mirror element, so a Teleport into it renders
//    into the retained tree without being painted, which is what a browser's
//    off-document fragment does too.
defineGlobal(
  "DocumentFragment",
  class DocumentFragment {
    constructor() {
      const node = mk(__vn_create_element(I("fragment")), 1, "fragment");
      Object.defineProperty(node, "nodeType", { value: 11, configurable: true });
      return node;
    }
  },
);

// -- NodeFilter. `document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, …)`
//    is what reka's FocusScope walks with; the missing GLOBAL is what throws
//    first, before the walker is even built.
defineGlobal("NodeFilter", {
  SHOW_ALL: 0xffffffff,
  SHOW_ELEMENT: 1,
  SHOW_TEXT: 4,
  SHOW_COMMENT: 128,
  FILTER_ACCEPT: 1,
  FILTER_REJECT: 2,
  FILTER_SKIP: 3,
});

// -- Tag-specific element classes. redjs ships Element/HTMLElement/SVGElement
//    only, and reka-ui brands nodes with `instanceof HTMLInputElement`
//    (FocusScope's isSelectableInput). A MISSING class is a ReferenceError,
//    which is strictly worse than a false `instanceof` — so each one exists and
//    answers for the tag it names.
const TAG_CLASSES = {
  HTMLInputElement: "INPUT",
  HTMLTextAreaElement: "TEXTAREA",
  HTMLSelectElement: "SELECT",
  HTMLButtonElement: "BUTTON",
  HTMLAnchorElement: "A",
  HTMLFormElement: "FORM",
  HTMLLabelElement: "LABEL",
  HTMLDivElement: "DIV",
  HTMLSpanElement: "SPAN",
  HTMLImageElement: "IMG",
  HTMLIFrameElement: "IFRAME",
};
for (const [name, tag] of Object.entries(TAG_CLASSES)) {
  if (typeof globalThis[name] !== "undefined") continue;
  // Mirror nodes are all plain HTMLElement, so `instanceof` is answered by the
  // tag name instead of by the prototype chain — which is the ONLY way to make
  // `el instanceof HTMLInputElement` true for a real `<input>` here.
  const klass = class {};
  Object.defineProperty(klass, "name", { value: name });
  Object.defineProperty(klass, Symbol.hasInstance, {
    value: (x) => !!x && typeof x === "object" && x.tagName === tag,
  });
  defineGlobal(name, klass);
}

// -- getComputedStyle. The values come from the REAL cascade (see
//    `crates/vn-tree/src/dom.rs`): the properties reused components actually
//    read at runtime, the USED width/height off the painted rectangle, and
//    every custom property. A property nothing declared reads `""`, exactly as
//    an unsupported one would in a browser — with one advisory naming it.
const computedCache = new WeakMap();
function computedStyleFor(node) {
  // `document.body` / `documentElement` are not nodes in the retained tree;
  // they stand for the app root, so that is whose style they report.
  const id = typeof node?.id === "number" ? node.id : appRoot?.id;
  const raw = id ? __vn_computed_style(id) : "";
  const map = Object.create(null);
  for (const line of raw.split("\n")) {
    if (!line) continue;
    const i = line.indexOf("=");
    if (i > 0) map[line.slice(0, i)] = line.slice(i + 1);
  }
  const read = (k) => {
    const key = String(k);
    // Framework probes, not CSS: Vue's reactivity asks every object it touches
    // for `__v_isReadonly`/`__v_raw`, promise-detection asks for `then`. None
    // of those are properties a stylesheet can declare, so they answer
    // `undefined` (not `""`) and never warn.
    if (!/^[a-zA-Z-]/.test(key) || key.startsWith("__") || key === "then") return undefined;
    const css = key.startsWith("--") ? key : hyphenate(key);
    if (css in map) return map[css];
    warnCompat(
      `getComputedStyle:${css}`,
      `getComputedStyle().${key} reads "" — vue-native's computed style publishes the properties ` +
        `a reused component actually reads (transition-*, transform, display, position, ` +
        `overflow-*, width/height, direction) plus every custom property, not all 340 longhands`,
    );
    return "";
  };
  return new Proxy(
    { getPropertyValue: read, getPropertyPriority: () => "", get length() { return 0; } },
    {
      get: (t, k) => (k in t ? t[k] : typeof k === "string" ? read(k) : undefined),
      has: () => true,
    },
  );
}
defineGlobal("getComputedStyle", (node) => {
  let cached = computedCache.get(node);
  // One live object per element, as in a browser — but the VALUES are read at
  // access time, so it tracks the cascade instead of freezing it.
  if (!cached) {
    cached = new Proxy(
      {},
      {
        get: (_t, k) => computedStyleFor(node)[k],
        has: () => true,
      },
    );
    if (node && typeof node === "object") computedCache.set(node, cached);
  }
  return cached;
});

// -- the document surface. redjs ships a `document` object with no tree behind
//    it; these are the entry points a reused component reaches through.
const doc = globalThis.document;
if (doc) {
  const defDoc = (name, value) => {
    if (!(name in doc)) Object.defineProperty(doc, name, { value, configurable: true, writable: true });
  };
  const defDocGet = (name, get) => {
    if (!(name in doc)) Object.defineProperty(doc, name, { get, configurable: true });
  };
  // These three are FORCE-installed, not filled in: redjs ships its own
  // `querySelector`/`getElementById` backed by a registry of the elements IT
  // created, which is empty here — the retained tree is ours. Leaving the
  // engine's in place is the difference between "finds the node" and "silently
  // finds nothing", which is what made every `DialogContent` log a false
  // "requires a DialogTitle" accessibility warning.
  const forceDoc = (name, value) =>
    Object.defineProperty(doc, name, { value, configurable: true, writable: true });
  const rootId = () => (appRoot ? appRoot.id : 0);
  forceDoc("querySelector", (sel) => queryOne(sel, rootId()));
  forceDoc("querySelectorAll", (sel) => hostQuery(sel, rootId(), 0));
  forceDoc("getElementById", (id) =>
    queryOne(`[id=${JSON.stringify(String(id))}]`, rootId()),
  );
  defDoc("createDocumentFragment", () => new globalThis.DocumentFragment());
  defDoc("createTreeWalker", (root, whatToShow = 0xffffffff, filter = null) => {
    // A real pre-order walker over the mirror tree — focus traps use it to
    // collect tabbable candidates, and an empty one silently traps nothing.
    const flat = [];
    const walk = (n) => {
      for (const c of n.childNodes || []) {
        const show = c.kind === 1 ? 1 : c.kind === 2 ? 4 : 128;
        if (whatToShow & show) flat.push(c);
        walk(c);
      }
    };
    if (root) walk(root);
    let i = -1;
    const accepts = (n) => {
      if (!filter) return true;
      const fn = typeof filter === "function" ? filter : filter.acceptNode;
      return !fn || fn.call(filter, n) === 1;
    };
    return {
      root,
      currentNode: root,
      nextNode() {
        while (++i < flat.length) {
          if (accepts(flat[i])) return (this.currentNode = flat[i]);
        }
        return null;
      },
      previousNode() {
        while (--i >= 0) {
          if (accepts(flat[i])) return (this.currentNode = flat[i]);
        }
        return null;
      },
    };
  });
  defDocGet("defaultView", () => globalThis);
  defDocGet("activeElement", () => null);
  // `document.addEventListener("keydown"|"pointerdown"|…)` is how every
  // dismissable layer in the ecosystem closes itself.
  interceptGlobalTarget(doc);
  interceptGlobalTarget(globalThis);
  // The window's own viewport metrics. redjs reports 0 for these; the window IS
  // the app root here, so they are the root's painted box — and floating
  // positioning reads them.
  //
  // …EXCEPT when the windowing host is present (crates/vn-app/src/
  // window.prelude.js). Then these four are real: `inner*` is the client area
  // the host pushes through `redjs_dom_resize` (and re-pushes with every
  // trusted `resize` event), and `outer*` is `GetWindowRect` — a window fact
  // the app root cannot know, and one that is deliberately NOT equal to
  // `inner*` (docs/WINDOWING.md §1.2). Clobbering them with the root's box
  // would make `outerWidth === innerWidth` and silently break `screenX`'s
  // neighbours. The root box stays the source for `document.documentElement`
  // and `getBoundingClientRect` below, which is what floating positioning
  // measures against.
  if (!globalThis.__VN_WINDOW__) {
    for (const [name, axis] of [
      ["innerWidth", "width"],
      ["innerHeight", "height"],
      ["outerWidth", "width"],
      ["outerHeight", "height"],
    ]) {
      try {
        Object.defineProperty(globalThis, name, {
          get: () => viewportRect()[axis],
          configurable: true,
        });
      } catch {
        /* a non-configurable engine property — leave it alone */
      }
    }
  }
  for (const name of ["scrollX", "scrollY", "pageXOffset", "pageYOffset"]) {
    try {
      if (typeof globalThis[name] !== "number") {
        Object.defineProperty(globalThis, name, { value: 0, configurable: true, writable: true });
      }
    } catch {
      /* ditto */
    }
  }
  // `document.body` / `documentElement` are plain objects in redjs, and three
  // libraries (aria-hidden, reka's scroll lock, floating-ui) reach straight for
  // `.style` / `.contains` on them. Point them at the app root once it exists,
  // so those calls act on the real tree instead of on nothing.
  for (const name of ["body", "documentElement", "head"]) {
    const el = doc[name];
    if (!el || typeof el !== "object") continue;
    // `document.body.style.pointerEvents = "none"` (reka's scroll lock) and
    // `documentElement.style.removeProperty("--scrollbar-width")` are the two
    // shapes that actually turn up. redjs already gives these objects a plain
    // `style` bag with NO methods on it, so the methods are what must be filled
    // in — a missing `removeProperty` throws inside an unmount hook and leaves
    // the component half-torn-down.
    //
    // The writes are a property bag, not a style: `document.body` is not a node
    // in the retained tree, so there is nothing to apply them to.
    if (!el.style || typeof el.style !== "object") el.style = {};
    const s = el.style;
    if (typeof s.setProperty !== "function") {
      s.setProperty = (k, v) => {
        s[String(k)] = v == null ? "" : String(v);
      };
    }
    if (typeof s.removeProperty !== "function") {
      s.removeProperty = (k) => {
        const prev = s[String(k)] ?? "";
        delete s[String(k)];
        return prev;
      };
    }
    if (typeof s.getPropertyValue !== "function") s.getPropertyValue = (k) => s[String(k)] ?? "";
    if (!el.querySelector) el.querySelector = (s) => queryOne(s, appRoot ? appRoot.id : 0);
    if (!el.querySelectorAll) el.querySelectorAll = (s) => hostQuery(s, appRoot ? appRoot.id : 0, 0);
    if (!el.contains) el.contains = (n) => !!appRoot && (n === appRoot || isAncestor(appRoot, n));
    if (!el.getAttribute) el.getAttribute = () => null;
    if (!el.setAttribute) el.setAttribute = () => {};
    if (!el.hasAttribute) el.hasAttribute = () => false;
    if (!el.removeAttribute) el.removeAttribute = () => {};
    if (!el.appendChild) el.appendChild = (n) => n;
    if (!el.removeChild) el.removeChild = (n) => n;
    if (!el.insertAdjacentElement) el.insertAdjacentElement = (_w, n) => n;
    el.getBoundingClientRect = () => ({ ...viewportRect(), toJSON() { return this; } });
    if (!el.addEventListener) el.addEventListener = () => {};
    if (!el.removeEventListener) el.removeEventListener = () => {};
    // The viewport dimensions, ALL of them. @floating-ui computes the document
    // rect as `Math.max(html.scrollWidth, html.clientWidth, body.scrollWidth,
    // body.clientWidth)` — one missing member makes that NaN, and the NaN then
    // travels silently through every offset until nothing is positioned at all.
    for (const [name, axis] of [
      ["clientWidth", "width"],
      ["clientHeight", "height"],
      ["offsetWidth", "width"],
      ["offsetHeight", "height"],
      ["scrollWidth", "width"],
      ["scrollHeight", "height"],
    ]) {
      Object.defineProperty(el, name, { get: () => viewportRect()[axis], configurable: true });
    }
    for (const name of ["clientTop", "clientLeft"]) {
      Object.defineProperty(el, name, { get: () => 0, configurable: true });
    }
    // …and the scroll offsets under BOTH names. @floating-ui reads
    // `element.scrollLeft` when `element instanceof Element` and
    // `element.scrollX` when it is not — and `document.documentElement` here is
    // a plain redjs object, not an Element, so it takes the second branch. A
    // missing `scrollX` made every horizontal offset NaN while the vertical
    // ones stayed correct, which is a genuinely baffling way for positioning to
    // fail.
    for (const name of ["scrollLeft", "scrollTop", "scrollX", "scrollY"]) {
      if (typeof el[name] !== "number") el[name] = 0;
    }
    if (!("children" in el)) Object.defineProperty(el, "children", { get: () => (appRoot ? appRoot.children : []), configurable: true });
    if (!("ownerDocument" in el)) Object.defineProperty(el, "ownerDocument", { get: () => doc, configurable: true });
    if (!("parentNode" in el)) Object.defineProperty(el, "parentNode", { get: () => null, configurable: true });
  }
}

// ---- runtime-dom-shaped directives ----------------------------------------
// Standard compiled output (`<input v-model="x">` etc.) must run UNMODIFIED —
// we implement the directives against our host instead of forking the
// compiler (compat-first policy).
const assignKey = Symbol("vnModelAssign");

function setElValue(el, value) {
  const v = value == null ? "" : String(value);
  if (el.value === v) return;
  el.value = v;
  __vn_set_prop(el.id, I("value"), v);
}

export const vModelText = {
  created(el, { modifiers = {} }, vnode) {
    el[assignKey] = vnode.props && vnode.props["onUpdate:modelValue"];
    el.addEventListener(modifiers.lazy ? "change" : "input", () => {
      let v = el.value;
      if (modifiers.trim) v = v.trim();
      if (modifiers.number) {
        const n = parseFloat(v);
        if (!Number.isNaN(n)) v = n;
      }
      if (el[assignKey]) el[assignKey](v);
    });
  },
  mounted(el, { value }) {
    setElValue(el, value);
  },
  beforeUpdate(el, { value }, vnode) {
    el[assignKey] = vnode.props && vnode.props["onUpdate:modelValue"];
    setElValue(el, value);
  },
};

// v1: every input renders as a text field, so the dynamic/checkbox/radio/
// select variants fall back to the text behavior with an advisory.
function modelFallback(kind) {
  return {
    created(el, binding, vnode) {
      warnCompat(
        `v-model:${kind}`,
        `v-model on ${kind} inputs is not implemented natively yet — falling back to text ` +
          `semantics; use modelValue/onUpdate:modelValue on a component meanwhile`,
      );
      vModelText.created(el, binding, vnode);
    },
    mounted: vModelText.mounted,
    beforeUpdate: vModelText.beforeUpdate,
  };
}
export const vModelDynamic = modelFallback("dynamic");
export const vModelCheckbox = modelFallback("checkbox");
export const vModelRadio = modelFallback("radio");
export const vModelSelect = modelFallback("select");

export const vShow = {
  mounted(el, { value }) {
    __vn_set_style(el.id, I("display"), value ? "" : "none");
  },
  updated(el, { value, oldValue }) {
    if (!value === !oldValue) return;
    __vn_set_style(el.id, I("display"), value ? "" : "none");
  },
};

// ---- template helpers (compiler-dom emits these for @event modifiers) -----
const modifierGuards = {
  stop: (e) => e.stopPropagation(),
  prevent: (e) => e.preventDefault(),
  self: (e) => e.target !== e.currentTarget,
  ctrl: (e) => !e.ctrlKey,
  shift: (e) => !e.shiftKey,
  alt: (e) => !e.altKey,
  meta: (e) => !e.metaKey,
  left: (e) => "button" in e && e.button !== 0,
  middle: (e) => "button" in e && e.button !== 1,
  right: (e) => "button" in e && e.button !== 2,
  exact: (e, m) => ["ctrl", "shift", "alt", "meta"].some((x) => e[x + "Key"] && !m.includes(x)),
};

export function withModifiers(fn, modifiers) {
  return (event, ...args) => {
    for (const m of modifiers) {
      const guard = modifierGuards[m];
      if (guard && guard(event, modifiers)) return;
    }
    return fn(event, ...args);
  };
}

const keyAliases = {
  esc: "escape",
  space: " ",
  up: "arrow-up",
  left: "arrow-left",
  right: "arrow-right",
  down: "arrow-down",
  delete: "backspace",
};

export function withKeys(fn, keys) {
  return (event) => {
    if (!("key" in event)) return;
    const eventKey = hyphenate(String(event.key));
    if (keys.some((k) => k === eventKey || keyAliases[k] === eventKey)) return fn(event);
  };
}

// ---- windowing (docs/WINDOWING.md tier 3) ---------------------------------
//
// `appWindow` itself is built in the host's JS prelude (crates/vn-app/src/
// window.prelude.js) because the host observer, the state snapshot and the
// standard tier-1 globals all have to exist before the app bundle evaluates.
// This module is only its NAMESPACE: apps say
//
//     import { appWindow } from "vue-native";
//
// rather than reaching for a global, which is what keeps the extension
// namespaced (tier 3) instead of pretending to be a web API.
//
// `getCurrentWindow()` exists only so ported Tauri code runs unmodified. When
// multi-window lands it keeps meaning "this window" and `appWindow` keeps being
// its binding — neither gets renamed.
const windowHost = globalThis.__VN_WINDOW__;
// The advisory is LAZY, not eager: a host without the window prelude (the unit
// test harness, a future embedder) must not be scolded for an API it never
// touches — an advisory that fires on code nobody wrote is noise.
const missing = () =>
  warnCompat(
    "appWindow",
    "the windowing host is missing (crates/vn-app/src/window.prelude.js did not evaluate) — " +
      "`appWindow` is inert. See docs/WINDOWING.md.",
  );
const inertWindow = new Proxy(
  {},
  {
    get(_t, key) {
      missing();
      // `isMaximized()` and friends must still answer a boolean, and every
      // verb must still be callable, so inert code paths do not throw.
      return typeof key === "string" && key.startsWith("is") ? () => false : () => undefined;
    },
    set() {
      missing();
      return true;
    },
  },
);

export const appWindow = windowHost ? windowHost.appWindow : inertWindow;
export const defineWindow = windowHost ? windowHost.defineWindow : missing;
export const getCurrentWindow = () => appWindow;
// ---- <Transition> / <TransitionGroup> (compat roadmap G9) ------------------
// runtime-core exports BaseTransition (the state machine) but NOT these two —
// they live in runtime-dom, because the CSS half is DOM-specific. Libraries
// import them from "vue" unconditionally (@vueuse/core does, at MODULE SCOPE),
// so their absence is `undefined` in dev and a hard build failure in prod.
//
// This is runtime-dom's implementation, ported onto our host: the same class
// phases, the same hook shapes, the same `mode`/`appear` handling (which is
// BaseTransition's, so it comes for free), riding vn-css's REAL transition
// engine. One divergence, stated once at runtime: there is no `transitionend`
// event, so the end of a transition is detected purely by the timer runtime-dom
// itself already uses as its fallback — read off the element's computed
// `transition-duration`/`transition-delay`.

const TRANSITION_PROPS = {
  name: String,
  type: String,
  css: { type: Boolean, default: true },
  duration: [String, Number, Object],
  enterFromClass: String,
  enterActiveClass: String,
  enterToClass: String,
  appearFromClass: String,
  appearActiveClass: String,
  appearToClass: String,
  leaveFromClass: String,
  leaveActiveClass: String,
  leaveToClass: String,
};

// runtime-dom adds the phase class to `el.classList` AND records it in `_vtc`,
// because in a browser those are two views of the same attribute. Here they are
// two SOURCES for one attribute (`writeClass` joins them), so recording it once
// is the whole job — adding it to `className` as well would emit it twice.
function addTransitionClass(el, cls) {
  (el._vtc || (el._vtc = new Set())).add(cls);
  writeClass(el);
}
function removeTransitionClass(el, cls) {
  if (el._vtc) {
    el._vtc.delete(cls);
    if (!el._vtc.size) el._vtc = undefined;
  }
  writeClass(el);
}

/// Two frames, as runtime-dom does it: one to let the *-from class be committed
/// and cascaded, the next to swap in *-to so the engine sees a real change.
function nextFrame(cb) {
  const raf = globalThis.requestAnimationFrame;
  if (typeof raf === "function") raf(() => raf(cb));
  else setTimeout(() => setTimeout(cb, 0), 0);
}

const toMs = (s) => (s === "auto" || !s ? 0 : Number(String(s).slice(0, -1).replace(",", ".")) * 1000);
function listTimeout(delays, durations) {
  while (delays.length < durations.length) delays = delays.concat(delays);
  return Math.max(0, ...durations.map((d, i) => toMs(d) + toMs(delays[i])));
}

/// How long the element's CSS says its transition takes. Same computation
/// runtime-dom does, off the same two properties.
function transitionTimeout(el) {
  const styles = globalThis.getComputedStyle(el);
  const split = (k) => String(styles[k] || "").split(", ");
  return listTimeout(split("transitionDelay"), split("transitionDuration"));
}

let endId = 0;
/// End a `<Transition>` phase on the REAL `transitionend`, with the computed
/// timeout as the safety net — which is exactly runtime-dom's own strategy, and
/// is now possible because vn-css's animator sources the event (m12 item 6).
///
/// The event matters rather than merely being tidier: the timer reports the
/// declared duration whatever actually happened, so a transition the engine cut
/// short (reduced motion, a competing write to the same property, the element
/// being restyled mid-flight) left the phase class on for its full nominal
/// length. The event fires when the track actually settles.
///
/// The net is still armed, and for the same two reasons runtime-dom arms it: an
/// element whose transition never starts (the property did not actually change)
/// fires nothing at all, and a property that is not animatable here settles
/// without an event.
function whenTransitionEnds(el, explicitTimeout, resolve) {
  const id = (el._endId = ++endId);
  let done = false;
  const resolveIfNotStale = () => {
    if (done || id !== el._endId) return;
    done = true;
    el.removeEventListener?.("transitionend", onEnd);
    resolve();
  };
  const onEnd = (e) => {
    // Only this element's own transitions end this phase — a descendant's
    // bubbles through, and runtime-dom filters it the same way.
    if (e.target === el) resolveIfNotStale();
  };
  if (explicitTimeout != null) {
    setTimeout(resolveIfNotStale, explicitTimeout);
    return;
  }
  const timeout = transitionTimeout(el);
  if (!timeout) {
    resolve();
    return;
  }
  el.addEventListener?.("transitionend", onEnd);
  setTimeout(resolveIfNotStale, timeout + 1);
}

const callHook = (hook, args = []) => {
  if (Array.isArray(hook)) for (const h2 of hook) h2(...args);
  else if (hook) hook(...args);
};
const hasExplicitCallback = (hook) =>
  hook ? (Array.isArray(hook) ? hook.some((h2) => h2.length > 1) : hook.length > 1) : false;

function normalizeDuration(duration) {
  if (duration == null) return null;
  if (typeof duration === "object") return [Number(duration.enter), Number(duration.leave)];
  const n = Number(duration);
  return [n, n];
}

export function resolveTransitionProps(rawProps) {
  const baseProps = {};
  for (const key in rawProps) if (!(key in TRANSITION_PROPS)) baseProps[key] = rawProps[key];
  if (rawProps.css === false) return baseProps;
  const {
    name = "v",
    duration,
    enterFromClass = `${name}-enter-from`,
    enterActiveClass = `${name}-enter-active`,
    enterToClass = `${name}-enter-to`,
    appearFromClass = enterFromClass,
    appearActiveClass = enterActiveClass,
    appearToClass = enterToClass,
    leaveFromClass = `${name}-leave-from`,
    leaveActiveClass = `${name}-leave-active`,
    leaveToClass = `${name}-leave-to`,
  } = rawProps;
  const durations = normalizeDuration(duration);
  const enterDuration = durations && durations[0];
  const leaveDuration = durations && durations[1];
  const {
    onBeforeEnter,
    onEnter,
    onEnterCancelled,
    onLeave,
    onLeaveCancelled,
    onBeforeAppear = onBeforeEnter,
    onAppear = onEnter,
    onAppearCancelled = onEnterCancelled,
  } = baseProps;

  const finishEnter = (el, isAppear, done) => {
    removeTransitionClass(el, isAppear ? appearToClass : enterToClass);
    removeTransitionClass(el, isAppear ? appearActiveClass : enterActiveClass);
    done && done();
  };
  const finishLeave = (el, done) => {
    el._isLeaving = false;
    removeTransitionClass(el, leaveFromClass);
    removeTransitionClass(el, leaveToClass);
    removeTransitionClass(el, leaveActiveClass);
    done && done();
  };
  const makeEnterHook = (isAppear) => (el, done) => {
    const hook = isAppear ? onAppear : onEnter;
    const resolve = () => finishEnter(el, isAppear, done);
    callHook(hook, [el, resolve]);
    nextFrame(() => {
      removeTransitionClass(el, isAppear ? appearFromClass : enterFromClass);
      addTransitionClass(el, isAppear ? appearToClass : enterToClass);
      if (!hasExplicitCallback(hook)) whenTransitionEnds(el, enterDuration, resolve);
    });
  };

  return Object.assign(baseProps, {
    onBeforeEnter(el) {
      callHook(onBeforeEnter, [el]);
      addTransitionClass(el, enterFromClass);
      addTransitionClass(el, enterActiveClass);
    },
    onBeforeAppear(el) {
      callHook(onBeforeAppear, [el]);
      addTransitionClass(el, appearFromClass);
      addTransitionClass(el, appearActiveClass);
    },
    onEnter: makeEnterHook(false),
    onAppear: makeEnterHook(true),
    onLeave(el, done) {
      el._isLeaving = true;
      const resolve = () => finishLeave(el, done);
      addTransitionClass(el, leaveFromClass);
      addTransitionClass(el, leaveActiveClass);
      nextFrame(() => {
        if (!el._isLeaving) return;
        removeTransitionClass(el, leaveFromClass);
        addTransitionClass(el, leaveToClass);
        if (!hasExplicitCallback(onLeave)) whenTransitionEnds(el, leaveDuration, resolve);
      });
      callHook(onLeave, [el, resolve]);
    },
    onEnterCancelled(el) {
      finishEnter(el, false, undefined);
      callHook(onEnterCancelled, [el]);
    },
    onAppearCancelled(el) {
      finishEnter(el, true, undefined);
      callHook(onAppearCancelled, [el]);
    },
    onLeaveCancelled(el) {
      finishLeave(el);
      callHook(onLeaveCancelled, [el]);
    },
  });
}

export const Transition = /*@__PURE__*/ defineComponent({
  name: "Transition",
  displayName: "Transition",
  inheritAttrs: false,
  props: /*@__PURE__*/ Object.assign({}, BaseTransitionPropsValidators, TRANSITION_PROPS),
  setup(props, { slots }) {
    return () => h(BaseTransition, resolveTransitionProps(props), slots);
  },
});

export const TransitionGroup = /*@__PURE__*/ defineComponent({
  name: "TransitionGroup",
  props: /*@__PURE__*/ Object.assign({}, BaseTransitionPropsValidators, TRANSITION_PROPS, {
    tag: String,
    moveClass: String,
  }),
  setup(props, { slots }) {
    const instance = getCurrentInstance();
    const state = useTransitionState();
    let children;
    return () => {
      const rawProps = toRaw(props);
      const cssTransitionProps = resolveTransitionProps(rawProps);
      const tag = rawProps.tag || Fragment;
      if (rawProps.moveClass || (children && children.length)) {
        warnCompat(
          "TransitionGroup:move",
          "<TransitionGroup> animates enter and leave, but NOT the move of the items that stay " +
            "(the FLIP technique needs a per-element transform, and gpui applies no matrix to a " +
            "laid out box) — `moveClass` is ignored and reordered items jump to their new place",
        );
      }
      children = slots.default ? getTransitionRawChildren(slots.default()) : [];
      for (const child of children) {
        if (child.key != null) {
          setTransitionHooks(
            child,
            resolveTransitionHooks(child, cssTransitionProps, state, instance),
          );
        }
      }
      return createVNode(tag, null, children);
    };
  },
});

// Compiled SFC code and app code import everything Vue-ish from here
// ("vue" / "vue-native" are aliased to this module). Local exports win over
// the star re-export, so our createApp shadows runtime-core's.
export * from "@vue/runtime-core";
