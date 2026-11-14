import type { MirrorElement } from "./types";
import type { GlobalEventMethods, GlobalEventSlot } from "./listeners";

// Mutable state that more than one module writes. A leaf, so it is initialized before any module that installs a
// hook evaluates, whatever order the import cycle between them takes.

export const hooks: {
  /** Claims `<style>`/`<link>` insertions and removals for the stylesheet registry. */
  foreignStyle: ((n: object, inserted: boolean) => boolean) | null;
  /** Replays pre-mount `documentElement` writes onto the real root. */
  flushDocumentElement: ((root: MirrorElement) => void) | null;
} = { foreignStyle: null, flushDocumentElement: null };

export const interceptedGlobals: { slot: GlobalEventSlot; delegate: GlobalEventMethods }[] = [];
