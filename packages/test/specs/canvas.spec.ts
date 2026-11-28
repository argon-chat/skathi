import { afterAll, expect, test } from "bun:test";
import { launch, MACOS, readPng, SCREENSHOTS } from "../src/index";

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAADklEQVR4nGP4z8AAQv8BD/kD/YURmXYAAAAASUVORK5CYII=",
  "base64",
);
/** A 4x2 SVG, red | blue: gpui rasterises it at twice that. */
const SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2">' +
  '<rect width="2" height="2" fill="#f00"/><rect x="2" width="2" height="2" fill="#00f"/></svg>';
const server = Bun.serve({
  hostname: "127.0.0.1",
  port: 0,
  fetch(req) {
    const path = new URL(req.url).pathname;
    if (path.startsWith("/a.png")) return new Response(PNG, { headers: { "content-type": "image/png" } });
    if (path === "/a.svg") return new Response(SVG, { headers: { "content-type": "image/svg+xml" } });
    return new Response("no", { status: 404 });
  },
});

const app = await launch({
  app: "packages/test/fixtures/events/src/main.ts",
  window: [480, 320],
  label: "canvas",
});
afterAll(async () => {
  await app.close();
  server.stop(true);
});

const T = 60_000;

/** Evaluate an async body in the page; its settled value, JSON-decoded. */
async function inPage(body: string): Promise<unknown> {
  await app.eval(
    `globalThis.__cv = undefined; globalThis.__cvStep = "page evaluation"; (async () => { ${body} })().then(` +
      `v => { globalThis.__cv = JSON.stringify({ ok: v }); }, ` +
      `e => { globalThis.__cv = JSON.stringify({ err: (e && e.name) + ': ' + (e && e.message) }); }); 'started'`,
  );
  const until = Date.now() + 20_000;
  while (Date.now() < until) {
    const raw = await app.eval("globalThis.__cv === undefined ? '' : globalThis.__cv");
    if (raw) return JSON.parse(String(raw));
    await Bun.sleep(50);
  }
  throw new Error("canvas test timed out during " + (await app.eval("globalThis.__cvStep")));
}

test(
  "a mirror <canvas> has a native 2D context that draws and reads back",
  async () => {
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
  },
  T,
);

test(
  "an SVG data URL drawn into a canvas comes back out of toDataURL (the taskbar icons)",
  async () => {
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
  },
  T,
);

test(
  "tint by source-atop, a thumbhash read, and toBlob",
  async () => {
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
  },
  T,
);

test.skipIf(!SCREENSHOTS)(
  "an on-screen <canvas> gets the bitmap's size, and CSS scales the bitmap",
  async () => {
    const got = await inPage(`
    const add = (id, style) => {
      const c = document.createElement('canvas');
      c.setAttribute('id', id);
      if (style) c.setAttribute('style', style);
      document.body.appendChild(c);
      return c;
    };
    add('cv-default', 'position: fixed; left: 0; top: 200px');
    const sized = add('cv-sized', 'position: fixed; left: 320px; top: 200px');
    sized.width = 50; sized.height = 20;
    // A 4x2 bitmap stretched to 120x60: left half red, right half blue.
    const big = add('cv-paint', 'position: fixed; left: 20px; top: 20px; width: 120px; height: 60px');
    big.width = 4; big.height = 2;
    const ctx = big.getContext('2d');
    ctx.fillStyle = '#f00'; ctx.fillRect(0, 0, 2, 2);
    ctx.fillStyle = '#00f'; ctx.fillRect(2, 0, 2, 2);
    await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
    return true;
  `);
    expect(got).toEqual({ ok: true });

    const box = async (sel: string) => (await app.query(sel)).nodes[0]?.bounds;
    expect(await box("#cv-default")).toMatchObject({ w: 300, h: 150 });
    expect(await box("#cv-sized")).toMatchObject({ w: 50, h: 20 });
    const b = (await box("#cv-paint"))!;
    expect(b).toMatchObject({ w: 120, h: 60 });

    const shot = readPng(await app.screenshot());
    const scale = shot.width / Number(await app.eval("window.innerWidth"));
    const at = (x: number, y: number) => shot.at(Math.round((b.x + x) * scale), Math.round((b.y + y) * scale));
    const [lr, lg, lb] = at(30, 30);
    const [rr, rg, rb] = at(90, 30);
    expect({ left: lr > 200 && lg < 60 && lb < 60, right: rb > 200 && rr < 60 && rg < 60 }).toEqual({
      left: true,
      right: true,
    });
  },
  T,
);

