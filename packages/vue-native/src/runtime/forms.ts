import type { MirrorElement, MirrorNode } from "./types";
import { defineDom } from "./dom-members";
import { isMirrorNode } from "./listeners";
import { activate } from "./links";
import { firstInTree } from "./labels";
import { forEachDeep } from "./nodes";
import { hasWarned, warnCompat } from "../compat";
import { isRangeInput, syncRangeValue } from "./controls";
import { propagate, withProps } from "./events";

// el.click(): a MouseEvent through the host-click walk, then activation unless cancelled.
// A disabled control, or a click() inside its own click handler (HTML's "click in progress"), does nothing.
const DISABLEABLE = new Set(["button", "input", "select", "textarea", "fieldset", "optgroup", "option"]);
const clicking = new Set<MirrorElement>();
defineDom("click", function (this: MirrorElement) {
  const mirror: boolean = isMirrorNode(this);
  if (
    mirror
      ? isDisabledControl(this)
      : DISABLEABLE.has(this.tag) && this.attrs.disabled != null && this.attrs.disabled !== "false"
  )
    return;
  if (clicking.has(this)) return;
  clicking.add(this);
  try {
    let ev: VnEvent;
    const init = { bubbles: true, cancelable: true, composed: true, detail: 1 };
    try {
      ev = new MouseEvent("click", init);
    } catch {
      ev = new CustomEvent("click", init);
    }
    if (propagate(this, "click", ev)) activate(this, true);
  } finally {
    clicking.delete(this);
  }
});

// Forms: submission. Divergences, each advised once: no constraint validation, no navigation after an uncancelled
// submit, and reset does not restore control values.

export const RUNTIME_FIRED_EVENTS = new Set<string>([
  "submit",
  "reset",
  "fullscreenchange",
  "gotpointercapture",
  "lostpointercapture",
  "select",
  "dragstart",
  "drag",
  "dragenter",
  "dragover",
  "dragleave",
  "drop",
  "dragend",
]);

const hasFlag = (n: MirrorElement, name: string): boolean => n.attrs?.[name] != null && n.attrs[name] !== "false";
const FIELDSET_DISABLES = new Set(["button", "input", "select", "textarea", "fieldset"]);
export function isDisabledControl(n: MirrorElement): boolean {
  if (!DISABLEABLE.has(n.tag)) return false;
  if (hasFlag(n, "disabled")) return true;
  if (!FIELDSET_DISABLES.has(n.tag)) return false;
  let child: MirrorNode = n;
  for (let p = n.parentNode; p; child = p, p = p.parentNode) {
    if (p.kind === 1 && p.tag === "fieldset" && hasFlag(p as MirrorElement, "disabled")) {
      const legend = p.childNodes.find((c) => c.kind === 1 && c.tag === "legend");
      if (child !== legend) return true;
    }
  }
  return false;
}
const typeOf = (n: MirrorElement): string =>
  String(n.attrs?.type ?? "")
    .trim()
    .toLowerCase();

export function isSubmitButton(n: unknown): n is MirrorElement {
  const e = n as MirrorElement | null | undefined;
  if (!e || e.kind !== 1) return false;
  if (e.tag === "button") return typeOf(e) !== "button" && typeOf(e) !== "reset";
  return e.tag === "input" && (typeOf(e) === "submit" || typeOf(e) === "image");
}

export function isResetButton(n: unknown): n is MirrorElement {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && (e.tag === "button" || e.tag === "input") && typeOf(e) === "reset";
}

const IMPLICIT_SUBMIT_TYPES = new Set([
  "text",
  "search",
  "url",
  "tel",
  "email",
  "password",
  "date",
  "month",
  "week",
  "time",
  "datetime-local",
  "number",
]);
export function blocksImplicitSubmission(n: unknown): n is MirrorElement {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "input" && IMPLICIT_SUBMIT_TYPES.has(typeOf(e) || "text");
}

const LISTED = new Set(["button", "fieldset", "input", "object", "output", "select", "textarea"]);

function topOf(n: MirrorNode): MirrorNode {
  while (n.parentNode) n = n.parentNode;
  return n;
}

function formOwner(n: MirrorElement): MirrorElement | null {
  const fid = n.attrs?.form;
  if (fid != null && LISTED.has(n.tag)) {
    const f = firstInTree(topOf(n), (x) => x.attrs?.id === fid);
    return f && f.tag === "form" ? f : null;
  }
  for (let p = n.parentNode; p; p = p.parentNode) if (p.kind === 1 && p.tag === "form") return p as MirrorElement;
  return null;
}

function formControls(form: MirrorElement): MirrorElement[] {
  const out: MirrorElement[] = [];
  forEachDeep(topOf(form), (n) => {
    if (n.kind === 1 && LISTED.has(n.tag) && formOwner(n as MirrorElement) === form) out.push(n as MirrorElement);
  });
  return out;
}

/** A native event class the engine provides (redjs), or `null` where it has none. */
export function nativeEventClass<I>(name: string): (new (type: string, init?: I) => VnEvent) | null {
  const C = (globalThis as Record<string, unknown>)[name];
  return typeof C === "function" ? (C as new (type: string, init?: I) => VnEvent) : null;
}

