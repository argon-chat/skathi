<script setup lang="ts">
import { onActivated, onMounted, onUnmounted, ref, shallowRef } from "vue";
import { useSession } from "./stores/session";

// `name` is what <KeepAlive>'s include/exclude matches on — the shell excludes exactly this page,
// so it is the one tab that really unmounts and remounts on every visit.
defineOptions({ name: "LabPage" });

const localVisits = ref(0);
const scratch = ref("");
console.log("[app] LabPage setup ran (NOT kept alive — this prints every visit)");
onMounted(() => {
  localVisits.value++;
  console.log("[app] LabPage mounted (local state is brand new)");
});
onUnmounted(() => console.log("[app] LabPage unmounted (left the tab)"));
// …and onActivated never fires for a page outside the cache.
onActivated(() => console.log("[app] LabPage ACTIVATED — should NEVER appear"));

const session = useSession();
const mountId = Math.random().toString(36).slice(2, 7);

const locked = ref(false);
const checked = ref(false);
const open = ref(false);

const WS_URL = "ws://127.0.0.1:8787/";
// Indexed by `readyState`, which is 0..3 — so the lookup can miss as far as the compiler is
// concerned, and every use below falls back rather than rendering "undefined".
const STATES = ["CONNECTING", "OPEN", "CLOSING", "CLOSED"];
const stateName = (s: SkWebSocket) => STATES[s.readyState] ?? "UNKNOWN";

// shallowRef, not ref: a WebSocket is a host object and wrapping it in Vue's deep reactive proxy
// would put a Proxy between us and its native getters.
const sock = shallowRef<SkWebSocket | null>(null);
const wsState = ref("CLOSED");
const wsProtocol = ref("");
const wsBuffered = ref(0);
const wsBinaryType = ref("blob");
const txText = ref(0);
const txBin = ref(0);
const rxText = ref("—");
const rxBin = ref("—");
const wsLog = ref("idle — start the echo server, then press connect");

function syncWs() {
  const s = sock.value;
  wsState.value = s ? stateName(s) : "CLOSED";
  wsBuffered.value = s ? s.bufferedAmount : 0;
  wsProtocol.value = s ? s.protocol : "";
  wsBinaryType.value = s ? s.binaryType : "blob";
}

function wsConnect() {
  wsClose();
  const s = new WebSocket(WS_URL);
  sock.value = s;
  wsLog.value = "connecting to " + WS_URL;
  syncWs();
  s.onopen = () => {
    wsLog.value = "open — readyState " + s.readyState;
    syncWs();
  };
  s.onerror = () => {
    wsLog.value = "error — is `bun packages/cli/devtools/ws-echo.ts` running?";
    syncWs();
  };
  s.onclose = (e) => {
    wsLog.value = `close code=${e.code} reason="${e.reason}" wasClean=${e.wasClean}`;
    syncWs();
  };
  s.onmessage = async (e) => {
    const data = e.data;
    if (typeof data === "string") {
      rxText.value = data;
    } else {
      const isBlob = data instanceof Blob;
      const kind = isBlob ? "Blob" : "ArrayBuffer";
      const buf = isBlob ? await data.arrayBuffer() : data;
      const bytes = Array.from(new Uint8Array(buf));
      rxBin.value = `${kind}(${bytes.length}) [${bytes.join(", ")}]`;
    }
    syncWs();
  };
}

function wsSendText() {
  const s = sock.value;
  if (!s || s.readyState !== WebSocket.OPEN) {
    wsLog.value = "send refused — the socket is " + (s ? stateName(s) : "absent");
    return;
  }
  txText.value += 1;
  s.send("text counter " + txText.value);
  syncWs();
}

function wsSendBinary() {
  const s = sock.value;
  if (!s || s.readyState !== WebSocket.OPEN) {
    wsLog.value = "send refused — the socket is " + (s ? stateName(s) : "absent");
    return;
  }
  txBin.value += 1;
  const n = txBin.value;
  s.send(new Uint8Array([0xa5, n & 255, (n >> 8) & 255, 0x5a]).buffer);
  syncWs();
}

function wsToggleBinaryType() {
  const s = sock.value;
  if (!s) return;
  s.binaryType = s.binaryType === "blob" ? "arraybuffer" : "blob";
  syncWs();
}

