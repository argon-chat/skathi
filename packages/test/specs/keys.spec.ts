import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/keys/src/main.ts",
  window: [420, 300],
  label: "keys",
});
afterAll(async () => {
  await app.close();
});

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function logOf(run: () => Promise<void>): Promise<string[]> {
  await app.eval("__SK_TEST_KEYS__.log.length = 0");
  await run();
  await sleep(200);
  return JSON.parse(String(await app.eval("JSON.stringify(__SK_TEST_KEYS__.log)"))) as string[];
}

describe("one key, one keydown", () => {
  test(
    "a key on a focused box that listens for nothing: once, at the box, through each listening ancestor",
    async () => {
      const log = await logOf(() => app.locator("#inner").press("x"));
      bunExpect(log).toEqual([
        "box:keydown:x t=inner",
        "wrap:keydown:x t=inner",
        "document:keydown:x t=inner",
        "document:keyup:x t=inner",
      ]);
    },
    T,
  );

  test(
    "Escape in a field that closes itself reaches document once, and nothing after it is gone",
    async () => {
      const log = await logOf(() => app.locator("#field").press("Escape"));
      bunExpect(log.filter((l) => l.includes(":keydown:"))).toEqual([
        "field:keydown:Escape t=field",
        "document:keydown:Escape t=field",
      ]);
      bunExpect(log.filter((l) => l.startsWith("document:keyup:Escape")).length).toBe(1);
      bunExpect((await app.query("#field")).count).toBe(0);
    },
    T,
  );

  test(
    "a field with no listener of its own: the key is still its, once",
    async () => {
      const log = await logOf(() => app.locator("#plain").press("Escape"));
      bunExpect(log).toEqual(["document:keydown:Escape t=plain", "document:keyup:Escape t=plain"]);
    },
    T,
  );

  test(
    "typing several keys in one go delivers every one, once each",
    async () => {
      await app.locator("#reopen").click();
      await sleep(200);
      const log = await logOf(() => app.locator("#field").type("abc"));
      const downs = log.filter((l) => l.startsWith("document:keydown:"));
      bunExpect(downs).toEqual([
        "document:keydown:a t=field",
        "document:keydown:b t=field",
        "document:keydown:c t=field",
      ]);
      bunExpect(log.filter((l) => l.startsWith("field:keydown:")).length).toBe(3);
    },
    T,
  );

  test(
    "a field's keydown handler may ask where the focus is and focus the field",
    async () => {
      // An emoji suggestion list picked with the arrows: Argon crashed with "cannot read
      // TextInputState while it is already being updated".
      const log = await logOf(async () => {
        await app.locator("#suggest").press("ArrowDown");
        await app.locator("#suggest").press("ArrowUp");
        await app.locator("#suggest").press("Enter");
      });
      bunExpect(log.filter((l) => l.startsWith("suggest:"))).toEqual([
        "suggest:keydown:ArrowDown active=suggest",
        "suggest:keydown:ArrowUp active=suggest",
        "suggest:keydown:Enter active=suggest",
      ]);
      bunExpect(await app.eval("1 + 1")).toBe(2);
    },
    T,
  );
});
