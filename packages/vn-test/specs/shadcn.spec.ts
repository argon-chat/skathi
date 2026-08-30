// shadcn-vue on vue-native, driven end to end.
//
// The three things this app exists to prove, asserted rather than eyeballed:
// a Teleport'd dialog with a focus trap and a dismissable layer, a reka-ui
// primitive whose state lives in a `data-state` attribute, and a whole theme
// that is one class on one element.
//
// It also happens to be the PARALLELISM proof: this file launches its own app,
// hello.spec.ts launches another, and `bun test packages/vn-test/specs` runs
// both without either noticing. Each gets a private dist slot, so each host
// binds its own ephemeral port and writes its own devtools.json.

import { afterAll, describe, test } from "bun:test";
import {
  artifactPath,
  changedRegion,
  expect,
  inset,
  launch,
  parseHsla,
  readPng,
  styleValue,
} from "../src/index";
import type { Rect } from "../src/index";

const app = await launch({ app: "shadcn", window: [1400, 900], label: "shadcn" });
afterAll(() => app.close());

const T = 30_000;

/** The themed root: the one element `.dark` / `.light` is stamped on. */
const ROOT = ".h-full.bg-background";

/**
 * Waits out an overlay's enter/exit animation before a PIXEL assertion.
 *
 * A DOM assertion does not need this — the node and its `data-state` are there
 * on the first frame — but a screenshot taken 20 ms after the click catches a
 * half-transparent box mid-slide, which is not the picture the test is about.
 * 150 ms is the whole of shadcn's `duration-200`-beaten-by-`animate-in`, plus
 * room for a debug build's frame.
 */
const settle = () => new Promise((r) => setTimeout(r, 400));

const rootBackground = async () => {
  const { computed } = await app.locator(ROOT).styles();
  return parseHsla(styleValue(computed.live ?? [], "background") ?? "");
};

