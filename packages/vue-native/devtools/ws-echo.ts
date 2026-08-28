// A loopback WebSocket echo server for the hello app's Lab tab.
//
// Dev-only, dependency-free (Bun's built-in server), and deliberately dumb: it
// echoes every frame back with the same opcode, so a text message returns a
// string and a binary message returns bytes. That is the whole point — the Lab
// panel is proving that redjs's native `WebSocket` carries BOTH, and an echo
// that quietly converted one to the other would hide the interesting half.
//
//   bun packages/vue-native/devtools/ws-echo.ts [port]     # default 8787
//
// It also answers a plain GET with a one-line status, so `curl` (or a browser)
// can tell "the server is up" from "nothing is listening" without a client.

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
