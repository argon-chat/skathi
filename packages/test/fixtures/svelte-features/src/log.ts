export const events: string[] = [];
const releases = new Map<string, () => void>();
const scope = globalThis as { __sf?: string[]; __release?: (name: string) => boolean };
scope.__sf = events;
scope.__release = (name) => {
  const done = releases.get(name);
  done?.();
  return !!done;
};

export function log(event: string): void {
  events.push(event);
}

/** Resolves with `value` when the spec calls `__release(name)`. */
export function release<T>(name: string, value: T): Promise<T> {
  return new Promise((resolve) => releases.set(name, () => resolve(value)));
}
