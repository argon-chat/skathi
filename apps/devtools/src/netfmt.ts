// What the Network details pane and its copy actions are made of — pure
// functions, no Vue and no engine globals, so `bun test apps/devtools/test`
// can hold them down without a host.
//
// The wire (`net_detail`, crates/vn-app/src/devtools/net.rs) carries a body as
// `utf8` text or `base64` bytes. Everything a person reads — the hex dump, the
// pretty JSON, the form fields, "Copy as curl" — is derived here, in the
// inspector, from those two shapes.

import type { FormField, HeaderPair, NetBody, NetDetail } from "./protocol";

// ---------------------------------------------------------------------------
// bytes
// ---------------------------------------------------------------------------

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64_INDEX: Int16Array = (() => {
  const t = new Int16Array(128).fill(-1);
  for (let i = 0; i < 64; i++) t[B64.charCodeAt(i)] = i;
  // base64url as well — it costs two table entries.
  t["-".charCodeAt(0)] = 62;
  t["_".charCodeAt(0)] = 63;
  return t;
})();

/**
 * RFC 4648 base64 -> bytes. Not `atob`: the inspector's declared surface
 * (apps/env.d.ts) has no `atob`, and a binary string would then need a second
 * pass to become bytes anyway.
 */
export function fromBase64(text: string): Uint8Array {
  const out = new Uint8Array(Math.floor((text.length * 3) / 4) + 3);
  let o = 0;
  let acc = 0;
  let bits = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    const v = c < 128 ? B64_INDEX[c] : -1;
    if (v < 0) continue; // padding, whitespace
    acc = ((acc << 6) | v) & 0xffffff;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[o++] = (acc >> bits) & 0xff;
    }
  }
  return out.subarray(0, o);
}

/** Bytes per hex-dump row. */
export const HEX_ROW = 16;

export function hexRows(length: number): number {
  return Math.ceil(length / HEX_ROW);
}

/** `00000010  de ad be ef 00 01 02 03  04 05 06 07 08 09 0a 0b  ....…` */
export function hexLine(bytes: Uint8Array, row: number): string {
  const at = row * HEX_ROW;
  let hex = "";
  let ascii = "";
  for (let i = 0; i < HEX_ROW; i++) {
    const p = at + i;
    if (p < bytes.length) {
      const b = bytes[p];
      hex += (b < 16 ? "0" : "") + b.toString(16);
      ascii += b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".";
    } else {
      hex += "  ";
    }
    hex += i === 7 ? "  " : " ";
  }
  return `${at.toString(16).padStart(8, "0")}  ${hex} ${ascii}`;
}

/** The whole dump, for the clipboard. */
export function hexDump(bytes: Uint8Array): string {
  const rows: string[] = [];
  for (let r = 0; r < hexRows(bytes.length); r++) rows.push(hexLine(bytes, r));
  return rows.join("\n");
}

// ---------------------------------------------------------------------------
// text
// ---------------------------------------------------------------------------

/**
 * A text cut into rows of at most `cols` characters.
 *
 * The body view is a virtual list of fixed-height rows (`../virtual.ts`), and
 * nothing in this runtime can measure text, so a long line is HARD-wrapped
 * here at a column count the pane computes from its width. Tabs become two
 * spaces for the same reason: a fixed advance width is the only one we know.
 */
export function textLines(text: string, cols: number, max = 250_000): string[] {
  const width = Math.max(8, Math.floor(cols));
  const out: string[] = [];
  for (const raw of text.split("\n")) {
    const line = raw.replace(/\r$/, "").replace(/\t/g, "  ");
    if (line.length <= width) out.push(line);
    else for (let i = 0; i < line.length; i += width) out.push(line.slice(i, i + width));
    if (out.length >= max) break;
  }
  return out;
}

export function isJsonMime(mime: string | undefined): boolean {
  const essence = (mime || "").split(";")[0].trim().toLowerCase();
  return essence === "application/json" || essence.endsWith("+json") || essence === "application/x-ndjson";
}

/** Two-space JSON, or `null` when the text is not a JSON object or array. */
export function prettyJson(text: string): string | null {
  const t = text.trim();
  if (!t || (t[0] !== "{" && t[0] !== "[")) return null;
  try {
    return JSON.stringify(JSON.parse(t), null, 2);
  } catch {
    return null;
  }
}

