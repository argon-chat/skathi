// `appHost`: the app's side of an embedded vue-native (see crates/vn-app/src/embed.rs).
// Arguments and results cross as JSON. Outside an embedder `available` is false, `call` rejects
// (and says why once) and `on` never fires.

/** A rejection from the embedder: its message, and which function it was. */
export interface AppHostError extends Error {
  readonly function: string;
}

export interface AppHost {
  readonly available: boolean;
  readonly functions: readonly string[];
  /** Calls the embedder's function `name`; its answer, parsed from JSON. */
  call<T = unknown>(name: string, ...args: unknown[]): Promise<T>;
  on(event: string, handler: (data: unknown) => void): () => void;
  off(event: string, handler: (data: unknown) => void): void;
}

interface HostFns {
  setDispatcher: typeof __vn_host_set_dispatcher;
  call: typeof __vn_host_call;
  functions: typeof __vn_host_functions;
}

const host: HostFns | null =
  typeof __vn_host_call === "function"
    ? { setDispatcher: __vn_host_set_dispatcher, call: __vn_host_call, functions: __vn_host_functions }
    : null;

// A reload keeps the JS realm: retire the old facade so stale subscriptions and promises cannot reach the next app.
const lifecycle = globalThis as { __VN_HOST__?: AppHost; __VN_HOST_LIFECYCLE__?: { reset: () => void } };
lifecycle.__VN_HOST_LIFECYCLE__?.reset();

const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void; name: string }>();
const listeners = new Map<string, Set<(data: unknown) => void>>();
let names: readonly string[] | null = null;

function hostError(name: string, message: string): AppHostError {
  const e = new Error(message) as Error & { function: string };
  e.name = "AppHostError";
  e.function = name;
  return e;
}

function functionNames(): readonly string[] {
  if (names) return names;
  let list: string[] = [];
  if (host) {
    try {
      const parsed: unknown = JSON.parse(host.functions());
      if (Array.isArray(parsed)) list = parsed.filter((n): n is string => typeof n === "string");
    } catch {
      /* a host that answers garbage has no functions we can name */
    }
  }
  names = Object.freeze(list);
  return names;
}

let notEmbeddedSaid = false;
function adviseNotEmbedded(name: string): void {
  if (notEmbeddedSaid) return;
  notEmbeddedSaid = true;
  console.warn(
    `[vue-native compat] appHost.call(${JSON.stringify(name)}): this app is not embedded in another program, ` +
      `so there is no host to call and the promise rejects. appHost exists when an application runs vue-native ` +
      `as a library; check appHost.available to run both ways.`,
  );
}

function dispatchEvent(text: string): void {
  let msg: { event?: unknown; data?: unknown };
  try {
    msg = JSON.parse(text) as { event?: unknown; data?: unknown };
  } catch {
    return;
  }
  const set = listeners.get(String(msg.event));
  if (!set) return;
  for (const handler of [...set]) {
    try {
      handler(msg.data);
    } catch (e) {
      console.error(`[appHost] a handler for ${JSON.stringify(msg.event)} threw:`, e);
    }
  }
}

if (host) {
  host.setDispatcher((token, ok, text) => {
    if (token === 0) {
      dispatchEvent(text);
      return;
    }
    const p = pending.get(token);
    if (!p) return;
    pending.delete(token);
    if (!ok) {
      p.reject(hostError(p.name, text));
      return;
    }
    try {
      p.resolve(JSON.parse(text));
    } catch {
      p.reject(hostError(p.name, "the host answered with text that is not JSON"));
    }
  });
}

export const appHost: AppHost = {
  get available() {
    return host !== null;
  },
  get functions() {
    return functionNames();
  },
  call<T = unknown>(name: string, ...args: unknown[]): Promise<T> {
    if (!host) {
      adviseNotEmbedded(name);
      return Promise.reject(hostError(name, "appHost: this app is not embedded in another program"));
    }
    let json: string;
    try {
      json = JSON.stringify(args);
    } catch (e) {
      return Promise.reject(new TypeError(`appHost.call(${JSON.stringify(name)}): the arguments are not JSON: ${String(e)}`));
    }
    const token = host.call(String(name), json);
    if (token === 0) {
      const known = functionNames();
      return Promise.reject(
        new TypeError(
          `appHost.call: the embedder has no function ${JSON.stringify(name)} ` +
            `(it has ${known.length ? known.map((n) => JSON.stringify(n)).join(", ") : "none"})`,
        ),
      );
    }
    return new Promise<T>((resolve, reject) => {
      pending.set(token, { resolve: resolve as (v: unknown) => void, reject, name });
    });
  },
  on(event: string, handler: (data: unknown) => void): () => void {
    let set = listeners.get(event);
    if (!set) listeners.set(event, (set = new Set()));
    set.add(handler);
    return () => appHost.off(event, handler);
  },
  off(event: string, handler: (data: unknown) => void): void {
    listeners.get(event)?.delete(handler);
  },
};

export function resetAppHost(): void {
  listeners.clear();
  for (const p of pending.values()) p.reject(hostError(p.name, "the page unloaded before the host answered"));
  pending.clear();
}

// Reachable by app adapters that build without the vue-native module alias.
lifecycle.__VN_HOST__ = appHost;
lifecycle.__VN_HOST_LIFECYCLE__ = { reset: resetAppHost };
