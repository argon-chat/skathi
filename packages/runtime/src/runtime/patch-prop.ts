import type { EventHandlerOrList, MirrorElement } from "./types";
import { hyphenate, patchStyle } from "./inline-style";
import { I, canonicalEvent, hostCode } from "./event-names";
import { warnCompat } from "../compat";
import { handlers } from "./nodes";
import {
  adviseListBox,
  isOptionEl,
  isSelectEl,
  patchSelectValue,
  resetSelect,
  selectednessChanged,
  setOptionSelected,
} from "./select";
import { isRangeInput, isToggleInput, syncRangeValue, writeControlValue } from "./controls";
import { notifyMutation } from "./mutation-observer";
import { RUNTIME_FIRED_EVENTS } from "./forms";
import { adviseJsSourced } from "./labels";
import { isColorInput, syncFileInput } from "./file-input";
import { JS_SOURCED_EVENTS, dndRearm, dndState, ensureBuckets, passiveOf, syncListenerFlag } from "./listeners";
import { REFLECTED } from "./reflected";
import type { Reflected } from "./reflected";
import { noteWrite } from "./perf-advice";

function reflectedOf(el: MirrorElement, key: string): Reflected | null {
  const r = REFLECTED[key];
  return r && (!r.tags || r.tags.has(el.tag)) ? r : null;
}

/** WebIDL `long`: ToNumber, then truncated into 32 bits; NaN and the infinities are 0. */
export function toLong(v: unknown): number {
  const n = Number(v);
  return Number.isFinite(n) ? Math.trunc(n) | 0 : 0;
}

/** `@vue/shared`'s `isOn`: `on` then anything but a lowercase letter. `once`, `online` are attributes. */
export function isOn(key: string): boolean {
  const c = key.charCodeAt(2);
  return key.charCodeAt(0) === 111 && key.charCodeAt(1) === 110 && (c > 122 || c < 97);
}

/** The renderer's prop hook; `prev`/`next` are `unknown` since a template can bind anything. `namespace` is set inside `<svg>`/`<math>`, where every key is an attribute. */
export function patchProp(el: MirrorElement, key: string, prev: unknown, next: unknown, namespace?: unknown): void {
  if (key === "srcObject" && el.tag === "video") {
    el.srcObject = next;
    return;
  }
  if (key === "style") {
    // A null/undefined value in a record clears its property (@floating-ui/vue's
    // `floatingStyles` is undefined before its first measurement).
    patchStyle(el, prev, next);
  } else if (key === "class") {
    if (prev !== next) {
      el._class = next == null ? "" : String(next);
      writeClass(el, next == null);
    }
  } else if (isOn(key)) {
    // runtime-dom's `parseName`: strip the option suffixes, then read the name. `on:loopComplete`
    // is the case-preserving spelling compiler-dom emits for a custom element's camelCase event.
    let rest = key.slice(2);
    let capture = false;
    let once = false;
    let passive = false;
    for (;;) {
      if (rest.endsWith("Once")) {
        once = true;
        rest = rest.slice(0, -4);
      } else if (rest.endsWith("Capture")) {
        capture = true;
        rest = rest.slice(0, -7);
      } else if (rest.endsWith("Passive")) {
        passive = true;
        rest = rest.slice(0, -7);
      } else break;
    }
    const namespaced = rest.startsWith(":");
    // `\B` keeps a leading capital bare ("Click" -> "click", "LoopComplete" -> "loop-complete");
    // `canonicalEvent` folds that back onto the host's camelCase name.
    const name = namespaced ? rest.slice(1) : canonicalEvent(hyphenate(rest));
    const code = hostCode(name);
    if (!code && RUNTIME_FIRED_EVENTS.has(name)) {
    } else if (!code && JS_SOURCED_EVENTS.has(name)) {
      // `change` / `cancel`: the runtime fires them on a file input (they bubble), so the handler is kept.
      if (next) adviseJsSourced(el, name, `@${name}`);
    } else if (!code) {
      // A listener the host will never deliver must not vanish silently. A `:` inside the name
      // marks a component-emit prop that leaked onto an element; that stays silent.
      if (namespaced || !rest.includes(":")) {
        warnCompat(
          `event:${name}`,
          `@${name} — this event type is not delivered by the native host yet; ` + `the handler will never fire`,
        );
      }
      return;
    }
    let m = handlers.get(el);
    if (!m) {
      m = {};
      handlers.set(el, m);
    }
    let buckets = ensureBuckets(m, name);
    const pass = passiveOf(passive ? { passive } : undefined, el, name);
    if (capture) {
      buckets.capture = (next as EventHandlerOrList) || null;
      buckets.onceC = once;
      buckets.passiveC = pass;
    } else {
      buckets.bubble = (next as EventHandlerOrList) || null;
      buckets.onceB = once;
      buckets.passiveB = pass;
    }
    syncListenerFlag(el, name);
  } else if (key === "key" || key === "ref") {
  } else if (key === "textContent" && !foreignNamespace(namespace)) {
    if (next != null) (el as { textContent: unknown }).textContent = next;
  } else if (key === "value" && isRangeInput(el) && !foreignNamespace(namespace)) {
    writeControlValue(el, next);
  } else if (key === "value" && (isSelectEl(el) || isOptionEl(el)) && !foreignNamespace(namespace)) {
    patchSelectValue(el, next);
  } else if (key === "selected" && isOptionEl(el) && !foreignNamespace(namespace)) {
    patchAttribute(el, ...attributeOf(el, key, next, namespace));
    setOptionSelected(el, next === "" || !!next);
  } else if ((key === "checked" || key === "indeterminate") && el.tag === "input" && !foreignNamespace(namespace)) {
    // runtime-dom assigns the property: the attribute stays the default, as in a browser.
    el[key] = next === "" || !!next;
  } else {
    if (key === "true-value") el._trueValue = next;
    else if (key === "false-value") el._falseValue = next;
    else if (key === "value" && isToggleInput(el) && !foreignNamespace(namespace)) el._rawValue = next;
    patchAttribute(el, ...attributeOf(el, key, next, namespace));
  }
}