function decodeComponent(s: string): string {
  try {
    return decodeURIComponent(s.replace(/\+/g, " "));
  } catch {
    return s;
  }
}

/** `a=1&b=two+words` -> `[["a","1"],["b","two words"]]`. */
export function parseForm(text: string): HeaderPair[] {
  const out: HeaderPair[] = [];
  for (const part of text.replace(/^\?/, "").split("&")) {
    if (!part) continue;
    const eq = part.indexOf("=");
    out.push(
      eq < 0 ? [decodeComponent(part), ""] : [decodeComponent(part.slice(0, eq)), decodeComponent(part.slice(eq + 1))],
    );
  }
  return out;
}

/** The URL's query string as pairs — the Payload tab's first section. */
export function queryOf(url: string): HeaderPair[] {
  const q = url.indexOf("?");
  if (q < 0) return [];
  const hash = url.indexOf("#", q);
  return parseForm(url.slice(q + 1, hash < 0 ? undefined : hash));
}

export function formatBytes(n: number | null | undefined): string {
  if (n === null || n === undefined || !Number.isFinite(n)) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

export function formatMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return "—";
  return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(2)} s`;
}

/** `12:03:04.123`, local time. */
export function clock(at: number): string {
  const d = new Date(at);
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

// ---------------------------------------------------------------------------
// headers, and the ones that are secrets
// ---------------------------------------------------------------------------

/**
 * Headers whose VALUE is a credential. Shown — this is the developer's own
 * app — but masked until clicked: a screenshot of a Network panel is the most
 * common way a session cookie leaves a machine, and the Argon client's
 * `ArgonSecure` cookie is exactly that.
 */
export function isSensitive(name: string): boolean {
  const n = name.toLowerCase();
  return n === "cookie" || n === "set-cookie" || n === "authorization" || n === "proxy-authorization";
}

function maskValue(v: string): string {
  return v ? `•••••• (${v.length})` : "";
}

/**
 * A masked header value that still says WHICH cookies there are: the names
 * stay, the values go. `a=1; b=22` -> `a=•••••• (1); b=•••••• (2)`. A
 * `Set-Cookie` line keeps its attributes (`Path`, `Expires`, …), which are not
 * secret and are usually the question.
 */
export function maskHeader(name: string, value: string): string {
  const n = name.toLowerCase();
  if (n === "cookie") {
    return value
      .split(/;\s*/)
      .map((part) => {
        const eq = part.indexOf("=");
        return eq < 0 ? maskValue(part) : `${part.slice(0, eq)}=${maskValue(part.slice(eq + 1))}`;
      })
      .join("; ");
  }
  if (n === "set-cookie") {
    const semi = value.indexOf(";");
    const first = semi < 0 ? value : value.slice(0, semi);
    const rest = semi < 0 ? "" : value.slice(semi);
    const eq = first.indexOf("=");
    return (eq < 0 ? maskValue(first) : `${first.slice(0, eq)}=${maskValue(first.slice(eq + 1))}`) + rest;
  }
  // Authorization: the scheme is not the secret.
  const sp = value.indexOf(" ");
  return sp > 0 && sp < 16 ? `${value.slice(0, sp)} ${maskValue(value.slice(sp + 1))}` : maskValue(value);
}

export interface HeaderRow {
  name: string;
  value: string;
  sensitive: boolean;
  /** Where a row came from when it is not simply "the app passed it". */
  note?: string;
}

/**
 * The request headers as they went out, as far as anything can know: the
 * app's own, the Content-Type its body implied, and the jar's Cookie (redjs
 * appends it AFTER a script-set one, `webapi::cookies` C-5). The transport's
 * own (Host, Content-Length, …) are never visible to script and are not
 * invented here.
 */
export function requestHeaderRows(d: NetDetail): HeaderRow[] {
  const rows: HeaderRow[] = (d.request?.headers || []).map(([name, value]) => ({
    name,
    value,
    sensitive: isSensitive(name),
  }));
  const implied = d.request?.bodyInfo?.implied;
  if (implied && !rows.some((r) => r.name.toLowerCase() === "content-type")) {
    rows.push({ name: "content-type", value: implied, sensitive: false, note: "implied by the body" });
  }
  const jar = d.cookies?.sent;
  if (jar) {
    const at = rows.findIndex((r) => r.name.toLowerCase() === "cookie");
    if (at >= 0 && rows[at].value.trim()) {
      rows[at] = { ...rows[at], value: `${rows[at].value}; ${jar}`, note: "the app's, then the jar's" };
    } else if (at >= 0) {
      rows[at] = { ...rows[at], value: jar, note: "from the cookie jar" };
    } else {
      rows.push({ name: "cookie", value: jar, sensitive: true, note: "from the cookie jar" });
    }
  }
  return rows;
}

/** The response headers, plus the Set-Cookie lines reconstructed from the jar. */
export function responseHeaderRows(d: NetDetail): HeaderRow[] {
  const rows: HeaderRow[] = (d.response?.headers || []).map(([name, value]) => ({
    name,
    value,
    sensitive: isSensitive(name),
  }));
  for (const line of d.cookies?.set || []) {
    rows.push({ name: "set-cookie", value: line, sensitive: true, note: "reconstructed from the jar" });
  }
  return rows;
}

export function headersText(rows: readonly { name: string; value: string }[]): string {
  return rows.map((r) => `${r.name}: ${r.value}`).join("\n");
}

// ---------------------------------------------------------------------------
// bodies
// ---------------------------------------------------------------------------

/** The kept bytes of a base64 body, or `null` for a text one. */
export function bodyBytes(body: NetBody | null | undefined): Uint8Array | null {
  if (!body || body.encoding !== "base64" || body.data === undefined) return null;
  return fromBase64(body.data);
}

/** One line on what is (not) in a body — the bar above every body view. */
export function bodySummary(body: NetBody | null | undefined, budget: number): string {
  if (body === undefined) return "not read yet";
  if (body === null) return "no body";
  if (body.evicted) {
    return `body evicted — the tee keeps the most recent ${formatBytes(budget)} of bodies, oldest out first`;
  }
  const parts: string[] = [];
  if (body.mime) parts.push(body.mime);
  const total = body.total;
  const unit = body.unit === "chars" ? " chars" : "";
  if (body.truncated) {
    const whole =
      total !== undefined && total !== null
        ? unit
          ? `${total}${unit}`
          : formatBytes(total)
        : body.seen
          ? `at least ${formatBytes(body.seen)}`
          : "an unknown size";
    parts.push(`truncated: first ${formatBytes(body.stored)} of ${whole}`);
  } else {
    parts.push(formatBytes(body.stored));
  }
  return parts.join(" · ");
}

/** What "Copy response body" puts on the clipboard: the text, or the hex dump. */
export function bodyAsText(body: NetBody | null | undefined): string {
  if (!body || body.data === undefined) return "";
  if (body.encoding === "base64") return hexDump(fromBase64(body.data));
  return body.data;
}

// ---------------------------------------------------------------------------
// copy as curl / fetch
// ---------------------------------------------------------------------------

/** A POSIX-shell single-quoted word. */
export function shellQuote(s: string): string {
  return `'${s.replace(/'/g, `'\\''`)}'`;
}

