#!/usr/bin/env bun
/**
 * vn-devtools MCP server — the same wire as `ctl.ts`, exposed as MCP tools so
 * an agent can drive a running vue-native dev host without shelling out.
 *
 *   bun packages/vue-native/devtools/mcp.ts [--dist <slot>]
 *
 * Registered per-repo in `.mcp.json`. One server instance per dist slot: the
 * `--dist` argument decides which `<dist>/devtools.json` handshake it reads,
 * and every tool re-reads it per call, so restarting the host (new port, new
 * token) does not require restarting the MCP server.
 *
 * The tools are deliberately 1:1 with the wire commands — no convenience
 * layering, so anything `ctl.ts` can do is reachable here with the same
 * arguments and the same error text.
 */

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { CallToolRequestSchema, ListToolsRequestSchema } from "@modelcontextprotocol/sdk/types.js";
import { connect } from "node:net";
import { readFileSync } from "node:fs";
import { resolve, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const distFlag = argv.indexOf("--dist");
const DIST = distFlag >= 0 && argv[distFlag + 1] ? resolve(argv[distFlag + 1]) : resolve(HERE, "..", "dist");
const TIMEOUT_MS = 10_000;

type Handshake = { port: number; pid: number; token: string };

function handshake(): Handshake {
  const path = join(DIST, "devtools.json");
  const raw = readFileSync(path, "utf8"); // throws → reported as the tool error
  const h = JSON.parse(raw) as Handshake;
  if (!h.port || !h.token) throw new Error(`${path} has no port/token`);
  return h;
}

function send(port: number, payload: string): Promise<string> {
  return new Promise((resolveReply, reject) => {
    const socket = connect({ host: "127.0.0.1", port });
    let buffer = "";
    const timer = setTimeout(() => {
      socket.destroy();
      reject(new Error(`no reply within ${TIMEOUT_MS}ms — the host's frame pump may be wedged`));
    }, TIMEOUT_MS);
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
      finish(() => reject(new Error(`cannot reach 127.0.0.1:${port}: ${e.message}`))),
    );
    socket.on("close", () => finish(() => reject(new Error("connection closed before a reply"))));
  });
}

async function call(request: Record<string, unknown>) {
  const h = handshake();
  const line = await send(h.port, JSON.stringify({ token: h.token, ...request }));
  return JSON.parse(line) as { ok?: boolean; data?: unknown; error?: string };
}

// ---------------------------------------------------------------------------
// tool table — 1:1 with the wire commands (docs/DEVTOOLS.md)
// ---------------------------------------------------------------------------

const str = (description: string) => ({ type: "string" as const, description });
const int = (description: string) => ({ type: "number" as const, description });

const SELECTOR = str("a CSS selector, matched by the same vn-css cascade matcher the app styles with");
const INDEX = int("which match to use when the selector matches several (0-based)");

