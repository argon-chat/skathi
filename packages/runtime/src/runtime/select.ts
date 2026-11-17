import type { MirrorElement, MirrorNode } from "./types";
import { I } from "./event-names";
import { patchAttribute, toLong } from "./patch-prop";
import { propagate } from "./events";
import { fileEvent } from "./file-input";
import { warnCompat } from "../compat";
import { HTMLCollectionProto, liveCollection, staticList } from "./collections";

// <select> and <option>: each option's selectedness and dirtiness live in the mirror, and the selectedness setting
// algorithm reruns when a select's options, `multiple` or `size` change. The host draws the select from the reserved
// `:selected` prop and reports a pick as the select's `input` host event with the option's index (`pickOption`); it
// draws a drop-down whatever `multiple` and `size` say (`adviseListBox`).

// Plain predicates, not type guards: `this` in an accessor is already a
// MirrorElement, and a guard's false branch would narrow it to `never`.
export function isSelectEl(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "select" && !e._ns;
}

export function isOptionEl(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "option" && !e._ns;
}

function isOptGroup(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "optgroup";
}

const attrOn = (el: MirrorElement, k: string): boolean => el.attrs[k] != null && el.attrs[k] !== "false";

function ownerSelect(n: MirrorNode): MirrorElement | null {
  const p = n.parentNode;
  if (isSelectEl(p)) return p;
  if (p && isOptGroup(p) && isSelectEl(p.parentNode)) return p.parentNode;
  return null;
}

export function selectOptions(sel: MirrorElement): MirrorElement[] {
  const out: MirrorElement[] = [];
  for (const c of sel.childNodes) {
    if (isOptionEl(c)) out.push(c as MirrorElement);
    else if (isOptGroup(c)) for (const g of c.childNodes) if (isOptionEl(g)) out.push(g as MirrorElement);
  }
  return out;
}

export const isMultipleSelect = (sel: MirrorElement): boolean => attrOn(sel, "multiple");

function displaySize(sel: MirrorElement): number {
  const n = parseInt(sel.attrs.size ?? "", 10);
  return n > 0 ? n : isMultipleSelect(sel) ? 4 : 1;
}

function optionDisabled(o: MirrorElement): boolean {
  return attrOn(o, "disabled") || (!!o.parentNode && isOptGroup(o.parentNode) && attrOn(o.parentNode, "disabled"));
}

const stripCollapse = (s: string): string =>
  s
    .split(/[ \t\n\f\r]+/)
    .filter(Boolean)
    .join(" ");

function optionText(o: MirrorElement): string {
  return stripCollapse(String((o as unknown as { textContent: unknown }).textContent ?? ""));
}

export function optionValue(o: MirrorElement): string {
  return o.attrs.value ?? optionText(o);
}

export function modelValueOf(o: MirrorElement): unknown {
  return Object.prototype.hasOwnProperty.call(o, "_rawValue") ? o._rawValue : optionValue(o);
}

export function selectedIndexOf(sel: MirrorElement): number {
  const opts = selectOptions(sel);
  for (let i = 0; i < opts.length; i++) if (opts[i]!._selected) return i;
  return -1;
}

export function selectValue(sel: MirrorElement): string {
  const o = selectOptions(sel).find((x) => x._selected);
  return o ? optionValue(o) : "";
}

function showSelected(sel: MirrorElement): void {
  for (const o of selectOptions(sel)) {
    const on = !!o._selected;
    if (!!o._shownChecked === on) continue;
    o._shownChecked = on;
    __sk_set_prop(o.__skId, I(":checked"), on ? "" : undefined);
  }
  const ix = selectedIndexOf(sel);
  if (sel._shown === ix) return;
  sel._shown = ix;
  __sk_set_prop(sel.__skId, I(":selected"), String(ix));
}

/** HTML's "selectedness setting algorithm": a single drop-down with nothing selected selects its first enabled option, one with several selected keeps the last; then the host is told. */
export function resetSelect(sel: MirrorElement): void {
  if (!isMultipleSelect(sel)) {
    const opts = selectOptions(sel);
    const on = opts.filter((o) => o._selected);
    if (on.length === 0 && displaySize(sel) === 1) {
      const first = opts.find((o) => !optionDisabled(o));
      if (first) first._selected = true;
    } else if (on.length > 1) {
      for (const o of on.slice(0, -1)) o._selected = false;
    }
  }
  showSelected(sel);
}

