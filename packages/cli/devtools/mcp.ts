#!/usr/bin/env bun
/**
 * sk-devtools MCP server: the same wire as `ctl.ts`, exposed as MCP tools `sk_<cmd>`.
 *
 *   bun packages/cli/devtools/mcp.ts [--dist <slot> | --app <name>]
 *
 * `--app <name>` attaches to a packaged app by its sk.config name, re-resolved per call (./apps.ts).
 * Every tool re-reads `<dist>/devtools.json` per call, so restarting the host does not require
 * restarting the server.
 */

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { CallToolRequestSchema, ListToolsRequestSchema } from "@modelcontextprotocol/sdk/types.js";
import { connect } from "node:net";
import { readFileSync } from "node:fs";
import { resolve, dirname, join } from "node:path";
import { StringDecoder } from "node:string_decoder";
import { fileURLToPath } from "node:url";

import { appSlot } from "./apps";
import { mcpTools } from "./commands";

const HERE = dirname(fileURLToPath(import.meta.url));
const argv = process.argv.slice(2);
const distFlag = argv.indexOf("--dist");
const DIST = distFlag >= 0 && argv[distFlag + 1] ? resolve(argv[distFlag + 1]!) : resolve(HERE, "..", "dist");
// `--app <name>`: a packaged app. Re-resolved on every call: each launch is a new pid and so a new slot.
const appFlag = argv.indexOf("--app");
const APP = appFlag >= 0 && argv[appFlag + 1] ? argv[appFlag + 1]! : null;
const TIMEOUT_MS = 10_000;

type Handshake = { port: number; pid: number; token: string };

function slotDir(): string {
  if (!APP) return DIST;
  const found = appSlot(APP);
  if ("error" in found) throw new Error(found.error);
  return found.dir;
}

function handshake(): Handshake {
  const path = join(slotDir(), "devtools.json");
  const raw = readFileSync(path, "utf8"); // throws → reported as the tool error
  const h = JSON.parse(raw) as Handshake;
  if (!h.port || !h.token) throw new Error(`${path} has no port/token`);
  return h;
}

function send(port: number, payload: string): Promise<string> {
  return new Promise((resolveReply, reject) => {
    const socket = connect({ host: "127.0.0.1", port });
    let buffer = "";
    const decoder = new StringDecoder("utf8");
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
      buffer += decoder.write(chunk);
      const nl = buffer.indexOf("\n");
      if (nl >= 0) finish(() => resolveReply(buffer.slice(0, nl)));
    });
    socket.on("error", (e) => finish(() => reject(new Error(`cannot reach 127.0.0.1:${port}: ${e.message}`))));
    socket.on("close", () => finish(() => reject(new Error("connection closed before a reply"))));
  });
}

async function call(request: Record<string, unknown>) {
  const h = handshake();
  const line = await send(h.port, JSON.stringify({ token: h.token, ...request }));
  return JSON.parse(line) as { ok?: boolean; data?: unknown; error?: string };
}

const TOOLS = mcpTools();

const server = new Server({ name: "sk-devtools", version: "1.0.0" }, { capabilities: { tools: {} } });

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
    const reply = await call(tool.build(args));
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
          text: APP
            ? `${(e as Error).message}\n(packaged app: ${APP})`
            : `${(e as Error).message}\n` +
              `(dist slot: ${DIST} — is a dev host running with this dist? ` +
              `start one with SK_DIST=${DIST} target/x86_64-pc-windows-msvc/debug/sk-app.exe)`,
        },
      ],
    };
  }
});

await server.connect(new StdioServerTransport());
