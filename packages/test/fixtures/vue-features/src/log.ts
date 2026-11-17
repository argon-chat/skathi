export const events: string[] = [];
const releases = new Map<string, () => void>();
const scope = globalThis as { __vf?: string[]; __release?: (name: string) => boolean };
scope.__vf = events;
scope.__release = (name) => {
  const done = releases.get(name);
  done?.();
  return !!done;
};

export function log(event: string): void {
  events.push(event);
}

/** Resolves when the spec calls `__release(name)`. */
export function release(name: string): Promise<void> {
  return new Promise((resolve) => releases.set(name, resolve));
}