export function optionsChangedIn(n: MirrorNode | null): void {
  if (!n || n.kind !== 1) return;
  const tag = (n as MirrorElement).tag;
  if (tag === "select") {
    if (isSelectEl(n)) resetSelect(n);
  } else if (tag === "optgroup" && isSelectEl(n.parentNode)) {
    resetSelect(n.parentNode as MirrorElement);
  }
}

export function selectednessChanged(o: MirrorElement): void {
  const sel = ownerSelect(o);
  if (!sel) return;
  if (o._selected && !isMultipleSelect(sel)) for (const x of selectOptions(sel)) if (x !== o) x._selected = false;
  resetSelect(sel);
}

export function setOptionSelected(o: MirrorElement, on: boolean): void {
  o._dirty = true;
  o._selected = on;
  selectednessChanged(o);
}

export function setSelectedIndex(sel: MirrorElement, ix: number): void {
  const opts = selectOptions(sel);
  for (const o of opts) o._selected = false;
  const o = ix >= 0 ? opts[ix] : undefined;
  if (o) {
    o._selected = true;
    o._dirty = true;
  }
  showSelected(sel);
}

export function setSelectValue(sel: MirrorElement, v: string): void {
  let found = false;
  for (const o of selectOptions(sel)) {
    o._selected = !found && optionValue(o) === v;
    if (o._selected) {
      o._dirty = true;
      found = true;
    }
  }
  showSelected(sel);
}

/** A select's or an option's `value` prop (runtime-dom assigns the PROPERTY). An option keeps the bound value in `_rawValue` for `v-model`, so `<option :value="1">` gives the model a number. */
export function patchSelectValue(el: MirrorElement, next: unknown): void {
  if (isOptionEl(el)) {
    el._rawValue = next;
    patchAttribute(el, "value", next == null ? null : String(next));
    return;
  }
  const v = next == null ? "" : String(next);
  if (selectValue(el) !== v) setSelectValue(el, v);
}

export function pickOption(sel: MirrorElement, ix: number): boolean {
  const opts = selectOptions(sel);
  const o = opts[ix];
  if (!o || optionDisabled(o) || attrOn(sel, "disabled")) return true;
  if (o._selected && opts.every((x) => x === o || !x._selected)) return true;
  for (const x of opts) x._selected = x === o;
  o._dirty = true;
  showSelected(sel);
  propagate(sel, "input", fileEvent("input"));
  propagate(sel, "change", fileEvent("change"));
  return true;
}

export function adviseListBox(sel: MirrorElement): void {
  if (!isMultipleSelect(sel) && displaySize(sel) === 1) return;
  warnCompat(
    "select:list-box",
    "<select multiple> / <select size>: drawn as a drop-down here (the first selected option, and a popup that " +
      "picks one option at a time), not as Chromium's list box; the selection model, selectedOptions and " +
      "v-model with an array work",
  );
}

export function defineSelectMembers(sel: MirrorElement): void {
  const options = liveCollection<MirrorElement>(HTMLCollectionProto, true, () => selectOptions(sel));
  const acc = (get: () => unknown, set?: (v: unknown) => void): PropertyDescriptor => ({
    get,
    ...(set ? { set } : {}),
    configurable: true,
    enumerable: true,
  });
  Object.defineProperties(sel, {
    options: acc(() => options),
    selectedOptions: acc(() =>
      staticList(
        HTMLCollectionProto,
        selectOptions(sel).filter((o) => o._selected),
      ),
    ),
    selectedIndex: acc(
      () => selectedIndexOf(sel),
      (v) => setSelectedIndex(sel, toLong(v)),
    ),
    length: acc(() => selectOptions(sel).length),
    item: { value: (i: unknown) => selectOptions(sel)[toLong(i)] ?? null, configurable: true, writable: true },
  });
}

export function defineOptionMembers(o: MirrorElement): void {
  o._selected = false;
  o._dirty = false;
  const acc = (get: () => unknown, set?: (v: unknown) => void): PropertyDescriptor => ({
    get,
    ...(set ? { set } : {}),
    configurable: true,
    enumerable: true,
  });
  Object.defineProperties(o, {
    selected: acc(
      () => !!o._selected,
      (v) => setOptionSelected(o, !!v),
    ),
    index: acc(() => {
      const sel = ownerSelect(o);
      return sel ? selectOptions(sel).indexOf(o) : 0;
    }),
    text: acc(
      () => optionText(o),
      (v) => {
        (o as unknown as { textContent: unknown }).textContent = String(v);
      },
    ),
    label: acc(
      () => o.attrs.label ?? optionText(o),
      (v) => patchAttribute(o, "label", String(v)),
    ),
  });
}