function wsClose() {
  const s = sock.value;
  if (s && s.readyState < WebSocket.CLOSING) s.close(1000, "lab");
  syncWs();
}

// The Lab tab really unmounts on every visit, so a socket left open here would keep receiving into
// a dead component.
onUnmounted(wsClose);
</script>

<template>
  <div class="stack">
    <div class="title">
      Lab — the one tab OUTSIDE &lt;KeepAlive&gt;
      <span class="label">·scoped·</span>
    </div>

    <!-- Local refs reset; the store does not. -->
    <div class="ka">
      <div class="ka-title">local component state vs pinia store state</div>
      <div class="hint">
        local: mount id <span class="bad">{{ mountId }}</span> — a fresh random per instance, so it changes on every
        visit ({{ localVisits }} mount on this one)
      </div>
      <input class="field" v-model="scratch" tabindex="6" placeholder="type — this is NOT kept" />
      <div class="hint">local scratch: "{{ scratch }}" (empty again on every visit)</div>
      <div class="hint good">
        pinia session.badge: {{ session.badge }} · notes {{ session.noteCount }} — survives anyway
      </div>
      <div class="ka-row">
        <div class="btn" @click="session.bump()">visits++ from Lab</div>
      </div>
    </div>

    <div class="ws">
      <div class="ws-title">WebSocket — text and binary against a local echo</div>
      <div class="ka-row">
        <div class="btn ws-connect" @click="wsConnect">connect</div>
        <div class="btn ws-send-text" @click="wsSendText">send text</div>
        <div class="btn ws-send-bin" @click="wsSendBinary">send binary</div>
        <div class="btn ws-binary-type" @click="wsToggleBinaryType">binaryType: {{ wsBinaryType }}</div>
        <div class="btn ws-close" @click="wsClose">close(1000)</div>
      </div>
      <div class="hint ws-state">
        readyState <span class="good">{{ wsState }}</span> · protocol "{{ wsProtocol }}" · bufferedAmount
        {{ wsBuffered }} · sent {{ txText }} text / {{ txBin }} binary
      </div>
      <div class="hint ws-rx-text">text ← {{ rxText }}</div>
      <div class="hint ws-rx-bin">binary ← {{ rxBin }}</div>
      <div class="hint ws-log">{{ wsLog }}</div>
    </div>

    <div class="r5">
      <div class="r5-title">:focus · props · [attr] · @media</div>

      <input class="field r5-focus" tabindex="3" placeholder="Tab or click me — :focus ring" />

      <!-- :focus on a plain tabbable div rides gpui's .focus() rung; :focus-visible only lights up
           when focus arrived from the KEYBOARD, exactly as in a browser. -->
      <div class="r5-box" tabindex="4">plain div, tabindex=4 — blue on :focus, amber on :focus-visible (Tab)</div>

      <div class="r5-row">
        <input class="field r5-input" :disabled="locked" :value="locked ? 'locked' : 'editable'" />
        <div class="r5-toggle" @click="locked = !locked">{{ locked ? "unlock" : "lock" }} — :disabled</div>
      </div>

      <div class="r5-check" :checked="checked" @click="checked = !checked">:checked = {{ checked }} — click me</div>

      <div class="r5-state" :data-state="open ? 'open' : 'closed'" @click="open = !open">
        [data-state="{{ open ? "open" : "closed" }}"] — click me
      </div>

      <div class="r5-media">@media width: GREY &lt; 640px · BLUE ≥ 640px · GREEN ≥ 900px</div>
      <div class="r5-dark">@media (prefers-color-scheme: dark) — AMBER while the OS theme is dark</div>
      <div class="r5-motion">@media (prefers-reduced-motion: reduce) — RED when the OS asks for less motion</div>

      <div
        class="flex flex-col gap-2 p-3 rounded-lg bg-white/5 border border-white/20 text-xs text-white/60 sm:text-sky-300 lg:text-emerald-300 dark:bg-amber-400/10"
      >
        <div class="font-mono">tailwind: sm:text-sky-300 · lg:text-emerald-300 · dark:bg-amber-400/10</div>
        <input
          class="field focus:border-sky-400 disabled:opacity-40"
          tabindex="5"
          :disabled="locked"
          placeholder="tailwind focus: + disabled:"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.stack {
  display: flex;
  flex-direction: column;
  gap: 12px;
  padding: 16px;
  background: #ffffff14;
  border-radius: 12px;
}
.title {
  font-size: 15px;
  color: #ffffff;
}
.field {
  height: 32px;
  width: 260px;
  padding: 6px;
  background: #00000040;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
  font-size: 13px;
  color: #ffffff;
}
.label {
  font-size: 12px;
  color: #7fd4ff;
}
.hint {
  font-size: 11px;
  color: #ffffff88;
}
.good {
  color: #ffd479;
}
.bad {
  color: #ff9aa2;
}