const TOOLS = [
  {
    name: "vn_screenshot",
    description:
      "Capture the app window to a PNG. Default backend is gpui's render_to_image readback: the " +
      "CLIENT area exactly (clientX/clientY come back 0), works unfocused, occluded, off-screen " +
      "and minimized, and never raises or activates the window — but it cannot see a mica/acrylic " +
      "backdrop, which DWM composites under the app. Pass backend=print_window for the composed " +
      "image (PrintWindow(PW_RENDERFULLCONTENT), whole window rect, stale when minimized).",
    inputSchema: {
      type: "object",
      properties: {
        path: str("output PNG path (default: <dist>/screenshots/<n>.png)"),
        backend: str('"auto" (default), "render_to_image" or "print_window"'),
      },
    },
    build: (a: any) => ({
      cmd: "screenshot",
      ...(a.path ? { path: a.path } : {}),
      ...(a.backend ? { backend: a.backend } : {}),
    }),
  },
  {
    name: "vn_query",
    description:
      "querySelectorAll through the REAL cascade matcher. Returns [{id, tag, classes, text, " +
      "bounds, scopes}]. An unsupported selector (`:has()`, `::before`, `:visited`, …) is an " +
      "error carrying the same advisory the stylesheet compiler prints.",
    inputSchema: { type: "object", properties: { selector: SELECTOR }, required: ["selector"] },
    build: (a: any) => ({ cmd: "query", selector: a.selector }),
  },
  {
    name: "vn_click",
    description:
      "Synthesize mousedown -> mouseup -> click at the node's centre, through the app's own " +
      "dispatch (capture/bubble, preventDefault honored). NOT a real pointer: :hover / :active " +
      "visuals are not exercised. Errors on 0 or >1 matches unless `index` is given.",
    inputSchema: {
      type: "object",
      properties: { selector: SELECTOR, index: INDEX, button: int("0 left, 1 middle, 2 right") },
      required: ["selector"],
    },
    build: (a: any) => ({ cmd: "click", selector: a.selector, index: a.index, button: a.button }),
  },
  {
    name: "vn_type",
    description:
      "Focus the node and type `text` one keystroke at a time through gpui's real key path, so " +
      "keydown / input / v-model all fire exactly as they do for a physical key — including the " +
      "app's own preventDefault() guards.",
    inputSchema: {
      type: "object",
      properties: { selector: SELECTOR, text: str("the text to type"), index: INDEX },
      required: ["selector", "text"],
    },
    build: (a: any) => ({ cmd: "type", selector: a.selector, text: a.text, index: a.index }),
  },
  {
    name: "vn_key",
    description:
      "Send one keystroke: `Enter`, `Escape`, `Shift+Tab`, `Ctrl+A`, `ArrowDown`, … With a " +
      "selector the node is focused first; without one it goes to whatever already has focus.",
    inputSchema: {
      type: "object",
      properties: { key: str("key name, optionally Mod+Key"), selector: SELECTOR, index: INDEX },
      required: ["key"],
    },
    build: (a: any) => ({ cmd: "key", key: a.key, selector: a.selector, index: a.index }),
  },
  {
    name: "vn_scroll",
    description:
      "Scroll a container by (dx, dy) CSS pixels through its own gpui ScrollHandle and fire the " +
      "`scroll` event. The node must have `overflow: scroll|auto` in the cascade.",
    inputSchema: {
      type: "object",
      properties: { selector: SELECTOR, dy: int("vertical delta, positive = down"), dx: int("horizontal delta"), index: INDEX },
      required: ["selector"],
    },
    build: (a: any) => ({ cmd: "scroll", selector: a.selector, dx: a.dx ?? 0, dy: a.dy ?? 0, index: a.index }),
  },
  {
    name: "vn_text",
    description: "textContent of the matched subtree (or a native <input>'s live buffer).",
    inputSchema: { type: "object", properties: { selector: SELECTOR, index: INDEX }, required: ["selector"] },
    build: (a: any) => ({ cmd: "text", selector: a.selector, index: a.index }),
  },
  {
    name: "vn_tree",
    description: "Dump the retained tree (tag / classes / props / listeners / bounds / children).",
    inputSchema: {
      type: "object",
      properties: { selector: str("subtree root (default: the mounted root)"), depth: int("levels to descend (default 6)") },
    },
    build: (a: any) => ({ cmd: "tree", selector: a.selector, depth: a.depth }),
  },
  {
    name: "vn_styles",
    description: "The node's folded ComputedStates: base + the hover/active/focus/focus-visible rungs, transitions and extras.",
    inputSchema: { type: "object", properties: { selector: SELECTOR, index: INDEX }, required: ["selector"] },
    build: (a: any) => ({ cmd: "styles", selector: a.selector, index: a.index }),
  },
  {
    name: "vn_eval",
    description:
      "Evaluate JavaScript inside the app's JSC, in global scope. Returns {value, type} or " +
      "{exception, stack}. The escape hatch for everything the other tools do not cover.",
    inputSchema: { type: "object", properties: { js: str("JavaScript source") }, required: ["js"] },
    build: (a: any) => ({ cmd: "eval", js: a.js }),
  },
  {
    name: "vn_logs",
    description:
      "The host's log ring: console output, [js uncaught] errors, CSS diagnostics and compat " +
      "advisories, plus the current status line. Page with `since` (the `next` from the last call).",
    inputSchema: { type: "object", properties: { since: int("first sequence number to return") } },
    build: (a: any) => ({ cmd: "logs", since: a.since }),
  },
];

// ---------------------------------------------------------------------------

const server = new Server(
  { name: "vn-devtools", version: "1.0.0" },
  { capabilities: { tools: {} } },
);

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: TOOLS.map(({ name, description, inputSchema }) => ({ name, description, inputSchema })),
}));

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const tool = TOOLS.find((t) => t.name === request.params.name);
  if (!tool) {
    return { isError: true, content: [{ type: "text", text: `unknown tool ${request.params.name}` }] };
  }
  const args = (request.params.arguments ?? {}) as Record<string, unknown>;
  try {
    const wire = tool.build(args);
    // Drop undefined so the host never sees `"index": null`.
    for (const k of Object.keys(wire)) if ((wire as any)[k] === undefined) delete (wire as any)[k];
    const reply = await call(wire);
    if (!reply.ok) {
      return { isError: true, content: [{ type: "text", text: reply.error ?? "unknown error" }] };
    }
    return { content: [{ type: "text", text: JSON.stringify(reply.data, null, 2) }] };
  } catch (e) {
    return {
      isError: true,
      content: [
        {
          type: "text",
          text:
            `${(e as Error).message}\n` +
            `(dist slot: ${DIST} — is a dev host running with this dist? ` +
            `start one with VN_DIST=${DIST} target/x86_64-pc-windows-msvc/debug/vn-app.exe)`,
        },
      ],
    };
  }
});

await server.connect(new StdioServerTransport());
