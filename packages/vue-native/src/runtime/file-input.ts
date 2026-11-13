import type { MirrorElement } from "./types";
import { warnCompat } from "../compat";
import { isMirrorNode, syncListenerFlag } from "./listeners";
import { defineDom } from "./dom-members";
import { I } from "./event-names";
import { patchProp } from "./patch-prop";
import { activation, fsHost } from "./activation";
import { isMirrorEl, isRangeInput, writeControlValue } from "./controls";
import { propagate, synthesize } from "./events";
import { activeElement } from "./focus";
import { isDisabledControl } from "./forms";
import { fireDetached } from "./labels";
import type { Detached } from "./labels";

// A file input's click opens the picker through vn-desktop; its `files`, `value`, `accept`, `multiple` and `webkitdirectory`
// are own accessors installed when `type` becomes "file". Detached `document.createElement("input")` (VueUse's
// `useFileDialog`) gets the same.

function fileStatus(files: ArrayLike<{ name?: unknown }> | null): string {
  const n = files?.length ?? 0;
  if (n === 0) return "No file chosen";
  return n === 1 ? String(files![0]?.name ?? "") : `${n} files`;
}

export function isFileInput(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return (
    !!e &&
    e.kind === 1 &&
    e.tag === "input" &&
    String(e.attrs?.type ?? "")
      .trim()
      .toLowerCase() === "file"
  );
}

export function isColorInput(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return (
    !!e &&
    e.kind === 1 &&
    e.tag === "input" &&
    String(e.attrs?.type ?? "")
      .trim()
      .toLowerCase() === "color"
  );
}

