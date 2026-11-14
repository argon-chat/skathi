<script setup lang="ts">
// The JS side already knows the URL, the method, the status and the timing.
import { computed, onMounted, onUnmounted, watch } from "vue";
import { onKeydown } from "../keys";
import { NET_COLS_H, NET_H, PANE_HEAD_H, SPLIT, viewport } from "../layout";
import { clearNet, copyText, detailFor, flash, openMenu, refreshNet, selectNet, state, ui, visibleNet } from "../store";
import { asCurl, asFetch, bodyAsText, headersText, responseHeaderRows } from "../netfmt";
import { useVirtual } from "../virtual";
import NetDetail from "./NetDetail.vue";
import type { MenuItem, NetDetail as Detail, NetEntry } from "../protocol";

const props = defineProps<{ height: number }>();
const emit = defineEmits<{ (e: "drag", event: VnMouseEvent): void }>();

const open = computed(() => state.netSelected !== null);
const bodyH = computed(() => Math.max(60, props.height - PANE_HEAD_H));
const listHeight = computed(() => Math.max(40, bodyH.value - NET_COLS_H));
const listW = computed(() => Math.max(220, Math.min(viewport.w - 320, state.netListWidth)));
const detailW = computed(() => Math.max(300, viewport.w - listW.value - SPLIT));

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
  cache?: string;
}): string {
  if (entry.error) return "failed";
  if (entry.phase) {
    if (entry.phase === "close") return `closed ${entry.status ?? ""}`.trim();
    return entry.phase;
  }
  if (entry.cache === "revalidated") return "304";
  return entry.status === undefined ? "—" : String(entry.status);
}

function statusTitle(entry: NetEntry): string {
  if (entry.cache === "revalidated")
    return `304 Not Modified: the body came from the disk cache; the app saw ${entry.status ?? "the stored status"}`;
  if (entry.cache === "hit") return "from the disk cache: no request was made";
  return "";
}

function shortUrl(url: string): { host: string; rest: string } {
  const m = /^([a-z]+:\/\/[^/]+)(\/.*)?$/i.exec(url);
  if (!m) return { host: "", rest: url };
  return { host: m[1]!, rest: m[2] || "/" };
}

/** An answer from the HTTP cache had none, and says so the way Chromium's Size column does. */
function protoText(entry: NetEntry): string {
  if (entry.cache === "hit") return "(disk cache)";
  return entry.proto || "";
}

function connTitle(entry: NetEntry): string {
  if (entry.conn === undefined) return "no connection: not a network response, or it failed first";
  return `connection #${entry.conn} — ${entry.reused ? "reused: it had carried an earlier request" : "new: opened for this request"}`;
}

const pool = computed(() => {
  const p = state.netPool;
  if (!p || !p.requests) return null;
  const conns = [p.h2 ? `${p.h2} h2` : "", p.h1 ? `${p.h1} h1` : ""].filter(Boolean).join(" + ") || "0";
  return {
    text: `${conns} conn · ${p.reused}/${p.requests} reused`,
    retries: p.retries,
    title:
      `The target's connection pool since it started: ${p.h2} HTTP/2 and ${p.h1} HTTP/1.1 connections opened, ` +
      `${p.requests} requests put on them (a retry counts twice), ${p.reused} on a connection that had carried ` +
      `one before, ${p.retries} retried on a fresh connection.`,
  };
});

// The target's host puts it where it ran the app again in the same realm, and `cause` says why:
// `location.reload()`, or a dev host's cold restart.

function isMarker(entry: NetEntry): boolean {
  return entry.kind === "reload";
}

