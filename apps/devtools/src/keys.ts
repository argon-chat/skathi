// Every keyboard handler in this app therefore needs the same one-line narrowing, and doing it at
// each call site would put a cast in five components.

export function onKeydown(handler: (event: VnKeyboardEvent) => void): () => void {
  const wrapped = (event: VnEvent) => handler(event as VnKeyboardEvent);
  addEventListener("keydown", wrapped);
  return () => removeEventListener("keydown", wrapped);
}
