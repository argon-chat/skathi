// vue-native JS runtime: @vue/runtime-core custom renderer over the __vn_*
// host protocol (crates/vn-tree). Apps import `createApp` from "vue-native";
// compiled SFC render functions import their helpers from "vue" — both are
// aliased to this module by the bundler (packages/vue-native/build.ts).

import { createRenderer, version } from "@vue/runtime-core";

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
};
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
const ElementProto = typeof HTMLElement === "function" ? HTMLElement.prototype : Object.prototype;

function mk(id, kind, tag) {
  const n = Object.create(kind === 1 ? ElementProto : Object.prototype);
  n.id = id;
  n.kind = kind;
  n.tag = tag;
  n.parentNode = null;
  n.children = [];
  if (kind === 1) {
    n.tagName = String(tag).toUpperCase();
    n.value = "";
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
  for (const c of n.children) forEachDeep(c, f);
}

const handlers = new Map(); // node id -> { click: fn }

const nodeOps = {
  createElement: (type) => mk(__vn_create_element(I(type)), 1, type),
  createText: (text) => {
    const n = mk(__vn_create_text(), 2, "#text");
    __vn_set_text(n.id, String(text));
    return n;
  },
  createComment: () => mk(__vn_create_anchor(), 3, "#comment"),
  setText: (node, text) => __vn_set_text(node.id, String(text)),
  setElementText: (node, text) => {
    for (const c of node.children) {
      c.parentNode = null;
      forEachDeep(c, (n) => {
        handlers.delete(n.id);
        nodesById.delete(n.id);
      });
    }
    node.children.length = 0;
    __vn_set_element_text(node.id, String(text));
  },
  insert: (child, parent, anchor) => {
    // insert is also move: detach from the old mirror parent first.
    if (child.parentNode) {
      const cs = child.parentNode.children;
      const i = cs.indexOf(child);
      if (i >= 0) cs.splice(i, 1);
    }
    let idx = parent.children.length;
    if (anchor != null) {
      const i = parent.children.indexOf(anchor);
      if (i >= 0) idx = i;
    }
    parent.children.splice(idx, 0, child);
    child.parentNode = parent;
    __vn_insert(child.id, parent.id, anchor ? anchor.id : 0);
  },
  remove: (child) => {
    const p = child.parentNode;
    if (p) {
      const i = p.children.indexOf(child);
      if (i >= 0) p.children.splice(i, 1);
      child.parentNode = null;
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
    const i = p.children.indexOf(node);
    return i >= 0 ? (p.children[i + 1] ?? null) : null;
  },
  // <style scoped>: Vue stamps every element a scoped component renders (and
  // child component roots) with its data-v id — the Rust matcher buckets on it.
  setScopeId: (el, id) => __vn_add_scope(el.id, id),
};

// @vue/shared's hyphenate, verbatim: `\B` so a leading capital is only
// lowercased, never prefixed. Style keys are camelCase either way, but
// `withKeys` runs it over `KeyboardEvent.key` — "Enter" must become "enter",
// not "-enter", or every `@keydown.enter` silently stops matching.
const hyphenate = (s) => s.replace(/\B([A-Z])/g, "-$1").toLowerCase();

function patchProp(el, key, prev, next) {
  if (key === "style") {
    const p = prev || {};
    const n = next || {};
    for (const k in p) if (!(k in n)) __vn_set_style(el.id, I(hyphenate(k)), "");
    for (const k in n) if (p[k] !== n[k]) __vn_set_style(el.id, I(hyphenate(k)), String(n[k]));
  } else if (key === "class") {
    if (prev !== next) __vn_set_class(el.id, next == null ? "" : String(next));
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
    const code = EV_CODES[name];
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
    syncListenerFlag(el.id, name, buckets);
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
    if (key === "value") el.value = next == null ? "" : String(next);
  }
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

function syncListenerFlag(id, name, buckets) {
  const code = EV_CODES[name];
  if (code) __vn_set_listener(id, code, bucketsAlive(buckets));
}

// EventTarget-shaped registration for reused browser components/directives
// (vModelText uses this path). Merged into the same propagation walk.
function addExtraListener(node, rawName, fn, opts) {
  if (!fn) return;
  const name = canonicalEvent(rawName);
  const code = EV_CODES[name];
  if (!code) {
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
  syncListenerFlag(node.id, name, buckets);
}

function removeExtraListener(node, rawName, fn, opts) {
  const name = canonicalEvent(rawName);
  const buckets = handlers.get(node.id)?.[name];
  if (!buckets) return;
  const capture = opts === true || (opts && opts.capture);
  const arr = capture ? buckets.extraC : buckets.extraB;
  const i = arr.findIndex((e) => e.fn === fn);
  if (i >= 0) arr.splice(i, 1);
  syncListenerFlag(node.id, name, buckets);
}

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

function synthesize(name, a, b, c, d, key) {
  const common = { bubbles: !NO_BUBBLE.has(name), cancelable: !NO_BUBBLE.has(name) };
  const M = mods(d);
  try {
    switch (name) {
      case "click":
      case "mousedown":
      case "mouseup":
        return new MouseEvent(name, { ...common, ...M, clientX: a, clientY: b, button: c });
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
          : name === "focus" || name === "blur" || name === "scroll"
            ? {}
            : name === "complete" || name === "loopComplete" || name === "enterFrame"
              ? lottieDetail(name, a, b, c, d)
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
  syncListenerFlag(node.id, name, buckets);
}

function propagate(target, name, ev) {
  const path = [];
  for (let n = target; n; n = n.parentNode) path.push(n);
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
    const buckets = handlers.get(node.id)?.[name];
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
  // input/change: the payload string is the new value — mirror it onto the
  // node BEFORE dispatch so `e.target.value` reads like the browser.
  if ((name === "input" || name === "change") && key != null) target.value = key;
  // scroll: same idea — the position lives on the element, not on the event.
  if (name === "scroll") {
    target.scrollLeft = a;
    target.scrollTop = b;
  }
  const ev = synthesize(name, a, b, c, d, key);
  return propagate(target, name, ev);
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
    };
    __vn_log(`Vue ${version} app mounted (vue-native runtime)`);
    return vm;
  };
  return app;
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

// Compiled SFC code and app code import everything Vue-ish from here
// ("vue" / "vue-native" are aliased to this module). Local exports win over
// the star re-export, so our createApp shadows runtime-core's.
export * from "@vue/runtime-core";