type SubmitEventInit = { bubbles?: boolean; cancelable?: boolean; composed?: boolean; submitter?: unknown };
const SubmitEventClass = nativeEventClass<SubmitEventInit>("SubmitEvent");

type ClipboardEventInit = { bubbles?: boolean; cancelable?: boolean; composed?: boolean; clipboardData?: unknown };
export const ClipboardEventClass = nativeEventClass<ClipboardEventInit>("ClipboardEvent");

function adviseNoNavigation(via: string): void {
  warnCompat(
    "form:navigate",
    `${via}: in a browser an uncancelled form submission navigates to the form's action. vue-native has ` +
      `no document to navigate, so it does nothing. Cancel it (@submit.prevent) and send the data from JS`,
  );
}

function adviseNoValidation(form: MirrorElement): void {
  if (hasWarned("form:validation")) return;
  const constrained = formControls(form).some(
    (c) =>
      ["required", "pattern", "min", "max", "minlength", "maxlength"].some((a) => c.attrs?.[a] != null) ||
      (c.tag === "input" && ["email", "url", "number"].includes(typeOf(c))),
  );
  if (!constrained) return;
  warnCompat(
    "form:validation",
    "a <form> without novalidate is submitted WITHOUT constraint validation: required, pattern, " +
      "type=email and the rest are not checked, no validation bubble is shown, and the submit event " +
      "fires anyway. Validate in the submit handler (and add novalidate to say so)",
  );
}

const submitting = new WeakSet<MirrorElement>();

function submitForm(form: MirrorElement, submitter: MirrorElement | null, fromSubmitMethod: boolean): void {
  if (fromSubmitMethod) {
    // `form.submit()` fires no event and skips validation, then navigates.
    adviseNoNavigation("form.submit()");
    return;
  }
  if (submitting.has(form)) return;
  if (!hasFlag(form, "novalidate") && !(submitter && hasFlag(submitter, "formnovalidate"))) adviseNoValidation(form);
  submitting.add(form);
  let proceed = true;
  try {
    const init = { bubbles: true, cancelable: true, submitter };
    const ev = SubmitEventClass
      ? new SubmitEventClass("submit", init)
      : withProps(new CustomEvent("submit", init), { submitter });
    proceed = propagate(form, "submit", ev);
  } finally {
    submitting.delete(form);
  }
  if (proceed) adviseNoNavigation("a submit event nobody cancelled");
}

function resetForm(form: MirrorElement): void {
  const init = { bubbles: true, cancelable: true };
  const ev = typeof Event === "function" ? new Event("reset", init) : new CustomEvent("reset", init);
  if (propagate(form, "reset", ev)) {
    const controls = formControls(form);
    for (const control of controls) {
      if (!isRangeInput(control)) continue;
      control._rangeDirty = false;
      control._value = control.attrs.value ?? "";
      syncRangeValue(control);
    }
    if (!controls.some((c) => !isRangeInput(c) && (c.tag === "input" || c.tag === "textarea" || c.tag === "select")))
      return;
    warnCompat(
      "form:reset",
      "form reset (a reset button or form.reset()) fired `reset` but cannot restore the controls: the " +
        "mirror keeps no default values. Reset the model in the handler",
    );
  }
}

export function activateFormButton(n: MirrorElement): void {
  if (isDisabledControl(n)) return;
  const form = formOwner(n);
  if (!form) return;
  if (isSubmitButton(n)) submitForm(form, n, false);
  else if (isResetButton(n)) resetForm(form);
}

export function implicitSubmit(field: MirrorElement): void {
  const form = formOwner(field);
  if (!form) return;
  const controls = formControls(form);
  const defaultButton = controls.find(isSubmitButton);
  if (defaultButton) {
    if (!isDisabledControl(defaultButton)) defaultButton.click();
    return;
  }
  if (controls.filter(blocksImplicitSubmission).length > 1) return;
  submitForm(form, null, false);
}

const asForm = (el: MirrorElement, method: string): MirrorElement => {
  if (!isMirrorNode(el) || el.tag !== "form") throw new TypeError(`${method} is not a function`);
  return el;
};
defineDom("requestSubmit", function (this: MirrorElement, submitter?: unknown) {
  const form = asForm(this, "requestSubmit");
  if (submitter != null) {
    if (!isSubmitButton(submitter)) {
      throw new TypeError(
        "Failed to execute 'requestSubmit' on 'HTMLFormElement': The specified element is not a submit button.",
      );
    }
    if (formOwner(submitter) !== form) {
      throw new DOMException(
        "Failed to execute 'requestSubmit' on 'HTMLFormElement': The specified element is not owned by this form element.",
        "NotFoundError",
      );
    }
  }
  submitForm(form, (submitter as MirrorElement | null | undefined) ?? null, false);
});
defineDom("submit", function (this: MirrorElement) {
  submitForm(asForm(this, "submit"), null, true);
});
defineDom("reset", function (this: MirrorElement) {
  resetForm(asForm(this, "reset"));
});
