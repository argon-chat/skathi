<script setup lang="ts">
// Network: what the app asked for, and what came back.
//
// The feed is a JS-side tee over `fetch` and `WebSocket`
// (`crates/vn-app/src/devtools/net.rs`) rather than a hook inside redjs. That
// is the whole reason this panel exists at all in one milestone: the engine's
// `fetch` runs on a worker thread and its sockets on their own threads, so a
// Rust-side hook would have been a fork patch and a second description of every
// request. The JS side already knows the URL, the method, the status and the
// timing.
//
// It records from BOOT, not from when you opened this panel, because the
// request you want to see is usually the one the app made while starting up.
// The ring is 300 entries (`net.rs` CAPACITY) for the same reason the log ring
// is bounded: an app that polls must not grow the process.
import { computed } from "vue";
import { NET_H, PANE_HEAD_H } from "../layout";
import { clearNet, refreshNet, state, visibleNet } from "../store";
import { useVirtual } from "../virtual";

const props = defineProps<{ height: number }>();

const listHeight = computed(() => Math.max(40, props.height - PANE_HEAD_H - 22));
const rows = computed(() => visibleNet());
const list = useVirtual(
  () => rows.value.map(() => NET_H),
  () => listHeight.value,
);
const slice = computed(() => rows.value.slice(list.start, list.end));

function statusClass(entry: { status?: number; ok?: boolean; error?: string; phase?: string }): string {
  if (entry.error) return "bad";
  if (entry.phase === "error") return "bad";
  if (entry.status === undefined) return "";
  if (entry.status >= 500) return "bad";
  if (entry.status >= 400) return "warn";
  return "ok";
}

function statusText(entry: {
  status?: number;
  error?: string;
  phase?: string;
  sent?: number;
  received?: number;
}): string {
  if (entry.error) return "failed";
  if (entry.phase) {
    if (entry.phase === "close") return `closed ${entry.status ?? ""}`.trim();
    return entry.phase;
  }
  return entry.status === undefined ? "—" : String(entry.status);
}

/** The last path segment, with the origin kept as a dim prefix. */
function shortUrl(url: string): { host: string; rest: string } {
  const m = /^([a-z]+:\/\/[^/]+)(\/.*)?$/i.exec(url);
  if (!m) return { host: "", rest: url };
  return { host: m[1], rest: m[2] || "/" };
}

const totals = computed(() => {
  const fetches = state.net.filter((e) => e.kind === "fetch");
  const failed = state.net.filter((e) => e.error || e.phase === "error").length;
  const ms = fetches.reduce((a, e) => a + (e.ms || 0), 0);
  return { fetches: fetches.length, failed, avg: fetches.length ? ms / fetches.length : 0 };
});
</script>

<template>
  <section class="pane net" :style="{ height: props.height + 'px' }">
    <header class="pane-head">
      <span class="label">Network</span>
      <span class="chip">{{ state.net.length }} recorded</span>
      <span v-if="totals.failed" class="chip bad">{{ totals.failed }} failed</span>
      <span v-if="totals.fetches" class="chip">avg {{ totals.avg.toFixed(0) }} ms</span>
      <input
        v-model="state.netFilter"
        class="input filter"
        placeholder="filter by url"
      />
      <div class="spacer"></div>
      <div class="btn" @click="refreshNet()">↻</div>
      <div class="btn danger" @click="clearNet()">clear</div>
    </header>

    <div class="cols mono">
      <span class="c-kind">kind</span>
      <span class="c-method">method</span>
      <span class="c-url">url</span>
      <span class="c-status">status</span>
      <span class="c-time">time</span>
    </div>

    <div class="list" :style="{ height: listHeight + 'px' }" @wheel="list.onWheel($event)">
      <div class="viewport" :style="{ marginTop: list.padTop + 'px' }">
        <div v-for="entry in slice" :key="entry.seq" class="row mono">
          <span class="c-kind" :class="entry.kind">{{ entry.kind }}</span>
          <span class="c-method">{{ entry.method || "—" }}</span>
          <span class="c-url" :title="entry.url">
            <span class="host">{{ shortUrl(entry.url).host }}</span
            ><span class="rest">{{ shortUrl(entry.url).rest }}</span>
          </span>
          <span class="c-status" :class="statusClass(entry)">{{ statusText(entry) }}</span>
          <span class="c-time">{{ entry.ms === undefined ? "" : Math.round(entry.ms) + " ms" }}</span>
        </div>
      </div>
      <div
        v-if="list.thumb"
        class="vscroll"
        :style="{ top: list.thumb.top + 'px', height: list.thumb.height + 'px' }"
      ></div>
      <div v-if="!rows.length" class="empty">
        Nothing yet. Every <b>fetch()</b> and every <b>WebSocket</b> the app opens is recorded here
        from boot — including the ones it made before you opened this panel.<br />
        <span class="t-faint"
          >WebSocket messages are counted rather than logged: a chatty socket would evict every
          other entry in the ring within seconds.</span
        >
      </div>
    </div>
  </section>
</template>

<style scoped>
.net {
  flex: 1;
  min-width: 0;
}
.spacer {
  flex: 1;
}
.filter {
  width: 160px;
}
.cols {
  height: 22px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding-left: 10px;
  padding-right: 14px;
  font-size: 10px;
  color: #5c6b80;
  background: #0f141d;
  border-bottom-width: 1px;
  border-color: #1a2230;
}
.list {
  position: relative;
  overflow: hidden;
  min-height: 0;
}
.viewport {
  display: flex;
  flex-direction: column;
}
.row {
  height: 20px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
  padding-left: 10px;
  padding-right: 14px;
  font-size: 11px;
  color: #b9c6d6;
  border-bottom-width: 1px;
  border-color: #ffffff08;
  white-space: nowrap;
}
.row:hover {
  background: #ffffff0a;
}
.c-kind {
  width: 44px;
  flex-shrink: 0;
  color: #8494a8;
}
.c-kind.ws {
  color: #d2a8ff;
}
.c-kind.fetch {
  color: #76e3ea;
}
.c-method {
  width: 56px;
  flex-shrink: 0;
  color: #8494a8;
}
/* `white-space` is set HERE rather than inherited from `.row`: the engine
 * maps text properties onto gpui per element, and a flex item's inline
 * children do not pick the parent's `nowrap` up — a long URL wrapped to two
 * lines and overflowed a 20 px row. Stated because it is a real divergence
 * from a browser, not a style preference. */
.c-url {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.host,
.rest {
  white-space: nowrap;
}
.host {
  color: #5c6b80;
}
.rest {
  color: #d7e0ea;
}
.c-status {
  width: 74px;
  flex-shrink: 0;
  text-align: right;
  color: #8494a8;
}
.c-status.ok {
  color: #56d364;
}
.c-status.warn {
  color: #e3b341;
}
.c-status.bad {
  color: #ff7b72;
}
.c-time {
  width: 64px;
  flex-shrink: 0;
  text-align: right;
  color: #6d7c90;
}
</style>