const foreignNamespace = (ns: unknown): boolean => ns === "svg" || ns === "mathml";

/** runtime-dom's `patchProp` for a non-style/class/listener key, reduced to the attribute it leaves: booleans are `""` or absent (writing "false" would make `[hidden]` match), camelCase names map to their attribute (`tabIndex` -> tabindex), a `long` is an integer, the rest verbatim. Returns `[attribute, value]`; `null` removes. */
function attributeOf(el: MirrorElement, key: string, next: unknown, namespace: unknown): [string, unknown] {
  const on = (): string | null => (next === "" || !!next ? "" : null); // runtime-dom's includeBooleanAttr
  if (SPECIAL_BOOLEAN_ATTRS.has(key)) return [key, on()];
  if (foreignNamespace(namespace)) return [key, next];
  if (key === "selected" && el.tag === "option") return [key, on()];
  // A checkbox's model value can be any object; `_rawValue` keeps it, the attribute gets its string.
  if (key === "value" && isToggleInput(el) && next !== null && typeof next === "object") return [key, String(next)];
  const r = reflectedOf(el, key);
  if (!r) return [key, next];
  if (r.kind === "bool") return [r.attr, on()];
  if (r.attr === key || next == null) return [r.attr, next];
  return [r.attr, r.kind === "long" ? String(toLong(next)) : String(next)];
}

const SPECIAL_BOOLEAN_ATTRS = new Set([
  "itemscope",
  "allowfullscreen",
  "formnovalidate",
  "ismap",
  "nomodule",
  "novalidate",
  "readonly",
]);

