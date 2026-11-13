import type { MirrorElement } from "./types";
import { writeControlValue } from "./controls";
import { warnCompat } from "../compat";
import {
  isMultipleSelect,
  modelValueOf,
  selectOptions,
  selectedIndexOf,
  setOptionSelected,
  setSelectedIndex,
} from "./select";
import { nextTick } from "@vue/runtime-core";
import { hyphenate, inlineDisplay, setInlineDisplay, vShowHidden, vShowOriginalDisplay } from "./inline-style";

const assignKey = Symbol("vnModelAssign");

interface DirectiveBindingLike {
  value?: unknown;
  oldValue?: unknown;
  modifiers?: Partial<Record<string, boolean>>;
}
interface VNodeLike {
  props?: Record<string, unknown> | null;
}

function setElValue(el: MirrorElement, value: unknown): void {
  const v = value == null ? "" : String(value);
  // Compared with what the field holds now — typing keeps `_value` current —
  // so a model reset to the string the user has since typed over still lands.
  if (el._value === v) return;
  writeControlValue(el, v);
}

export const vModelText = {
  created(el: MirrorElement, { modifiers = {} }: DirectiveBindingLike, vnode: VNodeLike) {
    el[assignKey] = vnode.props && vnode.props["onUpdate:modelValue"];
    // `.lazy` listens for `change`, which the host never sources; fall back to `input` and say so.
    if (modifiers.lazy) {
      warnCompat(
        "v-model:lazy",
        "v-model.lazy updates on every keystroke here, not on `change` — the native host does " +
          "not source a `change` event yet, so binding one would never fire at all",
      );
    }
    el.addEventListener("input", () => {
      if (el.composing) return;
      // `.trim` runs before `.number`, so `v` is still a string at both casts.
      let v: string | number = el.value;
      if (modifiers.trim) v = (v as string).trim();
      if (modifiers.number) {
        const n = parseFloat(v as string);
        if (!Number.isNaN(n)) v = n;
      }
      if (el[assignKey]) (el[assignKey] as (value: unknown) => void)(v);
    });
    // The composition ends re-dispatch `input`, which assigns the IME's text. Runtime-dom binds them only without `.lazy`;
    // here `.lazy` listens for `input`, so it needs them too.
    el.addEventListener("compositionstart", () => {
      el.composing = true;
    });
    el.addEventListener("compositionend", () => {
      if (!el.composing) return;
      el.composing = false;
      el.dispatchEvent(new Event("input"));
    });
  },
  mounted(el: MirrorElement, { value }: DirectiveBindingLike) {
    setElValue(el, value);
  },
  beforeUpdate(el: MirrorElement, { value }: DirectiveBindingLike, vnode: VNodeLike) {
    el[assignKey] = vnode.props && vnode.props["onUpdate:modelValue"];
    // Leave a composing field alone: assigning would end the IME's composition (vuejs/core#2302).
    if (el.composing) return;
    setElValue(el, value);
  },
};