function clock(at: number): string {
  const d = new Date(at);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function markerText(entry: NetEntry): string {
  const at = typeof entry.at === "number" && entry.at > 0 ? `${clock(entry.at)}  ` : "";
  return `${at}${entry.cause || "reload"}: the app ran again, same JS realm (rows above: the previous run)`;
}

function markerTitle(entry: NetEntry): string {
  return (
    `The host ran the app again in the same JS realm (${entry.cause || "reload"}). The rows above are the ` +
    "previous run's. A request it still had in flight, or a socket the unload closed, can settle below."
  );
}

const recorded = computed(() => state.net.reduce((n, e) => (isMarker(e) ? n : n + 1), 0));

const totals = computed(() => {
  const fetches = state.net.filter((e) => e.kind === "fetch" || e.kind === "xhr");
  const failed = state.net.filter((e) => e.error || e.phase === "error").length;
  const ms = fetches.reduce((a, e) => a + (e.ms || 0), 0);
  return { fetches: fetches.length, failed, avg: fetches.length ? ms / fetches.length : 0 };
});

function onRowClick(entry: NetEntry): void {
  if (isMarker(entry)) return;
  void selectNet(entry.seq);
}

watch(
  () => state.netSelected,
  (seq) => {
    if (seq === null) return;
    const i = rows.value.findIndex((e) => e.seq === seq);
    if (i >= 0) list.ensureVisible(i);
  },
);

function step(delta: number): void {
  const all = rows.value.filter((e) => !isMarker(e));
  if (!all.length) return;
  const at = all.findIndex((e) => e.seq === state.netSelected);
  const next = at < 0 ? (delta > 0 ? 0 : all.length - 1) : Math.max(0, Math.min(all.length - 1, at + delta));
  void selectNet(all[next]!.seq);
}

function withDetail(seq: number, what: string, make: (d: Detail) => string | null): void {
  detailFor(seq).then(
    (d) => {
      const text = make(d);
      if (text === null) return;
      void copyText(text, what);
    },
    (e) => flash(`could not read the details: ${String((e && (e as Error).message) || e)}`, true),
  );
}

function copyBody(seq: number): void {
  withDetail(seq, "the response body", (d) => {
    const body = d.response?.body;
    if (!body || body.data === undefined || body.evicted) {
      flash(
        body?.evicted
          ? "the body was evicted to keep the devtools inside their memory budget"
          : body?.note
            ? `no body: ${body.note}`
            : "no body was captured for this request",
        true,
      );
      return null;
    }
    return bodyAsText(body);
  });
}

function menuFor(entry: NetEntry): MenuItem[] {
  if (isMarker(entry)) return [{ label: "Clear", run: () => clearNet() }];
  const ws = entry.kind === "ws";
  const items: MenuItem[] = [];
  if (state.netSelected !== entry.seq) items.push({ label: "Open details", run: () => void selectNet(entry.seq) });
  items.push({
    label: "Copy URL",
    hint: state.netSelected === entry.seq ? "Ctrl+C" : undefined,
    divider: items.length > 0,
    run: () => void copyText(entry.url, "the URL"),
  });
  if (!ws) {
    items.push({ label: "Copy as cURL (bash)", run: () => withDetail(entry.seq, "the request as curl", asCurl) });
    items.push({ label: "Copy as fetch", run: () => withDetail(entry.seq, "the request as fetch()", asFetch) });
    items.push({ label: "Copy response body", divider: true, run: () => copyBody(entry.seq) });
    items.push({
      label: "Copy response headers",
      run: () => withDetail(entry.seq, "the response headers", (d) => headersText(responseHeaderRows(d))),
    });
  }
  items.push({ label: "Clear", divider: true, run: () => clearNet() });
  return items;
}

function onRowDown(entry: NetEntry, event: VnMouseEvent): void {
  if (event.button !== 2) return;
  ui.copyTarget = "network";
  openMenu(event.clientX, event.clientY, menuFor(entry));
}

function onKey(event: VnKeyboardEvent): void {
  if (state.tab !== "network" || ui.typing || ui.menu) return;
  const ctrl = event.ctrlKey || event.metaKey;
  if (ctrl && event.key.toLowerCase() === "c" && !event.shiftKey) {
    if (globalThis.getSelection?.()?.toString()) return;
    if (ui.copyTarget !== "network" || state.netSelected === null) return;
    const entry = state.net.find((e) => e.seq === state.netSelected);
    if (!entry) return;
    event.preventDefault();
    void copyText(entry.url, "the URL");
    return;
  }
  if (ui.copyTarget !== "network" && state.netSelected === null) return;
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    step(event.key === "ArrowDown" ? 1 : -1);
  }
}

