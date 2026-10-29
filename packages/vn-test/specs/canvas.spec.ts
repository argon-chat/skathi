// `<canvas>` and the 2D context in a real host (m26 client sweep, Canvas2D
// step 1: redjs webapi/canvas2d + packages/vue-native/src/canvas.ts).
//
//   bun test packages/vn-test/specs/canvas.spec.ts
//
// What the Argon client does with a canvas, minus the screen: an SVG data URL
// drawn into a mirror <canvas> and read back with toDataURL (the taskbar
// icons, desktopTaskbar.ts), a tint by source-atop (custom emoji), a
// thumbhash-style getImageData over an OffscreenCanvas, and toBlob. The host
// does not paint canvases yet, so nothing here looks at pixels on screen.
// Any fixture will do; this one borrows `events`.

import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "canvas",
});
afterAll(() => app.close());

const T = 60_000;

/** Evaluate an async body in the page; its settled value, JSON-decoded. */
async function inPage(body: string): Promise<unknown> {
  await app.eval(
    `globalThis.__cv = undefined; (async () => { ${body} })().then(` +
      `v => { globalThis.__cv = JSON.stringify({ ok: v }); }, ` +
      `e => { globalThis.__cv = JSON.stringify({ err: (e && e.name) + ': ' + (e && e.message) }); }); 'started'`,
  );
  const until = Date.now() + 20_000;
  while (Date.now() < until) {
    const raw = await app.eval("globalThis.__cv === undefined ? '' : globalThis.__cv");
    if (raw) return JSON.parse(String(raw));
    await Bun.sleep(50);
  }
  throw new Error("the page body never settled");
}

test("a mirror <canvas> has a native 2D context that draws and reads back", async () => {
  const got = await inPage(`
    const c = document.createElement('canvas');
    c.width = 8; c.height = 4;
    const ctx = c.getContext('2d');
    ctx.fillStyle = 'rgb(10, 20, 30)';
    ctx.fillRect(0, 0, 4, 4);
    const px = Array.from(ctx.getImageData(1, 1, 1, 1).data).join();
    const empty = Array.from(ctx.getImageData(6, 1, 1, 1).data).join();
    return {
      tag: Object.prototype.toString.call(ctx),
      same: c.getContext('2d') === ctx && ctx.canvas === c,
      size: [c.width, c.height, c.getAttribute('width')],
      px, empty,
      backend: __rjs_canvas_info(ctx).backend,
    };
  `);
  expect(got).toEqual({
    ok: {
      tag: "[object CanvasRenderingContext2D]",
      same: true,
      size: [8, 4, "8"],
      px: "10,20,30,255",
      empty: "0,0,0,0",
      backend: "vello_cpu",
    },
  });
}, T);

test("an SVG data URL drawn into a canvas comes back out of toDataURL (the taskbar icons)", async () => {
  const got = await inPage(`
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#f00"/></svg>';
    const img = new Image();
    await new Promise((res, rej) => { img.onload = res; img.onerror = () => rej(new Error('svg did not load')); img.src = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg); });
    const c = document.createElement('canvas');
    c.width = 32; c.height = 32;
    const ctx = c.getContext('2d');
    ctx.drawImage(img, 0, 0, 32, 32);
    const url = c.toDataURL('image/png');
    const back = new Image();
    await new Promise((res, rej) => { back.onload = res; back.onerror = () => rej(new Error('png did not load')); back.src = url; });
    const o = new OffscreenCanvas(32, 32).getContext('2d');
    o.drawImage(back, 0, 0);
    return [url.slice(0, 22), back.naturalWidth, Array.from(o.getImageData(16, 16, 1, 1).data).join()];
  `);
  expect(got).toEqual({ ok: ["data:image/png;base64,", 32, "255,0,0,255"] });
}, T);

test("tint by source-atop, a thumbhash read, and toBlob", async () => {
  const got = await inPage(`
    const oc = new OffscreenCanvas(4, 1);
    const ctx = oc.getContext('2d', { willReadFrequently: true });
    ctx.fillStyle = '#fff'; ctx.fillRect(0, 0, 2, 1);
    ctx.globalCompositeOperation = 'source-atop';
    ctx.fillStyle = 'oklch(0.452 0.313 264.05)';
    ctx.fillRect(0, 0, 4, 1);
    const d = ctx.getImageData(0, 0, 4, 1).data;
    const c = document.createElement('canvas');
    const blob = await new Promise(r => c.toBlob(r, 'image/jpeg', 0.8));
    return { tinted: d[2] > 200 && d[0] < 40 && d[3] === 255, untouched: d[15] === 0, blob: [blob.type, blob.size > 100] };
  `);
  expect(got).toEqual({ ok: { tinted: true, untouched: true, blob: ["image/jpeg", true] } });
}, T);

test("the app logged nothing broken", async () => {
  await app.expectNoErrors();
}, T);
