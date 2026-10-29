<script setup lang="ts">
// Network: what the app asked for, and what came back.
//
// The feed is a JS-side tee over `fetch`, `WebSocket` and (where the engine
// has one) `XMLHttpRequest` (`crates/vn-app/src/devtools/net.rs`) rather than a
// hook inside redjs. That is the whole reason this panel exists at all in one
// milestone: the engine's `fetch` runs on a worker thread and its sockets on
// their own threads, so a Rust-side hook would have been a fork patch and a
// second description of every request. The JS side already knows the URL, the
// method, the status and the timing.
//
// It records from BOOT, not from when you opened this panel, because the
// request you want to see is usually the one the app made while starting up.
// The ring is 300 entries (`net.rs` CAPACITY) for the same reason the log ring
// is bounded: an app that polls must not grow the process.
//
// m26: a row OPENS — click it and the details pane (`NetDetail.vue`) splits
// the panel, Chromium-style: headers, payload, response body, timing. The
// details are fetched on demand (`net_detail`), one command per click.
//
// Copying is explicit (the right-click menu, Ctrl+C for the selected row's URL)
// because painted text cannot be selected yet — see the INTERIM note in
// ../store.ts.
import { computed, onMounted, onUnmounted, watch } from "vue";
import { onKeydown } from "../keys";
import { NET_COLS_H, NET_H, PANE_HEAD_H, SPLIT, viewport } from "../layout";
import {
  clearNet,
  copyText,
  detailFor,
  flash,
  openMenu,
  refreshNet,
  selectNet,
  state,
  ui,
  visibleNet,
} from "../store";
import { asCurl, asFetch, bodyAsText, headersText, responseHeaderRows } from "../netfmt";
import { useVirtual } from "../virtual";
import NetDetail from "./NetDetail.vue";
import type { MenuItem, NetDetail as Detail, NetEntry } from "../protocol";

const props = defineProps<{ height: number }>();
const emit = defineEmits<{ (e: "drag", event: VnMouseEvent): void }>();

const open = computed(() => state.netSelected !== null);
const bodyH = computed(() => Math.max(60, props.height - PANE_HEAD_H));
const listHeight = computed(() => Math.max(40, bodyH.value - NET_COLS_H));
/** The list's width while the details are open; the details take the rest. */
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

/** `h2`, `http/1.1` — Chromium's Protocol column; empty when there was no connection. */
function protoText(entry: NetEntry): string {
  return entry.proto || "";
}

/** Chromium's Connection ID column: the id, with the first request on a connection marked. */
function connTitle(entry: NetEntry): string {
  if (entry.conn === undefined) return "no connection: not a network response, or it failed first";
  return `connection #${entry.conn} — ${entry.reused ? "reused: it had carried an earlier request" : "new: opened for this request"}`;
}

/** The pooled transport's counters, one chip. */
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

const totals = computed(() => {
  const fetches = state.net.filter((e) => e.kind === "fetch" || e.kind === "xhr");
  const failed = state.net.filter((e) => e.error || e.phase === "error").length;
  const ms = fetches.reduce((a, e) => a + (e.ms || 0), 0);
  return { fetches: fetches.length, failed, avg: fetches.length ? ms / fetches.length : 0 };
});

// ---- selection ----------------------------------------------------------------

function onRowClick(entry: NetEntry): void {
  void selectNet(entry.seq);
}

/** Keeps the open row on screen when the selection moves by keyboard. */
watch(
  () => state.netSelected,
  (seq) => {
    if (seq === null) return;
    const i = rows.value.findIndex((e) => e.seq === seq);
    if (i >= 0) list.ensureVisible(i);
  },
);

function step(delta: number): void {
  const all = rows.value;
  if (!all.length) return;
  const at = all.findIndex((e) => e.seq === state.netSelected);
  const next = at < 0 ? (delta > 0 ? 0 : all.length - 1) : Math.max(0, Math.min(all.length - 1, at + delta));
  void selectNet(all[next].seq);
}

// ---- copy -------------------------------------------------------------------

/** Runs `make` over the row's details — fetched for this, if they are not the open ones. */
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
  // The right button. There is no `contextmenu` event in this engine; see
  // ContextMenu.vue.
  if (event.button !== 2) return;
  ui.copyTarget = "network";
  openMenu(event.clientX, event.clientY, menuFor(entry));
}

// ---- the keyboard -----------------------------------------------------------

function onKey(event: VnKeyboardEvent): void {
  if (state.tab !== "network" || ui.typing || ui.menu) return;
  const ctrl = event.ctrlKey || event.metaKey;
  if (ctrl && event.key.toLowerCase() === "c" && !event.shiftKey) {
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
      <span class="chip">{{ state.net.length }} recorded</span>
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
            <div
              v-for="entry in slice"
              :key="entry.seq"
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
              <span v-if="!open" class="c-proto" :class="{ h2: entry.proto === 'h2' }">{{ protoText(entry) }}</span>
              <span v-if="!open" class="c-conn" :class="{ fresh: entry.conn !== undefined && !entry.reused }" :title="connTitle(entry)">{{
                entry.conn === undefined ? "" : "#" + entry.conn
              }}</span>
              <span class="c-status" :class="statusClass(entry)">{{ statusText(entry) }}</span>
              <span v-if="!open" class="c-time">{{ entry.ms === undefined ? "" : Math.round(entry.ms) + " ms" }}</span>
            </div>
          </div>
          <div
            v-if="list.thumb"
            class="vscroll"
            :style="{ top: list.thumb.top + 'px', height: list.thumb.height + 'px' }"
          ></div>
          <div v-if="!rows.length" class="empty">
            Nothing yet. Every <b>fetch()</b>, <b>XMLHttpRequest</b> and <b>WebSocket</b> the app opens is
            recorded here from boot — including the ones it made before you opened this panel. Click a row
            for its headers, payload, response body and timing; right-click to copy it as curl or fetch.<br />
            <span class="t-faint"
              >WebSocket messages are counted rather than logged: a chatty socket would evict every other entry
              in the ring within seconds. Bodies are kept for the most recent requests only (12 MB in all, 256
              KB each).</span
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
  background: #0a0e16;
  border-left-width: 1px;
  border-right-width: 1px;
  border-color: #1e2733;
  cursor: col-resize;
}
.split:hover,
.split.dragging {
  background: #2f6ea8;
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
.row.sel {
  background: #14344f;
}
.row.sel:hover {
  background: #17405f;
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
.c-kind.xhr {
  color: #e3b341;
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
/* …and the two runs are a flex ROW, not inline text: two inline spans that do
 * not fit break between them whatever `nowrap` says (the engine's inline
 * fallback wraps at span boundaries), which put the path of a long URL on a
 * second line once the details pane narrowed the list. In a row the origin
 * gives way first and the path keeps its ellipsis. */
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
  color: #5c6b80;
  flex-shrink: 50;
}
.rest {
  color: #d7e0ea;
  flex-shrink: 1;
}
.rest-only {
  color: #d7e0ea;
}
/* Chromium's Protocol and Connection ID columns, from the transport's own
 * account of the connection (redjs `fetch::net_info`, spliced onto the row by
 * the host). A new connection's id is drawn brighter than a reused one's, so
 * a burst of fresh dials — the thing worth seeing — stands out. */
.c-proto {
  width: 58px;
  flex-shrink: 0;
  color: #6d7c90;
}
.c-proto.h2 {
  color: #a5d6ff;
}
.c-conn {
  width: 40px;
  flex-shrink: 0;
  text-align: right;
  color: #5c6b80;
}
.c-conn.fresh {
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
