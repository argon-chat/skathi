<script setup lang="ts">
// Chromium's shape — Headers / Payload / Response / Timing for a request, Socket / Timing for a
// WebSocket — over what `net_detail` returns (crates/vn-app/src/devtools/net.rs, `Ring::detail`).
import { computed, reactive, watch } from "vue";
import { BODY_BAR_H, DETAIL_TABS_H, MONO_W } from "../layout";
import { closeNetDetail, copyText, flash, loadNetDetail, state } from "../store";
import {
  asCurl,
  asFetch,
  clock,
  formatBytes,
  formatMs,
  headersText,
  maskHeader,
  parseForm,
  queryOf,
  requestHeaderRows,
  responseHeaderRows,
  textLines,
  wrapWords,
} from "../netfmt";
import BodyView from "./BodyView.vue";
import type { NetBody, NetDetail, NetTab } from "../protocol";

const props = defineProps<{ width: number; height: number }>();

interface KV {
  name: string;
  value: string;
  sensitive?: boolean;
  note?: string | undefined;
  faint?: boolean;
  tone?: "ok" | "warn" | "bad" | undefined;
}

interface Section {
  id: string;
  title: string;
  rows: KV[];
  copy?: () => string;
  empty?: string;
}

const d = computed<NetDetail | null>(() => state.netDetail);
const entry = computed(() => d.value?.entry || state.net.find((e) => e.seq === state.netSelected) || null);
const isWs = computed(() => entry.value?.kind === "ws");
const recorded = computed(() => !!d.value && d.value.detail !== null);

const tabs = computed<{ id: NetTab; label: string }[]>(() =>
  isWs.value
    ? [
        { id: "socket", label: "Socket" },
        { id: "timing", label: "Timing" },
      ]
    : [
        { id: "headers", label: "Headers" },
        { id: "payload", label: "Payload" },
        { id: "response", label: "Response" },
        { id: "timing", label: "Timing" },
      ],
);

const contentH = computed(() => Math.max(60, props.height - DETAIL_TABS_H));

const revealed = reactive<Record<string, boolean>>({});
const collapsed = reactive<Record<string, boolean>>({});
watch(
  () => state.netSelected,
  () => {
    for (const k of Object.keys(revealed)) delete revealed[k];
  },
);

const nameW = computed(() => Math.round(Math.max(110, Math.min(210, props.width * 0.3))));
const EYE_W = 38;
const valueCols = computed(() =>
  Math.max(16, Math.floor((props.width - nameW.value - EYE_W - 10 - 14 - 20 - 8) / MONO_W)),
);
const nameCols = computed(() => Math.max(8, Math.floor(nameW.value / MONO_W)));

function shown(kv: KV, key: string): string {
  const v = kv.sensitive && !revealed[key] ? maskHeader(kv.name, kv.value) : kv.value;
  return wrapWords(v, valueCols.value, 400).join("\n");
}
function nameOf(kv: KV): string {
  return textLines(kv.name, nameCols.value, 6).join("\n");
}
function reveal(kv: KV, key: string): void {
  if (kv.sensitive) revealed[key] = !revealed[key];
}

const CACHE_LABEL: Record<string, string> = {
  hit: "(disk cache) — served from disk, no request was made",
  revalidated: "revalidated — the server answered 304; the body came from disk",
  stored: "stored — from the network, and kept",
  miss: "not stored — from the network",
};

/** Why a connection fact is missing. */
const noTransport = computed(() => {
  const e = entry.value;
  if (!e) return "—";
  if (/^(data|blob|file):/i.test(e.url)) return "none — this response never crossed a network";
  if (e.error || d.value?.error) return "none — the request failed before a response";
  if (e.kind === "xhr") return "not reported for XMLHttpRequest";
  if (e.kind === "fetch" && d.value?.response?.status === undefined) return "pending";
  return "not reported by this host";
});

function protocolLabel(p: string): string {
  return p === "h2" ? "h2 (HTTP/2)" : p;
}

function statusTone(status: number | undefined): KV["tone"] {
  if (status === undefined) return undefined;
  if (status >= 500 || status === 0) return "bad";
  if (status >= 400) return "warn";
  return "ok";
}

