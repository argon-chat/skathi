// `<canvas>`: its bitmap and its 2D context (the host+media stream's module).
//
// A STUB until that module lands: runtime.ts already calls `adoptCanvas` on
// every mirror `<canvas>` (`nodeOps.createElement`, which the renderer and
// `document.createElement` share), so the call site and the import exist
// before the implementation does. Until then a canvas has no `getContext`, as
// before. The real module replaces this file whole.
//
// It must stay importable on its own: test/runtime-harness.ts evaluates it once
// and binds `adoptCanvas` into runtime.ts as a parameter, so nothing here may
// touch a `__vn_*` host function at module scope.

/** Gives a mirror `<canvas>` its canvas members. The stub gives none. */
export function adoptCanvas(_el: object): void {}
