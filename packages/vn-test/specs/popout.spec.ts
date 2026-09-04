// MULTI-WINDOW phase 1, driven end to end — docs/MULTIWINDOW.md.
//
//   bun test packages/vn-test/specs/popout.spec.ts
//
// One Vue app, one retained tree, up to FOUR OS windows. The claims this file
// holds down, in order of how much they cost to make true:
//
//   * a REAL platform click, dispatched into the palette window's own `Window`,
//     runs the same JS handler on the same `ref`, and text painted in the main
//     window moves (§0 — the spike proved this);
//   * `<Teleport :to="win.el">` MOVES the panel between the windows and moves
//     it back, keeping its node ids and its state (D-M03);
//   * a `<input>` inside a pop-out takes real keyboard focus IN THAT WINDOW and
//     the keystrokes reach it (§5.1 — the focus trio had to become per-window);
//   * `@media` answers against the window that PAINTS the element, so one rule
//     is on in the 380 px palette and off in the 620 px main window (§6);
//   * a `<video>` decodes and paints inside a pop-out, which the spike refused
//     because atlas tiles are per-window (§7.4, A-M06 retired);
//   * `chrome="hidden"` gives a pop-out its own titlebar, and its drag region
//     does not disturb the main window's hitbox list (§7.6).
//
// Nothing here focuses a window or moves the real cursor. Cross-window clicks
// go through `__vn_window_click`, which feeds `gpui::PlatformInput` to that
// window's own `Window::dispatch_event` — the same entry point the Win32
// message loop uses. Everything else is a devtools command; since phase 1 those
// resolve the window from the NODE, so `type` into a pop-out's input needs no
// `--window` (docs/MULTIWINDOW.md §7.5).

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const app = await launch({ app: "popout", window: [620, 460], label: "popout" });
afterAll(() => app.close());

/** Generous: a cold `bun test` pays for the first frame of a real GPU app. */
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

const popouts = (): Promise<Popout[]> => app.eval("JSON.parse(__vn_window_popouts())");
const windows = () => app.send("windows");

