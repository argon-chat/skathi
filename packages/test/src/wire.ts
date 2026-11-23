// `devtools/ctl.ts` opens a socket, writes one request and hangs up, because a CLI's whole life IS
// one command.

import { connect, type Socket } from "node:net";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { StringDecoder } from "node:string_decoder";

export type Handshake = { port: number; pid: number; token: string };

/** The host's envelope, verbatim: `{ok:true,data}` or `{ok:false,error}`. */
export type Reply = { ok: boolean; data?: any; error?: string };

/** Thrown when the host answered `{ok:false}`. */
export class SkCommandError extends Error {
  constructor(
    readonly command: string,
    readonly hostError: string,
  ) {
    super(`sk ${command}: ${hostError}`);
    this.name = "SkCommandError";
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
  // A reply's UTF-8 is split wherever TCP splits it.
  #decoder = new StringDecoder("utf8");
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
        // This one never has one set, so the annotation names which half arrives instead of
        // widening `#onData`.
        socket.on("data", (chunk: Buffer) => this.#onData(chunk));
        this.#socket = socket;
        this.#connecting = null;
        resolve(socket);
      });
    });
    return this.#connecting;
  }

  #onData(chunk: Buffer) {
    this.#buffer += this.#decoder.write(chunk);
    for (;;) {
      const nl = this.#buffer.indexOf("\n");
      if (nl < 0) break;
      const line = this.#buffer.slice(0, nl);
      this.#buffer = this.#buffer.slice(nl + 1);
      const waiter = this.#waiters.shift();
      if (!waiter) continue;
      clearTimeout(waiter.timer);
      waiter.resolve(line);
    }
  }

  /** Tears the socket down and fails everything still in flight. */
  #drop(cause: Error) {
    const socket = this.#socket;
    this.#socket = null;
    this.#buffer = "";
    this.#decoder = new StringDecoder("utf8");
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

  /** One command, one reply. */
  async send(cmd: string, args: Record<string, unknown> = {}, timeoutMs?: number): Promise<any> {
    if (this.#closed) throw new Error(`sk ${cmd}: the app has been closed`);
    const ms = timeoutMs ?? this.defaultTimeoutMs;
    const socket = await this.#connect();
    const label = `sk ${cmd}`;
    const line = await new Promise<string>((resolve, reject) => {
      const timer = setTimeout(() => {
        // The reply may still be coming, so the pairing is now unknowable — drop the socket rather
        // than mis-attribute the next line.
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
    if (!reply.ok) throw new SkCommandError(cmd, reply.error ?? "unknown error");
    return reply.data;
  }
}

/** A one-shot `ping` on a fresh socket — used while waiting for a host to boot. */
export function pingOnce(port: number, token: string, timeoutMs: number): Promise<any> {
  return new Promise((resolve, reject) => {
    const socket = connect({ host: "127.0.0.1", port });
    let buffer = "";
    const decoder = new StringDecoder("utf8");
    const done = (fn: () => void) => {
      clearTimeout(timer);
      socket.removeAllListeners();
      socket.destroy();
      fn();
    };
    const timer = setTimeout(() => done(() => reject(new Error("ping timed out"))), timeoutMs);
    socket.on("connect", () => socket.write(JSON.stringify({ token, cmd: "ping" }) + "\n"));
    socket.on("data", (chunk) => {
      buffer += decoder.write(chunk);
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