describe("shadcn", () => {
  test("the trial page mounted", async () => {
    await expect(app.locator(ROOT)).toHaveCount(1);
    await expect(app.locator("button")).toHaveCount(26);
    await expect(app.locator("#theme-toggle")).toHaveText("theme: system");
  }, T);

  test("the dialog opens on click and closes on Escape", async () => {
    const dialog = app.locator("[role=dialog]");
    await expect(dialog).toHaveCount(0);

    // reka's DialogTrigger. Two things on the page declare `aria-haspopup`
    // (this and the Popover); the dialog's is first in document order.
    await app.locator("[aria-haspopup=dialog]").first().click();

    await expect(dialog).toBeVisible();
    // Teleported OUT of the column it is written in and INTO the app root —
    // if the portal had not resolved, this text would still be inside a
    // 3-column grid cell rather than over the whole window.
    await expect(app.locator("[role=dialog] h2")).toHaveText("Portalled content");
    const box = await dialog.boundingBox();
    const root = await app.locator(ROOT).boundingBox();
    expect(box.w).toBeGreaterThan(root.w / 4);

    // Escape goes to whatever holds focus — which, if the FocusScope did its
    // job, is inside the dialog. A dialog that never took focus swallows this
    // silently and stays open, so this asserts the trap too.
    await app.press("Escape");
    await expect(dialog).toHaveCount(0);
  }, T);

  test("the checkbox toggles its data-state", async () => {
    // reka's CheckboxRoot keeps its state in the attribute the Tailwind
    // variant reads (`data-[state=checked]:bg-primary`), so the attribute IS
    // the component's public state.
    await expect(app.locator("#terms[data-state=checked]")).toHaveCount(1);
    await expect(app.locator("label[for=terms]")).toHaveText("Accept terms (true)");

    await app.locator("#terms").click();

    await expect(app.locator("#terms[data-state=checked]")).toHaveCount(0);
    await expect(app.locator("#terms[data-state=unchecked]")).toHaveCount(1);
    // …and v-model carried it back into the page's own ref.
    await expect(app.locator("label[for=terms]")).toHaveText("Accept terms (false)");

    await app.locator("#terms").click();
    await expect(app.locator("#terms[data-state=checked]")).toHaveCount(1);
  }, T);

  test("the dark-mode class flips the computed background", async () => {
    // The whole theme is custom properties: `.dark` redefines every token and
    // the tokens inherit down. Nothing below the root knows a theme exists, so
    // the root's own resolved background is the honest place to read the flip.
    const toggle = app.locator("#theme-toggle");
    await expect(toggle).toHaveText("theme: system");

    // system -> light. The machine's OS preference decides what "system" looks
    // like, so LIGHT is the state worth pinning: `--background` is plain white
    // there under every OS setting.
    await toggle.click();
    await expect(toggle).toHaveText("theme: light");
    await expect(app.locator(ROOT)).toHaveComputedStyle("background", "l: 1.0");
    await expect(app.locator(".dark")).toHaveCount(0);
    const light = await rootBackground();
    expect(light!.l).toBe(1);

    // light -> dark. One class, one element.
    await toggle.click();
    await expect(toggle).toHaveText("theme: dark");
    await expect(app.locator(".dark")).toHaveCount(1);
    const dark = await rootBackground();
    expect(dark!.l).toBeLessThan(0.1);

    // …and a token two levels down re-resolved with it, which is the part a
    // root-only assertion would miss.
    await expect(app.locator(".text-card-foreground").first()).toHaveComputedStyle(
      "color",
      /l: 0\.9[0-9]*/,
    );

    // Back to where the file found it, so the tests stay order-independent.
    await toggle.click();
    await expect(toggle).toHaveText("theme: system");
  }, T);

  // --------------------------------------------------------------------------
  // m20 — the three teleported-content paint defects.
  //
  // All three come from the same place: an overlay reaches its final position
  // through a `transform`, and three separate rects had to be mapped through it
  // before being compared with anything in window space. Two of them already
  // were; `Window::paint_layer` was not, so a glyph run's LAYER was opened at
  // the element's untransformed rect and the draw order it handed every glyph
  // was computed against a region the text does not occupy. The label was
  // painted, in the right place, in the right colour — underneath its own
  // button. Nothing but a pixel can see that, which is what these three do.
  // --------------------------------------------------------------------------

  test("the dialog's Confirm label is painted, not buried under its own button", async () => {
    await app.locator("[aria-haspopup=dialog]").first().click();
    try {
      await expect(app.locator("[role=dialog]")).toBeVisible();
      // The dialog really enters now (m19): 150 ms of fade + zoom + slide. A
      // screenshot taken while that runs catches a translucent box on its way
      // in, which is a different picture from the one this test is about.
      await settle();
      const dialog = await app.locator("[role=dialog]").boundingBox();
      const button = await app.locator("[role=dialog] button.bg-primary").boundingBox();
      const shot = readPng(await app.screenshot(artifactPath("shadcn-dialog-confirm", "png")));

      // shadcn centres the dialog with `-translate-x-1/2 -translate-y-1/2`, and
      // `getBoundingClientRect` reports the box LAYOUT gave it (a transform is
      // a paint-time offset here, m13) — so the pill is at one of two places.
      // Take whichever one is actually white; allowing both keeps this honest
      // if the rect ever starts reporting the transformed box.
      const candidates: Rect[] = [
        { x: button.x, y: button.y, w: button.w, h: button.h },
        { x: button.x - dialog.w / 2, y: button.y - dialog.h / 2, w: button.w, h: button.h },
      ];
      const pill = candidates.find((rect) => {
        const core = inset(rect, 8);
        return shot.count(core, (luma) => luma > 200) > core.w * core.h * 0.5;
      });
      expect(pill, `no white pill at ${JSON.stringify(candidates)}`).toBeDefined();

      // `bg-primary` is `l: 0.98` and `text-primary-foreground` is `l: 0.1`, so
      // the label is the only thing inside the pill that can be dark. Before
      // the fix this was 0: every pixel of the inner rect was the flat pill.
      const glyphPixels = shot.count(inset(pill!, 8), (luma) => luma < 120);
      expect(glyphPixels).toBeGreaterThan(30);
    } finally {
      // Closed on every path: a spec that leaves a modal open changes what the
      // next one is looking at, and the failure it then reports is not its own.
      await app.press("Escape");
      await expect(app.locator("[role=dialog]")).toHaveCount(0);
    }
  }, T);

  test("the popover paints its body text, not just its panel", async () => {
    // Two things declare `aria-haspopup=dialog`; the popover's is the second.
    const trigger = app.locator("[aria-haspopup=dialog]").nth(1);
    const triggerBox = await trigger.boundingBox();
    const before = readPng(await app.screenshot(artifactPath("shadcn-popover-before", "png")));
    await trigger.click();
    await expect(app.locator("[role=dialog][data-side]")).toBeVisible();
    await settle();
    const after = readPng(await app.screenshot(artifactPath("shadcn-popover-after", "png")));

    // Below the trigger, opening the popover is the only thing that changed, so
    // the changed region IS the panel — wherever @floating-ui put it. Finding
    // it this way sidesteps the untransformed-rect problem entirely; the band
    // keeps the trigger's own `data-state=open` restyle (and its bright label)
    // out of the answer.
    const band: Rect = {
      x: 0,
      y: triggerBox.y + triggerBox.h + 2,
      w: after.width,
      h: after.height,
    };
    const panel = changedRegion(after, before, 24, band);
    expect(panel, "the popover changed no pixels below its trigger").not.toBeNull();
    expect(panel!.w).toBeGreaterThan(100);

    // Two things have to be true of an opaque panel with white text on it, and
    // the defect broke both in the same stroke — the panel's quad and the
    // page's glyphs ended up in the wrong order relative to each other, so the
    // heading BEHIND the popover painted over it while the popover's own body
    // painted under it.
    //
    //   fresh    — bright now, dark before: the popover's own words.
    //   survived — bright before AND after: page text the panel did not cover.
    //
    // Inset past `shadow-md`, which is the one part of the changed region that
    // is legitimately translucent.
    const core = inset(panel!, 10);
    let fresh = 0;
    let survived = 0;
    for (let y = core.y; y < core.y + core.h; y++) {
      for (let x = core.x; x < core.x + core.w; x++) {
        if (after.luma(x, y) <= 150) continue;
        if (before.luma(x, y) > 150) survived++;
        else fresh++;
      }
    }
    expect(fresh, "the popover's body text was never painted").toBeGreaterThan(50);
    expect(survived, "the page painted through an opaque panel").toBeLessThan(fresh / 4);

    await app.press("Escape");
    await expect(app.locator("[role=dialog][data-side]")).toHaveCount(0);
  }, T);

  test("the select viewport is as tall as its items, not as its trigger", async () => {
    await app.locator("[role=combobox]").first().click();
    await expect(app.locator("[role=option]")).toHaveCount(2);

    // reka writes `flex: 1` on the viewport inline and shadcn adds
    // `h-[--reka-select-trigger-height]`. A percentage flex basis REPLACES the
    // main-axis size, and an unresolvable one falls through to content sizing
    // (Flexbox §9.2.3 E) — so a browser sizes this to the items and ignores the
    // 40px. Ours honoured the 40px and clipped the list to its group label.
    const viewport = await app.locator("[data-reka-select-viewport]").boundingBox();
    const items = await app.locator("[role=option]").all();
    expect(items.length).toBe(2);
    for (const item of items) {
      expect(item.bounds!.y).toBeGreaterThanOrEqual(viewport.y - 1);
      expect(item.bounds!.y + item.bounds!.h).toBeLessThanOrEqual(viewport.y + viewport.h + 1);
    }
    // …and the box really did grow past the trigger's height.
    const trigger = await app.locator("[role=combobox]").first().boundingBox();
    expect(viewport.h).toBeGreaterThan(trigger.h * 2);

    await app.press("Escape");
    await expect(app.locator("[role=option]")).toHaveCount(0);
  }, T);

  test("the app logged nothing broken", async () => {
    // Advisories are NOT errors here on purpose: this app is the compat trial,
    // and the advisory stream is the deliverable rather than the defect.
    await app.expectNoErrors();
  }, T);
});
