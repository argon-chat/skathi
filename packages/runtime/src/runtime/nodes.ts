import { touchDom } from "./collections";
import { I } from "./event-names";
import { optionsChangedIn } from "./select";
import { notifyMutation } from "./mutation-observer";
import { releaseCapturesIn } from "./pointer-capture";
import type { HandlerMap, MirrorNode } from "./types";

export const nodesById = new Map<SkNodeId, MirrorNode>();

// Loose nodes (made by `document.createElement`, `createTextNode`, `new DocumentFragment()`, or taken out of the tree)
// are held weakly; `sweepLoose` frees the host nodes of collected subtrees. The host has no "detach", so a removed
// subtree is parked under an unattached "box" node, as <KeepAlive> parks a deactivated one.
// Invariant: a node is loose exactly when its topmost ancestor is in `looseRoots`; `nodeOps.insert` moves a subtree
// between the registries when it changes sides.
export const looseById = new Map<SkNodeId, WeakRef<MirrorNode>>();
/** The root of each loose subtree, and the box a removed one is parked in (0: none, it never had a parent). */
export const looseRoots = new Map<SkNodeId, { ref: WeakRef<MirrorNode>; box: SkNodeId }>();
export function nodeById(id: SkNodeId): MirrorNode | undefined {
  return nodesById.get(id) ?? (looseById.size ? looseById.get(id)?.deref() : undefined);
}
__sk_set_node_lookup(nodeById);
export const isLoose = (n: MirrorNode): boolean => looseById.size !== 0 && looseById.get(n.__skId)?.deref() === n;
export function loosen(n: MirrorNode): void {
  forEachDeep(n, (d) => {
    if (nodesById.get(d.__skId) === d) nodesById.delete(d.__skId);
    looseById.set(d.__skId, new WeakRef(d));
  });
}
export function tighten(n: MirrorNode): void {
  forEachDeep(n, (d) => {
    looseById.delete(d.__skId);
    nodesById.set(d.__skId, d);
  });
}
let sweepQueued = false;
const looseFinalizer: FinalizationRegistry<SkNodeId> | null =
  typeof FinalizationRegistry === "function"
    ? new FinalizationRegistry<SkNodeId>(() => {
        if (sweepQueued) return;
        sweepQueued = true;
        queueMicrotask(sweepLoose);
      })
    : null;
/** Without a FinalizationRegistry the sweep runs every this many new loose roots. */
let rootsSinceSweep = 0;
function addLooseRoot(n: MirrorNode, box: SkNodeId): void {
  looseRoots.set(n.__skId, { ref: new WeakRef(n), box });
  if (looseFinalizer) looseFinalizer.register(n, n.__skId, n);
  else if (++rootsSinceSweep >= 256) {
    rootsSinceSweep = 0;
    sweepLoose();
  }
}
export function dropLooseRoot(n: MirrorNode): SkNodeId {
  if (looseRoots.size === 0) return 0;
  const r = looseRoots.get(n.__skId);
  if (!r || r.ref.deref() !== n) return 0;
  looseRoots.delete(n.__skId);
  looseFinalizer?.unregister(n);
  return r.box;
}
function sweepLoose(): void {
  sweepQueued = false;
  for (const [id, r] of looseRoots) {
    if (r.ref.deref()) continue;
    looseRoots.delete(id);
    __sk_remove(r.box || id);
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
  const box = __sk_create_element(I("sk-detached"));
  __sk_insert(child.__skId, box, 0);
  if (!isLoose(child)) loosen(child);
  addLooseRoot(child, box);
  if (notify) notifyMutation(p, "childList", { addedNodes: [], removedNodes: [child] });
}
/** Forgets a subtree whose host nodes are freed: registries, handlers, and its ids (set to 0, since host ids are recycled). */
export function forgetDeep(n: MirrorNode): void {
  releaseCapturesIn(n);
  forEachDeep(n, (d) => {
    handlers.delete(d);
    if (nodesById.get(d.__skId) === d) nodesById.delete(d.__skId);
    else if (looseById.size) looseById.delete(d.__skId);
    d.__skId = 0;
  });
}

export function forEachDeep(n: MirrorNode, f: (n: MirrorNode) => void) {
  f(n);
  for (const c of n.childNodes) forEachDeep(c, f);
}

// node -> { click: fn }. Weak, so a loose node is not kept alive by its own listeners.
export const handlers = new WeakMap<MirrorNode, HandlerMap>();
