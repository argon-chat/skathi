// The host's inline and computed style (crates/sk-tree/src/style_decl.rs, inline_style.rs) for the bun harness:
// the same declarations, attribute text and hook calls, logging each declaration change as the `__sk_set_style`
// op the tree applies.

interface Decl {
  value: string;
  important: boolean;
}
interface Inline {
  decls: Map<string, Decl>;
  present: boolean;
  raw: string | null;
}

export interface StyleHostOptions {
  /** A declaration the tree applies: `""` removes, an important value ends in ` !important`. */
  op(id: number, name: string, value: string): void;
  /** `name=value` lines for a node's computed style, `""` when the host has none. */
  computed(id: number): string;
  /** The root's id, for a computed style of something that is not a node. */
  root(): number;
  warnCompat(key: string, msg: string): void;
}

const hyphenate = (s: string): string => s.replace(/\B([A-Z])/g, "-$1").toLowerCase();
function cssName(k: string): string {
  if (k.startsWith("--")) return k;
  if (k === "cssFloat") return "float";
  if (k.includes("-")) return k.toLowerCase();
  const h = hyphenate(k);
  return /^(webkit|moz|ms)-/.test(h) ? "-" + h : h;
}
const cssomName = (k: string): string => (k.startsWith("--") ? k : k.toLowerCase());
const IMPORTANT_RE = /\s*!\s*important\s*$/i;
const PROP_NAME_RE = /^(?:--[^\s:;]+|-?[A-Za-z_\u0080-￿][\w\u0080-￿-]*)$/;
const storedValue = (v: string, custom: boolean): string => {
  const t = v.trim();
  return t === "" && custom ? " " : t;
};
const MEMBERS = new Set([
  "cssText",
  "cssFloat",
  "length",
  "parentRule",
  "item",
  "getPropertyValue",
  "getPropertyPriority",
  "setProperty",
  "removeProperty",
  "constructor",
  "hasOwnProperty",
  "isPrototypeOf",
  "propertyIsEnumerable",
  "toLocaleString",
  "toString",
  "valueOf",
  "then",
]);
const cssShaped = (k: string): boolean =>
  k.startsWith("--") ? k.length > 2 : /^-?[A-Za-z][A-Za-z0-9-]*$/.test(k) && !MEMBERS.has(k);

function parseDeclarations(text: string): Map<string, Decl> {
  const out = new Map<string, Decl>();
  let seg = "";
  let depth = 0;
  let quote = "";
  const flush = () => {
    const colon = seg.indexOf(":");
    if (colon > 0) {
      const raw = seg.slice(0, colon).trim();
      let value = seg.slice(colon + 1);
      const m = IMPORTANT_RE.exec(value);
      const important = !!m;
      if (m) value = value.slice(0, m.index);
      value = value.trim();
      const custom = raw.startsWith("--");
      if (PROP_NAME_RE.test(raw) && (value !== "" || custom)) {
        const name = cssomName(raw);
        const prev = out.get(name);
        if (!(prev?.important && !important)) {
          out.delete(name);
          out.set(name, { value: storedValue(value, custom), important });
        }
      }
    }
    seg = "";
  };
  for (let i = 0; i < text.length; i++) {
    const c = text[i]!;
    if (quote) {
      seg += c;
      if (c === "\\" && i + 1 < text.length) seg += text[++i];
      else if (c === quote) quote = "";
      continue;
    }
    if (c === "/" && text[i + 1] === "*") {
      const end = text.indexOf("*/", i + 2);
      i = end < 0 ? text.length : end + 1;
      seg += " ";
      continue;
    }
    if (c === "\\" && i + 1 < text.length) {
      seg += c + text[++i];
      continue;
    }
    if (c === '"' || c === "'") quote = c;
    else if (c === "(" || c === "[" || c === "{") depth++;
    else if ((c === ")" || c === "]" || c === "}") && depth > 0) depth--;
    else if (c === ";" && depth === 0) {
      flush();
      continue;
    }
    seg += c;
  }
  flush();
  return out;
}

