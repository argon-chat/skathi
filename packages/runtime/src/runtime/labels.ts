import type { MirrorElement, MirrorNode } from "./types";
import { hasWarned, warnCompat } from "../compat";
import {
  colorValue,
  fileAccessors,
  fileEvent,
  fileInputOps,
  fileStates,
  isColorInput,
  isFileInput,
  showColorPicker,
  showFilePicker,
} from "./file-input";
import type { FileInputOps, FileState } from "./file-input";
import { followHref } from "./links";
import { defineDom } from "./dom-members";
import { isMirrorNode } from "./listeners";
import { activation } from "./activation";
import { isSelectEl } from "./select";
import { callListener } from "./events";
import { isDisabledControl } from "./forms";

const LABELABLE = new Set(["button", "meter", "output", "progress", "select", "textarea"]);
const isLabelable = (n: MirrorNode): n is MirrorElement =>
  n.kind === 1 &&
  (LABELABLE.has(n.tag) ||
    (n.tag === "input" &&
      String(n.attrs.type ?? "")
        .trim()
        .toLowerCase() !== "hidden"));

export function firstInTree(root: MirrorNode, pred: (n: MirrorElement) => boolean): MirrorElement | null {
  if (root.kind === 1 && pred(root)) return root;
  for (const c of root.childNodes ?? []) {
    const hit = firstInTree(c, pred);
    if (hit) return hit;
  }
  return null;
}

/** A label's control: the element its `for` names if labelable, else its first labelable descendant. */
function labelControl(label: MirrorElement): MirrorElement | null {
  const forId = label.attrs.for;
  if (forId != null) {
    let root: MirrorNode = label;
    while (root.parentNode) root = root.parentNode;
    const found = firstInTree(root, (n) => n.attrs?.id === forId);
    return found && isLabelable(found) ? found : null;
  }
  return firstInTree(label, (n) => n !== label && isLabelable(n));
}

const FOCUS_ONLY_TYPES = new Set(["button", "submit", "reset", "image", "file", "checkbox", "radio", "range", "color"]);

export function activateLabel(label: MirrorElement, target: MirrorNode): void {
  const control = labelControl(label);
  if (!control) return;
  for (let n: MirrorNode | null = target; n; n = n.parentNode) if (n === control) return;
  const textual =
    control.tag === "textarea" ||
    control.tag === "select" ||
    (control.tag === "input" &&
      !FOCUS_ONLY_TYPES.has(
        String(control.attrs.type ?? "text")
          .trim()
          .toLowerCase(),
      ));
  if (textual) control.focus();
  control.click();
}

export function adviseJsSourced(el: MirrorElement, name: string, via: string): void {
  const key = `event:${name}`;
  if (hasWarned(key)) return;
  queueMicrotask(() => {
    if (
      hasWarned(key) ||
      firstInTree(
        el,
        (n) =>
          isFileInput(n) ||
          (name === "change" &&
            (isSelectEl(n) ||
              isColorInput(n) ||
              (n.tag === "input" && String(n.attrs.type).toLowerCase() === "range"))),
      )
    )
      return;
    warnCompat(
      key,
      `${via} — this event type is not delivered by the native host yet: only <input type=file> ` +
        `(and <select>, <input type=range/color>, for \`change\`) fires it here, so on this element it will never fire`,
    );
  });
}

// Detached `document.createElement("a" | "input")` are plain objects on HTMLElement.prototype with no-op
// attribute methods; `wrapDetached` gives them a reflected attribute map and an input the file accessors.
// Their events go through the engine's own EventTarget.

const REFLECTED_BOOL = new Set(["multiple", "webkitdirectory", "disabled"]);
const REFLECTED_STR = new Set(["href", "download", "type", "accept", "rel", "target"]);
const nativeDispatch =
  typeof EventTarget === "function"
    ? (EventTarget.prototype.dispatchEvent as (this: object, ev: Event) => boolean)
    : null;

export type Detached = Record<string, unknown>;

