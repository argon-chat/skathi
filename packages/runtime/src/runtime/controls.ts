import type { MirrorElement } from "./types";
import { I } from "./event-names";
import { colorValue, isColorInput } from "./file-input";
import { defineDom, defineDomGet } from "./dom-members";
import { patchAttribute, toLong } from "./patch-prop";
import { isOptionEl, isSelectEl, optionValue, selectValue, setSelectValue } from "./select";
// Form-control `value` / `checked` are real accessors: reka-ui's hidden inputs call
// `Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, v)` and dispatch `input`/`change`
// themselves. A mirror element's own `value` reaches the host, and the tag classes carry OWN accessors delegating to
// it (`TAG_ACCESSORS`), refusing a wrong receiver with "Illegal invocation".

export function isMirrorEl(x: unknown): x is MirrorElement {
  return !!x && typeof x === "object" && Object.prototype.hasOwnProperty.call(x, "_value");
}

/** A checkbox or radio: the host holds its state and runs its activation. */
export function isToggleInput(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  if (!e || e.kind !== 1 || e.tag !== "input") return false;
  const type = String(e.attrs?.type ?? "")
    .trim()
    .toLowerCase();
  return type === "checkbox" || type === "radio";
}

export function isRangeInput(el: MirrorElement): boolean {
  return el.kind === 1 && el.tag === "input" && String(el.attrs?.type ?? "").toLowerCase() === "range";
}
function rangeNumber(value: unknown): number | undefined {
  const s = String(value ?? "");
  if (!/^-?(?:[0-9]+(?:\.[0-9]+)?|\.[0-9]+)(?:[eE][+-]?[0-9]+)?$/.test(s)) return undefined;
  const n = Number(s);
  return Number.isFinite(n) ? n : undefined;
}
function rangeSpec(el: MirrorElement): { min: number; max: number; step: number | null; base: number } {
  const min = rangeNumber(el.attrs.min);
  const step = rangeNumber(el.attrs.step);
  return {
    min: min ?? 0,
    max: rangeNumber(el.attrs.max) ?? 100,
    step: String(el.attrs.step ?? "").toLowerCase() === "any" ? null : step != null && step > 0 ? step : 1,
    base: min ?? rangeNumber(el.attrs.value) ?? 0,
  };
}
export function rangeValue(el: MirrorElement, raw: unknown): string {
  const { min, max, step, base } = rangeSpec(el);
  if (max < min) return String(min);
  let value = Math.min(max, Math.max(min, rangeNumber(raw) ?? min / 2 + max / 2));
  if (step !== null) {
    const lo = Math.ceil((min - base) / step - 1e-10),
      hi = Math.floor((max - base) / step + 1e-10);
    if (lo <= hi) {
      const index = Math.min(hi, Math.max(lo, Math.floor((value - base) / step + 0.5 + 1e-10)));
      value = Math.min(max, Math.max(min, Number((base + index * step).toPrecision(15))));
    }
  }
  return String(value);
}
export function syncRangeValue(el: MirrorElement): void {
  const base = rangeSpec(el).base;
  __sk_set_prop(el.__skId, I(":range-base"), String(base));
  writeControlValue(el, el._value, false);
}

const VALUE_TAGS = new Set(["input", "textarea", "select"]);

/** `el.value = v` on a form control: updates mirror and host on EVERY assignment (the host's write counter re-forces a field the user typed over); fires no `input` / `change`. */
export function writeControlValue(el: MirrorElement, v: unknown, dirty = true): void {
  if (dirty && isRangeInput(el)) el._rangeDirty = true;
  const s = isRangeInput(el)
    ? rangeValue(el, v)
    : isColorInput(el)
      ? colorValue(v)
      : v == null && el.tag !== "select"
        ? ""
        : String(v);
  el._value = s;
  __sk_set_prop(el.__skId, I("value"), s);
}

export function ownExpando(self: object, key: string, v: unknown): void {
  Object.defineProperty(self, key, { value: v, writable: true, enumerable: true, configurable: true });
}

defineDomGet(
  "value",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    if (isSelectEl(this)) return selectValue(this);
    if (isOptionEl(this)) return optionValue(this);
    if (isToggleInput(this)) return this.attrs.value ?? "on";
    if (isRangeInput(this)) return rangeValue(this, this._value);
    if (isColorInput(this)) return colorValue(this._value);
    return this._value;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this)) return ownExpando(this, "value", v);
    if (isSelectEl(this)) setSelectValue(this, String(v));
    else if (isOptionEl(this) || isToggleInput(this)) patchAttribute(this, "value", String(v));
    else if (VALUE_TAGS.has(this.tag)) writeControlValue(this, v);
    else this._value = v as string;
  } as (value: never) => void,
);

defineDomGet(
  "valueAsNumber",
  function (this: MirrorElement) {
    if (!isMirrorEl(this) || this.tag !== "input") return NaN;
    return isRangeInput(this)
      ? Number(rangeValue(this, this._value))
      : String(this.attrs.type ?? "").toLowerCase() === "number"
        ? (rangeNumber(this._value) ?? NaN)
        : NaN;
  },
  function (this: MirrorElement, raw: unknown) {
    if (!isMirrorEl(this) || this.tag !== "input") return;
    const value = Number(raw);
    if (!Number.isFinite(value) && !Number.isNaN(value)) throw new TypeError("The provided value is non-finite.");
    if (isRangeInput(this)) writeControlValue(this, Number.isNaN(value) ? "" : String(value));
    else if (String(this.attrs.type ?? "").toLowerCase() === "number")
      writeControlValue(this, Number.isNaN(value) ? "" : String(value));
    else throw new DOMException("This input element does not support Number values.", "InvalidStateError");
  } as (value: never) => void,
);
function stepRange(el: MirrorElement, count: unknown, direction: number): void {
  if (!isRangeInput(el)) throw new DOMException("This input element does not support stepping.", "InvalidStateError");
  const { step } = rangeSpec(el);
  if (step === null) throw new DOMException("This input element does not support stepping.", "InvalidStateError");
  writeControlValue(el, Number(rangeValue(el, el._value)) + direction * toLong(count === undefined ? 1 : count) * step);
}
defineDom("stepUp", function (this: MirrorElement, n?: unknown) {
  stepRange(this, n, 1);
});
defineDom("stepDown", function (this: MirrorElement, n?: unknown) {
  stepRange(this, n, -1);
});

defineDomGet(
  "defaultValue",
  function (this: MirrorElement) {
    if (!isMirrorEl(this)) return undefined;
    if (this.tag === "input") return this.attrs.value ?? "";
    if (this.tag === "textarea") return (this as unknown as { textContent: string }).textContent ?? "";
    return undefined;
  },
  function (this: MirrorElement, v: unknown) {
    if (!isMirrorEl(this) || (this.tag !== "input" && this.tag !== "textarea"))
      return ownExpando(this, "defaultValue", v);
    if (this.tag === "input") this.setAttribute("value", v == null ? "" : String(v));
    else (this as unknown as { textContent: string }).textContent = v == null ? "" : String(v);
  } as (value: never) => void,
);
