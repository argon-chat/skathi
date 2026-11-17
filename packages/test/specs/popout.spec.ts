// Cross-window clicks go through `__sk_window_click`, which feeds `gpui::PlatformInput` to that
// window's own `Window::dispatch_event` — the same entry point the Win32 message loop uses.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { tmpdir } from "node:os";
import { expect, launch, readPng, SCREENSHOTS } from "../src/index";

const app = await launch({ app: "popout", window: [620, 460], label: "popout" });
afterAll(() => app.close());

const T = 30_000;

type Popout = {
  node: number;
  windowId: number;
  title: string;
  chrome: string;
  width: number;
  height: number;
  focused: boolean;
};

const popouts = (): Promise<Popout[]> => app.eval("JSON.parse(__sk_window_popouts())");
const windows = () => app.send("windows");

async function until<T>(what: string, probe: () => Promise<T>, ms = 8000): Promise<NonNullable<T>> {
  const deadline = Date.now() + ms;
  let last: unknown;
  for (;;) {
    const v = await probe();
    if (v) return v;
    last = v;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what} (last: ${JSON.stringify(last)})`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

/**
 * The styles dump is gpui's own `StyleRefinement` debug, so `Hsla { h: 0.616…` is the blue
 * `#7aa2ff1f` and `h: 0.066…` the orange `#ff9d5c26`.
 */
async function backgroundHue(selector: string): Promise<number> {
  const dump = JSON.stringify(await app.locator(selector).styles());
  const m = /Hsla \{ h: ([0-9.]+)/.exec(dump);
  if (!m) throw new Error(`no background colour in the styles dump for ${selector}`);
  return Number(m[1]);
}

/**
 * `__sk_media_events(id)` DRAINS the element's event ring — it is the JS side's "what happened
 * since you last asked" — so a poller has to accumulate.
 */
const mediaSeen = new Map<number, Set<string>>();
async function mediaEvents(id: number): Promise<Set<string>> {
  const bag = mediaSeen.get(id) ?? new Set<string>();
  mediaSeen.set(id, bag);
  const raw: string = await app.eval(`__sk_media_events(${id})`);
  for (const name of raw.split(/[\s,]+/)) if (name) bag.add(name);
  return bag;
}

async function mediaState(id: number): Promise<Record<string, string>> {
  const raw: string = await app.eval(`__sk_media_state(${id})`);
  const out: Record<string, string> = {};
  for (const line of raw.split("\n")) {
    const i = line.indexOf("=");
    if (i > 0) out[line.slice(0, i)] = line.slice(i + 1);
  }
  return out;
}

/** The panel's rectangle in window A, captured before it is popped out. */
let inWindowA: { x: number; y: number; w: number; h: number };
let palette: Popout;
let wideHue: number;

describe("pop-out", () => {
  test(
    "the app mounts with the panel painted in the main window",
    async () => {
      await expect(app.locator(".panel")).toHaveCount(1);
      await expect(app.locator(".count")).toHaveText("0");
      await expect(app.locator(".mirror")).toContainText("count = 0");
      bunExpect(await popouts()).toEqual([]);

      inWindowA = await app.locator(".panel").boundingBox();
      bunExpect(inWindowA.y).toBeGreaterThan(100);
      bunExpect(inWindowA.w).toBeGreaterThan(500);

      // Captured here so the per-window test below is a comparison rather than a magic number.
      wideHue = await backgroundHue(".panel");
      bunExpect(wideHue).toBeGreaterThan(0.5);
    },
    T,
  );

  test(
    "state is shared before the split, so the two readouts agree",
    async () => {
      await app.locator(".inc-a").click();
      await expect(app.locator(".count")).toHaveText("1");
      await expect(app.locator(".mirror")).toContainText("count = 1");
    },
    T,
  );

  test(
    "popping out opens a SECOND OS window painting that subtree",
    async () => {
      const panelBefore = (await app.query(".panel")).nodes[0]!.id;

      await app.locator(".pop").click();

      const list = await until("the palette window to exist", async () => {
        const l = await popouts();
        return l.length === 1 ? l : null;
      });
      palette = list[0]!;
      bunExpect(palette.windowId).toBeGreaterThan(0);
      bunExpect(palette.node).not.toBe(panelBefore);
      bunExpect((await app.query(".panel")).nodes[0]!.id).toBe(panelBefore);
      bunExpect(palette.title).toBe("Palette - charcoal");
      bunExpect(palette.chrome).toBe("default");

      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.text.includes("popped out into window"))).toBe(true);

      await expect(app.locator(".hint")).toBeVisible();
      // A window opened visible reports its first show too, not only one created hidden.
      await expect(app.locator(".pal-shown")).toHaveText("1");
    },
    T,
  );

  test(
    "the subtree left the main window's layout but stayed in the tree",
    async () => {
      await expect(app.locator(".panel")).toHaveCount(1);
      await expect(app.locator(".count")).toHaveText("1");

      const box = await app.locator(".panel").boundingBox();
      bunExpect(box.w).toBe(380);
      bunExpect(box.w).not.toBe(inWindowA.w);
      bunExpect(box.y).toBeLessThan(inWindowA.y);
      bunExpect(box.y).toBeLessThan(20);
    },
    T,
  );

  test(
    "A REAL CLICK IN THE PALETTE WINDOW UPDATES TEXT IN WINDOW A",
    async () => {
      const box = await app.locator(".inc-b").boundingBox();
      const x = box.x + box.w / 2;
      const y = box.y + box.h / 2;

      const hit = await app.eval(`__sk_window_click(${palette.node}, ${x}, ${y})`);
      bunExpect(hit).toBe(true);

      await expect(app.locator(".count")).toHaveText("2");
      await expect(app.locator(".mirror")).toContainText("count = 2");

      await app.eval(`__sk_window_click(${palette.node}, ${x}, ${y})`);
      await expect(app.locator(".mirror")).toContainText("count = 3");
    },
    T,
  );

  test(
    "window A still drives the same state while the panel is away",
    async () => {
      await app.locator(".inc-a").click();
      await expect(app.locator(".mirror")).toContainText("count = 4");
      await expect(app.locator(".count")).toHaveText("4");
    },
    T,
  );

  test(
    "devtools addresses both windows, and the pop-out reports its own metrics",
    async () => {
      const data = await windows();
      bunExpect(data.windows.length).toBe(2);
      const [main, pop] = data.windows;
      bunExpect(main.kind).toBe("main");
      bunExpect(main.root).toBe(0);
      bunExpect(pop.kind).toBe("popout");
      bunExpect(pop.id).toBe(palette.windowId);
      bunExpect(pop.root).toBe(palette.node);
      bunExpect(pop.viewport.w).toBe(380);
      bunExpect(main.viewport.w).toBe(620);

      await expect(app.locator(".pal-size")).toHaveText("380x320");
    },
    T,
  );

  test(
    "@media answers against the window that PAINTS the element",
    async () => {
      // So the SAME element, at the SAME moment, in ONE stylesheet, takes the narrow branch —
      // because the cascade is now evaluated against the window that paints it.
      const narrowHue = await backgroundHue(".panel");
      bunExpect(narrowHue).toBeLessThan(0.2);
      bunExpect(wideHue).toBeGreaterThan(0.5);
    },
    T,
  );

  test(
    "@keyframes resolve against the painting window too, not the primary's",
    async () => {
      // It is `forwards`, so each element parks on its own answer and the `live` bucket — `base`
      // refined by what the animator holds — is a settled value rather than a race against a clock.
      await expect(app.locator(".pulse-palette")).toHaveEngineStyle("opacity", "0.8");
      await expect(app.locator(".pulse-main")).toHaveEngineStyle("opacity", "0.2");
    },
    T,
  );

  test(
    "…and a THIRD window opening does not hand the pop-out the primary's answers back",
    async () => {
      // The env now rides the walk and switches at the one node where the window can change: a
      // painted root (`windows::env_at_root`).
      await app.locator(".pop-notes").click();
      await until("the notes window to exist", async () => (await popouts()).find((p) => p.title === "Notes") ?? null);
      // The open is DEFERRED, and the pass that was wrong is the one it triggers a frame later — so
      // read the answer after it, not before.
      await new Promise((r) => setTimeout(r, 600));
      bunExpect(await backgroundHue(".panel")).toBeLessThan(0.2);

      await app.locator(".pop-notes").click();
      await until("the notes window to be gone", async () => (await popouts()).every((p) => p.title !== "Notes"));
      await new Promise((r) => setTimeout(r, 600));
      bunExpect(await backgroundHue(".panel")).toBeLessThan(0.2);
    },
    T,
  );

  test(
    "an <input> in the pop-out takes focus IN THAT WINDOW and receives keystrokes",
    async () => {
      await app.send("type", { selector: ".brush-name", text: "!" });
      // The edit reached the shared `ref`, so the MAIN window's readout moved.
      await expect(app.locator(".brush-mirror")).toContainText("charcoal!");
      await until("the caption to follow the prop", async () => {
        const [p] = await popouts();
        return p?.title === "Palette - charcoal!";
      });

      // Focus is per-window: the pop-out's input has it, and the main window's own focus was never
      // taken (`activate` defaults to false, so opening a palette does not steal the caret).
      const focused = await app.send("query", { selector: ".brush-name" });
      bunExpect(focused.count).toBe(1);
    },
    T,
  );

  test.skipIf(!SCREENSHOTS)(
    "a screenshot can be taken of the pop-out window by id",
    async () => {
      const shot = await app.send("screenshot", {
        path: `${tmpdir()}/sk-popout-palette.png`,
        window: palette.windowId,
      });
      const png = readPng(shot.path);
      bunExpect([png.width, png.height]).toEqual([Math.round(380 * shot.scale), Math.round(320 * shot.scale)]);
      bunExpect([shot.width, shot.height]).toEqual([png.width, png.height]);

      const unknown = app.send("screenshot", {
        path: `${tmpdir()}/sk-popout-none.png`,
        window: palette.windowId + 1000,
      });
      await bunExpect(unknown).rejects.toThrow();
    },
    T,
  );

  test(
    "closing the window returns the subtree and fires the JS callback",
    async () => {
      await app.locator(".back").click();

      await until("the pop-out to be gone", async () => (await popouts()).length === 0);

      // The `@close` emit ran — `palette` is false again, so the "bring it back" button is hidden
      // and the hint is gone.
      await expect(app.locator(".hint")).toBeHidden();
      // `v-if` unmounted the component, and `@close` still reached the app.
      await expect(app.locator(".pal-closed")).toHaveText("1");

      await expect(app.locator(".count")).toHaveText("4");
      await until("the panel to re-cascade against the main window", async () => (await backgroundHue(".panel")) > 0.5);
      await expect(app.locator(".brush-mirror")).toContainText("charcoal!");
      const box = await until("the panel to be laid out in window A again", async () => {
        const b = await app.locator(".panel").boundingBox();
        return b.y > 60 ? b : null;
      });
      bunExpect(box.w).toBeGreaterThan(500);

      const { entries } = await app.logs(0);
      bunExpect(entries.some((e) => e.text.includes("closed — the subtree is back"))).toBe(true);
    },
    T,
  );

  test(
    "a <video> plays inside a pop-out",
    async () => {
      await app.locator(".pop-player").click();
      const player = await until("the player window to exist", async () => {
        const l = await popouts();
        return l.find((p) => p.title === "Player") ?? null;
      });
      bunExpect(player.title).toBe("Player");

      const box = await until("the clip to be laid out in the player window", async () => {
        const b = await app.locator(".clip").boundingBox();
        return b && b.w > 0 ? b : null;
      });
      bunExpect(box.w).toBe(320);
      bunExpect(box.h).toBe(180);

      const id = (await app.query(".clip")).nodes[0]!.id;
      const events = await until("the video to report `playing`", async () => {
        const bag = await mediaEvents(id);
        return bag.has("playing") ? bag : null;
      });
      // `loadeddata` is the browser's "the first frame is available" edge, and it is the one the
      // atlas upload rides on — so it is worth naming.
      bunExpect([...events]).toContain("loadeddata");

      const state = await until("the transport clock to advance", async () => {
        const s = await mediaState(id);
        return Number(s.currentTime) > 0 ? s : null;
      });
      bunExpect(Number(state.videoWidth)).toBeGreaterThan(0);
      bunExpect(Number(state.readyState)).toBeGreaterThanOrEqual(2);
      bunExpect(state.paused).toBe("false");

      await app.locator(".pop-player").click();
      await until("the player to be gone", async () => (await popouts()).every((p) => p.title !== "Player"));
    },
    T,
  );

  test(
    "a pop-out can draw its OWN titlebar without disturbing the main window's",
    async () => {
      // The main window is `titleBarStyle: "default"`, so the OS owns its caption and it declares no
      // hitboxes of its own.
      const before = JSON.parse(await app.eval("__sk_window_hitboxes()"));
      bunExpect(before).toEqual([]);

      await app.locator(".pop-notes").click();
      const notes = await until("the notes window to exist", async () => {
        const l = await popouts();
        return l.find((p) => p.title === "Notes") ?? null;
      });
      bunExpect(notes.chrome).toBe("hidden");

      const boxes = await until("the notes window to paint its chrome", async () => {
        const list = JSON.parse(await app.eval(`__sk_window_hitboxes(${notes.windowId})`));
        return list.length ? list : null;
      });
      const areas: string[] = boxes.map((b: { area: string }) => b.area);
      bunExpect(areas).toContain("drag");
      bunExpect(areas).toContain("close");

      // Before phase 1 a pop-out's build cleared it, which is both a wrong answer and a live bug in
      // the fallback-titlebar retire rule.
      bunExpect(JSON.parse(await app.eval("__sk_window_hitboxes()"))).toEqual([]);

      await app.locator(".pop-notes").click();
      await until("the notes window to be gone", async () => (await popouts()).every((p) => p.title !== "Notes"));
    },
    T,
  );

  test(
    "el.click() on a `--window-control: close` button closes its window, as a real press does",
    async () => {
      await app.locator(".pop-notes").click();
      await until("the notes window to exist", async () => (await popouts()).find((p) => p.title === "Notes") ?? null);
      await expect(app.locator(".note-mirror")).toBeVisible();

      await app.eval(`document.querySelector(".bar-close").click(); "ok"`);

      await until("the notes window to be gone", async () => (await popouts()).every((p) => p.title !== "Notes"));
      await expect(app.locator(".note-mirror")).toBeHidden();
    },
    T,
  );

  test(
    "A-M03: a `position: fixed` box sits at the POP-OUT's edge, not the document's",
    async () => {
      await app.locator(".pop-notes").click();
      // Since JS runs without frames (the background tick), a devtools eval can land in that beat.
      const notes = await until("the notes window to be painted", async () => {
        const l = await popouts();
        return l.find((p) => p.title === "Notes" && p.width > 0) ?? null;
      });
      bunExpect(notes.width).toBe(320);
      bunExpect(notes.height).toBe(260);

      const tool = await until("the fixed toolbar to be laid out", async () => {
        const b = await app.locator(".wtool").boundingBox();
        return b.w > 0 ? b : null;
      });
      // Against the document's 620x460 viewport those numbers would be 612 and 452, and against its
      // own parent's content box they would be neither.
      bunExpect(tool.x + tool.w).toBe(notes.width - 8);
      bunExpect(tool.y + tool.h).toBe(notes.height - 8);

      const { entries } = await app.logs(0);
      const spoken = entries.filter((e) => e.text.includes("A-M03"));
      bunExpect(spoken.length).toBe(1);

      await app.locator(".pop-notes").click();
      await until("the notes window to be gone", async () => (await popouts()).every((p) => p.title !== "Notes"));
    },
    T,
  );

  test(
    "a second pop-out of the same node is refused, not queued",
    async () => {
      await app.locator(".pop").click();
      const p = await until("the palette to exist", async () => {
        const l = await popouts();
        return l.find((w) => w.title.startsWith("Palette")) ?? null;
      });
      const before = (await popouts()).length;
      await app.eval(`__sk_window_open(${p.node}, "{}")`);
      bunExpect((await popouts()).length).toBe(before);
      await app.locator(".back").click();
      await until("the palette to be gone", async () => (await popouts()).every((w) => !w.title.startsWith("Palette")));
    },
    T,
  );

  test(
    "nothing in the run was a JS error",
    async () => {
      await app.expectNoErrors();
    },
    T,
  );
});