// macOS CI: the page body never settles.
test.skipIf(!SCREENSHOTS || MACOS)(
  "a draw with no DOM change repaints the canvas on the next frame",
  async () => {
    await inPage(`
    const ctx = document.querySelector('#cv-paint').getContext('2d');
    ctx.fillStyle = '#0f0'; ctx.fillRect(0, 0, 4, 2);
    await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
    return true;
  `);
    const b = (await app.query("#cv-paint")).nodes[0]!.bounds!;
    const shot = readPng(await app.screenshot());
    const scale = shot.width / Number(await app.eval("window.innerWidth"));
    const [r, g, bl] = shot.at(Math.round((b.x + 30) * scale), Math.round((b.y + 30) * scale));
    expect(g > 200 && r < 60 && bl < 60).toBe(true);
  },
  T,
);

test.skipIf(!SCREENSHOTS || MACOS)(
  "image-rendering: pixelated scales a canvas without blending its pixels",
  async () => {
    await inPage(`
    const c = document.createElement('canvas');
    c.id = 'cv-pixelated';
    c.width = 2; c.height = 1;
    c.style.cssText = 'position:absolute;left:10px;top:200px;width:120px;height:60px;image-rendering:pixelated';
    const ctx = c.getContext('2d');
    ctx.fillStyle = '#f00'; ctx.fillRect(0, 0, 1, 1);
    ctx.fillStyle = '#00f'; ctx.fillRect(1, 0, 1, 1);
    document.body.appendChild(c);
    await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
    return true;
  `);
    const b = (await app.query("#cv-pixelated")).nodes[0]!.bounds!;
    const shot = readPng(await app.screenshot());
    const scale = shot.width / Number(await app.eval("window.innerWidth"));
    const at = (x: number) => shot.at(Math.round((b.x + x) * scale), Math.round((b.y + 30) * scale));
    // Bilinear filtering would mix the two colours across the middle half of the box.
    expect([at(50), at(70)]).toEqual([
      [255, 0, 0],
      [0, 0, 255],
    ]);
  },
  T,
);

// A canvas keeps its bitmap premultiplied; gpui's atlas takes straight alpha and blends with
// SRC_ALPHA.
test.skipIf(!SCREENSHOTS || MACOS)(
  "translucent canvas pixels composite as in a browser: an alpha fill and an antialiased edge",
  async () => {
    const got = await inPage(`
    const wrap = document.createElement('div');
    wrap.setAttribute('id', 'cv-alpha-wrap');
    wrap.setAttribute('style', 'position: fixed; left: 160px; top: 20px; width: 120px; height: 60px; background: #000');
    const c = document.createElement('canvas');
    c.setAttribute('id', 'cv-alpha');
    c.setAttribute('style', 'display: block; width: 120px; height: 60px');
    c.width = 4; c.height = 2;
    wrap.appendChild(c);
    document.body.appendChild(wrap);
    const ctx = c.getContext('2d');
    // Bitmap column 0: white at alpha 0.5. Column 1: nothing.
    ctx.fillStyle = 'rgba(255, 255, 255, 0.5)'; ctx.fillRect(0, 0, 1, 2);
    // Columns 2 and 3: an opaque white rect from x = 2.5 to 3.5, so each
    // column is half covered: the antialiased edge of a shape.
    ctx.fillStyle = '#fff'; ctx.fillRect(2.5, 0, 1, 2);
    await new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)));
    return [...ctx.getImageData(0, 0, 4, 1).data];
  `);
    const data = (got as { ok: number[] }).ok;
    expect(data.slice(0, 4)).toEqual([255, 255, 255, 128]);
    expect(Math.abs(data[11]! - 128)).toBeLessThanOrEqual(2);

    const b = (await app.query("#cv-alpha")).nodes[0]!.bounds!;
    const shot = readPng(await app.screenshot());
    const scale = shot.width / Number(await app.eval("window.innerWidth"));
    const grey = (i: number) => shot.at(Math.round((b.x + 30 * i + 15) * scale), Math.round((b.y + 30) * scale))[1];
    const seen = { fill: grey(0), empty: grey(1), edgeA: grey(2), edgeB: grey(3) };
    const near = (v: number, want: number) => Math.abs(v - want) <= 12;
    expect({
      fill: near(seen.fill, 128),
      empty: near(seen.empty, 0),
      edgeA: near(seen.edgeA, 128),
      edgeB: near(seen.edgeB, 128),
    }).toEqual({ fill: true, empty: true, edgeA: true, edgeB: true });
  },
  T,
);

