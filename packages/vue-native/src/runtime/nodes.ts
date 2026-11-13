import { touchDom } from "./collections";
import { I } from "./event-names";
import { optionsChangedIn } from "./select";
import { notifyMutation } from "./mutation-observer";
import { releaseCapturesIn } from "./pointer-capture";
import type { HandlerMap, MirrorNode } from "./types";

export const nodesById = new Map<VnNodeId, MirrorNode>();

// Loose nodes (made by `document.createElement`, `createTextNode`, `new DocumentFragment()`, or taken out of the tree)
// are held weakly; `sweepLoose` frees the host nodes of collected subtrees. The host has no "detach", so a removed
// subtree is parked under an unattached "box" node, as <KeepAlive> parks a deactivated one.
// Invariant: a node is loose exactly when its topmost ancestor is in `looseRoots`; `nodeOps.insert` moves a subtree
// between the registries when it changes sides.
export const looseById = new Map<VnNodeId, WeakRef<MirrorNode>>();
/** The root of each loose subtree, and the box a removed one is parked in (0: none, it never had a parent). */
export const looseRoots = new Map<VnNodeId, { ref: WeakRef<MirrorNode>; box: VnNodeId }>();
export function nodeById(id: VnNodeId): MirrorNode | undefined {
  return nodesById.get(id) ?? (looseById.size ? looseById.get(id)?.deref() : undefined);
}
export const isLoose = (n: MirrorNode): boolean => looseById.size !== 0 && looseById.get(n.__vnId)?.deref() === n;
export function loosen(n: MirrorNode): void {
  forEachDeep(n, (d) => {
    if (nodesById.get(d.__vnId) === d) nodesById.delete(d.__vnId);
    looseById.set(d.__vnId, new WeakRef(d));
  });
}
export function tighten(n: MirrorNode): void {
  forEachDeep(n, (d) => {
    looseById.delete(d.__vnId);
    nodesById.set(d.__vnId, d);
  });
}
let sweepQueued = false;
const looseFinalizer: FinalizationRegistry<VnNodeId> | null =
  typeof FinalizationRegistry === "function"
    ? new FinalizationRegistry<VnNodeId>(() => {
        if (sweepQueued) return;
        sweepQueued = true;
        queueMicrotask(sweepLoose);
      })
    : null;
/** Without a FinalizationRegistry the sweep runs every this many new loose roots. */
let rootsSinceSweep = 0;
function addLooseRoot(n: MirrorNode, box: VnNodeId): void {
  looseRoots.set(n.__vnId, { ref: new WeakRef(n), box });
  if (looseFinalizer) looseFinalizer.register(n, n.__vnId, n);
  else if (++rootsSinceSweep >= 256) {
    rootsSinceSweep = 0;
    sweepLoose();
  }
}
export function dropLooseRoot(n: MirrorNode): VnNodeId {
  if (looseRoots.size === 0) return 0;
  const r = looseRoots.get(n.__vnId);
  if (!r || r.ref.deref() !== n) return 0;
  looseRoots.delete(n.__vnId);
  looseFinalizer?.unregister(n);
  return r.box;
}
function sweepLoose(): void {
  sweepQueued = false;
  for (const [id, r] of looseRoots) {
    if (r.ref.deref()) continue;
    looseRoots.delete(id);
    __vn_remove(r.box || id);
  }
  for (const [id, ref] of looseById) if (!ref.deref()) looseById.delete(id);
}
export function looseCreated<T extends MirrorNode>(n: T): T {
  loosen(n);
  addLooseRoot(n, 0);
  return n;
}
/** Takes `child` out of its parent WITHOUT freeing it (a script may re-insert it); its subtree becomes loose, parked in a box. */
export function detachNode(child: MirrorNode, notify = true): void {
  const p = child.parentNode;
  if (!p) return;
  releaseCapturesIn(child);
  const i = p.childNodes.indexOf(child);
  if (i >= 0) p.childNodes.splice(i, 1);
  child.parentNode = null;
  touchDom();
  optionsChangedIn(p);
  const box = __vn_create_element(I("vn-detached"));
  __vn_insert(child.__vnId, box, 0);
  if (!isLoose(child)) loosen(child);
  addLooseRoot(child, box);
  if (notify) notifyMutation(p, "childList", { addedNodes: [], removedNodes: [child] });
}
/** Forgets a subtree whose host nodes are freed: registries, handlers, and its ids (set to 0, since host ids are recycled). */
export function forgetDeep(n: MirrorNode): void {
  releaseCapturesIn(n);
  forEachDeep(n, (d) => {
    handlers.delete(d);
    if (nodesById.get(d.__vnId) === d) nodesById.delete(d.__vnId);
    else if (looseById.size) looseById.delete(d.__vnId);
    d.__vnId = 0;
  });
}

export function forEachDeep(n: MirrorNode, f: (n: MirrorNode) => void) {
  f(n);
  for (const c of n.childNodes) forEachDeep(c, f);
}

// node -> { click: fn }. Weak, so a loose node is not kept alive by its own listeners.
export const handlers = new WeakMap<MirrorNode, HandlerMap>();
