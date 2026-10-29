// XMLHttpRequest on the real host (m26 fixes, docs/DESKTOP.md §4.5): the
// Argon client's upload PUT, run in the app against a loopback server.
//
//   bun test packages/vn-test/specs/xhr.spec.ts
//
// redjs's own tests (src/webapi/xhr/jsc_tests.rs) cover the state machine on
// both transports. This spec only proves the class reaches a page on the
// shipped host (net-h2, the vue-native runtime installed over the engine), and
// that the client's `noXhr()` is false there, so its fetch-PUT fallback is not
// taken. Any fixture will do; this one borrows `events`.

import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  async fetch(req) {
    const body = new Uint8Array(await req.arrayBuffer());
    const path = new URL(req.url).pathname;
    return new Response(`${req.method} ${path} ${body.length} ${req.headers.get("content-type")}`, {
      headers: { ETag: '"part-3"' },
    });
  },
});

const app = await launch({
  app: "packages/vn-test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "xhr",
});
afterAll(async () => {
  await app.close();
  server.stop(true);
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

test("the client's xhrPut: upload progress, a sliding timeout, the ETag at load", async () => {
  expect(await app.eval("typeof XMLHttpRequest + ',' + typeof ProgressEvent")).toBe("function,function");
  const url = `http://127.0.0.1:${server.port}/part/3`;
  // lib/video/multipartUpload.ts's xhrPut, with a log instead of a promise.
  await app.eval(`(() => {
    const out = (globalThis.__XHR__ = { log: [], done: false });
    const body = new Blob([new Uint8Array(200 * 1024).fill(1)], { type: "video/mp4" });
    const xhr = new XMLHttpRequest();
    const startedAt = performance.now();
    xhr.onloadend = () => { out.done = true; };
    xhr.open("PUT", ${JSON.stringify(url)});
    xhr.timeout = 10000;
    xhr.upload.onprogress = (e) => {
      xhr.timeout = Math.ceil(performance.now() - startedAt) + 10000;
      out.log.push("up:" + e.loaded + "/" + e.total);
    };
    xhr.onload = () => out.log.push("load:" + xhr.status + ":" + xhr.getResponseHeader("ETag") + ":" + xhr.responseText);
    xhr.onerror = () => out.log.push("error");
    xhr.ontimeout = () => out.log.push("timeout");
    xhr.onabort = () => out.log.push("abort");
    xhr.send(body);
    return "sent";
  })()`);
  let got: { log: string[]; done: boolean } = { log: [], done: false };
  for (let i = 0; i < 100 && !got.done; i++) {
    await sleep(100);
    got = JSON.parse((await app.eval("JSON.stringify(__XHR__)")) as string);
  }
  expect(got.done).toBe(true);
  const n = 200 * 1024;
  // At least one upload progress, the last at the full length, then load.
  expect(got.log.length).toBeGreaterThanOrEqual(2);
  expect(got.log.at(-2)).toBe(`up:${n}/${n}`);
  expect(got.log.at(-1)).toBe(`load:200:"part-3":PUT /part/3 ${n} video/mp4`);
}, T);