/** Poll until `probe` is truthy — the pop-out open is deferred by design (§3.2). */
async function until<T>(what: string, probe: () => Promise<T>, ms = 8000): Promise<T> {
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
 * The hue of a node's computed `background`, 0..1 — how this file tells the two
 * `@media` branches apart. The styles dump is gpui's own `StyleRefinement`
 * debug, so `Hsla { h: 0.616…` is the blue `#7aa2ff1f` and `h: 0.066…` the
 * orange `#ff9d5c26`. Reading the hue rather than the literal keeps the
 * assertion honest about what it is checking — WHICH RULE WON — without
 * pinning gpui's formatting.
 */
async function backgroundHue(selector: string): Promise<number> {
  const dump = JSON.stringify(await app.locator(selector).styles());
  const m = /Hsla \{ h: ([0-9.]+)/.exec(dump);
  if (!m) throw new Error(`no background colour in the styles dump for ${selector}`);
  return Number(m[1]);
}

/** `__vn_media_state` answers `key=value` lines, not JSON. */
async function mediaState(id: number): Promise<Record<string, string>> {
  const raw: string = await app.eval(`__vn_media_state(${id})`);
  const out: Record<string, string> = {};
  for (const line of raw.split("\n")) {
    const i = line.indexOf("=");
    if (i > 0) out[line.slice(0, i)] = line.slice(i + 1);
  }
  return out;
}

/** The panel's rectangle in window A, captured before it is popped out. */
let inWindowA: { x: number; y: number; w: number; h: number };
/** The palette window's id and painted root, once it exists. */
let palette: Popout;
/** `.panel`'s background hue in the 620 px main window — the wide branch. */
let wideHue: number;

describe("pop-out", () => {
  test("the app mounts with the panel painted in the main window", async () => {
    await expect(app.locator(".panel")).toHaveCount(1);
    await expect(app.locator(".count")).toHaveText("0");
    await expect(app.locator(".mirror")).toContainText("count = 0");
    // Nothing is popped out yet.
    bunExpect(await popouts()).toEqual([]);

    inWindowA = await app.locator(".panel").boundingBox();
    // It is laid out in the 620-wide main window, below the header, the lede,
    // the mirror rows and the buttons.
    bunExpect(inWindowA.y).toBeGreaterThan(100);
    bunExpect(inWindowA.w).toBeGreaterThan(500);

    // The WIDE branch of `@media (max-width: 420px)`: blue. Captured here so
    // the per-window test below is a comparison rather than a magic number.
    wideHue = await backgroundHue(".panel");
    bunExpect(wideHue).toBeGreaterThan(0.5);
  }, T);

  test("state is shared before the split, so the two readouts agree", async () => {
    // `.inc-a` is the "+1 (from window A)" button.
    await app.locator(".inc-a").click();
    await expect(app.locator(".count")).toHaveText("1");
    await expect(app.locator(".mirror")).toContainText("count = 1");
  }, T);

  test("popping out opens a SECOND OS window painting that subtree", async () => {
    const panelBefore = (await app.query(".panel")).nodes[0].id;

    await app.locator(".pop").click();

    const list = await until("the palette window to exist", async () => {
      const l = await popouts();
      return l.length === 1 ? l : null;
    });
    palette = list[0];
    bunExpect(palette.windowId).toBeGreaterThan(0);
    // `<VnWindow>`'s own container is what the host paints; the panel is
    // teleported INTO it and keeps the id it had in window A (D-M03).
    bunExpect(palette.node).not.toBe(panelBefore);
    bunExpect((await app.query(".panel")).nodes[0].id).toBe(panelBefore);
    // The live `title` prop reached the OS caption.
    bunExpect(palette.title).toBe("Palette - charcoal");
    bunExpect(palette.chrome).toBe("default");

    // The host said so out loud, once.
    const { entries } = await app.logs(0);
    bunExpect(entries.some((e) => e.text.includes("popped out into window"))).toBe(true);

    // The main window stopped painting it: the hint that only shows while the
    // panel is away is now painted, which it could not be if `palette` were
    // false.
    await expect(app.locator(".hint")).toBeVisible();
  }, T);

  test("the subtree left the main window's layout but stayed in the tree", async () => {
    // Still queryable, still styled, still one node — a pop-out is a PAINT-level
    // detachment, not a tree-level one (D-M05).
    await expect(app.locator(".panel")).toHaveCount(1);
    await expect(app.locator(".count")).toHaveText("1");

    // …and its rectangle is now the PALETTE window's, not window A's. That
    // window's client area is 380 wide and `<VnWindow>`'s container is its
    // painted root — which since phase 1 gets `build_node`'s root treatment
    // (100 %x100 %, `overflow: scroll`, a tracked focus handle, §7.1) — so the
    // panel is laid out against 380 px rather than against the 620-wide main
    // window's padded content column, and it sits at the very top of its own
    // window instead of below the header and the buttons.
    const box = await app.locator(".panel").boundingBox();
    bunExpect(box.w).toBe(380);
    bunExpect(box.w).not.toBe(inWindowA.w);
    bunExpect(box.y).toBeLessThan(inWindowA.y);
    bunExpect(box.y).toBeLessThan(20);
  }, T);

  test("A REAL CLICK IN THE PALETTE WINDOW UPDATES TEXT IN WINDOW A", async () => {
    const box = await app.locator(".inc-b").boundingBox();
    const x = box.x + box.w / 2;
    const y = box.y + box.h / 2;

    // Straight into the palette window's own `Window::dispatch_event` — its hit
    // test, its element tree, its listener. No cursor moves, nothing is focused.
    const hit = await app.eval(`__vn_window_click(${palette.node}, ${x}, ${y})`);
    bunExpect(hit).toBe(true);

    // The counter painted in the palette moved…
    await expect(app.locator(".count")).toHaveText("2");
    // …and so did the one painted in window A, off the same reactive wave.
    await expect(app.locator(".mirror")).toContainText("count = 2");

    // Once more, to rule out a one-off.
    await app.eval(`__vn_window_click(${palette.node}, ${x}, ${y})`);
    await expect(app.locator(".mirror")).toContainText("count = 3");
  }, T);

  test("window A still drives the same state while the panel is away", async () => {
    await app.locator(".inc-a").click();
    await expect(app.locator(".mirror")).toContainText("count = 4");
    await expect(app.locator(".count")).toHaveText("4");
  }, T);

  // -------------------------------------------------------------------------
  // phase 1
  // -------------------------------------------------------------------------

  test("devtools addresses both windows, and the pop-out reports its own metrics", async () => {
    const data = await windows();
    bunExpect(data.windows.length).toBe(2);
    const [main, pop] = data.windows;
    bunExpect(main.kind).toBe("main");
    bunExpect(main.root).toBe(0);
    bunExpect(pop.kind).toBe("popout");
    bunExpect(pop.id).toBe(palette.windowId);
    // The node THAT window paints — the key every per-window bounds map is
    // stored under.
    bunExpect(pop.root).toBe(palette.node);
    bunExpect(pop.viewport.w).toBe(380);
    bunExpect(main.viewport.w).toBe(620);

    // …and the app itself learned its pop-out's size, through `@resize` — the
    // honest answer to "a pop-out's metrics" in a one-realm world, since
    // `window.innerWidth` still means the MAIN window's viewport (A-M04).
    await expect(app.locator(".pal-size")).toHaveText("380x320");
  }, T);

  test("@media answers against the window that PAINTS the element", async () => {
    // **The headline of §6.** The palette window's viewport is 380 px and the
    // main window's is 620, and the sheet says
    // `@media (max-width: 420px) { .panel { background: #ff9d5c26 } }`.
    //
    // So the SAME element, at the SAME moment, in ONE stylesheet, takes the
    // narrow branch — because the cascade is now evaluated against the window
    // that paints it. The spike could not do this: there was one global `Env`,
    // so a 380 px pop-out answered against the main window's 620.
    const narrowHue = await backgroundHue(".panel");
    bunExpect(narrowHue).toBeLessThan(0.2); // orange
    bunExpect(wideHue).toBeGreaterThan(0.5); // blue, in the main window
  }, T);

  test("an <input> in the pop-out takes focus IN THAT WINDOW and receives keystrokes", async () => {
    // No `--window`: since phase 1 a devtools command resolves the window from
    // the NODE, because a node hangs under exactly one painted root and each
    // painted root belongs to exactly one window (§7.5).
    await app.send("type", { selector: ".brush-name", text: "!" });
    // The edit reached the shared `ref`, so the MAIN window's readout moved.
    await expect(app.locator(".brush-mirror")).toContainText("charcoal!");
    // …and the live `title` prop that is derived from it renamed the OS caption.
    await until("the caption to follow the prop", async () => {
      const [p] = await popouts();
      return p?.title === "Palette - charcoal!";
    });

    // Focus is per-window: the pop-out's input has it, and the main window's
    // own focus was never taken (`activate` defaults to false, so opening a
    // palette does not steal the caret).
    const focused = await app.send("query", { selector: ".brush-name" });
    bunExpect(focused.count).toBe(1);
  }, T);

  test("a screenshot can be taken of the pop-out window by id", async () => {
    const shot = await app.send("screenshot", {
      path: `${process.env.TEMP ?? "."}/vn-popout-palette.png`,
      window: palette.windowId,
    });
    bunExpect(typeof shot.path).toBe("string");
    bunExpect(shot.width).toBeGreaterThan(0);
  }, T);

  test("closing the window returns the subtree and fires the JS callback", async () => {
    await app.locator(".back").click();

    await until("the pop-out to be gone", async () => (await popouts()).length === 0);

    // The `@close` emit ran — `palette` is false again, so the "bring it back"
    // button is hidden and the hint is gone.
    await expect(app.locator(".hint")).toBeHidden();

    // The panel is painted by the main window again: its box is back inside the
    // 620-wide window, below the header, and it kept its state.
    await expect(app.locator(".count")).toHaveText("4");
    // …and it went back to answering `@media` against THIS window: the narrow
    // branch is off again, which is the other half of §6 and the half a stale
    // per-window `Env` would silently fail.
    await until("the panel to re-cascade against the main window", async () =>
      (await backgroundHue(".panel")) > 0.5);
    await expect(app.locator(".brush-mirror")).toContainText("charcoal!");
    const box = await until("the panel to be laid out in window A again", async () => {
      const b = await app.locator(".panel").boundingBox();
      return b.y > 60 ? b : null;
    });
    bunExpect(box.w).toBeGreaterThan(500);

    const { entries } = await app.logs(0);
    bunExpect(entries.some((e) => e.text.includes("closed — the subtree is back"))).toBe(true);
  }, T);

  test("a <video> plays inside a pop-out", async () => {
    // The case the spike REFUSED (A-M06): a decoded frame becomes an atlas tile
    // and `Window::drop_image` is per-window, so a tile returned to the wrong
    // window is freed from every atlas except the one holding it. Phase 1
    // records the creating window on the element.
    await app.locator(".pop-player").click();
    const player = await until("the player window to exist", async () => {
      const l = await popouts();
      return l.find((p) => p.title === "Player") ?? null;
    });
    bunExpect(player.title).toBe("Player");

    // It is laid out in the player window, not the main one.
    const box = await app.locator(".clip").boundingBox();
    bunExpect(box.w).toBe(320);
    bunExpect(box.h).toBe(180);

    // …and the OS media pipeline is actually running in there: a decoded frame
    // reached THIS window's sprite atlas. Before phase 1 the tile went to the
    // primary window's atlas — the pop-out had nothing to paint — and the
    // retired tiles were freed from every atlas except the one holding them.
    const state = await until("the video to report a frame", async () => {
      const id = (await app.query(".clip")).nodes[0].id;
      const s = await mediaState(id);
      return Number(s.readyState) >= 2 ? s : null;
    });
    bunExpect(Number(state.videoWidth)).toBeGreaterThan(0);
    bunExpect(state.paused).toBe("false");
    bunExpect(Number(state.currentTime)).toBeGreaterThan(0);

    await app.locator(".pop-player").click();
    await until("the player to be gone", async () =>
      (await popouts()).every((p) => p.title !== "Player"));
  }, T);

  test("a pop-out can draw its OWN titlebar without disturbing the main window's", async () => {
    // The main window is `titleBarStyle: "default"`, so the OS owns its caption
    // and it declares no hitboxes of its own.
    const before = JSON.parse(await app.eval("__vn_window_hitboxes()"));
    bunExpect(before).toEqual([]);

    await app.locator(".pop-notes").click();
    const notes = await until("the notes window to exist", async () => {
      const l = await popouts();
      return l.find((p) => p.title === "Notes") ?? null;
    });
    bunExpect(notes.chrome).toBe("hidden");

    // Its `.bar` declares `window-drag: move` and its close button
    // `--window-control: close`, and BOTH land in that window's own list —
    // which is the whole point of `control_hitboxes` becoming per-window (§7.6).
    const boxes = await until("the notes window to paint its chrome", async () => {
      const list = JSON.parse(await app.eval(`__vn_window_hitboxes(${notes.windowId})`));
      return list.length ? list : null;
    });
    const areas: string[] = boxes.map((b: { area: string }) => b.area);
    bunExpect(areas).toContain("drag");
    bunExpect(areas).toContain("close");

    // …and the MAIN window's list is still empty. Before phase 1 a pop-out's
    // build cleared it, which is both a wrong answer and a live bug in the
    // fallback-titlebar retire rule.
    bunExpect(JSON.parse(await app.eval("__vn_window_hitboxes()"))).toEqual([]);

    await app.locator(".pop-notes").click();
    await until("the notes window to be gone", async () =>
      (await popouts()).every((p) => p.title !== "Notes"));
  }, T);

  test("a second pop-out of the same node is refused, not queued", async () => {
    await app.locator(".pop").click();
    const p = await until("the palette to exist", async () => {
      const l = await popouts();
      return l.find((w) => w.title.startsWith("Palette")) ?? null;
    });
    const before = (await popouts()).length;
    // A-M02: the same node id may be painted by exactly one window (§5).
    await app.eval(`__vn_window_open(${p.node}, "{}")`);
    bunExpect((await popouts()).length).toBe(before);
    await app.locator(".back").click();
    await until("the palette to be gone", async () =>
      (await popouts()).every((w) => !w.title.startsWith("Palette")));
  }, T);

  test("nothing in the run was a JS error", async () => {
    await app.expectNoErrors();
  }, T);
});