.ka {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px;
  border-radius: 10px;
  background: #ffffff0f;
  border: 1px solid #ffffff1f;
}
.ka-title {
  font-size: 12px;
  color: #ffffffcc;
}
.ka-row {
  display: flex;
  flex-direction: row;
  gap: 8px;
}
.btn {
  padding: 6px;
  border-radius: 6px;
  font-size: 11px;
  color: #ffffff;
  background: #4f8cff59;
  border: 1px solid #6ea4ff66;
  cursor: pointer;
  transition: background-color 150ms ease;
}
.btn:hover {
  background: #4f8cffcc;
}

.ws {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px;
  border-radius: 10px;
  background: #ffffff0f;
  border: 1px solid #ffffff1f;
}
.ws-title {
  font-size: 12px;
  color: #ffffffcc;
}
</style>

<style scoped>
.r5 {
  display: flex;
  flex-direction: column;
  gap: 8px;
  padding: 10px;
  border-radius: 10px;
  background: #ffffff0f;
  border: 1px solid #ffffff1f;
}
.r5-title {
  font-size: 12px;
  color: #ffffffcc;
}

/* :focus — on an <input> the rung is folded from the editor's own focus handle (tracking it on the
   wrapper too would double the tab stop). */
.r5-focus {
  width: 300px;
}
.r5-focus:focus {
  border-color: #7fd4ff;
  background: #7fd4ff26;
}

.field:focus {
  border-color: #7fd4ff;
}

/* :focus / :focus-visible on a plain tabbable div — gpui's .focus() and .focus_visible() rungs. */
.r5-box {
  width: 300px;
  padding: 6px;
  font-size: 11px;
  color: #ffffffaa;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
}
.r5-box:focus {
  border-color: #7fd4ff;
  color: #ffffff;
}
.r5-box:focus-visible {
  border-color: #ffcc00;
  color: #ffcc00;
}

/* :disabled / :checked are ELEMENT PROPERTIES, so they fold into the base bucket — there is no
   gpui rung for them and none is needed. */
.r5-row {
  display: flex;
  flex-direction: row;
  gap: 8px;
}
.r5-input {
  width: 150px;
}
.r5-input:disabled {
  opacity: 0.35;
  border-color: #ff3b3080;
}
.r5-toggle {
  padding: 6px;
  font-size: 11px;
  color: #ffffff;
  background: #4f8cff59;
  border-radius: 6px;
  cursor: pointer;
}
.r5-check {
  padding: 6px;
  font-size: 11px;
  color: #ffffff88;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
  cursor: pointer;
}
.r5-check:checked {
  color: #34c759;
  border-color: #34c759;
  background: #34c75926;
}

.r5-state {
  padding: 6px;
  font-size: 11px;
  color: #ffffff88;
  border: 1px solid #ffffff2e;
  border-radius: 6px;
  cursor: pointer;
}
.r5-state[data-state="open"] {
  color: #ff2d95;
  border-color: #ff2d95;
}

.r5-media,
.r5-dark,
.r5-motion {
  font-size: 11px;
  color: #ffffff66;
}
@media (min-width: 640px) {
  .r5-media {
    color: #7fd4ff;
  }
}
@media (min-width: 900px) {
  .r5-media {
    color: #34c759;
  }
}
@media (prefers-color-scheme: dark) {
  .r5-dark {
    color: #ffcc00;
  }
}
@media (prefers-reduced-motion: reduce) {
  .r5-motion {
    color: #ff3b30;
  }
}
</style>