const general = computed<KV[]>(() => {
  const e = entry.value;
  if (!e) return [];
  const det = d.value;
  const res = det?.response;
  const req = det?.request;
  const rows: KV[] = [
    { name: "Request URL", value: e.url },
    { name: "Request Method", value: (e.method || det?.method || "GET").toUpperCase() },
  ];
  if (det?.error || e.error) {
    const err = det?.error;
    rows.push({
      name: "Status",
      value: `failed — ${err?.name ? `${err.name}: ` : ""}${err?.message || e.error || "no response"}`,
      tone: "bad",
    });
  } else if (res?.status !== undefined || e.status !== undefined) {
    const status = res?.status ?? e.status;
    rows.push({
      name: "Status Code",
      value: `${status}${res?.statusText ? ` ${res.statusText}` : ""}`,
      tone: statusTone(status),
    });
  } else {
    rows.push({ name: "Status", value: "pending", faint: true });
  }
  if (res?.url && res.url !== e.url)
    rows.push({ name: "Final URL", value: res.url, note: res.redirected ? "after a redirect" : undefined });
  else if (res?.redirected) rows.push({ name: "Redirected", value: "yes" });
  if (res?.type) rows.push({ name: "Response Type", value: res.type });
  if (res?.responseType !== undefined) rows.push({ name: "responseType", value: res.responseType || '"" (text)' });
  const t = det?.transport || {};
  const missing = noTransport.value;
  rows.push({ name: "Remote Address", value: t.remote || missing, faint: !t.remote });
  rows.push({ name: "Protocol", value: t.protocol ? protocolLabel(t.protocol) : missing, faint: !t.protocol });
  rows.push({
    name: "Connection",
    value:
      t.connectionId !== undefined
        ? `#${t.connectionId} · ${t.reused ? "reused — it had carried an earlier request" : "new"}`
        : t.reused !== undefined
          ? t.reused
            ? "reused"
            : "new"
          : missing,
    faint: t.connectionId === undefined && t.reused === undefined,
  });
  if (t.cache && t.cache !== "off") rows.push({ name: "HTTP Cache", value: CACHE_LABEL[t.cache] ?? t.cache });
  if (req) {
    const bits = [
      req.credentials && `credentials ${req.credentials}`,
      req.mode && `mode ${req.mode}`,
      req.redirect && `redirect ${req.redirect}`,
      req.cache && `cache ${req.cache}`,
    ].filter(Boolean);
    if (bits.length) rows.push({ name: "Request Options", value: bits.join(" · ") });
  }
  rows.push({
    name: "Initiator",
    value: e.kind === "xhr" ? "XMLHttpRequest" : e.kind === "ws" ? "WebSocket" : "fetch()",
  });
  return rows;
});

const resRows = computed<KV[]>(() => (d.value ? responseHeaderRows(d.value) : []));
const reqRows = computed<KV[]>(() => (d.value ? requestHeaderRows(d.value) : []));

const headerSections = computed<Section[]>(() => {
  const det = d.value;
  const res = det?.response;
  const out: Section[] = [
    {
      id: "general",
      title: "General",
      rows: general.value,
      copy: () => general.value.map((r) => `${r.name}: ${r.value}`).join("\n"),
    },
  ];
  if (!recorded.value) return out;
  out.push({
    id: "res",
    title: "Response Headers",
    rows: resRows.value,
    copy: () => headersText(resRows.value),
    empty: det?.error
      ? "No response — the request failed."
      : res?.status === undefined
        ? "Waiting for the response…"
        : "None.",
  });
  out.push({
    id: "req",
    title: "Request Headers",
    rows: reqRows.value,
    copy: () => headersText(reqRows.value),
    empty: "The app set none.",
  });
  return out;
});

