import type { MirrorElement, MirrorKind, MirrorNode, MirrorNodeInit, MirrorText } from "./types";
import {
  detachNode,
  dropLooseRoot,
  forgetDeep,
  isLoose,
  looseById,
  loosen,
  nodeById,
  nodesById,
  tighten,
} from "./nodes";
import { ElementProto, TextProto } from "./protos";
import { childList, touchDom } from "./collections";
import { I } from "./event-names";
import { WINDOW_ROOT, windowRoots } from "../skwindow";
import { adoptCanvas } from "../canvas";
import { appRoot } from "./root";
import { defineOptionMembers, defineSelectMembers, optionsChangedIn } from "./select";
import { notifyMutation } from "./mutation-observer";
import { releaseCapturesIn } from "./pointer-capture";
import { queryOne } from "./dom-members";
import { addExtraListener, removeExtraListener, syncListenerFlag } from "./listeners";
import { noteChildren, noteWrite } from "./perf-advice";

export function mk(id: SkNodeId, kind: 1, tag: string): MirrorElement;
export function mk(id: SkNodeId, kind: 2 | 3, tag: string): MirrorText;
export function mk(id: SkNodeId, kind: MirrorKind, tag: string): MirrorNode;
export function mk(id: SkNodeId, kind: MirrorKind, tag: string): MirrorNode {
  const n = Object.create(kind === 1 ? ElementProto : TextProto) as MirrorNodeInit;
  n.__skId = id;
  n.kind = kind;
  n.tag = tag;
  n.parentNode = null;
  n.childNodes = childList();
  if (kind === 1) {
    n.tagName = String(tag).toUpperCase();
    // Storage under the prototype's `value` accessor; being OWN also tells a mirror element from a detached engine one.
    n._value = "";
    // Written only by `patchProp`. reka-ui reads `data-*`/`aria-*` back off nodes, so the mirror must remember them.
    n.attrs = Object.create(null);
    // Kept apart from `<Transition>`'s classes so a re-render does not wipe an in-flight `v-enter-active`.
    n._class = "";
    // `scrollTop`/`scrollLeft` are prototype accessors over the host's `ScrollHandle`; an own data property would shadow them.
    // Own methods shadow the prototype's: they route into the registry the renderer's props use, so propagation sees both.
    n.addEventListener = (name, fn, opts) => addExtraListener(n as MirrorElement, name, fn, opts);
    n.removeEventListener = (name, fn, opts) => removeExtraListener(n as MirrorElement, name, fn, opts);
  }
  nodesById.set(id, n as MirrorNode);
  return n as MirrorNode;
}

/** The tree operations a renderer drives the mirror with: Vue's `RendererOptions` without `patchProp`. */
export interface NodeOps {
  createElement(
    type: string,
    namespace?: "svg" | "mathml",
    isCustomizedBuiltIn?: string,
    props?: Record<string | symbol, unknown> | null,
  ): MirrorElement;
  createText(text: string): MirrorNode;
  createComment(text: string): MirrorNode;
  setText(node: MirrorNode, text: string): void;
  setElementText(node: MirrorElement, text: string): void;
  insert(el: MirrorNode, parent: MirrorElement, anchor?: MirrorNode | null): void;
  remove(el: MirrorNode): void;
  parentNode(node: MirrorNode): MirrorElement | null;
  nextSibling(node: MirrorNode): MirrorNode | null;
  querySelector(selector: string): MirrorElement | null;
  setScopeId(el: MirrorElement, id: string): void;
}

