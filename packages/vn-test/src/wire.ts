// The devtools transport, as a long-lived connection.
//
// `devtools/ctl.ts` opens a socket, writes one request and hangs up, because a
// CLI's whole life IS one command. A test does thousands: every auto-waiting
// locator is a `query` every 50 ms, and paying a TCP handshake for each one
// would make the poll interval a lie. The host explicitly allows pipelining
// ("A client may pipeline several commands down one socket; each still costs
// one frame, in order" — crates/vn-app/src/devtools/mod.rs `serve`), so this
// keeps ONE socket open per app and matches replies to requests FIFO.
//
// FIFO is safe because the host is strictly ordered: `serve` reads lines in
// order, `submit` pushes onto one queue, and `pump` drains exactly one job per
// frame — a parked `type`/`key` blocks the queue rather than letting a later
// command overtake it. The one thing that can desync the pairing is a
// CLIENT-side timeout (we stop waiting; the reply arrives later anyway), so a
// timeout tears the socket down instead of leaving it half a reply out of step.

import { connect, type Socket } from "node:net";
import { readFileSync } from "node:fs";
import { join } from "node:path";

export type Handshake = { port: number; pid: number; token: string };

/** The host's envelope, verbatim: `{ok:true,data}` or `{ok:false,error}`. */
export type Reply = { ok: boolean; data?: any; error?: string };

/** Thrown when the host answered `{ok:false}`. Carries the command for context. */
export class VnCommandError extends Error {
  constructor(
    readonly command: string,
    readonly hostError: string,
  ) {
    super(`vn ${command}: ${hostError}`);
    this.name = "VnCommandError";
  }
}

export function readHandshake(distDir: string): Handshake {
  const path = join(distDir, "devtools.json");
  const h = JSON.parse(readFileSync(path, "utf8")) as Handshake;
  if (!h.port || !h.token) throw new Error(`${path} carries no port/token`);
  return h;
}

type Waiter = {
  resolve: (line: string) => void;
  reject: (e: Error) => void;
  timer: ReturnType<typeof setTimeout>;
  label: string;
};

export class Wire {
  #socket: Socket | null = null;
  #connecting: Promise<Socket> | null = null;
  #waiters: Waiter[] = [];
  #buffer = "";
  #closed = false;

  constructor(
    private readonly handshake: Handshake,
    private readonly defaultTimeoutMs = 15_000,
  ) {}

  #connect(): Promise<Socket> {
    if (this.#socket) return Promise.resolve(this.#socket);
    if (this.#connecting) return this.#connecting;
    this.#connecting = new Promise<Socket>((resolve, reject) => {
      const socket = connect({ host: "127.0.0.1", port: this.handshake.port });
      socket.setNoDelay(true);
      const fail = (e: Error) => {
        this.#connecting = null;
        this.#drop(e);
        reject(e);
      };
      socket.once("error", fail);
      socket.once("connect", () => {
        socket.removeListener("error", fail);
        socket.on("error", (e) => this.#drop(e));
        socket.on("close", () => this.#drop(new Error("the host closed the devtools connection")));
        // `data` is `string | Buffer` in general — a socket with an encoding
        // set hands back a string. This one never has one set, so the
        // annotation names which half arrives instead of widening `#onData`.
        socket.on("data", (chunk: Buffer) => this.#onData(chunk));
        this.#socket = socket;
        this.#connecting = null;
        resolve(socket);
      });
    });
    return this.#connecting;
  }

  #onData(chunk: Buffer) {
    this.#buffer += chunk.toString("utf8");
    for (;;) {
      const nl = this.#buffer.indexOf("\n");
      if (nl < 0) break;
      const line = this.#buffer.slice(0, nl);
      this.#buffer = this.#buffer.slice(nl + 1);
      const waiter = this.#waiters.shift();
      if (!waiter) continue; // a reply to a request we already gave up on
      clearTimeout(waiter.timer);
      waiter.resolve(line);
    }
  }

  /** Tears the socket down and fails everything still in flight. */
  #drop(cause: Error) {
    const socket = this.#socket;
    this.#socket = null;
    this.#buffer = "";
    socket?.removeAllListeners();
    socket?.destroy();
    const waiters = this.#waiters;
    this.#waiters = [];
    for (const w of waiters) {
      clearTimeout(w.timer);
      w.reject(new Error(`${w.label}: ${cause.message}`));
    }
  }

  close() {
    this.#closed = true;
    this.#drop(new Error("the client closed the connection"));
  }

  /**
   * One command, one reply. `timeoutMs` is the CLIENT's patience; the host's own
   * cap is 60 s and exists only so a wedged pump cannot leak threads.
   */
  async send(cmd: string, args: Record<string, unknown> = {}, timeoutMs?: number): Promise<any> {
    if (this.#closed) throw new Error(`vn ${cmd}: the app has been closed`);
    const ms = timeoutMs ?? this.defaultTimeoutMs;
    const socket = await this.#connect();
    const label = `vn ${cmd}`;
    const line = await new Promise<string>((resolve, reject) => {
      const timer = setTimeout(() => {
        // The reply may still be coming, so the pairing is now unknowable —
        // drop the socket rather than mis-attribute the next line.
        this.#drop(
          new Error(
            `no reply within ${ms}ms — the host's frame pump may be wedged, or the window is ` +
              `mid-drag (gpui runs a modal loop there)`,
          ),
        );
      }, ms);
      this.#waiters.push({ resolve, reject, timer, label });
      socket.write(JSON.stringify({ token: this.handshake.token, cmd, ...args }) + "\n");
    });

    let reply: Reply;
    try {
      reply = JSON.parse(line);
    } catch {
      throw new Error(`${label}: the host sent something that is not JSON: ${line.slice(0, 400)}`);
    }
    if (!reply.ok) throw new VnCommandError(cmd, reply.error ?? "unknown error");
    return reply.data;
  }
}

/** A one-shot `ping` on a fresh socket — used while waiting for a host to boot. */
export function pingOnce(port: number, token: string, timeoutMs: number): Promise<any> {
  return new Promise((resolve, reject) => {
    const socket = connect({ host: "127.0.0.1", port });
    let buffer = "";
    const done = (fn: () => void) => {
      clearTimeout(timer);
      socket.removeAllListeners();
      socket.destroy();
      fn();
    };
    const timer = setTimeout(() => done(() => reject(new Error("ping timed out"))), timeoutMs);
    socket.on("connect", () => socket.write(JSON.stringify({ token, cmd: "ping" }) + "\n"));
    socket.on("data", (chunk) => {
      buffer += chunk.toString("utf8");
      const nl = buffer.indexOf("\n");
      if (nl < 0) return;
      done(() => {
        try {
          const reply = JSON.parse(buffer.slice(0, nl)) as Reply;
          reply.ok ? resolve(reply.data) : reject(new Error(reply.error ?? "ping failed"));
        } catch (e) {
          reject(e as Error);
        }
      });
    });
    socket.on("error", (e) => done(() => reject(e)));
    socket.on("close", () => done(() => reject(new Error("closed before a reply"))));
  });
}
