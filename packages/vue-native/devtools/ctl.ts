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
  tree     [selector] [--depth N] [--node ID]   dump the retained tree
  styles   <selector|--node ID> [--index N]     the node's folded ComputedStates
  rules    <selector|--node ID> [--index N]     the RULES that matched it, in
                                     cascade order — selector, @media, source
                                     order, declarations, per-rule advisories
  highlight <selector|--node ID> [--index N] [--ttl MS] [--label TEXT]
  highlight --clear                  the inspector outline, drawn over the app
  pick                               where the pointer is over the app and which
                                     node is under it (the picker's poll)
  click    <selector> [--index N] [--button N]
  focus    <selector> [--index N]
  type     <selector> <text> [--index N]
  key      <Key|Mod+Key> [selector] [--index N]
  mouse    [selector] [--x N --y N] [--kind move|down|up|click] [--button N]
                                     a REAL pointer event through gpui's hit
                                     test (hover, z-order, pointer-events all
                                     apply); reports which node has focus after
  scroll   <selector> <dy> [--dx N] [--index N]
  wheel    [selector] [--x N --y N] --dy N [--dx N]
                                     a REAL wheel event through gpui's hit
                                     test — what an app's own @wheel handler
                                     sees. DOM signs: --dy 200 scrolls DOWN.
                                     (\`scroll\` drives a container's handle
                                     directly and never involves the wheel)
  eval     <js source>
  logs     [--since N]
  screenshot [path] [--backend auto|render_to_image|print_window]
  windows                            every OS window this process paints (and
                                     every inspector it has open on itself)
  perf                               the rolling frame window: fps, p50/p95,
                                     node count, boot->first-frame
  net      [--since N]               the fetch / WebSocket log
  storage  [local|session] [list|set|remove|clear] [--key K] [--value V]
  raise                              bring the window to the front

options
  --dist <slot>    the dist directory the target host was launched with
                   (default: ${DEFAULT_DIST})
  --window <id>    which of the target's OS windows the command is about
                   (\`windows\` lists them; default: the primary one)
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
    // Booleans; everything else takes a value.
    if (name === "json" || name === "help" || name === "clear") flags[name] = true;
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
    case "focus":
      if (!rest[0]) die(`${cmd} needs a selector`);
      return withIndex({ cmd, selector: rest[0] });
    // These three take a NODE ID as well as a selector: an inspector holding a
    // node it walked to has no selector that is guaranteed to name it again.
    case "styles":
    case "rules": {
      const node = num(f.node);
      if (!rest[0] && node === undefined) die(`${cmd} needs a selector or --node <id>`);
      return withIndex({ cmd, ...(rest[0] ? { selector: rest[0] } : {}), ...(node !== undefined ? { node } : {}) });
    }
    case "highlight": {
      const node = num(f.node);
      if (f.clear || (!rest[0] && node === undefined)) return { cmd, clear: true };
      return withIndex({
        cmd,
        ...(rest[0] ? { selector: rest[0] } : {}),
        ...(node !== undefined ? { node } : {}),
        ...(num(f.ttl) !== undefined ? { ttlMs: num(f.ttl) } : {}),
        ...(typeof f.label === "string" && f.label ? { label: f.label } : {}),
      });
    }
    case "pick":
      return { cmd };
    case "windows":
    case "raise":
    case "perf":
      return { cmd };
    case "net":
      return { cmd, ...(num(f.since) !== undefined ? { since: num(f.since) } : {}) };
    case "storage":
      return {
        cmd,
        area: rest[0] || "local",
        op: rest[1] || "list",
        ...(typeof f.key === "string" ? { key: f.key } : {}),
        ...(typeof f.value === "string" ? { value: f.value } : {}),
      };
    case "tree":
      return {
        cmd,
        ...(rest[0] ? { selector: rest[0] } : {}),
        ...(num(f.node) !== undefined ? { node: num(f.node) } : {}),
        ...(num(f.depth) !== undefined ? { depth: num(f.depth) } : {}),
      };
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
    case "mouse": {
      // A REAL pointer event through gpui's hit test: `ctl mouse ".field"` or
      // `ctl mouse --x 640 --y 567 --kind click`. Unlike `click`, which
      // synthesizes into the app's dispatcher from a node id, this one is
      // decided by the hit test — so it is what proves an element is actually
      // reachable, and it is how `:hover` is exercised.
      const kind = f.kind ? String(f.kind) : "move";
      const base: Record<string, unknown> = { cmd, kind };
      if (num(f.button) !== undefined) base.button = num(f.button);
      if (rest[0]) return withIndex({ ...base, selector: rest[0] });
      const x = num(f.x), y = num(f.y);
      if (x === undefined || y === undefined) die("mouse needs a selector, or both --x and --y");
      return { ...base, x, y };
    }
    case "scroll": {
      if (!rest[0]) die("scroll needs a selector");
      const dy = num(rest[1]) ?? num(f.dy) ?? 0;
      const dx = num(f.dx) ?? 0;
      return withIndex({ cmd, selector: rest[0], dx, dy });
    }
    case "wheel": {
      const dy = num(f.dy) ?? 0;
      const dx = num(f.dx) ?? 0;
      if (!dy && !dx) die("wheel needs --dy (or --dx); DOM signs, so --dy 200 scrolls down");
      const base: Record<string, unknown> = { cmd, dx, dy };
      if (rest[0]) return withIndex({ ...base, selector: rest[0] });
      const x = num(f.x), y = num(f.y);
      if (x === undefined || y === undefined) die("wheel needs a selector, or both --x and --y");
      return { ...base, x, y };
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
const windowArg = Number(flags.window);
// `--window` rides on EVERY request rather than being spelled out per
// command: the host reads it uniformly (docs/DEVTOOLS.md, the `window`
// argument), and a client that never passes it addresses the primary window
// exactly as it always did.
const request = {
  token: handshake.token,
  ...(Number.isFinite(windowArg) ? { window: windowArg } : {}),
  ...buildRequest(positional, flags),
};

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
