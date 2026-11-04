// `appHost` — the app's side of an EMBEDDED vue-native (docs/EMBEDDING.md).
//
// Another program — a terminal UI, a launcher, a service — runs vue-native as
// a library (crates/vn-app/src/embed.rs) and gives the app functions to call
// and events to listen to:
//
//   import { appHost } from "vue-native";
//   const rows = await appHost.call<Row[]>("db.query", "select 1");
//   const off = appHost.on("status", (s) => (online.value = s.online));
//
// Arguments and results cross as JSON. In the vue-native exe nobody embeds the
// app: `available` is false, `call` rejects and says once why, and `on` never
// fires — so an app that runs both ways checks `available`.

/** A rejection from the embedder: its message, and which function it was. */
export interface AppHostError extends Error {
  readonly function: string;
}

export interface AppHost {
  /** True when another program embeds this app. */
  readonly available: boolean;
  /** The embedder's functions, sorted. Empty when not embedded. */
  readonly functions: readonly string[];
  /** Calls the embedder's function `name`; its answer, parsed from JSON. */
  call<T = unknown>(name: string, ...args: unknown[]): Promise<T>;
  /** Listens to the embedder's event `event`. Returns the unsubscribe. */
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
      `as a library (docs/EMBEDDING.md); check appHost.available to run both ways.`,
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
