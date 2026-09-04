// Screenshot driver for the multi-window demo — NOT a spec.
//
//   bun packages/vn-test/specs/_shots-popout.ts
//
// Opens `apps/popout`, photographs the panel AT HOME, then pops out the
// palette, the player and the notes window and photographs each OS window.
// Nothing here focuses a window or moves the real cursor: `render_to_image`
// reads the client area back off the GPU, so an unfocused, unraised pop-out
// photographs exactly as it is on screen.
//
// It also reads the panel's computed `background` in both windows, which is
// the per-window `@media` proof in numbers (docs/MULTIWINDOW.md §6): one rule,
// `@media (max-width: 420px)`, on in the 380 px palette and off in the 620 px
// main window, for the SAME element at the same moment.
//
// Output: research/reports/m22/shots/multiwindow-*.png + a hues.json beside it.
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

import { launch } from "../src/index";

const OUT = resolve(import.meta.dir, "../../../research/reports/m22/shots");
mkdirSync(OUT, { recursive: true });

const app = await launch({ app: "popout", window: [620, 460], label: "shots" });

type Win = { id: number; kind: string; title: string };

async function windows(): Promise<Win[]> {
  const r = (await app.send("windows")) as { windows: Win[] };
  return r.windows;
}

async function until<T>(what: string, probe: () => Promise<T>, ms = 15000): Promise<T> {
  const deadline = Date.now() + ms;
  for (;;) {
    const v = await probe();
    if (v) return v;
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

async function shoot(name: string, window?: number) {
  const path = resolve(OUT, `${name}.png`);
  await app.send("screenshot", window === undefined ? { path } : { path, window });
  console.log(`  → ${name}.png`);
}

/**
 * The panel's computed background colour — which `@media` branch won. The dump
 * is gpui's own `StyleRefinement` debug pretty-printed one line per source
 * line, so the `Hsla` sits a few lines below `background: Some(`; the hue is
 * what tells the two branches apart (`0.616…` is the blue `#7aa2ff1f`,
 * `0.066…` the orange `#ff9d5c26`).
 */
async function panelBackground(): Promise<string> {
  const dump = JSON.stringify(await app.locator(".panel").styles());
  const m = /Hsla \{ h: ([0-9.]+), s: ([0-9.]+), l: ([0-9.]+), a: ([0-9.]+)/.exec(dump);
  return m ? `h=${m[1]} s=${m[2]} l=${m[3]} a=${m[4]}` : "(no background in the dump)";
}

const hues: Record<string, unknown> = {};

/** The panel's rectangle in whichever window is painting it, in CSS px. */
const panelBox = () => app.locator(".panel").boundingBox();

try {
  // The panel is still in window A — the `@media (max-width: 420px)` rule is
  // OFF here, because window A's viewport is 620 px wide.
  hues.mainWindow = await panelBackground();
  hues.mainWindowBox = await panelBox();
  await shoot("multiwindow-panel-at-home");

  await app.locator(".btn.pop").click();
  await until("the palette window", async () =>
    (await windows()).find((w) => w.title.startsWith("Palette")),
  );
  // …and now the same element is painted by a 380 px window, so the same rule
  // is ON. One sheet, one element, two answers.
  await new Promise((r) => setTimeout(r, 400));
  hues.paletteWindow = await panelBackground();
  hues.paletteWindowBox = await panelBox();

  await app.locator(".btn.pop-player").click();
  await until("the player window", async () =>
    (await windows()).find((w) => w.title.toLowerCase().includes("player")),
  );
  await app.locator(".btn.pop-notes").click();
  await until("the notes window", async () =>
    (await windows()).find((w) => w.title.toLowerCase().includes("note")),
  );
  // Let the video get past its first decoded frame and the transitions settle.
  await new Promise((r) => setTimeout(r, 2500));

  const all = await windows();
  console.log(all.map((w) => `${w.id} ${w.kind} ${JSON.stringify(w.title)}`).join("\n"));
  const pick = (needle: string) =>
    all.find((w) => w.title.toLowerCase().includes(needle))?.id;

  await shoot("multiwindow-main", all.find((w) => w.kind === "main")?.id);
  await shoot("multiwindow-palette", pick("palette"));
  await shoot("multiwindow-player", pick("player"));
  await shoot("multiwindow-notes", pick("note"));

  writeFileSync(resolve(OUT, "multiwindow-media.json"), `${JSON.stringify(hues, null, 2)}\n`);
  console.log(JSON.stringify(hues, null, 2));
} finally {
  await app.close();
}