/** The fine print under the Headers tab — what this pane cannot see, and why. */
const headerNotes = computed<string[]>(() => {
  const det = d.value;
  const out: string[] = [];
  if (!det) return out;
  if (det.detail === null && det.note) out.push(det.note);
  if (!recorded.value) return out;
  out.push(
    "Request headers are the ones the app passed. The transport adds its own below script " +
      "(Host, User-Agent, Accept-Encoding, Content-Length, …); script never sees those, so they are not listed.",
  );
  const cred = det.request?.credentials;
  if (cred && cred !== "include" && !det.cookies?.sent) {
    out.push(
      `No Cookie header, and no Set-Cookie stored: credentials "${cred}". The app's origin (app://redjs) is ` +
        `opaque, so no URL is same-origin with it — cookies are sent and stored only with credentials: ` +
        `"include" (withCredentials for XHR).`,
    );
  }
  if (det.cookies?.note) out.push(`Cookies — ${det.cookies.note}`);
  if (det.metaTruncated) {
    out.push("Some of this record was dropped: it reached its 16 KB cap for headers, fields and timing.");
  }
  return out;
});

const query = computed<KV[]>(() =>
  entry.value ? queryOf(entry.value.url).map(([name, value]) => ({ name, value })) : [],
);

const formFields = computed<KV[]>(() => {
  const info = d.value?.request?.bodyInfo;
  const body = d.value?.request?.body;
  if (info?.kind === "formdata") {
    return (info.fields || []).map((f) =>
      f.file
        ? {
            name: f.name,
            value: `file "${f.file.name}" · ${formatBytes(f.file.size)} · ${f.file.type || "no type"}`,
            note: "the bytes are not captured",
            faint: true,
          }
        : { name: f.name, value: f.value ?? "" },
    );
  }
  const urlencoded = info?.kind === "urlencoded" || /x-www-form-urlencoded/i.test(body?.mime || "");
  if (urlencoded && body && body.encoding === "utf8" && body.data !== undefined) {
    return parseForm(body.data).map(([name, value]) => ({ name, value }));
  }
  return [];
});

const reqBody = computed(() => d.value?.request?.body);
const showReqBody = computed(() => !!reqBody.value && d.value?.request?.bodyInfo?.kind !== "formdata");

const payloadSections = computed<Section[]>(() => {
  const out: Section[] = [];
  if (query.value.length) {
    out.push({
      id: "query",
      title: "Query String Parameters",
      rows: query.value,
      copy: () => query.value.map((r) => `${r.name}=${r.value}`).join("\n"),
    });
  }
  if (formFields.value.length) {
    const multipart = d.value?.request?.bodyInfo?.kind === "formdata";
    out.push({
      id: "form",
      title: multipart ? "Form Data (multipart)" : "Form Data",
      rows: formFields.value,
      copy: () => formFields.value.map((r) => `${r.name}: ${r.value}`).join("\n"),
    });
  }
  return out;
});

const SEC_HEAD_H = 24;
const KV_H = 18;
const payloadTablesH = computed(() => {
  if (!showReqBody.value) return contentH.value;
  let need = 0;
  for (const s of payloadSections.value) need += SEC_HEAD_H + (collapsed[s.id] ? 0 : s.rows.length * KV_H) + 4;
  return Math.min(need, Math.floor(contentH.value * 0.45));
});
const payloadBodyH = computed(() => Math.max(BODY_BAR_H + 16, contentH.value - payloadTablesH.value - SEC_HEAD_H));

const noPayload = computed(() => recorded.value && !payloadSections.value.length && !reqBody.value);

const errorText = computed(() => {
  const det = d.value;
  const err = det?.error;
  if (err) return `${err.name ? `${err.name}: ` : ""}${err.message || "failed"}`;
  return entry.value?.error || "";
});

/**
 * `null` from the wire means "not handed over yet" for a response (the tee always reports one,
 * with data or with a reason) — unless the request failed, in which case there is simply none.
 */
const resBody = computed<NetBody | null | undefined>(() => {
  const b = d.value?.response?.body;
  if (b === null) return errorText.value ? null : undefined;
  return b;
});
const waitingForResponse = computed(
  () => recorded.value && !errorText.value && d.value?.response?.status === undefined,
);

// timing

