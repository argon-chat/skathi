// Every keyboard handler in this app therefore needs the same one-line narrowing, and doing it at
// each call site would put a cast in five components.

export function onKeydown(handler: (event: KeyboardEvent) => void): () => void {
  const wrapped = (event: Event) => handler(event as KeyboardEvent);
  addEventListener("keydown", wrapped);
  return () => removeEventListener("keydown", wrapped);
}