export const nodeOps: NodeOps = {
  createElement: (type, namespace, _is, props) => {
    const el = mk(__sk_create_element(I(type)), 1, type);
    // `<SkWindow>`'s root: registered before its content mounts (skwindow.ts).
    if (props && (props as Record<symbol, unknown>)[WINDOW_ROOT]) windowRoots.add(el);
    // `<svg>`/`<math>` and their content keep their tag's case (`linearGradient`), and `instanceof SVGElement`/`MathMLElement` hold.
    if (namespace === "svg" || namespace === "mathml") {
      el._ns = namespace;
      el.tagName = type;
    }
    // The host must deliver `click` for `<label>` (it activates its control) and `<button>` (it submits), and `keydown`
    // for `<input>` (implicit submission), whether or not anything listens.
    if (type === "label" || type === "button") syncListenerFlag(el, "click");
    if (type === "input") syncListenerFlag(el, "keydown");
    if (type === "input" || type === "textarea") syncListenerFlag(el, "input");
    if (type === "canvas") adoptCanvas(el);
    if (!namespace && type === "select") defineSelectMembers(el);
    else if (!namespace && type === "option") defineOptionMembers(el);
    return el;
  },
  createText: (text) => {
    const n = mk(__sk_create_text(), 2, "#text");
    n.text = String(text);
    __sk_set_text(n.__skId, n.text);
    return n;
  },
  createComment: () => mk(__sk_create_anchor(), 3, "#comment"),
  setText: (node, text) => {
    node.text = String(text);
    __sk_set_text(node.__skId, node.text);
    if (__DEV__) noteWrite(node);
  },
  setElementText: (node, text) => {
    // DETACH the children, do not free them, as `el.textContent = text` does: a leaving <Transition> child stays until
    // its leave ends, and a script-appended one is still the script's. Freeing either leaves a mirror over a recycled id.
    const kids = node.childNodes.length ? node.childNodes.slice() : null;
    if (kids) for (const c of kids) detachNode(c, false);
    node.ownText = String(text);
    __sk_set_element_text(node.__skId, String(text));
    if (__DEV__) noteWrite(node);
    if (kids) optionsChangedIn(node);
    notifyMutation(node, "childList", { addedNodes: [], removedNodes: kids ?? [] });
  },
  insert: (child, parent, anchor) => {
    if (__DEV__) noteChildren(parent);
    insertNode(child, parent, anchor ?? null, true);
  },
  remove: (child) => {
    const p = child.parentNode;
    if (__DEV__) noteChildren(p);
    if (p) {
      const i = p.childNodes.indexOf(child);
      if (i >= 0) p.childNodes.splice(i, 1);
      child.parentNode = null;
      touchDom();
      optionsChangedIn(p);
      notifyMutation(p, "childList", { addedNodes: [], removedNodes: [child] });
    }
    if (nodeById(child.__skId) !== child) return;
    const id = child.__skId;
    const box = dropLooseRoot(child);
    forgetDeep(child);
    __sk_remove(box || id);
  },
  parentNode: (node) => node.parentNode,
  nextSibling: (node) => {
    const p = node.parentNode;
    if (!p) return null;
    const i = p.childNodes.indexOf(node);
    return i >= 0 ? (p.childNodes[i + 1] ?? null) : null;
  },
  // <style scoped>: the Rust matcher buckets on the data-v id Vue stamps on scoped elements.
  setScopeId: (el, id) => __sk_add_scope(el.__skId, id),
  // Resolves a STRING Teleport `to` (reka-ui portals use "body"). Document roots mean the app root here; anything else
  // goes through the real matcher. A Teleport inside a `<SkWindow>` asks `windowTeleport` instead.
  querySelector: (sel) => {
    if (DOCUMENT_ROOTS.has(sel) && sel !== "#app") return appRoot;
    return queryOne(sel, appRoot ? appRoot.__skId : 0) ?? (sel === "#app" ? appRoot : null);
  },
};

const DOCUMENT_ROOTS = new Set(["body", "html", ":root", "#app"]);

/** The `<SkWindow>` root `node` paints under, or `null` for the main window (a pop-out's content stays in the one tree). */
export function windowRootOf(node: MirrorNode | null | undefined): MirrorElement | null {
  if (windowRoots.size === 0) return null;
  for (let n = node; n; n = n.parentNode) if (windowRoots.has(n)) return n as MirrorElement;
  return null;
}

interface MountingInstance {
  parent: MountingInstance | null;
  subTree?: { el?: unknown } | null;
}

/**
 * The `<SkWindow>` root that `container`, not in the document yet, will paint under, or `null`. Vue mounts children
 * before inserting the element, so a `<Teleport>` gets a container whose parents stop short of the window root; the
 * nearest `<SkWindow>` above the Teleport, the instance whose root element is a window root, says which window it is in.
 */
function mountingWindowRoot(
  container: MirrorNode | null | undefined,
  parent: MountingInstance | null | undefined,
): MirrorElement | null {
  if (windowRoots.size === 0 || !container) return null;
  for (let n: MirrorNode | null = container; n; n = n.parentNode) if (n === appRoot) return null;
  for (let i = parent; i; i = i.parent) {
    const el = i.subTree?.el as MirrorElement | null | undefined;
    if (el && windowRoots.has(el)) return el;
  }
  return null;
}

