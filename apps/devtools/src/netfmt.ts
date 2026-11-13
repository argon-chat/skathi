import type { FormField, HeaderPair, NetBody, NetDetail } from "./protocol";

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const B64_INDEX: Int16Array = (() => {
  const t = new Int16Array(128).fill(-1);
  for (let i = 0; i < 64; i++) t[B64.charCodeAt(i)] = i;
  t["-".charCodeAt(0)] = 62;
  t["_".charCodeAt(0)] = 63;
  return t;
})();

/**
 * Not `atob`: the inspector's declared surface (apps/env.d.ts) has no `atob`, and a binary string
 * would then need a second pass to become bytes anyway.
 */
export function fromBase64(text: string): Uint8Array {
  const out = new Uint8Array(Math.floor((text.length * 3) / 4) + 3);
  let o = 0;
  let acc = 0;
  let bits = 0;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    const v = c < 128 ? B64_INDEX[c]! : -1;
    if (v < 0) continue;
    acc = ((acc << 6) | v) & 0xffffff;
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out[o++] = (acc >> bits) & 0xff;
    }
  }
  return out.subarray(0, o);
}

export const HEX_ROW = 16;

export function hexRows(length: number): number {
  return Math.ceil(length / HEX_ROW);
}

export function hexLine(bytes: Uint8Array, row: number): string {
  const at = row * HEX_ROW;
  let hex = "";
  let ascii = "";
  for (let i = 0; i < HEX_ROW; i++) {
    const p = at + i;
    if (p < bytes.length) {
      const b = bytes[p]!;
      hex += (b < 16 ? "0" : "") + b.toString(16);
      ascii += b >= 0x20 && b < 0x7f ? String.fromCharCode(b) : ".";
    } else {
      hex += "  ";
    }
    hex += i === 7 ? "  " : " ";
  }
  return `${at.toString(16).padStart(8, "0")}  ${hex} ${ascii}`;
}

export function hexDump(bytes: Uint8Array): string {
  const rows: string[] = [];
  for (let r = 0; r < hexRows(bytes.length); r++) rows.push(hexLine(bytes, r));
  return rows.join("\n");
}

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

export function wrapWords(text: string, cols: number, max = 400): string[] {
  const width = Math.max(8, Math.floor(cols));
  const out: string[] = [];
  for (const raw of text.split("\n")) {
    let line = raw.replace(/\r$/, "").replace(/\t/g, "  ");
    while (line.length > width && out.length < max) {
      const space = line.lastIndexOf(" ", width);
      const at = space > width / 2 ? space : width;
      out.push(line.slice(0, at));
      line = space > width / 2 ? line.slice(at + 1) : line.slice(at);
    }
    out.push(line);
    if (out.length >= max) break;
  }
  return out;
}

export function isJsonMime(mime: string | undefined): boolean {
  const essence = (mime || "").split(";")[0]!.trim().toLowerCase();
  return essence === "application/json" || essence.endsWith("+json") || essence === "application/x-ndjson";
}

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

export function clock(at: number): string {
  const d = new Date(at);
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

export function isSensitive(name: string): boolean {
  const n = name.toLowerCase();
  return n === "cookie" || n === "set-cookie" || n === "authorization" || n === "proxy-authorization";
}

function maskValue(v: string): string {
  return v ? `•••••• (${v.length})` : "";
}

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
  const sp = value.indexOf(" ");
  return sp > 0 && sp < 16 ? `${value.slice(0, sp)} ${maskValue(value.slice(sp + 1))}` : maskValue(value);
}

export interface HeaderRow {
  name: string;
  value: string;
  sensitive: boolean;
  note?: string;
}

/**
 * The transport's own (Host, Content-Length, …) are never visible to script and are not invented
 * here.
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
    const existing = rows[at];
    if (existing && existing.value.trim()) {
      rows[at] = { ...existing, value: `${existing.value}; ${jar}`, note: "the app's, then the jar's" };
    } else if (existing) {
      rows[at] = { ...existing, value: jar, note: "from the cookie jar" };
    } else {
      rows.push({ name: "cookie", value: jar, sensitive: true, note: "from the cookie jar" });
    }
  }
  return rows;
}

export function responseHeaderRows(d: NetDetail): HeaderRow[] {
  const rows: HeaderRow[] = (d.response?.headers || []).map(([name, value]) => ({
    name,
    value,
    sensitive: isSensitive(name),
  }));
  const wire = d.cookies?.wire;
  if (wire) {
    const cred = d.request?.credentials;
    const note = cred && cred !== "include" ? `as received · not stored (credentials "${cred}")` : "as received";
    for (const line of wire) rows.push({ name: "set-cookie", value: line, sensitive: true, note });
  } else {
    for (const line of d.cookies?.set || []) {
      rows.push({ name: "set-cookie", value: line, sensitive: true, note: "reconstructed from the jar" });
    }
  }
  return rows;
}

export function headersText(rows: readonly { name: string; value: string }[]): string {
  return rows.map((r) => `${r.name}: ${r.value}`).join("\n");
}

export function bodyBytes(body: NetBody | null | undefined): Uint8Array | null {
  if (!body || body.encoding !== "base64" || body.data === undefined) return null;
  return fromBase64(body.data);
}

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

export function bodyAsText(body: NetBody | null | undefined): string {
  if (!body || body.data === undefined) return "";
  if (body.encoding === "base64") return hexDump(fromBase64(body.data));
  return body.data;
}

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
  if (info?.kind === "formdata" && info.fields?.some((f) => f.file))
    out.push("file parts are named but their bytes were not captured");
  if (body && !body.evicted && body.data === undefined && body.note && info?.kind !== "formdata")
    out.push(`no body: ${body.note}`);
  return out;
}

function appHeaders(d: NetDetail): HeaderPair[] {
  const rows = requestHeaderRows(d).filter((r) => r.name.toLowerCase() !== "cookie");
  // A multipart Content-Type names a boundary the engine generated and the tool will generate
  // again; copying it would point at the wrong one.
  const multipart = d.request?.bodyInfo?.kind === "formdata";
  return rows
    .filter((r) => !(multipart && r.name.toLowerCase() === "content-type"))
    .map((r) => [r.name, r.value] as HeaderPair);
}

function cookieOf(d: NetDetail): string {
  return requestHeaderRows(d).find((r) => r.name.toLowerCase() === "cookie")?.value || "";
}

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
 * No Cookie header — a script cannot set one in a browser; `credentials` is what carries the
 * cookies, and it is copied as the app gave it.
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
