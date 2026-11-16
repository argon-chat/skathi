import type { Details } from "../../details";

export const details: Details = {
  fetch: {
    examples: [
      {
        title: "Load JSON from your API and handle HTTP errors",
        lang: "ts",
        code: `async function loadProjects(token: string) {
  const res = await fetch("https://api.example.com/v1/projects", {
    headers: { Authorization: "Bearer " + token },
    credentials: "include",
    signal: AbortSignal.timeout(10_000),
  });
  if (!res.ok) throw new Error("Projects request failed: " + res.status);
  return res.json();
}`,
      },
      {
        title: "Read a file bundled with the app",
        lang: "ts",
        code: `const res = await fetch("assets/changelog.md");
if (res.status === 404) {
  console.warn("changelog.md is not in this build");
} else {
  changelog.value = await res.text();
}`,
        note: "A path without a scheme reads from the app's own files, in development and in a packaged build.",
      },
      {
        title: "Cancel a search request when the query changes",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref, watch } from "vue";

const query = ref("");
const results = ref<string[]>([]);
let pending: AbortController | null = null;

watch(query, async (q) => {
  pending?.abort();
  pending = new AbortController();
  try {
    const res = await fetch("https://api.example.com/search?q=" + encodeURIComponent(q), { signal: pending.signal });
    results.value = await res.json();
  } catch (e) {
    if ((e as DOMException).name !== "AbortError") throw e;
  }
});
</script>

<template>
  <input v-model="query" placeholder="Search" />
  <ul><li v-for="r in results" :key="r">{{ r }}</li></ul>
</template>`,
      },
    ],
    edgeCases: [
      "An HTTP error status such as `404` or `503` resolves with `ok: false`. Only a request that fails rejects, with a `TypeError` whose message starts with `Failed to fetch`.",
      "A relative path that is not in the app's package resolves with status `404` and logs an advisory. It never falls through to the working directory.",
      'Cookies are sent and stored only with `credentials: "include"`. The default, `same-origin`, sends none to any network URL, because the app\'s origin (`app://redjs`) is opaque. `Set-Cookie` never appears in `res.headers`.',
      '`redirect: "manual"`, `mode: "no-cors"`, `integrity` and the methods `CONNECT`, `TRACE` and `TRACK` reject with a `TypeError` that names the option.',
      "A connect that takes over 15 seconds, or a request on which no byte moves for 60 seconds, rejects even without a `signal`. An aborted request rejects with `signal.reason`: an `AbortError`, or a `TimeoutError` from `AbortSignal.timeout`.",
    ],
  },
  request: {
    examples: [
      {
        title: "Build an authenticated request once and send it",
        lang: "ts",
        code: `const save = new Request("https://api.example.com/v1/notes", {
  method: "POST",
  headers: { "Content-Type": "application/json", Authorization: "Bearer " + token },
  body: JSON.stringify({ title: "Draft", body: noteText }),
});
const res = await fetch(save);`,
      },
      {
        title: "Log a request's body before sending it",
        lang: "ts",
        code: `async function send(req: Request): Promise<Response> {
  if (debugNetwork) {
    console.debug(req.method, req.url, await req.clone().text());
  }
  return fetch(req);
}`,
      },
    ],
    edgeCases: [
      'The method is uppercased: `method: "post"` reads back as `"POST"`.',
      "`new Request` throws a `TypeError` for the options that `fetch` refuses, where `fetch` rejects.",
      "A string body adds `Content-Type: text/plain;charset=UTF-8`, a `URLSearchParams` body adds `application/x-www-form-urlencoded;charset=UTF-8`, and a `FormData` body adds `multipart/form-data` with its boundary, unless you set `Content-Type` yourself.",
      "Reading the body a second time rejects with a `TypeError`, and `clone()` after a read throws one.",
    ],
  },
  response: {
    examples: [
      {
        title: "Read a JSON page and its total count",
        lang: "ts",
        code: `const res = await fetch("https://api.example.com/v1/inbox?page=2");
const total = Number(res.headers.get("X-Total-Count") ?? 0);
const items = await res.json();
inbox.value = { items, total };`,
      },
      {
        title: "Answer requests from a test double",
        lang: "ts",
        code: `globalThis.fetch = async (input) => {
  const url = String(input instanceof Request ? input.url : input);
  if (url.endsWith("/v1/me")) return Response.json({ id: 7, name: "Ada" });
  return new Response("not found", { status: 404 });
};`,
      },
    ],
    edgeCases: [
      'A `204`, `205` or `304` response has `body === null`, and `text()` resolves to `""`.',
      "`new Response(body, { status })` throws a `TypeError` for a status outside 200–599.",
      '`Response.error()` is the only response with `status` 0, and its `type` is `"error"`.',
    ],
  },
  headers: {
    examples: [
      {
        title: "Add an auth header to every API call",
        lang: "ts",
        code: `function apiHeaders(token: string): Headers {
  const h = new Headers({ Accept: "application/json" });
  h.set("Authorization", "Bearer " + token);
  h.append("X-Client", "desktop");
  return h;
}`,
      },
      {
        title: "Log every response header",
        lang: "ts",
        code: `const res = await fetch(url);
for (const [name, value] of res.headers) {
  console.debug(name + ": " + value);
}`,
      },
    ],
    edgeCases: [
      "Names are case-insensitive and come back lowercased, and iteration is sorted by name.",
      "`append` on an existing name keeps both values, and `get` joins them with `, `.",
      "A value that contains CR or LF throws.",
      '`res.headers.get("set-cookie")` is `null` and `getSetCookie()` is `[]`: the cookie jar keeps them.',
    ],
  },
  "xml-http-request": {
    examples: [
      {
        title: "Show upload progress for a large file",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const percent = ref(0);

function upload(e: Event) {
  const file = (e.target as HTMLInputElement).files![0];
  const xhr = new XMLHttpRequest();
  xhr.open("PUT", "https://storage.example.com/uploads/" + encodeURIComponent(file.name));
  xhr.upload.onprogress = (p) => (percent.value = Math.round((p.loaded / p.total) * 100));
  xhr.onload = () => console.info("Stored with ETag", xhr.getResponseHeader("ETag"));
  xhr.onerror = () => console.error("Upload failed");
  xhr.send(file);
}
</script>

<template>
  <input type="file" @change="upload" />
  <div class="bar"><div class="fill" :style="{ width: percent + '%' }" /></div>
</template>`,
      },
      {
        title: "Extend the timeout while an upload keeps moving",
        lang: "ts",
        code: `const xhr = new XMLHttpRequest();
const startedAt = performance.now();
xhr.open("PUT", partUrl);
xhr.timeout = 10_000;
xhr.upload.onprogress = () => {
  xhr.timeout = Math.ceil(performance.now() - startedAt) + 10_000;
};
xhr.ontimeout = () => retryPart(partUrl);
xhr.send(part);`,
        note: "The deadline slides forward on every progress event, so a slow but moving upload never times out.",
      },
    ],
    edgeCases: [
      "`open(method, url, false)` throws `InvalidAccessError`: synchronous requests are refused.",
      '`responseXML` is `null`, and so is `response` with `responseType = "document"`.',
      "Upload progress is reported every 64 KiB, and `upload.onprogress` fires for the last piece too, before `load` and `loadend`.",
      "With `timeout` at 0 the request still ends in an `error` event after 15 seconds of connecting or 60 seconds without a byte moving.",
      "`withCredentials = true` sends and stores cookies. Without it, no cookies go to any network URL.",
    ],
  },
  "web-socket": {
    examples: [
      {
        title: "Keep a chat connection open and reconnect with backoff",
        lang: "ts",
        code: `let delay = 1000;

function connect() {
  const ws = new WebSocket("wss://chat.example.com/live");
  ws.onopen = () => (delay = 1000);
  ws.onmessage = (e) => receive(JSON.parse(e.data));
  ws.onclose = (e) => {
    if (e.code === 1000) return;
    setTimeout(connect, delay);
    delay = Math.min(delay * 2, 30_000);
  };
}
connect();`,
        note: "The socket never reconnects or pings on its own.",
      },
      {
        title: "Drop frames when the socket falls behind",
        lang: "ts",
        code: `function sendFrame(ws: WebSocket, frame: Uint8Array): boolean {
  if (ws.readyState !== WebSocket.OPEN) return false;
  if (ws.bufferedAmount > 1_000_000) return false;
  ws.send(frame);
  return true;
}`,
        note: "`bufferedAmount` counts the bytes the socket has not yet taken, so it is a real backpressure signal.",
      },
    ],
    edgeCases: [
      "`send()` while `CONNECTING` throws `InvalidStateError`. While `CLOSING` or `CLOSED` it sends nothing but still adds to `bufferedAmount`.",
      "`close(code)` throws `InvalidAccessError` unless the code is 1000 or in 3000–4999, and `SyntaxError` for a reason over 123 UTF-8 bytes.",
      '`binaryType` defaults to `"blob"`. A message over 64 MiB closes the connection with code 1009.',
      "At most 64 sockets can be open at once: the next constructor throws `InvalidStateError`.",
      "The handshake sends no `Origin` header, offers no `permessage-deflate`, and carries the cookie jar's cookies for the URL. Open sockets close on `location.reload()`.",
    ],
  },
  "web-socket-stream": {
    examples: [
      {
        title: "Run a streaming RPC channel",
        lang: "ts",
        code: `const stop = new AbortController();
const wss = new WebSocketStream("wss://rpc.example.com/v2", {
  protocols: ["rpc.v2"],
  signal: stop.signal,
});
const { readable, writable } = await wss.opened;
const writer = writable.getWriter();
await writer.write(JSON.stringify({ method: "subscribe", room: "general" }));
for await (const message of readable) {
  handleRpc(typeof message === "string" ? JSON.parse(message) : message);
}`,
      },
      {
        title: "Close with a code and wait for the result",
        lang: "ts",
        code: `wss.close({ closeCode: 4000, reason: "Signed out" });
try {
  const { closeCode, reason } = await wss.closed;
  console.info("Closed", closeCode, reason);
} catch (e) {
  console.warn("Connection dropped", e);
}`,
      },
    ],
    edgeCases: [
      "Binary messages arrive as `ArrayBuffer`, text messages as strings.",
      "If the connection never opens, `opened` rejects with a `NetworkError` DOMException. An unclean close rejects `closed` with one and errors `readable`.",
      "Aborting `signal`, cancelling `readable` or closing `writable` closes the socket.",
      "The socket keeps reading when your code stops: unread messages queue in memory instead of slowing the server. `write()` waits only once more than 1 MiB is buffered.",
      "A bad URL or protocol list throws synchronously, with the same error `new WebSocket` throws.",
    ],
  },
  "event-source": {
    examples: [
      {
        title: "Follow a build log with server-sent events",
        lang: "vue",
        code: `<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

const lines = ref<string[]>([]);
const status = ref("connecting");
let source: EventSource | null = null;

onMounted(() => {
  source = new EventSource("https://ci.example.com/builds/812/log", { withCredentials: true });
  source.onopen = () => (status.value = "live");
  source.onerror = () => (status.value = source!.readyState === EventSource.CLOSED ? "stopped" : "reconnecting");
  source.addEventListener("line", (e) => lines.value.push((e as MessageEvent).data));
  source.addEventListener("done", () => source!.close());
});
onUnmounted(() => source?.close());
</script>

<template>
  <p class="status">{{ status }}</p>
  <p v-for="(line, i) in lines" :key="i" class="log-line">{{ line }}</p>
</template>`,
      },
    ],
    edgeCases: [
      "An `event:` field dispatches an event of that type, which only `addEventListener` hears; `onmessage` gets the events without one. An event with no `data:` line is not dispatched.",
      "A reply other than `200`, or with a type other than `text/event-stream`, fires `error` and closes the source for good (`readyState` 2).",
      "When the stream ends, `error` fires with `readyState` back at `CONNECTING`, and the source reconnects after 3 seconds unless the server sent `retry:`.",
      "`withCredentials: true` sends and stores cookies. Without it the request carries none.",
    ],
  },
  url: {
    examples: [
      {
        title: "Build an API URL with query parameters",
        lang: "ts",
        code: `const url = new URL("/v1/search", "https://api.example.com");
url.searchParams.set("q", query);
url.searchParams.set("limit", "50");
const res = await fetch(url);`,
      },
      {
        title: "Validate a link the user pasted",
        lang: "ts",
        code: `function normalizeLink(input: string): string | null {
  if (!URL.canParse(input)) return null;
  const url = new URL(input);
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  url.hash = "";
  return url.href;
}`,
      },
      {
        title: "Preview a generated image through an object URL",
        lang: "vue",
        code: `<script setup lang="ts">
import { onUnmounted, ref } from "vue";

const preview = ref("");

function showThumbnail(blob: Blob) {
  if (preview.value) URL.revokeObjectURL(preview.value);
  preview.value = URL.createObjectURL(blob);
}
onUnmounted(() => preview.value && URL.revokeObjectURL(preview.value));
</script>

<template>
  <img v-if="preview" :src="preview" class="thumbnail" alt="" />
</template>`,
      },
    ],
    edgeCases: [
      "`URL.parse` is not defined. Test with `URL.canParse`, or catch the `TypeError` from `new URL`.",
      "Object URLs have the form `blob:app://redjs/<uuid>` and stay valid until you revoke them. A fragment is ignored when one is resolved.",
      "After `revokeObjectURL`, `fetch` and `createImageBitmap` reject with a `TypeError`, and an `<img>` shows a broken image with one advisory.",
      "`revokeObjectURL` ignores anything that is not a live `blob:` URL. `createObjectURL` with anything but a `Blob` throws a `TypeError`.",
    ],
  },
  "url-search-params": {
    examples: [
      {
        title: "Post a form-encoded token request",
        lang: "ts",
        code: `const body = new URLSearchParams({
  grant_type: "refresh_token",
  refresh_token: refreshToken,
});
const res = await fetch("https://auth.example.com/oauth/token", { method: "POST", body });`,
      },
      {
        title: "Read filters from a deep link",
        lang: "ts",
        code: `const params = new URL(deepLink).searchParams;
const tags = params.getAll("tag");
const page = Number(params.get("page") ?? "1");
showResults({ tags, page });`,
      },
    ],
    edgeCases: [
      "As a `fetch` body it sets `Content-Type: application/x-www-form-urlencoded;charset=UTF-8`, unless you set one.",
      "Changes made through `url.searchParams` write back to the URL's `search` and `href`.",
    ],
  },
  "url-pattern": {
    examples: [
      {
        title: "Route deep links to views",
        lang: "ts",
        code: `const routes: [URLPattern, string][] = [
  [new URLPattern({ pathname: "/projects/:id" }), "project"],
  [new URLPattern({ pathname: "/projects/:id/files/*" }), "file"],
  [new URLPattern({ pathname: "/settings/:section?" }), "settings"],
];

function route(pathname: string) {
  for (const [pattern, view] of routes) {
    const match = pattern.exec({ pathname });
    if (match) return { view, params: match.pathname.groups };
  }
  return { view: "not-found", params: {} };
}`,
      },
      {
        title: "Match API URLs across subdomains",
        lang: "ts",
        code: `const api = new URLPattern("https://*.example.com/v1/:resource");
if (api.test(request.url)) {
  const { resource } = api.exec(request.url)!.pathname.groups;
  console.debug("API call", resource);
}`,
      },
    ],
    edgeCases: [
      'A pattern string without a base, such as `"/users/:id"`, throws a `TypeError`. Pass a base URL as the second argument, or an object with `pathname`.',
      "`exec` returns `null` when nothing matches. An optional group that did not match is `undefined` in `groups`.",
      "`hasRegExpGroups` is `true` once any part uses a custom `(...)` group.",
    ],
  },
  "accept-language": {
    examples: [
      {
        title: "Ask for one response in a specific language",
        lang: "ts",
        code: `const res = await fetch("https://api.example.com/v1/help", {
  headers: { "Accept-Language": "de-DE,de;q=0.9" },
});
helpText.value = await res.text();`,
        note: "A header you set replaces the default for that request.",
      },
    ],
    edgeCases: [
      "`navigator.languages` of `ru-RU, en-US` is sent as `ru-RU,ru;q=0.9,en-US;q=0.8,en;q=0.7`: base languages are added after their regional tags, and each weight drops by 0.1, down to 0.1.",
      "WebSocket handshakes send it too.",
      "The system languages are read once, when the app starts.",
    ],
  },
  "navigator-send-beacon": {
    edgeCases: ["`navigator.sendBeacon` is `undefined`."],
  },
  "service-workers": {
    edgeCases: ["`navigator.serviceWorker` is `undefined`."],
  },
  "cache-api": {
    edgeCases: ["`caches` is `undefined`."],
  },
};