let offKey: (() => void) | null = null;
onMounted(() => {
  offKey = onKeydown(onKey);
});
onUnmounted(() => {
  offKey?.();
  offKey = null;
});
</script>

<template>
  <section class="pane net" :style="{ height: props.height + 'px' }">
    <header class="pane-head">
      <span class="label">Network</span>
      <span class="chip">{{ recorded }} recorded</span>
      <span v-if="totals.failed" class="chip bad">{{ totals.failed }} failed</span>
      <span v-if="totals.fetches && !open" class="chip">avg {{ totals.avg.toFixed(0) }} ms</span>
      <span v-if="pool" class="chip" :title="pool.title">{{ pool.text }}</span>
      <span v-if="pool && pool.retries" class="chip warn" :title="pool.title">{{ pool.retries }} retried</span>
      <input
        v-model="state.netFilter"
        class="input filter"
        placeholder="filter by url"
        @focus="ui.typing = true"
        @blur="ui.typing = false"
      />
      <div class="spacer"></div>
      <div class="btn" title="Ask the target for anything the push feed missed" @click="refreshNet()">↻</div>
      <div class="btn danger" @click="clearNet()">clear</div>
    </header>

    <div class="split-body" :style="{ height: bodyH + 'px' }">
      <div class="left" :class="{ narrow: open }" :style="open ? { width: listW + 'px' } : {}">
        <div class="cols mono">
          <span class="c-kind">kind</span>
          <span v-if="!open" class="c-method">method</span>
          <span class="c-url">url</span>
          <span v-if="!open" class="c-proto">protocol</span>
          <span v-if="!open" class="c-conn">conn</span>
          <span class="c-status">status</span>
          <span v-if="!open" class="c-time">time</span>
        </div>

        <div class="list" :style="{ height: listHeight + 'px' }" @wheel="list.onWheel($event)">
          <div class="viewport" :style="{ marginTop: list.padTop + 'px' }">
            <template v-for="entry in slice" :key="entry.seq">
              <div
                v-if="isMarker(entry)"
                class="row mono marker"
                :title="markerTitle(entry)"
                @mousedown="onRowDown(entry, $event)"
              >
                <span class="c-kind reload">reload</span>
                <span class="c-url mark-text">{{ markerText(entry) }}</span>
              </div>
              <div
                v-else
                class="row mono"
                :class="{ sel: entry.seq === state.netSelected }"
                @click="onRowClick(entry)"
                @mousedown="onRowDown(entry, $event)"
              >
                <span class="c-kind" :class="entry.kind">{{ entry.kind }}</span>
                <span v-if="!open" class="c-method">{{ entry.method || "—" }}</span>
                <span v-if="shortUrl(entry.url).host" class="c-url split-url" :title="entry.url">
                  <span class="host">{{ shortUrl(entry.url).host }}</span
                  ><span class="rest">{{ shortUrl(entry.url).rest }}</span>
                </span>
                <span v-else class="c-url rest-only" :title="entry.url">{{ entry.url }}</span>
                <span
                  v-if="!open"
                  class="c-proto"
                  :class="{ h2: entry.proto === 'h2', disk: entry.cache === 'hit' }"
                  :title="statusTitle(entry)"
                  >{{ protoText(entry) }}</span
                >
                <span
                  v-if="!open"
                  class="c-conn"
                  :class="{ fresh: entry.conn !== undefined && !entry.reused }"
                  :title="connTitle(entry)"
                  >{{ entry.conn === undefined ? "" : "#" + entry.conn }}</span
                >
                <span class="c-status" :class="statusClass(entry)" :title="statusTitle(entry)">{{
                  statusText(entry)
                }}</span>
                <span v-if="!open" class="c-time">{{
                  entry.ms === undefined ? "" : Math.round(entry.ms) + " ms"
                }}</span>
              </div>
            </template>
          </div>
          <div
            v-if="list.thumb"
            class="vscroll"
            :style="{ top: list.thumb.top + 'px', height: list.thumb.height + 'px' }"
          ></div>
          <div v-if="!rows.length" class="empty">
            Nothing yet. Every <b>fetch()</b>, <b>XMLHttpRequest</b> and <b>WebSocket</b> the app opens is recorded here
            from boot — including the ones it made before you opened this panel. Click a row for its headers, payload,
            response body and timing; right-click to copy it as curl or fetch.<br />
            <span class="t-faint"
              >WebSocket messages are counted rather than logged: a chatty socket would evict every other entry in the
              ring within seconds. Bodies are kept for the most recent requests only (12 MB in all, 256 KB each).</span
            >
          </div>
        </div>
      </div>

      <template v-if="open">
        <div class="split" :class="{ dragging: state.dragging === 'net' }" @mousedown="emit('drag', $event)"></div>
        <NetDetail :width="detailW" :height="bodyH" />
      </template>
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
.split-body {
  display: flex;
  flex-direction: row;
  min-height: 0;
}
.left {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.left.narrow {
  flex: none;
  flex-shrink: 0;
}
.split {
  width: 5px;
  flex-shrink: 0;
  background: var(--surface-base);
  border-left-width: 1px;
  border-right-width: 1px;
  border-color: var(--line);
  cursor: col-resize;
}
.split:hover,
.split.dragging {
  background: var(--surface-handle);
}
.cols {
  height: var(--h-head);
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-4);
  padding-left: var(--sp-4);
  padding-right: var(--sp-5);
  font-size: var(--fs-xs);
  color: var(--text-faint);
  background: var(--surface-panel);
  border-bottom-width: 1px;
  border-color: var(--line-subtle);
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
  height: var(--h-row);
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-4);
  padding-left: var(--sp-4);
  padding-right: var(--sp-5);
  font-size: var(--fs-s);
  color: var(--text);
  border-bottom-width: 1px;
  border-color: var(--line-subtle);
  white-space: nowrap;
}
.row:hover {
  background: var(--surface-hover);
}
.row.sel {
  background: var(--surface-active);
}
.row.sel:hover {
  background: var(--surface-active);
}
.c-kind {
  width: 44px;
  flex-shrink: 0;
  color: var(--text-muted);
}
.c-kind.ws {
  color: var(--info);
}
.c-kind.fetch {
  color: var(--cyan);
}
.c-kind.xhr {
  color: var(--warn);
}
.row.marker {
  background: var(--info-soft);
  border-color: var(--info-line);
  color: var(--info);
}
.row.marker:hover {
  background: var(--info-soft);
}
.c-kind.reload {
  color: var(--info);
}
.mark-text {
  color: var(--info);
}
.c-method {
  width: 56px;
  flex-shrink: 0;
  color: var(--text-muted);
}
.c-url {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.c-url.split-url {
  display: flex;
  flex-direction: row;
  flex-wrap: nowrap;
}
.host,
.rest {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  min-width: 0;
}
.host {
  color: var(--text-faint);
  flex-shrink: 50;
}
.rest {
  color: var(--text);
  flex-shrink: 1;
}
.rest-only {
  color: var(--text);
}
.c-proto {
  width: 84px;
  flex-shrink: 0;
  color: var(--text-faint);
}
.c-proto.h2 {
  color: var(--text);
}
.c-proto.disk {
  color: var(--text-muted);
  font-style: italic;
}
.c-conn {
  width: 40px;
  flex-shrink: 0;
  text-align: right;
  color: var(--text-faint);
}
.c-conn.fresh {
  color: var(--text);
}
.c-status {
  width: 74px;
  flex-shrink: 0;
  text-align: right;
  color: var(--text-muted);
}
.c-status.ok {
  color: var(--ok);
}
.c-status.warn {
  color: var(--warn);
}
.c-status.bad {
  color: var(--bad);
}
.c-time {
  width: 64px;
  flex-shrink: 0;
  text-align: right;
  color: var(--text-faint);
}
</style>