/**
 * `<Teleport to="body">` inside a `<SkWindow>` lands in THAT window. Teleport resolves a string `to` through the
 * renderer options it is handed (`internals.o.querySelector`), so it gets options whose document roots are the
 * window root its placeholder sits under.
 */
const TELEPORT_ORIGINAL = Symbol.for("sk.teleport.original");
type TeleportProcess = ((this: unknown, ...args: unknown[]) => unknown) & { [TELEPORT_ORIGINAL]?: TeleportProcess };
interface TeleportInternals {
  o: { querySelector?: (sel: string) => unknown };
}
export function patchTeleport(teleport: object): void {
  const impl = teleport as { process: TeleportProcess };
  // A reload may re-evaluate this module over the same Teleport: wrap Vue's own `process`, never a predecessor's
  // wrapper, whose window registry is the old app's.
  let process = impl.process;
  while (process[TELEPORT_ORIGINAL]) process = process[TELEPORT_ORIGINAL];
  const wrapped: TeleportProcess = function (this: unknown, ...args: unknown[]) {
    const container = args[2] as MirrorNode | null;
    const root = windowRootOf(container) ?? mountingWindowRoot(container, args[4] as MountingInstance | null);
    const internals = args[9] as TeleportInternals | undefined;
    if (root && internals?.o) {
      const select = internals.o.querySelector;
      args[9] = {
        ...internals,
        o: {
          ...internals.o,
          querySelector: (sel: string) => (DOCUMENT_ROOTS.has(sel) ? root : select ? select(sel) : null),
        },
      };
    }
    return process.apply(this, args);
  };
  wrapped[TELEPORT_ORIGINAL] = process;
  impl.process = wrapped;
}

/**
 * The document a node in a `<SkWindow>` belongs to (`ownerDocument`). floating-ui measures the viewport as
 * `el.ownerDocument.documentElement.clientWidth`, so the root elements answer with the window's root; the rest is
 * the real document. Not `=== document`, like a browser popup's.
 */
const WINDOW_DOCUMENTS = new WeakMap<object, object>();
export function windowDocument(root: MirrorElement): object {
  const known = WINDOW_DOCUMENTS.get(root);
  if (known) return known;
  const real = globalThis.document as unknown as object;
  const bound = new WeakMap<object, unknown>();
  const doc = new Proxy(real, {
    get(target, key) {
      if (key === "documentElement" || key === "body" || key === "scrollingElement") return root;
      const v: unknown = Reflect.get(target, key, target);
      if (typeof v !== "function") return v;
      // Methods answer for the real document (native `this`), the same function on every read.
      let b = bound.get(v);
      if (!b) bound.set(v, (b = (v as (...a: unknown[]) => unknown).bind(target)));
      return b;
    },
  });
  WINDOW_DOCUMENTS.set(root, doc);
  return doc;
}

/** Insert or move. `notifyAdd: false` leaves the addition record to the caller (one per fragment); the old parent always hears the removal. */
export function insertNode(
  child: MirrorNode,
  parent: MirrorElement,
  anchor: MirrorNode | null,
  notifyAdd: boolean,
): void {
  const old = child.parentNode;
  if (old && (isLoose(parent) || windowRootOf(child) !== windowRootOf(parent))) releaseCapturesIn(child);
  if (old) {
    const cs = old.childNodes;
    const i = cs.indexOf(child);
    if (i >= 0) cs.splice(i, 1);
  }
  let idx = parent.childNodes.length;
  if (anchor != null) {
    const i = parent.childNodes.indexOf(anchor);
    if (i >= 0) idx = i;
  }
  parent.childNodes.splice(idx, 0, child);
  child.parentNode = parent;
  touchDom();
  if (parent.ownText) parent.ownText = "";
  __sk_insert(child.__skId, parent.__skId, anchor ? anchor.__skId : 0);
  if (looseById.size) {
    const box = dropLooseRoot(child);
    if (box) __sk_remove(box);
    const now = isLoose(parent);
    if (now !== isLoose(child)) (now ? loosen : tighten)(child);
  }
  if (old && old !== parent) optionsChangedIn(old);
  optionsChangedIn(parent);
  if (old) notifyMutation(old, "childList", { addedNodes: [], removedNodes: [child] });
  if (notifyAdd) notifyMutation(parent, "childList", { addedNodes: [child], removedNodes: [] });
}
