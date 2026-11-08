// The inspector's two transports: commands over HTTP, events over WebSocket.
//
//   command  ──POST /────────────▶  one per connection, one per target frame
//   events   ◀─GET / + Upgrade───   one socket, pushed as things happen
//
// docs/DEVTOOLS.md's wire is newline-delimited JSON over a localhost TCP port.
// JS in this runtime cannot open a TCP socket — but it CAN `fetch()`, and it
// can open a `WebSocket`, and the host's listener answers both on that same
// port (`crates/vn-app/src/devtools/{mod,ws}.rs`).
//
// v1 had only the first half and polled: logs every 700 ms, the tree every
// 2.5 s. That is the difference between a console and a console you have to
// wait for, and it is why the push channel exists. Commands stayed on HTTP: they
// are already serialized, already one-per-frame, already proven, and a request
// that expects a reply has no business being multiplexed by hand.
//
// Discovery is the one thing neither transport can do — a handshake file has to
// be read off disk — so it comes from `__vn_devtools_targets()`, a host fn that
// exists in every build except `--features no-devtools` (docs/DEVTOOLS.md
// "Shape"; it was dev-only until m22). See docs/DEVTOOLS-UI.md §3.

import type { PushMessage, PushStream, Target, TargetList, WindowId } from "./protocol";

/** How long a single command may take before the UI stops waiting for it. */
const TIMEOUT_MS = 8000;

/**
 * Every attachable instance, right now.
 *
 * Returns `{ preferred, self, targets, error }`. `preferred` is the slot
 * `--target` named (via `VN_DEVTOOLS_TARGET`); `targets[].self` marks THIS
 * process, which is attachable too — the inspector inspecting itself is a
 * legitimate (and useful) thing to do.
 */
async function discoverTargets(): Promise<TargetList> {
  const empty: TargetList = { preferred: null, self: null, targets: [] };
  // `__vn_devtools_targets` is registered by every host that HAS devtools and
  // by no other (`devtools::install`), so the probe is the honest question
  // rather than a formality — which is why protocol.d.ts declares it a plain
  // function and this is the guard.
  if (typeof globalThis.__vn_devtools_targets !== "function") {
    return {
      ...empty,
      error:
        "__vn_devtools_targets is not registered — this looks like a host built without the " +
        "devtools (`--features prod` and no `devtools` feature)",
    };
  }
  try {
    const deadline = Date.now() + 30000;
    while (true) {
      const found = JSON.parse(globalThis.__vn_devtools_targets()) as Partial<TargetList> & {
        pending?: boolean;
      };
      if (!found.pending) return { ...empty, ...found };
      if (Date.now() >= deadline) return { ...empty, error: "target discovery timed out" };
      await new Promise<void>((resolve) => setTimeout(resolve, 50));
    }
  } catch (e) {
    return { ...empty, error: `the target list did not parse: ${e}` };
  }
}

let discovery: Promise<TargetList> | null = null;

/** Share one background scan between startup, the refresh button and the clock. */
export function listTargets(): Promise<TargetList> {
  if (!discovery) {
    discovery = discoverTargets().finally(() => { discovery = null; });
  }
  return discovery;
}

export interface SendOptions {
  priority?: "interactive" | "background";
  /** Checked before a queued request runs, e.g. after changing target. */
  isCurrent?: () => boolean;
}
interface Queued {
  background: boolean;
  run: () => Promise<void>;
}
interface Lane { running: boolean; pending: Queued[] }
const lanes = new Map<string, Lane>();

/** Order commands per target; a slow old target never blocks a new attachment.
 * Interactive requests overtake queued maintenance, never an in-flight command. */
