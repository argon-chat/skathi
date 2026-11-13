import { mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

import { launch } from "../src/index";

const OUT = resolve(import.meta.dir, "../../../.vn-test-artifacts/popout-shots");
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

async function panelBackground(): Promise<string> {
  const dump = JSON.stringify(await app.locator(".panel").styles());
  const m = /Hsla \{ h: ([0-9.]+), s: ([0-9.]+), l: ([0-9.]+), a: ([0-9.]+)/.exec(dump);
  return m ? `h=${m[1]} s=${m[2]} l=${m[3]} a=${m[4]}` : "(no background in the dump)";
}

const hues: Record<string, unknown> = {};

const panelBox = () => app.locator(".panel").boundingBox();

try {
  // The panel is still in window A — the `@media (max-width: 420px)` rule is OFF here, because
  // window A's viewport is 620 px wide.
  hues.mainWindow = await panelBackground();
  hues.mainWindowBox = await panelBox();
  await shoot("multiwindow-panel-at-home");

  await app.locator(".btn.pop").click();
  await until("the palette window", async () =>
    (await windows()).find((w) => w.title.startsWith("Palette")),
  );
  // …and now the same element is painted by a 380 px window, so the same rule is ON.
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
