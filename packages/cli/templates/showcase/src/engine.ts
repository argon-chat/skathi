// The web surface the pages use, resolved once. Engine parts an app leaves out ("windows": { "without": [...] }
// in sk.config.json) are absent globals rather than stubs, so each one is `null` here when its part is out.

function required<T>(value: T | undefined, name: string): T {
  if (value === undefined) throw new Error(`${name} is missing: this code runs in a Skathi window`);
  return value;
}

export const nav = required(typeof navigator === "object" ? navigator : undefined, "navigator");
export const dom = required(
  typeof document === "object" ? document : undefined,
  "document",
) as unknown as SkDomDocument;
export const http = required(typeof fetch === "function" ? fetch : undefined, "fetch");

export const audio = typeof AudioContext === "function" ? AudioContext : null;
export const clipboard = typeof nav.clipboard === "object" ? nav.clipboard : null;
export const storage = typeof localStorage === "object" ? localStorage : null;
export const idb = typeof indexedDB === "object" ? indexedDB : null;
export const gpu = nav.gpu ?? null;

/** Calls `draw` once per painted frame until the returned function is called. */
export function everyFrame(draw: (time: number) => void): () => void {
  const request = required(typeof requestAnimationFrame === "function" ? requestAnimationFrame : undefined, "rAF");
  const cancel = required(typeof cancelAnimationFrame === "function" ? cancelAnimationFrame : undefined, "cAF");
  let handle = request(function frame(time) {
    draw(time);
    handle = request(frame);
  });
  return () => cancel(handle);
}

export const describe = (e: unknown) => (e instanceof Error ? e.message : String(e));