const timing = computed<KV[]>(() => {
  const e = entry.value;
  const det = d.value;
  if (!e) return [];
  const t = det?.timing || {};
  const at = det?.at ?? e.at;
  const rows: KV[] = [{ name: "Started", value: clock(at) }];
  if (isWs.value) {
    const w = det?.ws || {};
    if (w.openMs !== undefined) rows.push({ name: "Open after", value: formatMs(w.openMs) });
    if (w.errorMs !== undefined) rows.push({ name: "Error after", value: formatMs(w.errorMs), tone: "bad" });
    if (w.closeMs !== undefined) rows.push({ name: "Closed after", value: formatMs(w.closeMs) });
    if (w.lastAt) rows.push({ name: "Last message", value: clock(w.lastAt) });
    return rows;
  }
  if (t.headersMs !== undefined)
    rows.push({ name: "Response", value: formatMs(t.headersMs), note: "the promise settled" });
  if (t.bodyMs !== undefined)
    rows.push({ name: "Body read", value: formatMs(t.bodyMs), note: "the tee finished reading its clone" });
  if (t.failedMs !== undefined) rows.push({ name: "Failed after", value: formatMs(t.failedMs), tone: "bad" });
  if (e.ms !== undefined) rows.push({ name: "Total", value: formatMs(e.ms) });
  if (errorText.value) rows.push({ name: "Error", value: errorText.value, tone: "bad" });
  return rows;
});

const timingBar = computed(() => {
  const t = d.value?.timing;
  if (!t || isWs.value) return null;
  const head = t.headersMs ?? t.failedMs;
  if (head === undefined) return null;
  const end = Math.max(head, t.bodyMs ?? head, 1);
  return {
    wait: (head / end) * 100,
    read: t.bodyMs !== undefined ? ((t.bodyMs - head) / end) * 100 : 0,
    failed: t.failedMs !== undefined,
  };
});

const CLOSE_CODES: Record<number, string> = {
  1000: "normal closure",
  1001: "going away",
  1002: "protocol error",
  1003: "unsupported data",
  1005: "no status received",
  1006: "abnormal closure (no close frame)",
  1007: "invalid payload",
  1008: "policy violation",
  1009: "message too big",
  1010: "mandatory extension",
  1011: "internal error",
  1015: "TLS handshake failed",
};

const socketSections = computed<Section[]>(() => {
  const e = entry.value;
  if (!e) return [];
  const w = d.value?.ws || {};
  const conn: KV[] = [{ name: "URL", value: e.url }];
  conn.push({
    name: "Requested",
    value: w.protocols && w.protocols.length ? w.protocols.join(", ") : "no subprotocol",
    faint: !w.protocols?.length,
  });
  if (w.openMs !== undefined) {
    conn.push({ name: "Protocol", value: w.protocol || "none negotiated", faint: !w.protocol });
    conn.push({ name: "Extensions", value: w.extensions || "none", faint: !w.extensions });
    if (w.binaryType) conn.push({ name: "binaryType", value: w.binaryType });
  }
  const phase =
    w.closeCode !== undefined ? "closed" : w.error ? "error" : w.openMs !== undefined ? "open" : "connecting";
  conn.push({ name: "State", value: phase, tone: phase === "open" ? "ok" : phase === "error" ? "bad" : undefined });
  if (w.closeCode !== undefined) {
    conn.push({
      name: "Close Code",
      value: `${w.closeCode}${CLOSE_CODES[w.closeCode] ? ` — ${CLOSE_CODES[w.closeCode]}` : ""}`,
      tone: w.closeCode === 1000 ? undefined : "warn",
    });
    conn.push({ name: "Reason", value: w.closeReason || "none given", faint: !w.closeReason });
    conn.push({ name: "Clean", value: w.wasClean ? "yes" : "no" });
  }
  const out: Section[] = [
    { id: "ws-conn", title: "Connection", rows: conn, copy: () => conn.map((r) => `${r.name}: ${r.value}`).join("\n") },
  ];
  if (recorded.value) {
    const msgs: KV[] = [
      { name: "Sent", value: `${w.sent ?? e.sent ?? 0} messages · ${formatBytes(w.sentBytes ?? 0)}` },
      { name: "Received", value: `${w.received ?? e.received ?? 0} messages · ${formatBytes(w.receivedBytes ?? 0)}` },
    ];
    out.push({ id: "ws-msgs", title: "Messages", rows: msgs });
    const cookies: KV[] = [];
    if (d.value?.cookies?.sent) {
      cookies.push({ name: "cookie", value: d.value.cookies.sent, sensitive: true, note: "from the cookie jar" });
    }
    for (const line of d.value?.cookies?.set || []) {
      cookies.push({ name: "set-cookie", value: line, sensitive: true, note: "reconstructed from the jar" });
    }
    out.push({ id: "ws-cookies", title: "Handshake Cookies", rows: cookies, empty: "None in the jar for this URL." });
  }
  return out;
});