export function styleHost(o: StyleHostOptions) {
  const inline = new Map<number, Inline>();
  let hook: ((id: number, old: string | null, present: boolean) => void) | null = null;
  let observing = false;
  const of = (id: number): Inline => {
    let st = inline.get(id);
    if (!st) inline.set(id, (st = { decls: new Map(), present: false, raw: null }));
    return st;
  };
  const host = (d: Decl): string => (d.important ? `${d.value} !important` : d.value);
  const text = (id: number): string =>
    [...(inline.get(id)?.decls ?? [])].map(([n, d]) => `${n}: ${d.value}${d.important ? " !important" : ""};`).join(" ");
  const attr = (id: number): string | null => {
    const st = inline.get(id);
    if (!st?.present) return null;
    return st.raw ?? text(id);
  };
  const noteSerialization = () =>
    o.warnCompat(
      "inline-style-serialize",
      "el.style.cssText / getAttribute('style') serialize the inline declarations as they were written: values are " +
        "not re-spelled (`#f00` stays `#f00`), a shorthand is not expanded into its longhands (after `margin: 0`, " +
        '`el.style.marginTop` reads ""), and a value the engine rejects is still listed',
    );
  const mutate = (id: number, change: (st: Inline) => boolean): void => {
    if (!id) return;
    const st = of(id);
    const before = observing ? attr(id) : null;
    const was = st.present;
    if (!change(st)) return;
    if ((observing || st.present !== was) && hook) hook(id, before, st.present);
  };
  const write = (id: number, name: string, value: string, important: boolean) =>
    mutate(id, (st) => {
      if (value === "") return remove(id, st, name) !== null;
      const prev = st.decls.get(name);
      if (prev && prev.value === value && prev.important === important) return false;
      const d = { value, important };
      st.decls.set(name, d);
      o.op(id, name, host(d));
      st.present = true;
      st.raw = null;
      return true;
    });
  const remove = (id: number, st: Inline, name: string): string | null => {
    const prev = st.decls.get(name);
    if (!prev) return null;
    st.decls.delete(name);
    o.op(id, name, "");
    st.present = true;
    st.raw = null;
    return prev.value;
  };
  const replace = (id: number, textIn: string, asAttribute: boolean) =>
    mutate(id, (st) => {
      const next = parseDeclarations(textIn);
      const old = st.decls;
      const order = [...old.keys()].filter((k) => next.has(k));
      for (const k of next.keys()) if (!old.has(k)) order.push(k);
      let i = 0;
      let inPlace = true;
      for (const k of next.keys()) if (order[i++] !== k) inPlace = false;
      for (const k of old.keys()) if (!inPlace || !next.has(k)) o.op(id, k, "");
      for (const [k, d] of next) {
        const p = old.get(k);
        if (inPlace && p && p.value === d.value && p.important === d.important) continue;
        o.op(id, k, host(d));
      }
      st.decls = next;
      st.present = true;
      st.raw = asAttribute ? textIn : null;
      return true;
    });
  const assign = (id: number, key: string, v: unknown) => {
    if (v === undefined) return;
    const name = cssName(key);
    const custom = name.startsWith("--");
    if (custom) {
      o.warnCompat(
        "style-custom-assign",
        `el.style["${name}"] = … sets the custom property here, but in a browser it is only an expando on the ` +
          `declaration object and changes nothing; use el.style.setProperty("${name}", …), which works in both`,
      );
    }
    const value = v === null ? "" : String(v);
    if (value === "") mutate(id, (st) => remove(id, st, name) !== null);
    else if (custom || !IMPORTANT_RE.test(value)) write(id, name, storedValue(value, custom), false);
  };
  const computedRead = (id: number | null, css: string, shown: string): string => {
    if (!id) return "";
    const raw = o.computed(id);
    const head = css + "=";
    const at = raw.startsWith(head) ? 0 : raw.indexOf("\n" + head);
    if (at >= 0) {
      const from = at + head.length + (at === 0 ? 0 : 1);
      const end = raw.indexOf("\n", from);
      return end < 0 ? raw.slice(from) : raw.slice(from, end);
    }
    if (raw && !css.startsWith("--")) {
      o.warnCompat(
        `getComputedStyle:${css}`,
        `getComputedStyle().${shown} reads "" — skathi's computed style publishes the properties a reused ` +
          "component actually reads (transition-*, animation-*, transform, display, position, overflow-*, " +
          "width/height, direction, font-size, color) plus every custom property, not all 340 longhands",
      );
    }
    return "";
  };
  const readOnly = (what: string) =>
    new DOMException(
      `Failed to set the '${what}' property on 'CSSStyleDeclaration': These styles are computed, and therefore the '${what}' property is read-only.`,
      "NoModificationAllowedError",
    );

  const nodes = new WeakMap<object, { node: unknown; computed: boolean }>();
  const self = (t: unknown) => {
    const s = nodes.get(t as object);
    if (!s) throw new TypeError("Illegal invocation");
    return s;
  };
  const idOf = (s: { node: unknown; computed: boolean }): number | null => {
    const id = (s.node as { __skId?: unknown } | null)?.__skId;
    if (typeof id === "number") return id > 0 ? id : null;
    return s.computed ? o.root() || null : null;
  };
  const names = (s: { node: unknown; computed: boolean }): string[] => {
    const id = idOf(s);
    if (!id) return [];
    if (!s.computed) return [...(inline.get(id)?.decls.keys() ?? [])];
    return o
      .computed(id)
      .split("\n")
      .filter(Boolean)
      .map((l) => l.slice(0, l.indexOf("=")));
  };
  const value = (s: { node: unknown; computed: boolean }, css: string, shown: string): string =>
    s.computed ? computedRead(idOf(s), css, shown) : (inline.get(idOf(s) ?? 0)?.decls.get(css)?.value ?? "");

  class CSSStyleDeclaration {
    constructor() {
      throw new TypeError("Illegal constructor");
    }
    getPropertyValue(k: unknown) {
      const s = self(this);
      return value(s, cssomName(String(k)), String(k));
    }
    getPropertyPriority(k: unknown) {
      const s = self(this);
      return !s.computed && inline.get(idOf(s) ?? 0)?.decls.get(cssomName(String(k)))?.important ? "important" : "";
    }
    setProperty(k: unknown, v: unknown, priority?: unknown) {
      const s = self(this);
      const name = cssomName(String(k));
      if (s.computed) throw readOnly(name);
      const id = idOf(s);
      const custom = name.startsWith("--");
      if ((v === undefined && !custom) || !id) return;
      const val = v == null ? "" : String(v);
      if (val === "") return mutate(id, (st) => remove(id, st, name) !== null);
      const p = priority == null ? "" : String(priority);
      if (p !== "" && p.toLowerCase() !== "important") return;
      if (!custom && IMPORTANT_RE.test(val)) return;
      write(id, name, storedValue(val, custom), p !== "");
    }
    removeProperty(k: unknown) {
      const s = self(this);
      const name = cssomName(String(k));
      if (s.computed) throw readOnly(name);
      const id = idOf(s);
      let prev = "";
      if (id)
        mutate(id, (st) => {
          const p = remove(id, st, name);
          if (p === null) return false;
          prev = p;
          return true;
        });
      return prev;
    }
    item(i: unknown) {
      return names(self(this))[Number(i)] ?? "";
    }
    get length() {
      return names(self(this)).length;
    }
    get parentRule() {
      self(this);
      return null;
    }
    get cssText() {
      const s = self(this);
      const id = idOf(s);
      const t = !s.computed && id ? text(id) : "";
      if (t) noteSerialization();
      return t;
    }
    set cssText(v: unknown) {
      const s = self(this);
      if (s.computed) throw readOnly("cssText");
      const id = idOf(s);
      if (id) replace(id, v == null ? "" : String(v), false);
    }
    get cssFloat() {
      const s = self(this);
      return s.computed ? "" : value(s, "float", "cssFloat");
    }
    set cssFloat(v: unknown) {
      const s = self(this);
      if (s.computed) throw readOnly("cssFloat");
      const id = idOf(s);
      if (id) assign(id, "float", v);
    }
  }
  Object.defineProperty(CSSStyleDeclaration.prototype, Symbol.iterator, {
    value: Array.prototype.values,
    writable: true,
    configurable: true,
  });

  const create = (node: unknown, computed: boolean): object => {
    const target = Object.create(CSSStyleDeclaration.prototype) as object;
    const state = { node, computed };
    const proxy = new Proxy(target, {
      get: (t, k, r) => {
        if (typeof k === "symbol" || k in t) return Reflect.get(t, k, r);
        if (/^\d+$/.test(k)) return names(state)[Number(k)];
        return cssShaped(k) ? value(state, cssName(k), k) : undefined;
      },
      set: (t, k, v, r) => {
        if (typeof k === "symbol" || k in t || !cssShaped(k)) return Reflect.set(t, k, v, r);
        if (/^\d+$/.test(k)) return true;
        if (state.computed) throw readOnly(k);
        const id = idOf(state);
        if (id) assign(id, k, v);
        return true;
      },
      has: (t, k) => k in t || (typeof k === "string" && cssShaped(k)),
      ownKeys: (t) => [...Reflect.ownKeys(t), ...names(state).map((_, i) => String(i))],
      getOwnPropertyDescriptor: (t, k) =>
        typeof k === "string" && /^\d+$/.test(k) && Number(k) < names(state).length
          ? { value: names(state)[Number(k)], enumerable: true, configurable: true, writable: false }
          : Reflect.getOwnPropertyDescriptor(t, k),
    });
    nodes.set(proxy, state);
    return proxy;
  };

  return {
    CSSStyleDeclaration,
    fns: {
      __sk_style_object: (node: object) => create(node, false),
      __sk_computed_object: (node: unknown) => create(node, true),
      __sk_style_assign(id: number, key: string, v: string) {
        const name = cssName(key);
        if (v === "") return mutate(id, (st) => remove(id, st, name) !== null);
        if (name.startsWith("--")) return write(id, name, storedValue(v, true), false);
        const m = /\s*!important$/.exec(v);
        if (!m) return write(id, name, storedValue(v, false), false);
        const bare = v.slice(0, m.index);
        if (bare.trim() === "") mutate(id, (st) => remove(id, st, name) !== null);
        else write(id, name, storedValue(bare, false), true);
      },
      __sk_style_get: (id: number, name: string) => inline.get(id)?.decls.get(name)?.value ?? "",
      __sk_style_replace: (id: number, t: string, asAttribute: boolean) => replace(id, t, asAttribute),
      __sk_style_clear(id: number) {
        mutate(id, (st) => {
          if (!st.present) return false;
          for (const k of st.decls.keys()) o.op(id, k, "");
          st.decls = new Map();
          st.present = false;
          st.raw = null;
          return true;
        });
      },
      __sk_style_attr(id: number) {
        const a = attr(id);
        if (a && inline.get(id)?.raw === null) noteSerialization();
        return a;
      },
      __sk_style_hook(f: typeof hook) {
        hook = f;
      },
      __sk_style_observe(on: boolean) {
        observing = on;
      },
    },
  };
}
