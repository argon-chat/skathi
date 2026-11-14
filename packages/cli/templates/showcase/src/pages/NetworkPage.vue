<script setup lang="ts">
import { onUnmounted, ref, shallowRef } from "vue";
import Card from "../components/Card.vue";
import Page from "../components/Page.vue";
import { describe } from "../engine";

const online = () => navigator.onLine;
const quote = ref("");
const took = ref("");
async function load() {
  const start = performance.now();
  try {
    const res = await fetch("https://api.github.com/zen");
    quote.value = res.ok ? await res.text() : `HTTP ${res.status}`;
  } catch (e) {
    quote.value = describe(e);
  }
  took.value = `${Math.round(performance.now() - start)} ms`;
}

const ECHO = "wss://echo.websocket.org";
const socket = shallowRef<WebSocket | null>(null);
const message = ref("Hello over a socket");
const log = ref<string[]>([]);
const note = (line: string) => (log.value = [line, ...log.value].slice(0, 6));

function connect() {
  const ws = new WebSocket(ECHO);
  ws.onopen = () => note("open");
  ws.onmessage = (e) => note(`← ${String(e.data)}`);
  ws.onclose = (e) => {
    note(`closed (${e.code})`);
    socket.value = null;
  };
  socket.value = ws;
}
function send(ws: WebSocket) {
  ws.send(message.value);
  note(`→ ${message.value}`);
}
onUnmounted(() => socket.value?.close());

const webView = skFeatures.iframe;
const embedded = ref(false);
</script>

<template>
  <Page
    title="Network"
    lead='fetch and WebSocket as in a browser; responses are cached on disk (sk.config.json "httpCache").'
  >
    <Card title="fetch" :api="online() ? 'online' : 'offline'">
      <div class="row">
        <button class="btn primary" @click="load">GET api.github.com/zen</button>
        <span class="faint">{{ took }}</span>
      </div>
      <p v-if="quote" class="quote">{{ quote }}</p>
    </Card>

    <Card title="WebSocket" :api="ECHO">
      <div class="row">
        <button v-if="!socket" class="btn" @click="connect">Connect</button>
        <template v-else>
          <input v-model="message" class="input grow" @keyup.enter="send(socket)" />
          <button class="btn primary" @click="send(socket)">Send</button>
          <button class="btn" @click="socket.close()">Close</button>
        </template>
      </div>
      <div v-for="(line, i) in log" :key="i" class="mono line">{{ line }}</div>
    </Card>

    <Card title="Embedded web page" api="<iframe>">
      <template v-if="webView">
        <div class="row">
          <button class="btn" @click="embedded = !embedded">{{ embedded ? "Remove it" : "Embed example.com" }}</button>
        </div>
        <iframe v-if="embedded" class="frame" src="https://example.com" />
      </template>
      <p v-else class="muted">This platform has no web view for &lt;iframe&gt; (it needs WebView2 on Windows).</p>
      <p class="faint">The page runs in the system web view, isolated from the app; postMessage talks across.</p>
    </Card>
  </Page>
</template>

<style scoped>
.grow {
  flex: 1;
}
.quote {
  margin: 0;
  font-size: 16px;
  font-style: italic;
  color: var(--text);
}
.line {
  color: var(--text-muted);
}
.frame {
  width: 100%;
  height: 260px;
  border-radius: 10px;
  border: 1px solid var(--line);
}
</style>
