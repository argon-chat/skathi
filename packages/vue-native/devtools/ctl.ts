#!/usr/bin/env bun
/**
 * vn-devtools CLI: the zero-setup way to drive a running dev host.
 *
 *   bun devtools/ctl.ts [--dist <slot> | --app <name>] <cmd> [args...]
 *
 * Reads `<dist>/devtools.json`, sends one newline-delimited JSON request over TCP to 127.0.0.1 and
 * prints the reply. Exit code 0 on `{ok:true}`, 1 otherwise. No npm dependencies on purpose: an
 * agent driving the app through Bash should not need `bun install`.
 */

import { connect } from "node:net";
import { readFileSync } from "node:fs";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { buildRequest, usageBlock, UsageError, splitArgs } from "./commands";
import { appSlot } from "./apps";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEFAULT_DIST = resolve(HERE, "..", "dist");
/** A wedged pump must never hang the caller. Overridable with --timeout <ms>. */
const DEFAULT_TIMEOUT_MS = 10_000;

const USAGE = `vn-devtools — drive a running vue-native dev host

  bun devtools/ctl.ts [--dist <slot>] [--timeout <ms>] <command> [args]

commands
${usageBlock()}

options
  --dist <slot>    the dist directory the target host was launched with
                   (default: ${DEFAULT_DIST})
  --app <name>     a PACKAGED app instead (vn build): its newest live instance
                   under %TEMP%\\vue-native\\apps\\<name>
  --window <id>    which of the target's OS windows the command is about
                   (\`windows\` lists them; default: the primary one)
  --timeout <ms>   client-side reply timeout (default ${DEFAULT_TIMEOUT_MS})
  --json           print the raw wire response instead of just \`data\`
`;

function die(message: string, code = 1): never {
  console.error(message);
  process.exit(code);
}

const { positional, flags } = splitArgs(process.argv.slice(2));
if (flags.help || positional.length === 0) {
  console.log(USAGE);
  process.exit(positional.length === 0 && !flags.help ? 1 : 0);
}

const distDir = (() => {
  if (typeof flags.app === "string" && flags.app) {
    const found = appSlot(flags.app);
    if ("error" in found) die(found.error);
    return found.dir;
  }
  return typeof flags.dist === "string" && flags.dist ? resolve(flags.dist) : DEFAULT_DIST;
})();
const timeoutMs = Number(flags.timeout ?? DEFAULT_TIMEOUT_MS) || DEFAULT_TIMEOUT_MS;

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

const handshake = readHandshake(distDir);
const windowArg = Number(flags.window);
// `--window` rides on every request; a client that never passes it addresses the primary window.
let request: Record<string, unknown>;
try {
  request = {
    token: handshake.token,
    ...(Number.isFinite(windowArg) ? { window: windowArg } : {}),
    ...buildRequest(positional, flags),
  };
} catch (e) {
  // `commands.ts` throws rather than exiting: the MCP server imports it too.
  if (!(e instanceof UsageError)) throw e;
  die(`${e.message}\n\n${USAGE}`);
}

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

// A thrown `eval` is a successful ROUND TRIP carrying `{ok:true, data:{exception}}`; the exit
// code is the shell's error channel, so surface it there too.
const threw = reply.ok === true && !!(reply.data as { exception?: string } | undefined)?.exception;

if (flags.json) {
  console.log(JSON.stringify(reply, null, 2));
} else if (reply.ok) {
  console.log(JSON.stringify(reply.data, null, 2));
} else {
  console.error(reply.error ?? "unknown error");
}
process.exit(reply.ok && !threw ? 0 : 1);
