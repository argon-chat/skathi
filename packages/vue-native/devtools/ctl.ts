#!/usr/bin/env bun
/**
 * vn-devtools CLI — the zero-setup way to drive a running dev host.
 *
 *   bun devtools/ctl.ts [--dist <slot>] <cmd> [args...]
 *
 * Reads `<dist>/devtools.json` (written by the host at boot), opens a TCP
 * connection to 127.0.0.1:<port>, sends ONE newline-delimited JSON request and
 * prints the reply. Exit code 0 on `{ok:true}`, 1 on anything else — so
 * `ctl click ... && ctl query ...` chains the way a shell expects.
 *
 * Zero dependencies on purpose: an agent driving the app through Bash should
 * not need `bun install` to have run.
 *
 * See docs/DEVTOOLS.md for the command table.
 */

import { connect } from "node:net";
import { readFileSync } from "node:fs";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEFAULT_DIST = resolve(HERE, "..", "dist");
/** A wedged pump must never hang the caller. Overridable with --timeout <ms>. */
const DEFAULT_TIMEOUT_MS = 10_000;

const USAGE = `vn-devtools — drive a running vue-native dev host

  bun devtools/ctl.ts [--dist <slot>] [--timeout <ms>] <command> [args]

commands
  ping                                is a host alive on this dist slot?
  query    <selector>                 match through the real vn-css cascade matcher
  text     <selector> [--index N]     textContent of the subtree (or an <input>'s buffer)
  tree     [selector] [--depth N]     dump the retained tree
  styles   <selector> [--index N]     the node's folded ComputedStates
  click    <selector> [--index N] [--button N]
  focus    <selector> [--index N]
  type     <selector> <text> [--index N]
  key      <Key|Mod+Key> [selector] [--index N]
  scroll   <selector> <dy> [--dx N] [--index N]
  eval     <js source>
  logs     [--since N]
  screenshot [path] [--backend auto|render_to_image|print_window]

options
  --dist <slot>    the dist directory the target host was launched with
                   (default: ${DEFAULT_DIST})
  --timeout <ms>   client-side reply timeout (default ${DEFAULT_TIMEOUT_MS})
  --json           print the raw wire response instead of just \`data\`
`;

function die(message: string, code = 1): never {
  console.error(message);
  process.exit(code);
}

// ---------------------------------------------------------------------------
// argv
// ---------------------------------------------------------------------------

type Flags = Record<string, string | boolean>;

function splitArgs(argv: string[]): { positional: string[]; flags: Flags } {
  const positional: string[] = [];
  const flags: Flags = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) {
      positional.push(a);
      continue;
    }
    const name = a.slice(2);
    // `--json` is the only boolean; everything else takes a value.
    if (name === "json" || name === "help") flags[name] = true;
    else flags[name] = argv[++i] ?? "";
  }
  return { positional, flags };
}

const { positional, flags } = splitArgs(process.argv.slice(2));
if (flags.help || positional.length === 0) {
  console.log(USAGE);
  process.exit(positional.length === 0 && !flags.help ? 1 : 0);
}

const distDir = typeof flags.dist === "string" && flags.dist ? resolve(flags.dist) : DEFAULT_DIST;
const timeoutMs = Number(flags.timeout ?? DEFAULT_TIMEOUT_MS) || DEFAULT_TIMEOUT_MS;

// ---------------------------------------------------------------------------
// discovery
// ---------------------------------------------------------------------------

type Handshake = { port: number; pid: number; token: string };

function readHandshake(dir: string): Handshake {
  const path = join(dir, "devtools.json");
  let raw: string;
  try {
    raw = readFileSync(path, "utf8");
  } catch {
    die(
      `no devtools.json at ${path} — is a dev host running with this dist?\n` +
        `  start one:  VN_DIST=${dir} target/x86_64-pc-windows-msvc/debug/vn-app.exe\n` +
        `  or point at another slot:  --dist <dir>`,
    );
  }
  try {
    const h = JSON.parse(raw) as Handshake;
    if (!h.port || !h.token) throw new Error("missing port/token");
    return h;
  } catch (e) {
    die(`${path} is not a valid devtools handshake: ${(e as Error).message}`);
  }
}

// ---------------------------------------------------------------------------
// request building
// ---------------------------------------------------------------------------

const num = (v: unknown): number | undefined => {
  if (v === undefined || v === "") return undefined;
  const n = Number(v);
  return Number.isFinite(n) ? n : undefined;
};