/** bash's `$'…'`, for bytes that are not printable. */
function ansiC(bytes: Uint8Array): string {
  let s = "$'";
  for (const b of bytes) {
    if (b === 0x27) s += "\\'";
    else if (b === 0x5c) s += "\\\\";
    else if (b >= 0x20 && b < 0x7f) s += String.fromCharCode(b);
    else s += `\\x${b < 16 ? "0" : ""}${b.toString(16)}`;
  }
  return `${s}'`;
}

function methodOf(d: NetDetail): string {
  return (d.entry.method || d.method || "GET").toUpperCase();
}

/** Why a copied request would not be byte-for-byte the one that went out. */
function caveats(d: NetDetail): string[] {
  const out: string[] = [];
  const body = d.request?.body;
  const info = d.request?.bodyInfo;
  if (body?.truncated) out.push(`the body was cut at ${formatBytes(body.stored)} by the devtools tee`);
  if (body?.evicted) out.push("the body was evicted from the devtools ring and is left out");
  if (info?.kind === "formdata" && info.fields?.some((f) => f.file)) out.push("file parts are named but their bytes were not captured");
  if (body && !body.evicted && body.data === undefined && body.note && info?.kind !== "formdata") out.push(`no body: ${body.note}`);
  return out;
}

function appHeaders(d: NetDetail): HeaderPair[] {
  const rows = requestHeaderRows(d).filter((r) => r.name.toLowerCase() !== "cookie");
  // A multipart Content-Type names a boundary the engine generated and the
  // tool will generate again; copying it would point at the wrong one.
  const multipart = d.request?.bodyInfo?.kind === "formdata";
  return rows
    .filter((r) => !(multipart && r.name.toLowerCase() === "content-type"))
    .map((r) => [r.name, r.value] as HeaderPair);
}

