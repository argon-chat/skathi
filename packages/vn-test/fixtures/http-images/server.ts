// The loopback server behind fixtures/http-images: N distinct PNGs, served the
// way the Argon client's files are (measured 2026-11-02, research/reports/m26/
// fixes-lag-1.md §4.12):
//
//   GET /files/<i>        302, `Cache-Control: private, max-age=300`, no validators
//   GET /cdn/<i>.png      200, `Cache-Control: max-age=2592000`, ETag, Last-Modified
//                         304 to a matching If-None-Match / If-Modified-Since
//
// `policy` overrides the 200's caching headers per test (`no-store`, `max-age=0`,
// a Vary, …). Every request is counted, so a test can say "the server saw
// nothing". Runs in the test process (Bun.serve); nothing is written to disk.
import { deflateSync } from "node:zlib";

export interface Seen {
  /** Every request, in order: `METHOD path status`. */
  log: string[];
  redirects: number;
  bodies: number;
  notModified: number;
}

export interface ImageServer {
  base: string;
  seen: Seen;
  /** The 302 URL of image `i` (what an `<img src>` is given). */
  file(i: number, query?: string): string;
  /** The final URL of image `i`. */
  cdn(i: number, query?: string): string;
  /** Headers the 200 (and its 304) carry instead of the defaults. */
  policy: Record<string, string> | null;
  reset(): void;
  stop(): void;
}

const LAST_MODIFIED = "Thu, 08 Oct 2026 23:44:14 GMT";

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();

function crc32(bytes: Uint8Array): number {
  let c = 0xffffffff;
  for (const b of bytes) c = CRC_TABLE[(c ^ b) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type: string, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + data.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, data.length);
  out.set(new TextEncoder().encode(type), 4);
  out.set(data, 8);
  view.setUint32(8 + data.length, crc32(out.subarray(4, 8 + data.length)));
  return out;
}

/** A `size`-px RGB PNG, distinct per `seed`: a gradient with noise, so it
 *  compresses about as badly as a photo avatar does (~40–60 KB at 160 px). */
export function png(seed: number, size = 160): Uint8Array {
  const raw = new Uint8Array(size * (1 + size * 3));
  let s = (seed * 2654435761) >>> 0 || 1;
  for (let y = 0; y < size; y++) {
    const row = y * (1 + size * 3);
    raw[row] = 0;
    for (let x = 0; x < size; x++) {
      s ^= s << 13;
      s ^= s >>> 17;
      s ^= s << 5;
      const noise = s & 0x3f;
      const o = row + 1 + x * 3;
      raw[o] = (x * 255) / size + noise;
      raw[o + 1] = (y * 255) / size + noise;
      raw[o + 2] = (seed * 37) & 0xff;
    }
  }
  const ihdr = new Uint8Array(13);
  const v = new DataView(ihdr.buffer);
  v.setUint32(0, size);
  v.setUint32(4, size);
  ihdr.set([8, 2, 0, 0, 0], 8);
  const parts = [
    new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", ihdr),
    chunk("IDAT", deflateSync(raw, { level: 6 })),
    chunk("IEND", new Uint8Array(0)),
  ];
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

export function serveImages(): ImageServer {
  const cache = new Map<number, Uint8Array>();
  const body = (i: number) => {
    let b = cache.get(i);
    if (!b) cache.set(i, (b = png(i)));
    return b;
  };
  const seen: Seen = { log: [], redirects: 0, bodies: 0, notModified: 0 };
  const api: ImageServer = {
    base: "",
    seen,
    policy: null,
    file: (i, q) => `${api.base}/files/${i}${q ? `?${q}` : ""}`,
    cdn: (i, q) => `${api.base}/cdn/${i}.png${q ? `?${q}` : ""}`,
    reset() {
      seen.log = [];
      seen.redirects = seen.bodies = seen.notModified = 0;
    },
    stop() {
      server.stop(true);
    },
  };
  const server = Bun.serve({
    port: 0,
    hostname: "127.0.0.1",
    fetch(req) {
      const url = new URL(req.url);
      const note = (status: number) => seen.log.push(`${req.method} ${url.pathname}${url.search} ${status}`);
      let m = /^\/files\/(\d+)$/.exec(url.pathname);
      if (m) {
        seen.redirects++;
        note(302);
        return new Response(null, {
          status: 302,
          headers: { location: `/cdn/${m[1]}.png${url.search}`, "cache-control": "private, max-age=300" },
        });
      }
      m = /^\/cdn\/(\d+)\.png$/.exec(url.pathname);
      if (m) {
        const i = Number(m[1]);
        const etag = `"img-${i}-v1"`;
        const headers: Record<string, string> = api.policy ?? {
          "cache-control": "max-age=2592000",
          etag,
          "last-modified": LAST_MODIFIED,
        };
        const validators = "etag" in headers || "last-modified" in headers;
        if (validators && (req.headers.get("if-none-match") === etag || req.headers.get("if-modified-since") === LAST_MODIFIED)) {
          seen.notModified++;
          note(304);
          return new Response(null, { status: 304, headers });
        }
        seen.bodies++;
        note(200);
        return new Response(body(i), { headers: { "content-type": "image/png", ...headers } });
      }
      note(404);
      return new Response("not found", { status: 404 });
    },
  });
  api.base = `http://127.0.0.1:${server.port}`;
  return api;
}