const socketNotes = computed<string[]>(() => {
  const out = [
    "Messages are counted, not logged: a chatty socket would evict every other entry in the 300-row ring within seconds.",
  ];
  if (d.value?.detail === null && d.value.note) out.unshift(d.value.note);
  return out;
});

const limitsText = computed(() => {
  const l = d.value?.limits;
  if (!l) return "";
  return (
    `Bodies: ${formatBytes(l.used)} of ${formatBytes(l.budget)} held, oldest evicted first ` +
    `(${l.evicted} so far) · each body capped at ${formatBytes(l.bodyCap)}`
  );
});

function copyUrl(): void {
  if (entry.value) void copyText(entry.value.url, "the URL");
}
function copyCurl(): void {
  if (!d.value) return flash("the details are not loaded yet", true);
  void copyText(asCurl(d.value), "the request as curl");
}
function copyFetch(): void {
  if (!d.value) return flash("the details are not loaded yet", true);
  void copyText(asFetch(d.value), "the request as fetch()");
}
function copySection(s: Section): void {
  if (s.copy) void copyText(s.copy(), s.title.toLowerCase());
}
function reload(): void {
  if (state.netSelected !== null) void loadNetDetail(state.netSelected);
}
</script>

<template>
  <section class="detail" :style="{ width: props.width + 'px', height: props.height + 'px' }">
    <div class="dtabs">
      <div class="x" title="Close  ·  Esc" @click="closeNetDetail()">✕</div>
      <div
        v-for="t in tabs"
        :key="t.id"
        class="dtab"
        :class="{ on: state.netTab === t.id }"
        @click="state.netTab = t.id"
      >
        {{ t.label }}
      </div>
      <div class="spacer"></div>
      <span class="copy-label">⧉</span>
      <div class="mini" title="Copy the URL" @click="copyUrl()">URL</div>
      <div v-if="!isWs" class="mini" title="Copy as cURL (bash)" @click="copyCurl()">curl</div>
      <div v-if="!isWs" class="mini" title="Copy as fetch()" @click="copyFetch()">fetch</div>
      <div class="mini" :class="{ busy: state.netDetailBusy }" title="Ask the target again" @click="reload()">↻</div>
    </div>

    <div v-if="state.netDetailError" class="empty bad-text">{{ state.netDetailError }}</div>
    <div v-else-if="!d" class="empty">Asking the target…</div>

    <div v-else-if="state.netTab === 'headers'" class="scroll" :style="{ height: contentH + 'px' }">
      <div v-for="s in headerSections" :key="s.id" class="sec">
        <div class="sec-head" @click="collapsed[s.id] = !collapsed[s.id]">
          <span class="tri">{{ collapsed[s.id] ? "▸" : "▾" }}</span>
          <span class="sec-title">{{ s.title }}</span>
          <span v-if="s.id !== 'general'" class="n">{{ s.rows.length }}</span>
          <div class="spacer"></div>
          <div
            v-if="s.copy && s.rows.length"
            class="mini"
            :title="'Copy ' + s.title.toLowerCase()"
            @click.stop="copySection(s)"
          >
            ⧉
          </div>
        </div>
        <template v-if="!collapsed[s.id]">
          <div v-for="(kv, i) in s.rows" :key="i" class="kv">
            <span class="k mono" :style="{ width: nameW + 'px' }">{{ nameOf(kv) }}</span>
            <div class="v" @click="reveal(kv, s.id + ':' + i)">
              <span
                class="vt mono"
                :class="[kv.tone, { faint: kv.faint, masked: kv.sensitive && !revealed[s.id + ':' + i] }]"
                >{{ shown(kv, s.id + ":" + i) }}</span
              >
              <span v-if="kv.note" class="vn">{{ kv.note }}</span>
            </div>
            <span v-if="kv.sensitive" class="eye" @click="reveal(kv, s.id + ':' + i)">{{
              revealed[s.id + ":" + i] ? "hide" : "show"
            }}</span>
            <span v-else class="eye-gap"></span>
          </div>
          <div v-if="!s.rows.length && s.empty" class="kv-empty">{{ s.empty }}</div>
        </template>
      </div>
      <div class="notes">
        <div v-for="(n, i) in headerNotes" :key="i" class="note-line">{{ n }}</div>
        <div v-if="limitsText" class="note-line">{{ limitsText }}</div>
      </div>
    </div>

    <div v-else-if="state.netTab === 'payload'" class="col" :style="{ height: contentH + 'px' }">
      <div v-if="!recorded" class="empty">{{ d.note || "No details were recorded for this request." }}</div>
      <div v-else-if="noPayload" class="empty">No payload — no query string and no request body.</div>
      <template v-else>
        <div v-if="payloadSections.length" class="scroll" :style="{ height: payloadTablesH + 'px' }">
          <div v-for="s in payloadSections" :key="s.id" class="sec">
            <div class="sec-head" @click="collapsed[s.id] = !collapsed[s.id]">
              <span class="tri">{{ collapsed[s.id] ? "▸" : "▾" }}</span>
              <span class="sec-title">{{ s.title }}</span>
              <span class="n">{{ s.rows.length }}</span>
              <div class="spacer"></div>
              <div v-if="s.copy" class="mini" :title="'Copy ' + s.title.toLowerCase()" @click.stop="copySection(s)">
                ⧉
              </div>
            </div>
            <template v-if="!collapsed[s.id]">
              <div v-for="(kv, i) in s.rows" :key="i" class="kv">
                <span class="k mono" :style="{ width: nameW + 'px' }">{{ nameOf(kv) }}</span>
                <div class="v">
                  <span class="vt mono" :class="{ faint: kv.faint }">{{ shown(kv, s.id + ":" + i) }}</span>
                  <span v-if="kv.note" class="vn">{{ kv.note }}</span>
                </div>
                <span class="eye-gap"></span>
              </div>
            </template>
          </div>
        </div>
        <template v-if="showReqBody">
          <div class="sec-head static">
            <span class="sec-title">Request Body</span>
            <span v-if="d.request?.bodyInfo?.kind" class="n">{{ d.request?.bodyInfo?.kind }}</span>
          </div>
          <BodyView
            :body="reqBody"
            :width="props.width"
            :height="payloadBodyH"
            :budget="d.limits?.budget || 0"
            what="the request body"
          />
        </template>
        <div v-else-if="d.request?.bodyInfo?.kind === 'formdata'" class="note-line pad">
          multipart/form-data: the engine encodes the parts itself, so the fields are shown instead of the bytes.
        </div>
      </template>
    </div>

    <div v-else-if="state.netTab === 'response'" class="col" :style="{ height: contentH + 'px' }">
      <div v-if="!recorded" class="empty">{{ d.note || "No details were recorded for this request." }}</div>
      <div v-else-if="waitingForResponse" class="empty">Waiting for the response…</div>
      <template v-else>
        <div v-if="errorText" class="errbar mono">✕ {{ errorText }}</div>
        <BodyView
          :body="resBody"
          :width="props.width"
          :height="contentH - (errorText ? 22 : 0)"
          :budget="d.limits?.budget || 0"
          what="the response body"
        />
      </template>
    </div>

    <!-- Timing -->
    <div v-else-if="state.netTab === 'timing'" class="scroll" :style="{ height: contentH + 'px' }">
      <div v-if="timingBar" class="bar-wrap">
        <div class="tbar">
          <div class="seg-wait" :class="{ failed: timingBar.failed }" :style="{ width: timingBar.wait + '%' }"></div>
          <div class="seg-read" :style="{ width: timingBar.read + '%' }"></div>
        </div>
        <div class="legend">
          <span class="sw wait" :class="{ failed: timingBar.failed }"></span
          ><span>{{ timingBar.failed ? "until it failed" : "until the response" }}</span>
          <template v-if="timingBar.read > 0">
            <span class="sw read"></span><span>the tee reading its clone</span>
          </template>
        </div>
      </div>
      <div class="sec">
        <div v-for="(kv, i) in timing" :key="i" class="kv">
          <span class="k mono" :style="{ width: nameW + 'px' }">{{ nameOf(kv) }}</span>
          <div class="v">
            <span class="vt mono" :class="[kv.tone, { faint: kv.faint }]">{{ shown(kv, "t:" + i) }}</span>
            <span v-if="kv.note" class="vn">{{ kv.note }}</span>
          </div>
          <span class="eye-gap"></span>
        </div>
      </div>
      <div class="notes">
        <div class="note-line">
          Times are measured by the tee in the app's own thread, from the call to fetch(). The transport does not report
          a DNS / connect / TLS breakdown, and in this engine a response settles once its body has arrived — so
          "Response" includes the download.
        </div>
      </div>
    </div>

    <div v-else-if="state.netTab === 'socket'" class="scroll" :style="{ height: contentH + 'px' }">
      <div v-for="s in socketSections" :key="s.id" class="sec">
        <div class="sec-head" @click="collapsed[s.id] = !collapsed[s.id]">
          <span class="tri">{{ collapsed[s.id] ? "▸" : "▾" }}</span>
          <span class="sec-title">{{ s.title }}</span>
          <div class="spacer"></div>
          <div v-if="s.copy" class="mini" :title="'Copy ' + s.title.toLowerCase()" @click.stop="copySection(s)">⧉</div>
        </div>
        <template v-if="!collapsed[s.id]">
          <div v-for="(kv, i) in s.rows" :key="i" class="kv">
            <span class="k mono" :style="{ width: nameW + 'px' }">{{ nameOf(kv) }}</span>
            <div class="v" @click="reveal(kv, s.id + ':' + i)">
              <span
                class="vt mono"
                :class="[kv.tone, { faint: kv.faint, masked: kv.sensitive && !revealed[s.id + ':' + i] }]"
                >{{ shown(kv, s.id + ":" + i) }}</span
              >
              <span v-if="kv.note" class="vn">{{ kv.note }}</span>
            </div>
            <span v-if="kv.sensitive" class="eye" @click="reveal(kv, s.id + ':' + i)">{{
              revealed[s.id + ":" + i] ? "hide" : "show"
            }}</span>
            <span v-else class="eye-gap"></span>
          </div>
          <div v-if="!s.rows.length && s.empty" class="kv-empty">{{ s.empty }}</div>
        </template>
      </div>
      <div class="notes">
        <div v-for="(n, i) in socketNotes" :key="i" class="note-line">{{ n }}</div>
      </div>
    </div>
  </section>
