import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch, readPng, type VnApp } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/blob-url/src/main.ts",
  window: [420, 200],
  label: "blob-url",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function redShare(a: VnApp, sel: string): Promise<number> {
  const box = (await a.query(sel)).nodes[0]!.bounds!;
  const shot = readPng(await a.screenshot());
  const scale = shot.width / Number(await a.eval("window.innerWidth"));
  const rect = { x: (box.x + 8) * scale, y: (box.y + 8) * scale, w: (box.w - 16) * scale, h: (box.h - 16) * scale };
  const red = shot.count(rect, (_l, [r, g, b]) => r > 200 && g < 60 && b < 60);
  return red / (rect.w * rect.h);
}

describe("blob: URLs on the host", () => {
  test(
    "the app made its URLs, and fetch() of a live one reads its bytes",
    async () => {
      await expect(app.locator("#status")).toHaveText("ready");
      const urls = JSON.parse((await app.eval("JSON.stringify(__VN_TEST_BLOB__)")) as string);
      bunExpect(urls.live).toMatch(/^blob:app:\/\/redjs\/[0-9a-f-]{36}$/);
      bunExpect(urls.results.fetch).toBe("200|image/png|75|true");
    },
    T,
  );

  test(
    "new Image() and createImageBitmap resolve the same URLs through redjs's store",
    async () => {
      const { results } = JSON.parse((await app.eval("JSON.stringify(__VN_TEST_BLOB__)")) as string);
      bunExpect(results.image).toBe("load|8|8|true");
      bunExpect(results.imageSliced).toBe("load|8|8|true");
      bunExpect(results.imageRevoked).toBe("error");
      bunExpect(results.bitmapUrl).toBe("8|8");
      bunExpect(results.bitmapRevoked).toBe("ERR TypeError");
      bunExpect(results.bitmapBlob).toBe("8|8");
    },
    T,
  );

  test(
    "an <img> paints a blob: URL made from bytes in memory",
    async () => {
      let share = 0;
      for (let i = 0; i < 40 && share < 0.8; i++) {
        await sleep(100);
        share = await redShare(app, "#live");
      }
      bunExpect(share).toBeGreaterThan(0.8);
    },
    T,
  );

  test(
    "a slice's URL paints the window it names",
    async () => {
      bunExpect(await redShare(app, "#sliced")).toBeGreaterThan(0.8);
    },
    T,
  );

  test(
    "a revoked URL is a broken image with one advisory saying why",
    async () => {
      bunExpect(await redShare(app, "#revoked")).toBeLessThan(0.05);
      const { entries } = await app.logs(0);
      const said = entries.filter((e) => e.kind === "advisory" && e.text.includes("not in the blob URL store"));
      bunExpect(said.length).toBe(1);
      bunExpect(
        entries.filter(
          (e) => e.kind === "advisory" && e.text.includes("blob:") && !e.text.includes("not in the blob URL store"),
        ),
      ).toEqual([]);
    },
    T,
  );

  test(
    "the app logged nothing broken",
    async () => {
      await app.expectNoErrors({ ignore: [/not in the blob URL store/] });
    },
    T,
  );
});
