// vue-native JS runtime, v0: @vue/runtime-core custom renderer over the
// __vn_* host protocol (see crates/vn-tree). One bundle, classic script.
// NOTE: the three __VUE_* feature flags are set by a prelude evaluated before
// this bundle (bun refuses to inline them — verified in research).

import { createRenderer, h, ref, version } from "@vue/runtime-core";

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

const EV_CODES = { click: 1 };

// ---- JS mirror nodes ------------------------------------------------------
// Vue writes symbol props onto host nodes and needs parentNode/nextSibling
// synchronously — both served from this mirror, zero host calls.
function mk(id, kind, tag) {
  return { id, kind, tag, parentNode: null, children: [] };
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
    for (const c of node.children) c.parentNode = null;
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
    forEachDeep(child, (n) => handlers.delete(n.id));
    __vn_remove(child.id);
  },
  parentNode: (node) => node.parentNode,
  nextSibling: (node) => {
    const p = node.parentNode;
    if (!p) return null;
    const i = p.children.indexOf(node);
    return i >= 0 ? (p.children[i + 1] ?? null) : null;
  },
};

function patchProp(el, key, prev, next) {
  if (key === "style") {
    const p = prev || {};
    const n = next || {};
    for (const k in p) if (!(k in n)) __vn_set_style(el.id, I(k), "");
    for (const k in n) if (p[k] !== n[k]) __vn_set_style(el.id, I(k), String(n[k]));
  } else if (key.startsWith("on")) {
    const name = key.slice(2).toLowerCase();
    const code = EV_CODES[name];
    if (!code) return;
    let m = handlers.get(el.id);
    if (!m) {
      m = {};
      handlers.set(el.id, m);
    }
    m[name] = next || undefined;
    if (!!prev !== !!next) __vn_set_listener(el.id, code, !!next);
  }
  // v0: class/attrs are dropped; the CSS layer will own them.
}

__vn_set_dispatcher((nodeId, evCode) => {
  const name = evCode === EV_CODES.click ? "click" : null;
  if (!name) return;
  const fn = handlers.get(nodeId)?.[name];
  if (fn) fn({ type: name, target: nodeId });
});

const renderer = createRenderer({ ...nodeOps, patchProp });

// ---- demo app: a reactive counter ----------------------------------------
const App = {
  setup() {
    const count = ref(0);
    return () =>
      h(
        "div",
        {
          style: {
            display: "flex",
            flexDirection: "column",
            gap: "12px",
            padding: "16px",
            background: "#ffffff14",
            borderRadius: "12px",
          },
        },
        [
          h(
            "div",
            { style: { fontSize: "15px", color: "#ffffff" } },
            `Vue ${version} says: count = ${count.value}`,
          ),
          h(
            "div",
            {
              style: {
                padding: "10px",
                width: "140px",
                background: "#4f8cff59",
                borderRadius: "8px",
                fontSize: "13px",
                color: "#ffffff",
              },
              onClick: () => count.value++,
            },
            "Click me (+1)",
          ),
          count.value > 0 && count.value % 5 === 0
            ? h(
                "div",
                { style: { fontSize: "12px", color: "#a5f3a5" } },
                `conditional branch: ${count.value} is divisible by 5`,
              )
            : null,
        ],
      );
  },
};

const root = mk(__vn_create_root(), 1, "root");
renderer.createApp(App).mount(root);
__vn_log(`Vue ${version} mounted through the __vn_* renderer`);