</template>

<style scoped>
.detail {
  display: flex;
  flex-direction: column;
  min-width: 0;
  background: var(--surface-panel);
}
.dtabs {
  height: var(--h-bar);
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-1);
  padding-left: var(--sp-2);
  padding-right: var(--sp-3);
  background: var(--surface-panel);
  border-bottom-width: 1px;
  border-color: var(--line);
}
.x {
  width: 20px;
  height: var(--h-control);
  border-radius: var(--r-s);
  font-size: var(--fs-s);
  color: var(--text-muted);
  display: flex;
  align-items: center;
  justify-content: center;
}
.x:hover {
  background: var(--bad-soft);
  color: var(--bad);
}
.dtab {
  height: 25px;
  padding-left: var(--sp-4);
  padding-right: var(--sp-4);
  font-size: var(--fs-s);
  color: var(--text-muted);
  display: flex;
  align-items: center;
  border-bottom-width: 2px;
  border-color: transparent;
}
.dtab:hover {
  color: var(--text);
}
.dtab.on {
  color: var(--text-strong);
  border-color: var(--line-focus);
}
.spacer {
  flex: 1;
}
.copy-label {
  font-size: var(--fs-s);
  color: var(--text-faint);
  padding-right: var(--sp-1);
}
.mini {
  height: var(--h-chip);
  padding-left: var(--sp-3);
  padding-right: var(--sp-3);
  border-radius: var(--r-s);
  border-width: 1px;
  border-color: var(--line);
  background: var(--surface-raised);
  font-size: var(--fs-xs);
  color: var(--text-muted);
  display: flex;
  align-items: center;
  white-space: nowrap;
}
.mini:hover {
  background: var(--surface-hover);
  border-color: var(--line-strong);
  color: var(--text-strong);
}
.mini.busy {
  color: var(--text);
}
/* The details are text to copy from (a header value, a timing); the section heads, which fold, are
   not. */
