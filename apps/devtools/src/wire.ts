import type { PushMessage, PushStream, Target, TargetList, WindowId } from "./protocol";

/** How long a single command may take before the UI stops waiting for it. */
const TIMEOUT_MS = 8000;

async function discoverTargets(): Promise<TargetList> {
  const empty: TargetList = { preferred: null, self: null, targets: [] };
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

/** Order commands per target; a slow old target never blocks a new attachment. */
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
  // For THIS app that is not a degraded mode — HTTP on the devtools port is half the transport —
  // so it is one guard with the reason attached rather than a silent `undefined` call.
  const http = fetch;
  if (!http) {
    throw new Error(
      `${cmd}: this host has no \`fetch\`, and the inspector's command transport is HTTP on the ` +
        `target's devtools port`,
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

export interface PushHandlers {
  onMessage(msg: PushMessage): void;
  onOpen?(): void;
  onClose?(why: string): void;
}

/** It is deliberately a THIN object. */
export class Push {
  private socket: VnWebSocket | null = null;
  private closed = false;

  constructor(target: Target, since: number, streams: PushStream[], private handlers: PushHandlers) {
    const url = `ws://127.0.0.1:${target.port}/?token=${encodeURIComponent(
      target.token,
    )}&since=${since}`;
    if (typeof WebSocket !== "function") {
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

  send(msg: Record<string, unknown>): void {
    if (!this.socket || this.socket.readyState !== 1) return;
    try {
      this.socket.send(JSON.stringify(msg));
    } catch {
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
    }
    this.socket = null;
  }

  get live(): boolean {
    return !!this.socket && this.socket.readyState === 1;
  }
}

/**
 * The optional shape stays: it is a JSON global from another process, and a reader that assumed
 * its fields would be the one thing standing between a protocol change and a blank window.
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

export function message(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (e && typeof e === "object" && typeof (e as { message?: unknown }).message === "string") {
    return (e as { message: string }).message;
  }
  return String(e);
}
