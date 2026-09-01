// One window-level `keydown` subscription, typed.
//
// `addEventListener` here is the engine's own `EventTarget`, so its listener
// parameter is `VnEvent` — the base shape, with no `key` on it. Every keyboard
// handler in this app therefore needs the same one-line narrowing, and doing it
// at each call site would put a cast in five components. This is that cast,
// once, with the reason attached.
//
// The keystroke reaches the app because the retained tree's ROOT holds a gpui
// focus handle, which is what puts `document`/`window` listeners on the focus
// path (crates/vn-tree/src/render.rs, `claim_press_focus`). That is the same
// mechanism every Escape-to-dismiss in the ecosystem rides on, so this is the
// ordinary path rather than a devtools-only trick.

/** Subscribes to `keydown`; returns the unsubscriber. */
export function onKeydown(handler: (event: VnKeyboardEvent) => void): () => void {
  const wrapped = (event: VnEvent) => handler(event as VnKeyboardEvent);
  addEventListener("keydown", wrapped);
  return () => removeEventListener("keydown", wrapped);
}
