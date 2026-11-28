import type { MirrorElement } from "./types";
import { looseById, looseRoots, nodesById, pendingImages } from "./nodes";
import { mk } from "./node-ops";
import { disposeGlobalListener, dndRearm, dndState, globalHandlers, syncListenerFlag } from "./listeners";
import { pointer, pointerCaptures, releaseCapturedPointer, touches } from "./pointer-capture";
import { observers } from "./mutation-observer";

export let appRoot: MirrorElement | null = null; // made when the runtime evaluates, and again after a teardown

export function teardownDocumentRoot(root: MirrorElement, unmount?: () => void): void {
  if (!root.__skId) return;
  try {
    unmount?.();
  } catch (e) {
    __sk_log("teardown: " + e);
  }
  pointer.buttons = 0;
  for (const pointerId of Array.from(pointerCaptures.keys())) releaseCapturedPointer(pointerId);
  touches.clear();
  dndState.gesture = null;
  dndState.seen = false;
  dndState.swallowClick = false;
  __sk_remove(root.__skId);
  // <KeepAlive> creates its cache container with createElement and never inserts or removes it; what is left in the
  // mirror after app.unmount() is such an orphan, freed here or the host arena grows by one node per KeepAlive on each
  // hot restart.
  for (const id of Array.from(nodesById.keys())) __sk_remove(id);
  // …and every loose subtree, held by script or not: the next app's nodes reuse these ids.
  for (const [id, r] of looseRoots) __sk_remove(r.box || id);
  for (const n of nodesById.values()) n.__skId = 0;
  for (const ref of looseById.values()) {
    const n = ref.deref();
    if (n) n.__skId = 0;
  }
  root.__skId = 0;
  nodesById.clear();
  pendingImages.clear();
  looseById.clear();
  looseRoots.clear();
  globalThis.__SK_CSS__ = [];
  globalThis.__SK_TEARDOWN__ = undefined;
  appRoot = null;
  observers.clear();
  __sk_style_observe(false);
  for (const map of globalHandlers.values()) {
    for (const buckets of Object.values(map)) {
      if (buckets) for (const entry of [...buckets.extraC, ...buckets.extraB]) disposeGlobalListener(entry);
    }
  }
  globalHandlers.clear();
}

export function ensureDocumentRoot(): MirrorElement {
  if (appRoot) return appRoot;
  // Named `html`, not `ROOT`: this root answers to `document.documentElement` and `document.body`, and DOM-walking
  // libraries stop at `html`: under any other name @floating-ui's `getOverflowAncestors` loops forever. The Rust tag
  // stays "root".
  const root = mk(__sk_create_root(), 1, "html");
  appRoot = root;
  globalThis.__SK_TEARDOWN__ = () => teardownDocumentRoot(root);
  // Listeners added to `document`/`window` at module scope had no root to carry their bit; sync them now.
  for (const m of globalHandlers.values()) for (const name of Object.keys(m)) syncListenerFlag(root, name);
  dndRearm();
  return root;
}
