import type { MirrorElement, MirrorNode } from "./types";
import { hasWarned, warnCompat } from "../compat";
import { fileInputOps, isColorInput, isFileInput, showColorPicker, showFilePicker } from "./file-input";
import { defineDom } from "./dom-members";
import { isMirrorNode } from "./listeners";
import { activation } from "./activation";
import { isSelectEl } from "./select";
import { isDisabledControl } from "./forms";
import { isToggleInput } from "./controls";

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
          (name === "showpicker" && isColorInput(n)) ||
          (name === "change" &&
            (isSelectEl(n) ||
              n.tag === "sk-editor" ||
              isToggleInput(n) ||
              isColorInput(n) ||
              (n.tag === "input" && String(n.attrs.type).toLowerCase() === "range"))),
      )
    )
      return;
    warnCompat(
      key,
      `${via} — this event type is not delivered by the native host yet: only <input type=file> ` +
        `(and <select>, checkboxes, radios, <input type=range/color>, <sk-editor>, for \`change\`) fires it here, so on this element it will never fire`,
    );
  });
}

// showPicker(): Chromium's checks (immutable -> InvalidStateError, no user activation -> NotAllowedError).
defineDom("showPicker", function (this: MirrorElement) {
  if (!isMirrorNode(this) || this.tag !== "input") {
    throw new TypeError("Failed to execute 'showPicker': this element has no picker (only <input> has one here).");
  }
  const type = String(this.attrs.type ?? "text")
    .trim()
    .toLowerCase();
  const fail = (name: string, msg: string) =>
    new DOMException(`Failed to execute 'showPicker' on 'HTMLInputElement': ${msg}`, name);
  const readonly = this.attrs.readonly != null && this.attrs.readonly !== "false";
  if (isDisabledControl(this) || (type !== "file" && type !== "color" && readonly)) {
    throw fail("InvalidStateError", "HTMLInputElement::showPicker() cannot be used on immutable controls.");
  }
  if (!activation.isActive()) throw fail("NotAllowedError", "HTMLInputElement::showPicker() requires a user gesture.");
  if (type === "color") return showColorPicker(this, "showPicker");
  if (type !== "file") {
    warnCompat(
      `showPicker:${type}`,
      `showPicker() on <input type=${type}> is a no-op: only file and color pickers exist here`,
    );
    return;
  }
  showFilePicker(fileInputOps(this), "showPicker");
});
