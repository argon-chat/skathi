// `appWindow.theme` as AppKit appearance on macOS: `NSApp.appearance` and each window's own, read
// through the devtools `windows` command.

import { afterAll, describe, expect, test } from "bun:test";
import { launch, MACOS } from "../src/index";

if (!MACOS) {
  describe("window appearance", () => {
    test.skip("macOS only: other platforms have no NSAppearance", () => {});
  });
} else {
  const app = await launch({ app: "packages/test/fixtures/window-appearance/src/main.ts", label: "window-appearance" });
  afterAll(() => app.close());

  const T = 30_000;
  const DARK = "NSAppearanceNameDarkAqua";
  const LIGHT = "NSAppearanceNameAqua";
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

  type Appearance = {
    window: string | null;
    effective: string | null;
    app: string | null;
  };
  type Entry = { kind: string; title: string; appearance?: Appearance };

  async function appearance(title: "main" | "Pane" | "Later"): Promise<Appearance | undefined> {
    const { windows } = await app.send("windows", { native: true });
    const w = (windows as Entry[]).find((w) => (title === "main" ? w.kind === "main" : w.title === title));
    return w?.appearance;
  }
  async function until<V>(what: string, probe: () => Promise<V>, ms = 8000): Promise<NonNullable<V>> {
    const deadline = Date.now() + ms;
    for (;;) {
      const v = await probe();
      if (v) return v;
      if (Date.now() > deadline) throw new Error(`timed out waiting for ${what} (last: ${JSON.stringify(v)})`);
      await sleep(50);
    }
  }
  type Seen = { theme: string; media: boolean; changes: boolean[] };
  const seen = async (): Promise<Seen> =>
    JSON.parse(
      await app.eval(`JSON.stringify({
        theme: __SK_WINDOW__.appWindow.theme,
        media: matchMedia("(prefers-color-scheme: dark)").matches,
        changes: __SK_TEST_APPEARANCE__.changes,
      })`),
    );
  const setTheme = async (t: string) => {
    await app.eval(`__SK_WINDOW__.appWindow.theme = ${JSON.stringify(t)}; "ok"`);
    await sleep(400);
  };

  describe("window appearance", () => {
    test(
      "window.theme from sk.config.json is on the main window, a pop-out and NSApp from the start",
      async () => {
        const pane = await until("the pop-out", () => appearance("Pane"));
        const main = (await appearance("main"))!;
        expect(main.window).toBe(DARK);
        expect(main.app).toBe(DARK);
        expect(pane.window).toBe(DARK);
        const s = await seen();
        expect(s.theme).toBe("dark");
        expect(s.media).toBe(true);
        expect(s.changes).toEqual([]);
      },
      T,
    );

    test(
      "light moves every window and NSApp, reads back, and fires change once",
      async () => {
        await setTheme("light");
        for (const w of ["main", "Pane"] as const) {
          const a = (await appearance(w))!;
          expect(a.window).toBe(LIGHT);
          expect(a.effective).toBe(LIGHT);
          expect(a.app).toBe(LIGHT);
        }
        const s = await seen();
        expect(s.theme).toBe("light");
        expect(s.media).toBe(false);
        expect(s.changes).toEqual([false]);
      },
      T,
    );

    test(
      "a non-activating pop-out opened later takes the current theme",
      async () => {
        await app.eval(`__SK_TEST_APPEARANCE__.S.later = true; "ok"`);
        const later = await until("the later pop-out", () => appearance("Later"));
        expect(later.window).toBe(LIGHT);
        expect(later.effective).toBe(LIGHT);
      },
      T,
    );

    test(
      "system clears them, and prefers-color-scheme is the OS's again",
      async () => {
        await setTheme("system");
        const main = (await appearance("main"))!;
        expect(main.window).toBeNull();
        expect(main.app).toBeNull();
        expect((await appearance("Pane"))!.window).toBeNull();
        const osDark = main.effective === DARK;
        const s = await seen();
        expect(s.theme).toBe("system");
        expect(s.media).toBe(osDark);
        expect(s.changes).toEqual(osDark ? [false, true] : [false]);
      },
      T,
    );

    test(
      "a material backdrop keeps its effectiveBackdrop under a forced theme",
      async () => {
        const effective = async () => (await app.eval(`__SK_WINDOW__.appWindow.effectiveBackdrop`)) as string;
        await app.eval(`__SK_WINDOW__.appWindow.backdrop = "acrylic"; "ok"`);
        await sleep(400);
        const drawn = await effective();
        for (const t of ["dark", "light", "system"]) {
          await setTheme(t);
          expect(await effective()).toBe(drawn);
        }
      },
      T,
    );
  });
}