function enqueue<T>(target: Target, run: () => Promise<T>, options: SendOptions): Promise<T> {
  const key = `${target.pid}:${target.port}:${target.token}`;
  let lane = lanes.get(key);
  if (!lane) { lane = { running: false, pending: [] }; lanes.set(key, lane); }
  const queue = lane;
  return new Promise<T>((resolve, reject) => {
    queue.pending.push({ background: options.priority === "background", run: async () => {
      try {
        if (options.isCurrent && !options.isCurrent()) throw new Error("attachment changed");
        resolve(await run());
      } catch (error) { reject(error); }
    } });
    if (!queue.running) void drain();
    async function drain(): Promise<void> {
      queue.running = true;
      while (queue.pending.length) {
        const interactive = queue.pending.findIndex((job) => !job.background);
        const job = queue.pending.splice(interactive < 0 ? 0 : interactive, 1)[0]!;
        await job.run();
      }
      queue.running = false;
      if (lanes.get(key) === queue) lanes.delete(key);
    }
  });
}

/**
 * One devtools command against `target`, unwrapped to its `data`.
 *
 * A `{ok:false}` reply becomes a thrown `Error` carrying the host's own words —
 * those messages are written to be read by a human (they list the candidates
 * when a selector is ambiguous, they name the unsupported selector syntax), so
 * the UI shows them verbatim rather than inventing its own.
 *
 * `T` is the caller's claim about the `data` shape, checked against nothing:
 * this is a wire, and the reply is whatever the host on the other end sent.
 * `./protocol.ts` is where those shapes are written down from the Rust that
 * builds them, so the claim has one place to be right or wrong.
 */
export function send<T>(
  target: Target | null,
  cmd: string,
  args: Record<string, unknown> = {},
  timeoutMs = TIMEOUT_MS,
  options: SendOptions = {},
): Promise<T> {
  if (!target) return Promise.reject(new Error("not attached to a target"));
  return enqueue(target, () => request<T>(target, cmd, args, timeoutMs), options);
}

async function request<T>(
  target: Target,
  cmd: string,
  args: Record<string, unknown>,
  timeoutMs: number,
): Promise<T> {
  const body = JSON.stringify({ token: target.token, cmd, ...args });
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  // `engine.d.ts` types `fetch` as possibly absent, and it means it: an
  // embedder can build redjs without it. For THIS app that is not a degraded
  // mode — HTTP on the devtools port is half the transport — so it is one guard
  // with the reason attached rather than a silent `undefined` call.
  const http = fetch;
  if (!http) {
    throw new Error(
      `${cmd}: this host has no \`fetch\`, and the inspector's command transport is HTTP on the ` +
        `target's devtools port (docs/DEVTOOLS-UI.md §3)`,
    );
  }
  let res;
  try {
    res = await http(`http://127.0.0.1:${target.port}/`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body,
      signal: controller.signal,
    });
  } catch (e) {
    throw new Error(
      `${cmd}: cannot reach ${target.slot} on port ${target.port} — the host exited, or its ` +
        `frame pump is wedged (${message(e)})`,
    );
  } finally {
    clearTimeout(timer);
  }
  // Same story as `fetch` above: `VnFetchResponse.text` is declared optional
  // because engine.d.ts types only what the voice shim reads. redjs's response
  // has it (`webapi::fetch`), and this app cannot work without it.
  if (!res.text) throw new Error(`${cmd}: this host's fetch response has no \`text()\``);
  const text = await res.text();
  let reply: { ok?: boolean; error?: string; data?: T };
  try {
    reply = JSON.parse(text);
  } catch {
    throw new Error(
      `${cmd}: the host answered with something that is not JSON: ${text.slice(0, 200)}`,
    );
  }
  if (!reply.ok) throw new Error(reply.error || `${cmd}: the host refused the command`);
  return reply.data as T;
}

// ---------------------------------------------------------------------------
// the push channel
// ---------------------------------------------------------------------------

export interface PushHandlers {
  onMessage(msg: PushMessage): void;
  onOpen?(): void;
  onClose?(why: string): void;
}