function buildRequest(argv: string[], f: Flags): Record<string, unknown> {
  const [cmd, ...rest] = argv;
  const index = num(f.index);
  const withIndex = (o: Record<string, unknown>) => (index === undefined ? o : { ...o, index });
  switch (cmd) {
    case "ping":
      return { cmd };
    case "query":
      if (!rest[0]) die("query needs a selector");
      return { cmd, selector: rest[0] };
    case "text":
    case "styles":
    case "focus":
      if (!rest[0]) die(`${cmd} needs a selector`);
      return withIndex({ cmd, selector: rest[0] });
    case "tree":
      return { cmd, ...(rest[0] ? { selector: rest[0] } : {}), ...(num(f.depth) !== undefined ? { depth: num(f.depth) } : {}) };
    case "click":
      if (!rest[0]) die("click needs a selector");
      return withIndex({ cmd, selector: rest[0], ...(num(f.button) !== undefined ? { button: num(f.button) } : {}) });
    case "type":
      if (rest.length < 2) die('type needs a selector and text: ctl type ".field" "hello"');
      return withIndex({ cmd, selector: rest[0], text: rest.slice(1).join(" ") });
    case "key":
      if (!rest[0]) die('key needs a key name: ctl key Enter [selector]');
      // With no selector the keystroke goes to whatever already has focus.
      return withIndex({ cmd, key: rest[0], ...(rest[1] ? { selector: rest[1] } : {}) });
    case "scroll": {
      if (!rest[0]) die("scroll needs a selector");
      const dy = num(rest[1]) ?? num(f.dy) ?? 0;
      const dx = num(f.dx) ?? 0;
      return withIndex({ cmd, selector: rest[0], dx, dy });
    }
    case "eval":
      if (!rest.length) die("eval needs a JS source string");
      return { cmd, js: rest.join(" ") };
    case "logs":
      return { cmd, ...(num(f.since) !== undefined ? { since: num(f.since) } : {}) };
    case "screenshot":
      return {
        cmd,
        ...(rest[0] ? { path: resolve(rest[0]) } : {}),
        ...(f.backend ? { backend: String(f.backend) } : {}),
      };
    default:
      die(`unknown command \`${cmd}\`\n\n${USAGE}`);
  }
}

// ---------------------------------------------------------------------------
// transport
// ---------------------------------------------------------------------------

function send(port: number, payload: string, ms: number): Promise<string> {
  return new Promise((resolveReply, reject) => {
    const socket = connect({ host: "127.0.0.1", port });
    let buffer = "";
    const timer = setTimeout(() => {
      socket.destroy();
      reject(
        new Error(
          `no reply within ${ms}ms — the host's frame pump may be wedged, or the window is ` +
            `mid-drag (gpui runs a modal loop there). Retry, or raise --timeout.`,
        ),
      );
    }, ms);
    const finish = (fn: () => void) => {
      clearTimeout(timer);
      socket.destroy();
      fn();
    };
    socket.on("connect", () => socket.write(payload + "\n"));
    socket.on("data", (chunk) => {
      buffer += chunk.toString("utf8");
      const nl = buffer.indexOf("\n");
      if (nl >= 0) finish(() => resolveReply(buffer.slice(0, nl)));
    });
    socket.on("error", (e) =>
      finish(() =>
        reject(
          new Error(
            `cannot reach 127.0.0.1:${port}: ${e.message} — devtools.json is stale ` +
              `(the host that wrote it exited). Relaunch the host, or --dist another slot.`,
          ),
        ),
      ),
    );
    socket.on("close", () => finish(() => reject(new Error("connection closed before a reply"))));
  });
}

// ---------------------------------------------------------------------------

const handshake = readHandshake(distDir);
const request = { token: handshake.token, ...buildRequest(positional, flags) };

let line: string;
try {
  line = await send(handshake.port, JSON.stringify(request), timeoutMs);
} catch (e) {
  die(String((e as Error).message));
}

let reply: { ok?: boolean; data?: unknown; error?: string };
try {
  reply = JSON.parse(line);
} catch {
  die(`the host sent something that is not JSON: ${line.slice(0, 400)}`);
}

// A thrown `eval` is a successful ROUND TRIP carrying a failure — the host
// reports it as `{ok:true, data:{exception}}`, like a devtools console does.
// The exit code is the shell's error channel though, so surface it there too.
const threw = reply.ok === true && !!(reply.data as { exception?: string } | undefined)?.exception;

if (flags.json) {
  console.log(JSON.stringify(reply, null, 2));
} else if (reply.ok) {
  console.log(JSON.stringify(reply.data, null, 2));
} else {
  console.error(reply.error ?? "unknown error");
}
process.exit(reply.ok && !threw ? 0 : 1);