export function colorValue(value: unknown): string {
  const text = String(value ?? "")
    .trim()
    .toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(text)) return text;
  if (/^#[0-9a-f]{3,4}$/.test(text)) return "#" + [...text.slice(1, 4)].map((c) => c + c).join("");
  if (/^#[0-9a-f]{8}$/.test(text)) return text.slice(0, 7);
  if (text)
    warnCompat(
      "input-color:value",
      "<input type=color> currently accepts hex colors (#rgb, #rgba, #rrggbb, #rrggbbaa); other CSS color values normalize to #000000",
    );
  return "#000000";
}

// A picker is asynchronous. An already-open picker cannot be replaced by a
// second click, and its result belongs to the original control.
const colorPickers = new WeakSet<object>();
export function showColorPicker(el: MirrorElement | Detached, via: "showPicker" | null): void {
  const mirror = isMirrorNode(el);
  if (mirror ? isDisabledControl(el as MirrorElement) : !!el.disabled) return;
  if (colorPickers.has(el)) return;
  if (via === null && !activation.isActive()) {
    console.warn("Color chooser dialog can only be shown with a user activation.");
    return;
  }
  if (!fsHost) {
    warnCompat("input-color:nohost", "<input type=color>: this host has no vn-desktop, so there is no color picker");
    return;
  }
  const before = colorValue(el.value);
  const id = mirror ? (el as MirrorElement).__vnId : 0;
  activation.consume();
  colorPickers.add(el);
  void fsHost
    .pickColor(before)
    .then((selected) => {
      if (selected == null) return;
      const next = colorValue(selected);
      if (mirror) {
        const control = el as MirrorElement;
        if (control.__vnId !== id || !isColorInput(control) || next === colorValue(control.value)) return;
        writeControlValue(control, next);
        propagate(control, "input", fileEvent("input"));
        propagate(control, "change", fileEvent("change"));
      } else {
        if (String(el.type).trim().toLowerCase() !== "color" || next === colorValue(el.value)) return;
        el.value = next;
        fireDetached(el, "input");
        fireDetached(el, "change");
      }
    })
    .finally(() => colorPickers.delete(el));
}

/** Is this one of the host's text widgets? An `<input>` other than file/range/color, or a `<textarea>` (`vn-tree` `build_node`). */
export function isTextWidget(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return (
    !!e &&
    e.kind === 1 &&
    (e.tag === "textarea" || (e.tag === "input" && !isFileInput(e) && !isRangeInput(e) && !isColorInput(e)))
  );
}

/** Select a field through its native editor, so the next keystroke, IME commit or clipboard operation does not replace a JS-only range. */
defineDom("select", function (this: MirrorElement) {
  if (!isMirrorEl(this) || (this.tag !== "input" && this.tag !== "textarea")) throw new TypeError("Illegal invocation");
  if (activeElement() !== this) this.focus();
  if (!isTextWidget(this) || !__vn_input_select(this.__vnId)) return;
  // HTML queues a bubbling, non-cancelable select event. It is not an edit:
  // no input/change event or value assignment is manufactured here.
  queueMicrotask(() => {
    if (this.__vnId) propagate(this, "select", synthesize("select", 0, 0, 0, 0, undefined));
  });
});

export function isLabel(n: unknown): boolean {
  const e = n as MirrorElement | null | undefined;
  return !!e && e.kind === 1 && e.tag === "label";
}

function isFileList(v: unknown): boolean {
  const FL = (globalThis as { FileList?: unknown }).FileList;
  if (typeof FL === "function") return v instanceof (FL as new () => object);
  return (
    !!v &&
    typeof v === "object" &&
    typeof (v as { length?: unknown }).length === "number" &&
    typeof (v as { item?: unknown }).item === "function"
  );
}

function emptyFileList(): unknown {
  const DT = (globalThis as { DataTransfer?: new () => { files: unknown } }).DataTransfer;
  if (typeof DT === "function") {
    try {
      return new DT().files;
    } catch {
      /* fall through to the array-like below */
    }
  }
  return Object.freeze(Object.assign([], { item: () => null }));
}

export interface FileState {
  files: unknown;
  changed: () => void;
}
export const fileStates = new WeakMap<object, FileState>();

const FAKEPATH = "C:\\fakepath\\";
const VALUE_REFUSED =
  "Failed to set the 'value' property on 'HTMLInputElement': This input element accepts a filename, " +
  "which may only be programmatically set to the empty string.";
const FILES_REFUSED =
  "Failed to set the 'files' property on 'HTMLInputElement': The provided value is not of type 'FileList'.";

/** HTML's `files` and filename-mode `value` over one state: `files` is always a FileList (setting it fires nothing); `value` is `C:\fakepath\<name>`, "" clears, anything else throws. */
export function fileAccessors(state: FileState, isFile: () => boolean, other?: { value: string }) {
  const list = () => (state.files ??= emptyFileList()) as ArrayLike<{ name?: unknown }>;
  return {
    files: {
      get: () => (isFile() ? list() : null),
      set: (v: unknown) => {
        if (!isFile() || v == null) return;
        if (!isFileList(v)) throw new TypeError(FILES_REFUSED);
        state.files = v;
        state.changed();
      },
      configurable: true,
      enumerable: true,
    },
    value: {
      get: () => {
        if (!isFile()) return other?.value ?? "";
        const f = list();
        return f.length ? FAKEPATH + String(f[0]?.name ?? "") : "";
      },
      set: (v: unknown) => {
        if (!isFile()) {
          if (other) other.value = v == null ? "" : String(v);
          return;
        }
        if (v != null && String(v) !== "") throw new DOMException(VALUE_REFUSED, "InvalidStateError");
        state.files = emptyFileList();
        state.changed();
      },
      configurable: true,
      enumerable: true,
    },
  };
}

let advisedFileLook = false;

export function syncFileInput(el: MirrorElement): void {
  const file = isFileInput(el);
  const had = fileStates.has(el);
  if (file && !had) {
    const state: FileState = { files: null, changed: () => {} };
    state.changed = () =>
      __vn_set_prop(el.__vnId, I(":file-label"), fileStatus(state.files as ArrayLike<{ name?: unknown }> | null));
    fileStates.set(el, state);
    const acc = fileAccessors(state, () => isFileInput(el));
    const flag = (k: string) => ({
      get: () => el.attrs[k] != null && el.attrs[k] !== "false",
      set: (v: unknown) => patchProp(el, k, el.attrs[k], v ? "" : null),
      configurable: true,
      enumerable: true,
    });
    Object.defineProperties(el, {
      files: acc.files,
      value: acc.value,
      accept: {
        get: () => el.attrs.accept ?? "",
        set: (v: unknown) => patchProp(el, "accept", el.attrs.accept, String(v)),
        configurable: true,
        enumerable: true,
      },
      multiple: flag("multiple"),
      webkitdirectory: flag("webkitdirectory"),
    });
    state.changed();
    if (!advisedFileLook) {
      advisedFileLook = true;
      warnCompat(
        "input-file:look",
        "<input type=file>: the button and status are an approximation of Chromium's, with its " +
          'English strings ("Choose File", "No file chosen"); restyle it with ::file-selector-button',
      );
    }
  } else if (!file && had) {
    fileStates.delete(el);
    for (const k of ["files", "value", "accept", "multiple", "webkitdirectory"])
      delete (el as unknown as Record<string, unknown>)[k];
    el._value = "";
    __vn_set_prop(el.__vnId, I(":file-label"), undefined);
  }
  for (const n of ["click", "keydown", "keyup"]) syncListenerFlag(el, n);
}

export interface FileInputOps {
  accept: string;
  multiple: boolean;
  directory: boolean;
  disabled: boolean;
  current: unknown;
  setFiles(list: unknown): void;
  fire(type: "input" | "change" | "cancel"): void;
}

export const fileEvent = (type: string): VnEvent =>
  new Event(type, { bubbles: true, composed: type === "input" }) as VnEvent;

export function fileInputOps(el: MirrorElement): FileInputOps {
  const flag = (k: string) => el.attrs[k] != null && el.attrs[k] !== "false";
  if (!fileStates.has(el)) syncFileInput(el);
  const state = fileStates.get(el)!;
  return {
    accept: el.attrs.accept ?? "",
    multiple: flag("multiple"),
    directory: flag("webkitdirectory"),
    disabled: flag("disabled"),
    current: state.files,
    setFiles: (list) => {
      state.files = list;
      state.changed();
    },
    fire: (type) => void propagate(el, type, fileEvent(type)),
  };
}

/** HTML "show the picker": a click without transient activation only warns, in Chromium's words. Picked: `files` changes, then `input` and `change`; dismissed or same selection: `cancel`. */
export function showFilePicker(ops: FileInputOps, via: "showPicker" | null): void {
  if (ops.disabled) return;
  if (via === null && !activation.isActive()) {
    console.warn("File chooser dialog can only be shown with a user activation.");
    return;
  }
  if (!fsHost) {
    warnCompat("input-file:nohost", "<input type=file>: this host has no vn-desktop, so there is no file picker");
    return;
  }
  void fsHost.pickForInput(ops.accept, ops.multiple, ops.directory, ops.current).then((list) => {
    if (list === undefined) return;
    if (list === null) return ops.fire("cancel");
    ops.setFiles(list);
    ops.fire("input");
    ops.fire("change");
  });
}
