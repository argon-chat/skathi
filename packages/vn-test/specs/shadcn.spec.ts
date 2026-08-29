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
import { expect, launch, parseHsla, styleValue } from "../src/index";

const app = await launch({ app: "shadcn", window: [1400, 900], label: "shadcn" });
afterAll(() => app.close());

const T = 30_000;

/** The themed root: the one element `.dark` / `.light` is stamped on. */
const ROOT = ".h-full.bg-background";

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

  test("the app logged nothing broken", async () => {
    // Advisories are NOT errors here on purpose: this app is the compat trial,
    // and the advisory stream is the deliverable rather than the defect.
    await app.expectNoErrors();
  }, T);
});