export function patchAttribute(el: MirrorElement, key: string, next: unknown): void {
  if (__DEV__) noteWrite(el);
  if (next == null) __sk_set_prop(el.__skId, I(key), undefined);
  else if (typeof next === "string" || typeof next === "number" || typeof next === "boolean")
    __sk_set_prop(el.__skId, I(key), String(next));
  else
    warnCompat(
      `prop:${key}`,
      `prop "${key}" with a non-primitive value is not supported by the native host yet; ` +
        `it was dropped — pass a string/number/boolean or handle it in the component`,
    );
  if (el.attrs) {
    const before = el.attrs[key];
    if (next == null) delete el.attrs[key];
    else if (typeof next !== "object" && typeof next !== "function") el.attrs[key] = String(next);
    if (before !== el.attrs[key]) {
      notifyMutation(el, "attributes", { attributeName: key, oldValue: before ?? null });
    }
  }
  if (key === "draggable" && !dndState.seen && next != null && String(next) === "true") {
    dndState.seen = true;
    dndRearm();
  }
  if (key === "value" && !(isRangeInput(el) && el._rangeDirty)) el._value = next == null ? "" : String(next);
  if (isRangeInput(el) && ["value", "type", "min", "max", "step"].includes(key)) syncRangeValue(el);
  if (isColorInput(el) && (key === "value" || key === "type" || key === "alpha")) writeControlValue(el, el._value);
  if (key === "selected" && isOptionEl(el) && !el._dirty) {
    el._selected = next != null;
    selectednessChanged(el);
  }
  if ((key === "multiple" || key === "size") && isSelectEl(el)) {
    resetSelect(el);
    adviseListBox(el);
  }
  // An `<a href>` is activatable whether or not anything listens, and the host only delivers a
  // click to a node whose click bit is on, so the bit follows the href.
  if (key === "href" && el.tag === "a") syncListenerFlag(el, "click");
  if (key === "type" && el.tag === "input") syncFileInput(el);
  if (key === "type" && (el.tag === "input" || el.tag === "button")) {
    syncListenerFlag(el, "click");
    if (el.tag === "input") syncListenerFlag(el, "keydown");
  }
  // The runtime honours a disabled fieldset; the host's native widgets read only their own `disabled`.
  if (key === "disabled" && el.tag === "fieldset" && next != null && next !== false && next !== "false") {
    warnCompat(
      "fieldset:disabled",
      "<fieldset disabled>: clicks, focus() and form submission treat the controls inside as disabled, " +
        "as a browser does, but the native <input>s inside still accept typing; disable each one as well",
    );
  }
}

export function writeAttribute(el: MirrorElement, key: string, value: string | null): void {
  if (key === "class") {
    if (value != null) return setClassText(el, value);
    el._class = "";
    el._vtc = undefined;
    writeClass(el, true);
  } else if (key.startsWith("on") && key.length > 2) patchProp(el, key, undefined, value);
  else patchAttribute(el, key, value);
}

// The class attribute is what Vue assigned plus what `<Transition>` layered on (`el._vtc`), so a re-render
// mid-transition keeps the phase class; `getAttribute`, `className` and `classList` read the merged value.
export function writeClass(el: MirrorElement, remove = false): void {
  const base = el._class || "";
  const extra = el._vtc;
  const value = extra && extra.size ? [base, ...extra].filter(Boolean).join(" ") : base;
  const before = el.attrs?.class;
  if (before !== value) {
    __sk_set_class(el.__skId, value);
    if (__DEV__) noteWrite(el);
  }
  if (!el.attrs) return;
  if (remove && !value) delete el.attrs.class;
  else el.attrs.class = value;
  if (before !== el.attrs.class) notifyMutation(el, "attributes", { attributeName: "class", oldValue: before ?? null });
}

/**
 * The class attribute becomes `text`, as `className = text` does. A transition class `text` still
 * names stays one (a read-modify-write must not bake `v-enter-active` in); one it drops is dropped.
 */
export function setClassText(el: MirrorElement, text: string): void {
  const vtc = el._vtc;
  if (vtc && vtc.size) {
    const tokens = text.split(/\s+/).filter(Boolean);
    for (const c of [...vtc]) if (!tokens.includes(c)) vtc.delete(c);
    if (!vtc.size) el._vtc = undefined;
    el._class = tokens.filter((c) => !vtc.has(c)).join(" ");
  } else {
    el._class = text;
  }
  writeClass(el);
}
