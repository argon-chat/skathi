import { afterAll, expect, test } from "bun:test";
import { existsSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { appDataDir, launch, type SkApp } from "../src/index";
import { serveImages } from "../fixtures/http-images/server";

const NAME = `sk-test-http-cache-${process.pid}`;
const ROOT = appDataDir(NAME);
const CACHE = join(ROOT, "cache", "http");
const N = 24;
const T = 90_000;

const srv = serveImages();
const urls = (q?: string) => Array.from({ length: N }, (_, i) => srv.file(i, q));
let app: SkApp | null = null;

afterAll(async () => {
  await app?.close();
  srv.stop();
  for (let i = 0; ; i++) {
    try {
      rmSync(ROOT, { recursive: true, force: true });
      return;
    } catch (e) {
      if (i >= 40) throw e;
      await Bun.sleep(250);
    }
  }
});

async function relaunch(label: string) {
  await app?.close();
  app = await launch({
    app: "packages/test/fixtures/http-images/src/main.ts",
    window: [640, 300],
    label,
    env: { SK_APP_NAME: NAME },
  });
  return app;
}

async function round(list: string[]) {
  await app!.eval(`__imgs.clear(); __imgs.start(${JSON.stringify(list)}); 1`);
  const until = Date.now() + 30_000;
  for (;;) {
    const st = JSON.parse(String(await app!.eval("JSON.stringify(__imgs.stats())")));
    if (st.loads.length + st.errors.length >= list.length || Date.now() > until) return st;
    await Bun.sleep(50);
  }
}

async function get(url: string, init: Record<string, unknown> = {}) {
  await app!.eval(`__imgs.get(${JSON.stringify(url)}, ${JSON.stringify(init)})`);
  const until = Date.now() + 15_000;
  for (;;) {
    const raw = String(await app!.eval("__imgs.got"));
    if (raw) return JSON.parse(raw);
    if (Date.now() > until) throw new Error(`fetch ${url} never settled`);
    await Bun.sleep(25);
  }
}

const cache = async () => (await app!.send("net_cache")) as any;

test(
  "a first run stores both hops of every image",
  async () => {
    await relaunch("http-cache-1");
    const st = await round(urls());
    expect(st.errors).toEqual([]);
    expect(st.loads.length).toBe(N);
    expect(srv.seen.redirects).toBe(N);
    expect(srv.seen.bodies).toBe(N);
    const c = await cache();
    expect(c.writable).toBe(true);
    expect(c.sealed).toBe(false);
    expect(c.entries).toBe(2 * N);
    expect(c.stored).toBe(2 * N);
    expect(c.dir.toLowerCase()).toBe(CACHE.toLowerCase());
    const net = (await app!.send("net", { since: 0 })) as any;
    const rows = net.entries.filter((e: any) => e.kind === "img");
    expect(rows.length).toBe(N);
    expect(rows.every((e: any) => e.cache === "stored" && e.status === 200 && e.proto)).toBe(true);
    // A Linux host's `mem` has no httpCache section yet.
    if (process.platform !== "linux") expect(((await app!.send("mem")) as any).httpCache.entries).toBe(2 * N);
  },
  T,
);

test(
  "a relaunch shows every image from disk without one request",
  async () => {
    srv.reset();
    await relaunch("http-cache-2");
    const st = await round(urls());
    expect(st.errors).toEqual([]);
    expect(st.loads.length).toBe(N);
    expect(srv.seen.log).toEqual([]);
    const c = await cache();
    expect(c.hits).toBe(2 * N);
    const rows = ((await app!.send("net", { since: 0 })) as any).entries.filter((e: any) => e.kind === "img");
    expect(rows.length).toBe(N);
    expect(rows.every((e: any) => e.cache === "hit" && e.conn === undefined)).toBe(true);
  },
  T,
);

test(
  "fetch's cache modes, from the page",
  async () => {
    srv.reset();
    expect(await get(srv.cdn(0))).toMatchObject({ status: 200, bytes: expect.any(Number) });
    expect(srv.seen.log).toEqual([]);
    await get(srv.cdn(0), { cache: "no-store" });
    expect(srv.seen.log).toEqual(["GET /cdn/0.png 200"]);
    const nc = await get(srv.cdn(0), { cache: "no-cache" });
    expect(nc.status).toBe(200);
    expect(srv.seen.notModified).toBe(1);
    const before = srv.seen.log.length;
    const miss = await get(srv.cdn(999), { cache: "only-if-cached", mode: "same-origin" });
    expect(miss.error).toContain("only-if-cached");
    expect(srv.seen.log.length).toBe(before);
    const fc = await get(srv.file(1), { cache: "force-cache" });
    expect(fc).toMatchObject({ status: 200, redirected: true, url: srv.cdn(1) });
    expect(srv.seen.log.length).toBe(before);
  },
  T,
);

test(
  "a stale entry is revalidated: the 302 is still fresh, the CDN answers 304",
  async () => {
    srv.policy = { "cache-control": "max-age=0", "last-modified": "Thu, 08 Oct 2026 23:44:14 GMT" };
    srv.reset();
    expect((await round(urls("r=stale"))).loads.length).toBe(N);
    expect(srv.seen.bodies).toBe(N);
    srv.reset();
    await relaunch("http-cache-3");
    const st = await round(urls("r=stale"));
    expect(st.errors).toEqual([]);
    expect(srv.seen.redirects).toBe(0);
    expect(srv.seen.notModified).toBe(N);
    expect(srv.seen.bodies).toBe(0);
    expect((await cache()).revalidated).toBe(N);
    srv.policy = null;
  },
  T,
);

test(
  "the cache follows the storage's encryption: sealed entries, and a purge on every change",
  async () => {
    expect(await app!.eval("__imgs.encrypt(true)")).toBe(true);
    let c = await cache();
    expect(c.sealed).toBe(true);
    expect(c.entries).toBe(0);
    expect(existsSync(join(CACHE, "key.sealed"))).toBe(true);
    srv.reset();
    expect((await round(urls("r=sealed"))).loads.length).toBe(N);
    c = await cache();
    expect(c.entries).toBe(2 * N);
    // Wait for the writer, then look at the files: no PNG and no URL in them.
    await Bun.sleep(500);
    const files = readdirSync(CACHE).filter((f) => f.endsWith(".e"));
    expect(files.length).toBeGreaterThan(0);
    for (const f of files) {
      const bytes = readFileSync(join(CACHE, f));
      expect(bytes.includes(Buffer.from("IHDR"))).toBe(false);
      expect(bytes.includes(Buffer.from("/cdn/"))).toBe(false);
    }
    expect(await app!.eval("__imgs.encrypt(false)")).toBe(false);
    c = await cache();
    expect(c.sealed).toBe(false);
    expect(c.entries).toBe(0);
    expect(existsSync(join(CACHE, "key.sealed"))).toBe(false);
  },
  T,
);

test(
  "net_cache clear empties it",
  async () => {
    srv.reset();
    await round(urls("r=clear"));
    expect((await cache()).entries).toBeGreaterThan(0);
    const out = (await app!.send("net_cache", { op: "clear" })) as any;
    expect(out.freed).toBeGreaterThan(0);
    expect(out.now.entries).toBe(0);
  },
  T,
);
