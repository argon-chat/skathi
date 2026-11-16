// Dev-build performance advisories: a sheet whose template reads the scroll offset re-renders every cell on
// each scroll, which must reach the console once and the recorded frames as hints.

import { afterAll, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/perf-advice/src/main.ts",
  window: [720, 520],
  label: "perf-advice",
});
afterAll(() => app.close());

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

interface Hint {
  code: string;
  message: string;
}
interface FramePage {
  nextOffset: number | null;
  frames: { hints: Hint[] }[];
}

test("scrolling a sheet whose template reads the scroll offset is a wasted re-render and a long list", async () => {
  await app.eval(`document.querySelector(".scroller").scrollTop = 24`);
  await sleep(300);
  await app.send("trace", { op: "recording-start" });
  for (let i = 2; i <= 6; i++) {
    await app.eval(`document.querySelector(".scroller").scrollTop = ${i * 96}`);
    await sleep(120);
  }
  await app.send("trace", { op: "recording-stop" });

  const hints: Hint[] = [];
  for (let offset: number | null = 0; offset !== null;) {
    const page: FramePage = await app.send("trace", { op: "frames", offset });
    for (const frame of page.frames) hints.push(...frame.hints);
    offset = page.nextOffset;
  }
  const wasted = hints.find((h) => h.code === "wasted-render");
  expect(wasted?.message).toMatch(
    /^<Sheet> \(\S*Sheet\.vue\) re-rendered \d+ elements on scroll and \d+ of them changed/,
  );
  expect(wasted?.message).toContain("It was triggered by scrollY.");
  expect(hints.some((h) => h.code === "long-list")).toBe(true);

  const { entries } = await app.logs(0);
  expect(entries.filter((e) => e.text.includes("[skathi perf] <Sheet>") && e.text.includes(" on scroll ")).length).toBe(
    1,
  );
});