.scroll {
  overflow-y: auto;
  overflow-x: hidden;
  min-height: 0;
  user-select: text;
}
.col {
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.sec {
  display: flex;
  flex-direction: column;
  padding-bottom: var(--sp-2);
}
.sec-head {
  user-select: none;
  height: 24px;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-3);
  padding-left: var(--sp-4);
  padding-right: var(--sp-4);
  background: var(--surface-panel);
  border-bottom-width: 1px;
  border-color: var(--line-subtle);
}
.sec-head:hover {
  background: var(--surface-panel);
}
.sec-head.static:hover {
  background: var(--surface-panel);
}
.tri {
  width: 10px;
  font-size: var(--fs-2xs);
  color: var(--text-faint);
}
.sec-title {
  font-size: var(--fs-s);
  font-weight: 600;
  color: var(--text);
  white-space: nowrap;
}
.n {
  font-size: var(--fs-xs);
  color: var(--text-faint);
}
.kv {
  display: flex;
  flex-direction: row;
  align-items: flex-start;
  gap: var(--sp-4);
  padding-left: var(--sp-4);
  padding-right: var(--sp-5);
  padding-top: var(--sp-1);
  padding-bottom: var(--sp-1);
  min-height: var(--h-row-dense);
}
.kv:hover {
  background: var(--surface-hover);
}
.k {
  flex-shrink: 0;
  font-size: var(--fs-s);
  line-height: 14px;
  color: var(--text-muted);
  white-space: pre;
  overflow: hidden;
}
.v {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
}
.vt {
  font-size: var(--fs-s);
  line-height: 14px;
  color: var(--text);
  white-space: pre;
  overflow: hidden;
}
.vt.faint {
  color: var(--text-faint);
}
.vt.masked {
  color: var(--warn);
}
.vt.ok {
  color: var(--ok);
}
.vt.warn {
  color: var(--warn);
}
.vt.bad {
  color: var(--bad);
}
.vn {
  font-size: var(--fs-xs);
  line-height: 13px;
  color: var(--text-faint);
}
.eye {
  width: 30px;
  flex-shrink: 0;
  height: 14px;
  border-radius: var(--r-s);
  font-size: var(--fs-2xs);
  color: var(--text);
  background: var(--surface-active);
  display: flex;
  align-items: center;
  justify-content: center;
}
.eye:hover {
  background: var(--surface-active);
  color: var(--text);
}
.eye-gap {
  width: 30px;
  flex-shrink: 0;
}
.kv-empty {
  padding-left: 28px;
  padding-top: var(--sp-1);
  padding-bottom: var(--sp-1);
  font-size: var(--fs-s);
  color: var(--text-faint);
}
.notes {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-4);
  padding-bottom: var(--sp-5);
}
.note-line {
  font-size: var(--fs-xs);
  line-height: 14px;
  color: var(--text-faint);
}
.note-line.pad {
  padding: var(--sp-4);
}
.errbar {
  height: var(--h-head);
  display: flex;
  align-items: center;
  padding-left: var(--sp-4);
  padding-right: var(--sp-4);
  font-size: var(--fs-s);
  color: var(--bad);
  background: var(--bad-soft);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.bad-text {
  color: var(--bad);
}
.bar-wrap {
  display: flex;
  flex-direction: column;
  gap: var(--sp-3);
  padding: var(--sp-5);
  padding-bottom: var(--sp-3);
}
.tbar {
  height: 10px;
  display: flex;
  flex-direction: row;
  border-radius: var(--r-s);
  background: var(--surface-raised);
  overflow: hidden;
}
.seg-wait {
  height: 10px;
  background: var(--surface-handle);
}
.seg-wait.failed {
  background: var(--bad);
}
.seg-read {
  height: 10px;
  background: var(--ok);
}
.legend {
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: var(--sp-3);
  font-size: var(--fs-xs);
  color: var(--text-muted);
}
.sw {
  width: 9px;
  height: 9px;
  border-radius: var(--r-s);
}
.sw.wait {
  background: var(--surface-handle);
}
.sw.wait.failed {
  background: var(--bad);
}
.sw.read {
  background: var(--ok);
  margin-left: var(--sp-4);
}
</style>
