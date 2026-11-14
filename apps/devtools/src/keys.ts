// Every keyboard handler in this app therefore needs the same one-line narrowing, and doing it at
// each call site would put a cast in five components.

export function onKeydown(handler: (event: SkKeyboardEvent) => void): () => void {
  const wrapped = (event: SkEvent) => handler(event as SkKeyboardEvent);
  addEventListener("keydown", wrapped);
  return () => removeEventListener("keydown", wrapped);
}