function cookieOf(d: NetDetail): string {
  return requestHeaderRows(d).find((r) => r.name.toLowerCase() === "cookie")?.value || "";
}

/**
 * Chromium's "Copy as cURL (bash)": the method, the headers the app passed,
 * the cookies that went with it (`-b`, real values — copying is the explicit
 * act masking defers to), and the body.
 */
export function asCurl(d: NetDetail): string {
  const method = methodOf(d);
  const parts = [`curl ${shellQuote(d.entry.url)}`];
  if (method !== "GET") parts.push(`-X ${shellQuote(method)}`);
  for (const [k, v] of appHeaders(d)) parts.push(`-H ${shellQuote(`${k}: ${v}`)}`);
  const cookie = cookieOf(d);
  if (cookie) parts.push(`-b ${shellQuote(cookie)}`);
  const info = d.request?.bodyInfo;
  const body = d.request?.body;
  if (info?.kind === "formdata") {
    for (const f of info.fields || []) {
      parts.push(
        f.file
          ? `-F ${shellQuote(`${f.name}=@${f.file.name || "file"}${f.file.type ? `;type=${f.file.type}` : ""}`)}`
          : `-F ${shellQuote(`${f.name}=${f.value ?? ""}`)}`,
      );
    }
  } else if (body && body.data !== undefined && !body.evicted) {
    if (body.encoding === "base64") parts.push(`--data-binary ${ansiC(fromBase64(body.data))}`);
    else parts.push(`--data-raw ${shellQuote(body.data)}`);
  }
  const notes = caveats(d).map((c) => `# NOTE: ${c}\n`);
  return notes.join("") + parts.join(" \\\n  ");
}

function formDataBuilder(fields: FormField[]): string {
  const lines = ["(() => {", "    const form = new FormData();"];
  for (const f of fields) {
    if (f.file) {
      lines.push(
        `    form.append(${JSON.stringify(f.name)}, new Blob([], { type: ${JSON.stringify(f.file.type)} }), ${JSON.stringify(
          f.file.name,
        )}); // ${f.file.size ?? "?"} bytes, not captured`,
      );
    } else {
      lines.push(`    form.append(${JSON.stringify(f.name)}, ${JSON.stringify(f.value ?? "")});`);
    }
  }
  lines.push("    return form;", "  })()");
  return lines.join("\n");
}

/**
 * Chromium's "Copy as fetch": the same request as a `fetch()` call. No Cookie
 * header — a script cannot set one in a browser; `credentials` is what carries
 * the cookies, and it is copied as the app gave it.
 */
export function asFetch(d: NetDetail): string {
  const method = methodOf(d);
  const headers: Record<string, string> = {};
  for (const [k, v] of appHeaders(d)) headers[k] = v;
  const init: string[] = [];
  init.push(`  "headers": ${JSON.stringify(headers, null, 2).replace(/\n/g, "\n  ")}`);
  const info = d.request?.bodyInfo;
  const body = d.request?.body;
  if (info?.kind === "formdata") {
    init.push(`  "body": ${formDataBuilder(info.fields || [])}`);
  } else if (body && body.data !== undefined && !body.evicted) {
    if (body.encoding === "base64") {
      const bytes = fromBase64(body.data);
      init.push(`  "body": new Uint8Array([${Array.from(bytes, (b) => `0x${b.toString(16)}`).join(",")}])`);
    } else {
      init.push(`  "body": ${JSON.stringify(body.data)}`);
    }
  }
  init.push(`  "method": ${JSON.stringify(method)}`);
  const req = d.request || {};
  if (req.mode && req.mode !== "cors") init.push(`  "mode": ${JSON.stringify(req.mode)}`);
  if (req.redirect && req.redirect !== "follow") init.push(`  "redirect": ${JSON.stringify(req.redirect)}`);
  init.push(`  "credentials": ${JSON.stringify(req.credentials || "same-origin")}`);
  const notes = caveats(d).map((c) => `// NOTE: ${c}\n`);
  return `${notes.join("")}fetch(${JSON.stringify(d.entry.url)}, {\n${init.join(",\n")}\n});`;
}
