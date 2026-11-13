// A loopback WebSocket echo server for the hello app's Lab tab (dev-only, Bun's server): echoes
// every frame with the same opcode, so text returns a string and binary returns bytes.
//   bun packages/vue-native/devtools/ws-echo.ts [port]     # default 8787
// A plain GET gets a one-line status, so `curl` can tell "up" from "nothing listening".

const port = Number(process.argv[2] ?? 8787);

const server = Bun.serve({
  port,
  hostname: "127.0.0.1",
  fetch(req, srv) {
    if (srv.upgrade(req)) return;
    return new Response(`ws-echo listening on ws://127.0.0.1:${port}/\n`, {
      headers: { "content-type": "text/plain" },
    });
  },
  websocket: {
    open(ws) {
      console.log(`[ws-echo] client connected (${ws.remoteAddress})`);
    },
    message(ws, message) {
      const kind = typeof message === "string" ? `text ${message.length} chars` : `binary ${message.byteLength} bytes`;
      console.log(`[ws-echo] echo ${kind}`);
      ws.send(message);
    },
    close(_ws, code, reason) {
      console.log(`[ws-echo] client closed ${code} ${JSON.stringify(reason ?? "")}`);
    },
  },
});

console.log(`[ws-echo] listening on ws://127.0.0.1:${server.port}/`);
