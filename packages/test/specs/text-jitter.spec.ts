import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { type Bounds, launch, ON_SCREEN, readPng, SCREENSHOTS, type SkApp, WINDOWS } from "../src/index";

// Content that does not change must land on the same device pixels on every frame, whichever path
// lays it out (inline, or alone under a retained-render cache boundary) and wherever a fractional
// layout or scroll offset puts it.
const T = 60_000;
const SAMPLES = 10;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Shot = {
  png: ReturnType<typeof readPng>;
  x: number;
  y: number;
  scale: number;
};

for (const scale of WINDOWS ? [1, 1.25, 1.5] : [undefined]) {
  describe(`at scale ${scale ?? "native"}`, () => {
    let app: SkApp;
    beforeAll(async () => {
      app = await launch({
        app: "packages/test/fixtures/text-jitter/src/main.ts",
        window: [1280, 800],
        label: "text-jitter",
        ...(scale === undefined ? {} : { scale }),
      });
    }, T);
    afterAll(() => app.close());

    async function boxes(selector: string): Promise<Bounds[]> {
      return (await app.query(selector)).nodes.flatMap((n) => (n.bounds ? [n.bounds] : []));
    }

    async function box(selector: string): Promise<Bounds> {
      const b = (await boxes(selector))[0];
      if (!b) throw new Error(`${selector} has no box`);
      return b;
    }

    async function shoot(): Promise<Shot> {
      const path = join(tmpdir(), `sk-text-jitter-${process.pid}.png`);
      const r = (await app.send("screenshot", {
        path,
        backend: ON_SCREEN,
      })) as {
        clientX?: number;
        clientY?: number;
        scale: number;
      };
      return {
        png: readPng(path),
        x: r.clientX ?? 0,
        y: r.clientY ?? 0,
        scale: r.scale,
      };
    }

    /** The device pixels of a box, which is laid out on whole device pixels. */
    function crop(shot: Shot, b: Bounds): number[] {
      const out: number[] = [];
      const [x0, y0] = [Math.round(b.x * shot.scale) + shot.x, Math.round(b.y * shot.scale) + shot.y];
      const [w, h] = [Math.round(b.w * shot.scale), Math.round(b.h * shot.scale)];
      for (let y = y0; y < y0 + h; y++) for (let x = x0; x < x0 + w; x++) out.push(...shot.png.at(x, y));
      return out;
    }

    async function pixels(regions: Bounds[]): Promise<number[]> {
      const shot = await shoot();
      return regions.flatMap((b) => crop(shot, b));
    }

    /** Distinct images, where the GPU's float noise on an antialiased edge (a level or two) is not a difference. */
    class Images {
      readonly seen: number[][] = [];
      add(img: number[]) {
        const same = (a: number[]) => a.length === img.length && a.every((v, i) => Math.abs(v - img[i]!) <= 2);
        if (!this.seen.some(same)) this.seen.push(img);
      }
      get size() {
        return this.seen.length;
      }
    }

    async function show(page: string) {
      await app.eval(`__SK_TEST_JITTER__.stop(); __SK_TEST_JITTER__.show(${JSON.stringify(page)}); 'ok'`);
      await sleep(500);
    }

    const toggleTicker = () => {
      let on = false;
      return async () => {
        on = !on;
        await app.eval(`__SK_TEST_JITTER__.${on ? "start" : "stop"}(); 'ok'`);
        await sleep(150);
      };
    };

    /** Distinct images of `regions`, before and after each change. */
    async function sample(change: () => Promise<void>, regions: Bounds[]): Promise<number> {
      const seen = new Images();
      seen.add(await pixels(regions));
      for (let i = 0; i < SAMPLES; i++) {
        await change();
        seen.add(await pixels(regions));
      }
      return seen.size;
    }

    const chat = async () => [await box(".channel"), await box(".messages")];

    test.skipIf(!SCREENSHOTS)(
      "static text keeps its device pixels while a sibling subtree starts and stops changing every frame",
      async () => {
        await sleep(800);
        const seen = await sample(toggleTicker(), await chat());
        await app.eval("__SK_TEST_JITTER__.stop(); 'ok'");
        expect(Number(await app.eval("__SK_TEST_JITTER__.tick()"))).toBeGreaterThan(SAMPLES);
        expect(seen).toBe(1);
      },
      T,
    );

    test.skipIf(!SCREENSHOTS)(
      "typing into the composer leaves the messages and avatars where they were",
      async () => {
        await sleep(300);
        const seen = await sample(
          async () => {
            await app.locator("#ed").type("a");
            await sleep(120);
          },
          await chat(),
        );
        expect(seen).toBe(1);
      },
      T,
    );

    test.skipIf(!SCREENSHOTS)(
      "boxes sized by flex shares and percentages keep their pixels while a sibling changes",
      async () => {
        await show("fractions");
        const seen = await sample(toggleTicker(), await boxes(".tile"));
        await app.eval("__SK_TEST_JITTER__.stop(); 'ok'");
        expect(seen).toBe(1);
      },
      T,
    );

    test.skipIf(!SCREENSHOTS)(
      "rows scrolled by fractional steps move as rigid blocks",
      async () => {
        await show("scroll");
        const ids = [2, 3, 4, 5, 6, 7];
        const seen = new Map(ids.map((id) => [id, new Images()]));
        for (let i = 0; i < 12; i++) {
          await app.eval(`document.getElementById('scroller').scrollTop = ${10 + i * 0.37}; 'ok'`);
          await sleep(150);
          const rows = await Promise.all(ids.map((id) => box(`#row-${id}`)));
          const shot = await shoot();
          // Short of the right edge, where the overlay scrollbar fades in and out.
          rows.forEach((b, i) => seen.get(ids[i]!)!.add(crop(shot, { ...b, w: b.w - 40 })));
        }
        expect(Number(await app.eval("document.getElementById('scroller').scrollTop"))).toBeGreaterThan(13);
        for (const [id, images] of seen) expect([id, images.size]).toEqual([id, 1]);
      },
      T,
    );

    test.skipIf(!SCREENSHOTS)(
      "an editor atom keeps its pixels while a sibling changes and text is typed after it",
      async () => {
        await show("atom");
        const atom = await boxes(".mention");
        const seen = await sample(toggleTicker(), atom);
        await app.eval("__SK_TEST_JITTER__.stop(); 'ok'");
        await app.locator("#aed").press("End");
        const typed = await sample(async () => {
          await app.locator("#aed").type("x");
          await sleep(120);
        }, atom);
        expect(seen).toBe(1);
        expect(typed).toBe(1);
      },
      T,
    );
  });
}