export function wrapDetached(el: Detached, tag: string): void {
  const attrs: Record<string, string> = Object.create(null);
  const define = (name: string, value: unknown) => {
    try {
      Object.defineProperty(el, name, { value, configurable: true, writable: true });
    } catch {
      /* a non-configurable engine slot keeps the engine's method */
    }
  };
  define("setAttribute", (name: unknown, value: unknown) => {
    const k = String(name).toLowerCase();
    const v = value == null ? "" : String(value);
    attrs[k] = v;
    if (REFLECTED_BOOL.has(k)) el[k] = true;
    else if (REFLECTED_STR.has(k)) el[k] = v;
  });
  define("getAttribute", (name: unknown) => {
    const k = String(name).toLowerCase();
    if (REFLECTED_BOOL.has(k)) return el[k] ? (attrs[k] ?? "") : null;
    if (REFLECTED_STR.has(k) && el[k] != null) return String(el[k]);
    return attrs[k] ?? null;
  });
  define("hasAttribute", (name: unknown) => (el.getAttribute as (n: unknown) => string | null)(name) !== null);
  define("removeAttribute", (name: unknown) => {
    const k = String(name).toLowerCase();
    delete attrs[k];
    if (REFLECTED_BOOL.has(k)) el[k] = false;
    else if (REFLECTED_STR.has(k)) delete el[k];
  });
  if (tag !== "input") return;
  const state: FileState = { files: null, changed: () => {} };
  fileStates.set(el, state);
  const other = { value: String(el.value ?? "") };
  const acc = fileAccessors(
    state,
    () =>
      String(el.type ?? "")
        .trim()
        .toLowerCase() === "file",
    other,
  );
  try {
    Object.defineProperties(el, {
      files: acc.files,
      value: {
        ...acc.value,
        get: () => (String(el.type ?? "").toLowerCase() === "color" ? colorValue(other.value) : acc.value.get()),
        set: (v: unknown) => acc.value.set(String(el.type ?? "").toLowerCase() === "color" ? colorValue(v) : v),
      },
    });
  } catch {
    /* as above */
  }
}

export function fireDetached(el: Detached, type: string): void {
  const ev = fileEvent(type);
  try {
    if (!nativeDispatch) throw new TypeError("no EventTarget");
    nativeDispatch.call(el, ev);
  } catch {
    // Not dispatchable: call the handler attribute with a target (VueUse reads `event.target.files`).
    const h = el[`on${type}`];
    Object.defineProperty(ev, "target", { value: el, configurable: true });
    if (typeof h === "function") callListener(h as (this: unknown, ev: Event) => unknown, el, ev);
  }
}

function detachedFileOps(el: Detached): FileInputOps {
  if (!fileStates.has(el)) wrapDetached(el, "input");
  const state = fileStates.get(el)!;
  return {
    accept: String(el.accept ?? ""),
    multiple: !!el.multiple,
    directory: !!el.webkitdirectory,
    disabled: !!el.disabled,
    current: state.files,
    setFiles: (list) => {
      state.files = list;
    },
    fire: (type) => fireDetached(el, type),
  };
}

export function activateDetached(target: object): void {
  const el = target as Detached;
  const tag = String(el.tagName ?? "").toUpperCase();
  if (tag === "A" && el.href != null) {
    followHref(String(el.href), el.download == null ? null : String(el.download));
  } else if (
    tag === "INPUT" &&
    String(el.type ?? "")
      .trim()
      .toLowerCase() === "file"
  ) {
    showFilePicker(detachedFileOps(el), null);
  } else if (
    tag === "INPUT" &&
    String(el.type ?? "")
      .trim()
      .toLowerCase() === "color"
  ) {
    showColorPicker(el, null);
  }
}

// showPicker(): Chromium's checks (immutable -> InvalidStateError, no user activation -> NotAllowedError).
defineDom("showPicker", function (this: MirrorElement) {
  const mirror = isMirrorNode(this);
  const raw = this as unknown as Detached;
  const tag = mirror ? this.tag : String(raw.tagName ?? "").toLowerCase();
  if (tag !== "input") {
    throw new TypeError("Failed to execute 'showPicker': this element has no picker (only <input> has one here).");
  }
  const attr = (k: string) => (mirror ? this.attrs[k] != null && this.attrs[k] !== "false" : !!raw[k]);
  const type = String((mirror ? this.attrs.type : raw.type) ?? "text")
    .trim()
    .toLowerCase();
  const fail = (name: string, msg: string) =>
    new DOMException(`Failed to execute 'showPicker' on 'HTMLInputElement': ${msg}`, name);
  if (
    (mirror ? isDisabledControl(this) : attr("disabled")) ||
    (type !== "file" && type !== "color" && attr("readonly"))
  ) {
    throw fail("InvalidStateError", "HTMLInputElement::showPicker() cannot be used on immutable controls.");
  }
  if (!activation.isActive()) throw fail("NotAllowedError", "HTMLInputElement::showPicker() requires a user gesture.");
  if (type === "color") return showColorPicker(mirror ? this : raw, "showPicker");
  if (type !== "file") {
    warnCompat(
      `showPicker:${type}`,
      `showPicker() on <input type=${type}> is a no-op: only file and color pickers exist here`,
    );
    return;
  }
  showFilePicker(mirror ? fileInputOps(this) : detachedFileOps(raw), "showPicker");
});