test(
  "a mirror <img> draws its decoded pixels: data:, blob: and http(s), origin recorded",
  async () => {
    const base = `http://127.0.0.1:${server.port}`;
    const got = await inPage(`
    const load = (src, attrs) => new Promise((res, rej) => {
      globalThis.__cvStep = "loading " + src.slice(0, 80);
      const i = document.createElement('img');
      for (const k in attrs || {}) i.setAttribute(k, attrs[k]);
      i.onload = () => res(i);
      i.onerror = () => rej(new Error('did not load: ' + src.slice(0, 48)));
      i.src = src;
    });
    const draw = (img) => {
      const c = document.createElement('canvas');
      c.width = img.naturalWidth; c.height = img.naturalHeight;
      const ctx = c.getContext('2d');
      ctx.drawImage(img, 0, 0);
      const px = (x) => Array.from(ctx.getImageData(x, 0, 1, 1).data).join();
      return [img.naturalWidth + 'x' + img.naturalHeight, px(0), px(c.width - 1), __rjs_canvas_info(ctx).originClean];
    };
    const src = document.createElement('canvas');
    src.width = 2; src.height = 1;
    const sx = src.getContext('2d');
    sx.fillStyle = '#f00'; sx.fillRect(0, 0, 1, 1);
    sx.fillStyle = '#00f'; sx.fillRect(1, 0, 1, 1);
    globalThis.__cvStep = "canvas.toBlob";
    const blob = await new Promise(r => src.toBlob(r, 'image/png'));
    const out = {
      data: draw(await load(src.toDataURL('image/png'))),
      blob: draw(await load(URL.createObjectURL(blob))),
      http: draw(await load('${base}/a.png')),
      cors: draw(await load('${base}/a.png?c', { crossorigin: 'anonymous' })),
      svg: draw(await load('${base}/a.svg')),
    };
    const bitmapSource = await load('${base}/a.png?b');
    globalThis.__cvStep = "createImageBitmap";
    const bmp = await createImageBitmap(bitmapSource);
    out.bitmap = [bmp.width, bmp.height];
    // Still loading: nothing drawn, nothing thrown.
    const pending = document.createElement('img');
    pending.src = '${base}/a.png?p';
    const pc = document.createElement('canvas').getContext('2d');
    pc.drawImage(pending, 0, 0);
    out.pending = Array.from(pc.getImageData(0, 0, 1, 1).data).join();
    // Broken: InvalidStateError.
    const b = document.createElement('img');
    globalThis.__cvStep = "waiting for missing.png error";
    const event = await new Promise((resolve, reject) => {
      b.onerror = e => resolve(e.type);
      b.onload = () => reject(new Error("missing.png unexpectedly loaded"));
      b.src = '${base}/missing.png';
    });
    try { pc.drawImage(b, 0, 0); out.broken = [event, 'drew']; } catch (e) { out.broken = [event, e.name]; }
    return out;
  `);
    const RED = "255,0,0,255";
    const BLUE = "0,0,255,255";
    expect(got).toEqual({
      ok: {
        data: ["2x1", RED, BLUE, true],
        blob: ["2x1", RED, BLUE, true],
        http: ["2x1", RED, BLUE, false],
        cors: ["2x1", RED, BLUE, true],
        svg: ["4x2", RED, BLUE, false],
        bitmap: [2, 1],
        pending: "0,0,0,0",
        broken: ["error", "InvalidStateError"],
      },
    });
  },
  T,
);

test(
  "a <canvas> is an instance of HTMLCanvasElement, and nothing else is",
  async () => {
    const got = await inPage(`
    const made = document.createElement('canvas');
    const mounted = document.body.appendChild(document.createElement('canvas'));
    const out = [
      made instanceof HTMLCanvasElement,
      mounted instanceof HTMLCanvasElement,
      made instanceof HTMLElement,
      document.createElement('div') instanceof HTMLCanvasElement,
      ({}) instanceof HTMLCanvasElement,
    ];
    mounted.remove();
    return out;
  `);
    expect(got).toEqual({ ok: [true, true, true, false, false] });
  },
  T,
);

test(
  "the app logged nothing broken",
  async () => {
    await app.expectNoErrors({ ignore: [/missing\.png/] });
  },
  T,
);
