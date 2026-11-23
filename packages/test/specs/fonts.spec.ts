// Faces load lazily, when text drawn with them asks, so the widths also say when a face arrived.

import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/fonts/src/main.ts",
  window: [480, 240],
  label: "fonts",
});
afterAll(() => app.close());

const T = 30_000;
const PIXEL = 128;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<V>(read: () => Promise<V>, ok: (v: V) => boolean, ms = 10_000): Promise<V> {
  const deadline = Date.now() + ms;
  let v = await read();
  while (!ok(v) && Date.now() < deadline) {
    await sleep(50);
    v = await read();
  }
  return v;
}

const width = async (sel: string) => (await app.query(sel)).nodes[0]?.bounds?.w ?? -1;
const result = async (name: string) =>
  JSON.parse(String(await app.eval(`JSON.stringify(__SK_TEST_FONTS__.results[${JSON.stringify(name)}] ?? null)`)));
const run = async (name: string) => {
  await app.eval(`__SK_TEST_FONTS__.run(${JSON.stringify(name)})`);
  return until(
    () => result(name),
    (v) => v !== "pending" && v !== null,
  );
};

const status = async (family: string) =>
  String(await app.eval(`[...document.fonts].find((f) => f.family === ${JSON.stringify(family)}).status`));

describe("@font-face in the stylesheet", () => {
  test(
    "the sheet's faces are document.fonts from boot, load when text asks, and ready waits for them",
    async () => {
      const boot = await result("boot");
      expect(boot.faces).toEqual([
        "SK Pixel:unloaded",
        "SK Tengwar:unloaded",
        "SK Late:unloaded",
        "SK Unused:unloaded",
      ]);
      expect(boot.size).toBe(4);
      expect(boot.status).toBe("loaded");
      // Latin text never asks for the PUA-only face.
      const ready = await until(
        () => result("ready"),
        (v) => v !== null,
      );
      expect(ready).toEqual({
        same: true,
        status: "loaded",
        faces: ["SK Pixel:loaded", "SK Tengwar:unloaded", "SK Late:unloaded", "SK Unused:unloaded"],
      });
      const events = await app.eval("JSON.stringify(__SK_TEST_FONTS__.events)");
      expect(JSON.parse(String(events))).toEqual(["loading:SK Pixel", "loadingdone:SK Pixel"]);
    },
    T,
  );

  test(
    "text in the declared family is drawn in the font, under the family it was declared as",
    async () => {
      expect(
        await until(
          () => width("#pixel"),
          (w) => w === PIXEL,
        ),
      ).toBe(PIXEL);
      expect(await width("#plain")).not.toBe(PIXEL);
    },
    T,
  );

  test(
    "an inline style that names the family is parsed again once the font is in",
    async () => {
      expect(
        await until(
          () => width("#inline"),
          (w) => w === PIXEL,
        ),
      ).toBe(PIXEL);
    },
    T,
  );

  test(
    "a PUA-only family that leads a stack leaves the Latin text to the next family",
    async () => {
      expect(
        await until(
          () => width("#stack"),
          (w) => w === PIXEL,
        ),
      ).toBe(PIXEL);
      const { entries } = await app.logs(0);
      expect(entries.filter((e) => e.kind === "advisory" && e.text.includes("SK Tengwar"))).toEqual([]);
    },
    T,
  );

  test(
    "a face loads when text is first drawn with it, and the text swaps to it",
    async () => {
      expect(await width("#late")).toBe(-1);
      const r = await run("late");
      expect(r.before).toBe("unloaded");
      expect(r.fallback).toBeGreaterThan(0);
      expect(r.fallback).not.toBe(PIXEL);
      expect(r.status).toBe("loaded");
      expect(
        await until(
          () => width("#late"),
          (w) => w === PIXEL,
        ),
      ).toBe(PIXEL);
      // A family whose rule matches only an empty box never loads.
      expect(r.unused).toBe("unloaded");
    },
    T,
  );

  test(
    "check() and load() select by family and text; the WOFF2 face loaded",
    async () => {
      const r = await run("check");
      expect(r).toEqual({
        pixel: true,
        tengwarPua: true,
        loaded: ["SK Tengwar:loaded:U+E000-E0FF"],
        system: true,
        bad: "SyntaxError",
      });
    },
    T,
  );
});

describe("FontFace from script", () => {
  test(
    "an ArrayBuffer face loads at once, joins the set, and an element naming it takes it",
    async () => {
      expect(await width("#script")).not.toBe(PIXEL);
      const r = await run("script");
      expect(r).toEqual({ first: "loading", status: "loaded", has: true, size: 5, check: true });
      expect(
        await until(
          () => width("#script"),
          (w) => w === PIXEL,
        ),
      ).toBe(PIXEL);
    },
    T,
  );

  test(
    "a face whose file is not there is an error, and its load() rejects",
    async () => {
      expect(await run("missing")).toEqual({ name: "NetworkError", status: "error" });
    },
    T,
  );

  test(
    "the app logged nothing broken, and no missing-font advisory for a declared family",
    async () => {
      expect(await status("SK Unused")).toBe("unloaded");
      const { entries } = await app.logs(0);
      expect(
        entries.filter((e) => e.text.includes("missing-font") || /no font installed for `font-family: SK/.test(e.text)),
      ).toEqual([]);
      expect(entries.some((e) => e.kind === "advisory" && e.text.includes("local() is not consulted"))).toBe(true);
      await app.expectNoErrors({ ignore: [/SK Missing/] });
    },
    T,
  );
});

describe("text that no installed face covers", () => {
  test(
    "a long paragraph of it lays out without holding the frame",
    async () => {
      const started = Date.now();
      // Supplementary Private Use Area-A: no system face and no face of this fixture has it.
      await app.eval(`(() => {
        const el = document.createElement("div");
        el.id = "uncovered";
        el.style.width = "400px";
        el.textContent = "\\u{F0000} ".repeat(20000);
        document.body.appendChild(el);
        return 1;
      })()`);
      const height = (await app.query("#uncovered")).nodes[0]?.bounds?.h ?? 0;
      const elapsed = Date.now() - started;
      await app.eval(`document.getElementById("uncovered").remove(), 1`);
      expect(height).toBeGreaterThan(0);
      expect(elapsed).toBeLessThan(3_000);
    },
    T,
  );
});
