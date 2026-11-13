import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/call-timer/src/main.ts",
  window: [200, 80],
  label: "window-theme",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Seen = { theme: string; dwm: boolean | null; media: boolean; changes: boolean[] };
const seen = async (): Promise<Seen> =>
  JSON.parse(
    await app.eval(`(() => {
      const s = JSON.parse(__vn_window_state());
      return JSON.stringify({
        theme: __VN_WINDOW__.appWindow.theme,
        dwm: s.systemDarkMode,
        media: matchMedia("(prefers-color-scheme: dark)").matches,
        changes: globalThis.__themeChanges,
      });
    })()`),
  );
const setTheme = async (t: string) => {
  await app.eval(`__VN_WINDOW__.appWindow.theme = ${JSON.stringify(t)}; "ok"`);
  await sleep(300);
};

describe("appWindow.theme", () => {
  let os: Seen;

  test("starts at `system`, with DWM and the media query on the machine's own setting", async () => {
    await sleep(300);
    await app.eval(`(() => {
      globalThis.__themeChanges = [];
      matchMedia("(prefers-color-scheme: dark)").addEventListener("change", (e) => __themeChanges.push(e.matches));
      return "ok";
    })()`);
    os = await seen();
    expect(os.theme).toBe("system");
    expect(os.dwm).toBe(os.media);
  }, T);

  test("forcing the other scheme moves DWM and prefers-color-scheme, and fires change", async () => {
    const other = os.media ? "light" : "dark";
    await setTheme(other);
    const s = await seen();
    expect(s.theme).toBe(other);
    expect(s.dwm).toBe(!os.media);
    expect(s.media).toBe(!os.media);
    expect(s.changes).toEqual([!os.media]);
  }, T);

  test("`system` hands both back to the OS", async () => {
    await setTheme("system");
    const s = await seen();
    expect(s.theme).toBe("system");
    expect(s.dwm).toBe(os.media);
    expect(s.media).toBe(os.media);
    expect(s.changes).toEqual([!os.media, os.media]);
  }, T);

  test("an unknown name is advised and changes nothing", async () => {
    await setTheme("Dark");
    const s = await seen();
    expect(s.theme).toBe("system");
    const { entries } = await app.logs(0);
    expect(entries.some((l) => /unknown `theme`/.test(l.text))).toBe(true);
  }, T);
});