// Every input renders as a text field: dynamic/checkbox/radio fall back to text behavior with an advisory.
function modelFallback(kind: string) {
  return {
    created(el: MirrorElement, binding: DirectiveBindingLike, vnode: VNodeLike) {
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

function looseEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  const date = (x: unknown): x is Date => x instanceof Date;
  if (date(a) || date(b)) return date(a) && date(b) ? a.getTime() === b.getTime() : false;
  if (typeof a === "symbol" || typeof b === "symbol") return a === b;
  if (Array.isArray(a) || Array.isArray(b)) {
    if (!Array.isArray(a) || !Array.isArray(b) || a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!looseEqual(a[i], b[i])) return false;
    return true;
  }
  const obj = (x: unknown): x is Record<string, unknown> => x !== null && typeof x === "object";
  if (obj(a) || obj(b)) {
    if (!obj(a) || !obj(b)) return false;
    if (Object.keys(a).length !== Object.keys(b).length) return false;
    for (const key in a) {
      const has = Object.prototype.hasOwnProperty;
      if (has.call(a, key) !== has.call(b, key) || !looseEqual(a[key], b[key])) return false;
    }
  }
  return String(a) === String(b);
}

const looseIndexOf = (arr: readonly unknown[], v: unknown): number => arr.findIndex((x) => looseEqual(x, v));

function looseToNumber(v: unknown): unknown {
  const n = parseFloat(v as string);
  return Number.isNaN(n) ? v : n;
}

function modelAssigner(vnode: VNodeLike): ((value: unknown) => void) | null {
  const fn = vnode.props?.["onUpdate:modelValue"];
  if (Array.isArray(fn)) return (value) => fn.forEach((f) => (f as (v: unknown) => void)(value));
  return typeof fn === "function" ? (fn as (value: unknown) => void) : null;
}

function setModelSelected(el: MirrorElement, value: unknown): void {
  const multiple = isMultipleSelect(el);
  const isArray = Array.isArray(value);
  if (multiple && !isArray && !(value instanceof Set)) {
    console.warn(
      `[Vue warn]: <select multiple v-model> expects an Array or Set value for its binding, but got ` +
        `${Object.prototype.toString.call(value).slice(8, -1)}.`,
    );
    return;
  }
  const opts = selectOptions(el);
  for (let i = 0; i < opts.length; i++) {
    const option = opts[i]!;
    const optionValue = modelValueOf(option);
    if (multiple) {
      let on: boolean;
      if (isArray) {
        const type = typeof optionValue;
        on =
          type === "string" || type === "number"
            ? (value as unknown[]).some((v) => String(v) === String(optionValue))
            : looseIndexOf(value as unknown[], optionValue) > -1;
      } else {
        on = (value as Set<unknown>).has(optionValue);
      }
      setOptionSelected(option, on);
    } else if (looseEqual(optionValue, value)) {
      if (selectedIndexOf(el) !== i) setSelectedIndex(el, i);
      return;
    }
  }
  if (!multiple && selectedIndexOf(el) !== -1) setSelectedIndex(el, -1);
}

export const vModelSelect = {
  deep: true,
  created(el: MirrorElement, { value, modifiers = {} }: DirectiveBindingLike, vnode: VNodeLike) {
    el._modelValue = value;
    el.addEventListener("change", () => {
      const picked = selectOptions(el)
        .filter((o) => o._selected)
        .map((o) => (modifiers.number ? looseToNumber(modelValueOf(o)) : modelValueOf(o)));
      const assign = el[assignKey] as ((value: unknown) => void) | null | undefined;
      if (assign) {
        assign(isMultipleSelect(el) ? (el._modelValue instanceof Set ? new Set(picked) : picked) : picked[0]);
      }
      el._assigning = true;
      void nextTick(() => {
        el._assigning = false;
      });
    });
    el[assignKey] = modelAssigner(vnode);
  },
  // A select's value depends on its options, which mount and update before it.
  mounted(el: MirrorElement, { value }: DirectiveBindingLike) {
    setModelSelected(el, value);
  },
  beforeUpdate(el: MirrorElement, { value }: DirectiveBindingLike, vnode: VNodeLike) {
    el._modelValue = value;
    el[assignKey] = modelAssigner(vnode);
  },
  updated(el: MirrorElement, { value }: DirectiveBindingLike) {
    if (!el._assigning) setModelSelected(el, value);
  },
};

function setVShowDisplay(el: MirrorElement, value: unknown): void {
  const d = value ? String(el[vShowOriginalDisplay] ?? "") : "none";
  setInlineDisplay(el, d);
  el[vShowHidden] = !value;
}
export const vShow = {
  beforeMount(el: MirrorElement, { value }: DirectiveBindingLike, vnode?: { transition?: unknown }) {
    if (vnode?.transition) {
      warnCompat(
        "v-show:transition",
        "v-show inside <Transition> flips display without the enter/leave phases here; " +
          "use v-if for an animated show/hide",
      );
    }
    const d = inlineDisplay(el);
    el[vShowOriginalDisplay] = d === "none" ? "" : d;
    setVShowDisplay(el, value);
  },
  updated(el: MirrorElement, { value, oldValue }: DirectiveBindingLike) {
    if (!value === !oldValue) return;
    setVShowDisplay(el, value);
  },
  beforeUnmount(el: MirrorElement, { value }: DirectiveBindingLike) {
    setVShowDisplay(el, value);
  },
};

const modifierGuards: Record<string, ((e: VnEvent, m: string[]) => unknown) | undefined> = {
  stop: (e) => e.stopPropagation(),
  prevent: (e) => e.preventDefault(),
  self: (e) => e.target !== e.currentTarget,
  ctrl: (e) => !(e as VnMouseEvent).ctrlKey,
  shift: (e) => !(e as VnMouseEvent).shiftKey,
  alt: (e) => !(e as VnMouseEvent).altKey,
  meta: (e) => !(e as VnMouseEvent).metaKey,
  left: (e) => "button" in e && (e as VnMouseEvent).button !== 0,
  middle: (e) => "button" in e && (e as VnMouseEvent).button !== 1,
  right: (e) => "button" in e && (e as VnMouseEvent).button !== 2,
  exact: (e, m) =>
    ["ctrl", "shift", "alt", "meta"].some(
      (x) => (e as unknown as Record<string, unknown>)[x + "Key"] && !m.includes(x),
    ),
};

export function withModifiers(fn: (event: VnEvent, ...args: unknown[]) => unknown, modifiers: string[]) {
  return (event: VnEvent, ...args: unknown[]) => {
    for (const m of modifiers) {
      const guard = modifierGuards[m];
      if (guard && guard(event, modifiers)) return;
    }
    return fn(event, ...args);
  };
}

const keyAliases: Record<string, string | undefined> = {
  esc: "escape",
  space: " ",
  up: "arrow-up",
  left: "arrow-left",
  right: "arrow-right",
  down: "arrow-down",
  delete: "backspace",
};

export function withKeys(fn: (event: VnEvent) => unknown, keys: string[]) {
  return (event: VnEvent) => {
    if (!("key" in event)) return;
    const eventKey = hyphenate(String((event as VnKeyboardEvent).key));
    if (keys.some((k) => k === eventKey || keyAliases[k] === eventKey)) return fn(event);
    return undefined;
  };
}
