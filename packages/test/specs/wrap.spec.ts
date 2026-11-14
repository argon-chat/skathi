import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/wrap/src/main.ts",
  window: [1280, 900],
  label: "wrap",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Box = { x: number; y: number; w: number; h: number; right: number };
let cache: Record<string, Box> | undefined;
async function boxes(): Promise<Record<string, Box>> {
  if (!cache) {
    await sleep(300);
    cache = JSON.parse(await app.eval(`JSON.stringify(__SK_TEST_WRAP__.boxes())`));
  }
  return cache!;
}
const lines = (b: Box) => Math.round(b.h / 20);

describe("long tokens in Argon's message bubble", () => {
  test(
    "a link in a narrow column wraps inside the column",
    async () => {
      const b = await boxes();
      const column = b.narrow!;
      expect(b["body-n-url"]!.w).toBe(216);
      for (const id of ["rb-n-url", "bw-n-url", "b-n-url"]) {
        expect(b[id]!.right).toBeLessThanOrEqual(column.right);
      }
      expect(b["u-n-url"]!.w).toBeLessThanOrEqual(216 - 24);
      expect(b["u-n-url"]!.h).toBeGreaterThan(b["u-url"]!.h);
    },
    T,
  );

  test(
    "a wide column's bubble stops at its max-width and the link wraps in it",
    async () => {
      const b = await boxes();
      for (const id of ["url", "plain", "hash"]) {
        expect(b[`b-${id}`]!.w).toBeLessThanOrEqual(520);
        expect(b[`t-${id}`]!.w).toBeLessThanOrEqual(496);
      }
      expect(b["t-plain"]!.h).toBeGreaterThan(40);
      expect(b["t-hash"]!.h).toBeGreaterThan(20);
    },
    T,
  );

  test(
    "a word that may not break lower than its bubble's max-width holds the row open",
    async () => {
      const b = await boxes();
      expect(b["body-n-plain"]!.w).toBe(216);
      expect(b["b-n-plain"]!.w).toBe(520);
      expect(b["t-n-plain"]!.h).toBeGreaterThan(40);
    },
    T,
  );
});

describe("one property each, in 120px", () => {
  test(
    "overflow-wrap: normal lets a long word overflow; break-word and anywhere break it",
    async () => {
      const b = await boxes();
      expect(lines(b["x-normal"]!)).toBe(1);
      expect(lines(b["x-break-word"]!)).toBe(3);
      expect(lines(b["x-anywhere"]!)).toBe(3);
      expect(lines(b["x-legacy"]!)).toBe(3);
    },
    T,
  );

  test(
    "word-break: break-all fills each line; keep-all keeps CJK together",
    async () => {
      const b = await boxes();
      expect(lines(b["x-normal-words"]!)).toBe(3);
      expect(lines(b["x-break-all"]!)).toBe(2);
      expect(lines(b["x-cjk"]!)).toBe(2);
      expect(lines(b["x-keep-all"]!)).toBe(1);
    },
    T,
  );

  test(
    "nowrap and pre do not break even under anywhere; pre-wrap does",
    async () => {
      const b = await boxes();
      expect(lines(b["x-nowrap"]!)).toBe(1);
      expect(lines(b["x-pre"]!)).toBe(1);
      expect(lines(b["x-pre-wrap"]!)).toBe(3);
    },
    T,
  );

  test(
    "a soft hyphen is a break opportunity unless hyphens: none; so is <wbr>",
    async () => {
      const b = await boxes();
      expect(lines(b["x-shy"]!)).toBe(2);
      expect(lines(b["x-shy-none"]!)).toBe(1);
      expect(lines(b["x-wbr"]!)).toBe(2);
    },
    T,
  );

  test(
    "anywhere lowers a flex item's min-content to a letter; break-word does not",
    async () => {
      const b = await boxes();
      expect(b["mc-anywhere"]!.w).toBe(120);
      expect(lines(b["mc-anywhere"]!)).toBeGreaterThan(1);
      expect(b["mc-break-word"]!.w).toBeGreaterThan(120);
      expect(lines(b["mc-break-word"]!)).toBe(1);
    },
    T,
  );
});

describe("fit-content in a column that does not stretch", () => {
  test(
    "a link, a line of text with a link, and a bubble all fit the 200px column",
    async () => {
      const b = await boxes();
      for (const id of ["fc-a", "fc-text", "u-fc-text", "fc-bubble"]) {
        expect(b[id]!.w).toBeLessThanOrEqual(200);
      }
      expect(b["fc-bubble"]!.w).toBe(200);
      expect(b["u-fc-bubble"]!.w).toBeLessThanOrEqual(176);
      expect(b["fc-a"]!.h).toBeGreaterThan(100);
    },
    T,
  );
});

describe("selection across breaks inside a word", () => {
  async function bounds(sel: string) {
    return (await app.query(sel)).nodes[0]!.bounds!;
  }
  async function mouse(x: number, y: number, kind: string, extra: Record<string, unknown> = {}) {
    await app.send("mouse", { x, y, kind, ...extra });
  }
  const selected = async () => String(await app.eval("String(getSelection())"));

  test(
    "a double click on the second line selects the whole broken word",
    async () => {
      const x = await bounds("#x-anywhere");
      await mouse(x.x + x.w / 2, x.y + 30, "down", { clicks: 2 });
      await mouse(x.x + x.w / 2, x.y + 30, "up", { clicks: 2 });
      await sleep(120);
      expect(await selected()).toBe("a".repeat(40));
    },
    T,
  );

  test(
    "a drag into the second line ends on that line, past the first's letters",
    async () => {
      const x = await bounds("#x-anywhere");
      const from = [x.x + 1, x.y + 10];
      const to = [x.x + x.w / 2, x.y + 30];
      await mouse(from[0]!, from[1]!, "down");
      for (let i = 1; i <= 4; i++) {
        const t = i / 4;
        await mouse(from[0]! + (to[0]! - from[0]!) * t, from[1]! + (to[1]! - from[1]!) * t, "move", { held: true });
        await sleep(30);
      }
      await mouse(to[0]!, to[1]!, "up");
      await sleep(120);
      const s = await selected();
      expect(s).toMatch(/^a+$/);
      expect(s.length).toBeGreaterThan(16);
      expect(s.length).toBeLessThan(32);
    },
    T,
  );
});