/**
 * A live subscription to a target: log lines, tree revisions, frame stats and
 * network entries, pushed as they happen.
 *
 * It is deliberately a THIN object. Reconnection is the store's job (it knows
 * whether the target is still supposed to be attached, and it is the thing that
 * has to re-sync the log cursor afterwards); this only knows how to be one
 * socket, and how to say when it stopped being one.
 *
 * The token rides in the query string because a `WebSocket` constructor cannot
 * set a header — not in a browser and not here. Same token, same guard: it is
 * checked before the 101 is written (`ws.rs`).
 */
export class Push {
  private socket: VnWebSocket | null = null;
  private closed = false;

  constructor(target: Target, since: number, streams: PushStream[], private handlers: PushHandlers) {
    const url = `ws://127.0.0.1:${target.port}/?token=${encodeURIComponent(
      target.token,
    )}&since=${since}`;
    if (typeof WebSocket !== "function") {
      // Not fatal: the store falls back to polling, and says so in the console
      // rather than silently feeling slow — which is the exact failure v2
      // exists to remove.
      handlers.onClose?.("this host has no WebSocket");
      return;
    }
    let socket: VnWebSocket;
    try {
      socket = new WebSocket(url);
    } catch (e) {
      handlers.onClose?.(message(e));
      return;
    }
    this.socket = socket;
    socket.onopen = () => {
      this.send({ sub: streams });
      handlers.onOpen?.();
    };
    socket.onmessage = (event: VnMessageEvent) => {
      if (this.closed) return;
      try {
        handlers.onMessage(JSON.parse(String(event.data)) as PushMessage);
      } catch {
        // A malformed frame is the host's bug, not a reason to tear the socket
        // down; the next one is probably fine.
      }
    };
    socket.onerror = () => {
      if (!this.closed) handlers.onClose?.("the push socket errored");
    };
    socket.onclose = (event: VnCloseEvent) => {
      if (this.closed) return;
      this.closed = true;
      handlers.onClose?.(`the push socket closed (${event.code || 0})`);
    };
  }

  /** Re-point the log cursor, or change the subscription. */
  send(msg: Record<string, unknown>): void {
    if (!this.socket || this.socket.readyState !== 1) return;
    try {
      this.socket.send(JSON.stringify(msg));
    } catch {
      // A send on a socket the peer already dropped; `onclose` is on its way.
    }
  }

  subscribe(streams: PushStream[]): void {
    this.send({ sub: streams });
  }

  close(): void {
    this.closed = true;
    try {
      this.socket?.close();
    } catch {
      // Already gone.
    }
    this.socket = null;
  }

  get live(): boolean {
    return !!this.socket && this.socket.readyState === 1;
  }
}

/**
 * What the host published about how this inspector was started.
 *
 * Every launch is a `--devtools` launch now (F12 spawns one, and
 * `bun devtools/ui.ts` execs one), so this is always present in practice. The
 * optional shape stays: it is a JSON global from another process, and a reader
 * that assumed its fields would be the one thing standing between a protocol
 * change and a blank window.
 */
export function devtoolsArgs(): { target: string | null; window: WindowId | null; embedded: boolean } {
  const raw = globalThis.__VN_DEVTOOLS_ARGS__;
  if (!raw || typeof raw !== "object") return { target: null, window: null, embedded: false };
  const args = raw as { target?: unknown; window?: unknown; embedded?: unknown };
  return {
    target: typeof args.target === "string" ? args.target : null,
    window: typeof args.window === "number" ? args.window : null,
    embedded: args.embedded === true,
  };
}

/**
 * A thrown value's message.
 *
 * `catch (e)` is `unknown` under `strict`, and it genuinely is: `fetch` rejects
 * with an `Error`, but an app being inspected can `throw` anything at all and
 * the console has to print SOMETHING either way.
 */
export function message(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object" && typeof (e as { message?: unknown }).message === "string") {
    return (e as { message: string }).message;
  }
  return String(e);
}
