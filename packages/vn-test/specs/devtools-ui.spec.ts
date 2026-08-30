// The inspector, inspected.
//
// `apps/devtools` is a vue-native app that attaches to ANOTHER vue-native
// instance over the devtools wire (docs/DEVTOOLS-UI.md). So this spec runs two
// hosts: a `hello` to be looked at, and an inspector pointed at its dist slot —
// and drives the inspector with the same protocol the inspector is driving
// `hello` with. Two windows, one wire, three hops:
//
//     bun test ──▶ inspector ──▶ hello
//
// What it pins is the loop, not the pixels: that attaching works without being
// told anything but a slot, that the tree the panel shows is the tree the
// target is rendering, that selecting a row produces that node's real computed
// style and the rules behind it, and that the console's `eval` runs in the
// TARGET (the value it prints can only have come from `hello`'s own JS).
//
// The protocol additions v1 needed — `rules`, `highlight`, `pick` — are pinned
// against the target directly at the bottom, where a failure says which
// COMMAND broke rather than which panel looked wrong.

import { afterAll, describe, test } from "bun:test";
import { expect, launch } from "../src/index";

const target = await launch({ app: "hello", window: [900, 640], label: "dt-target" });
// `VN_DEVTOOLS_TARGET` is what `--target` sets: the slot the inspector should
// prefer out of everything it can see. Launched second, so the handshake file
// it is pointed at already exists.
const inspector = await launch({
  app: "devtools",
  window: [1180, 820],
  label: "dt-ui",
  env: { VN_DEVTOOLS_TARGET: target.slotDir },
});

afterAll(async () => {
  await inspector.close();
  await target.close();
});

const T = 40_000;

describe("the inspector attaches to another instance", () => {
  test("it finds the target it was pointed at and says so", async () => {
    // The status line is `:class="state.status"` — `attached` is only there
    // once `ping` has come back from the OTHER process.
    await expect(inspector.locator(".status.attached")).toBeVisible();
    await expect(inspector.locator(".status")).toContainText(String(target.pid));
  }, T);

  test("the element tree is the target's tree", async () => {
    // `hello`'s own markup, read out of `hello`'s retained tree, rendered in a
    // different process's window.
    await expect(inspector.locator(".rows").first()).toContainText("frame");
    await expect(inspector.locator(".rows").first()).toContainText("titlebar");
    // …and the same tree the wire reports directly.
    const tree = await target.tree(undefined, 3);
    expect(tree.id).toBeGreaterThan(0);
  }, T);

  test("selecting a row shows that node's computed style", async () => {
    // Row 3 is `<header class="titlebar">` with the default expansion (root,
    // .frame, its anchor, then the header).
    await inspector.locator(".row").nth(3).click();
    await expect(inspector.locator(".who")).toContainText("titlebar");
    // The dump is the ENGINE's vocabulary (docs/DEVTOOLS.md) — `display` is a
    // gpui `Display`, and the titlebar is a flex row in hello's stylesheet.
    await expect(inspector.locator(".body").first()).toContainText("display");
    await expect(inspector.locator(".body").first()).toContainText("Flex");
  }, T);

  test("the rules tab shows where those values came from", async () => {
    await inspector.locator(".tab").nth(1).click();
    // hello's styles are `<style scoped>`, so every selector the cascade
    // matched carries the scope attribute the SFC compiler stamped.
    await expect(inspector.locator(".body").first()).toContainText("data-v-");
    await expect(inspector.locator(".body").first()).toContainText("spec ");
  }, T);

  test("the console evaluates in the TARGET", async () => {
    const marker = `vn-${Date.now()}`;
    // Set from here, in hello's JSC…
    await target.eval(`globalThis.__DT_SPEC__ = ${JSON.stringify(marker)}`);
    // …read from there, through the inspector's own prompt.
    await inspector.locator(".prompt .input").type("globalThis.__DT_SPEC__");
    await inspector.locator(".prompt .input").press("Enter");
    await expect(inspector.locator(".row.result")).toContainText(marker);
  }, T);
});

describe("the protocol the inspector runs on", () => {
  test("`rules` reports provenance in cascade order", async () => {
    const tab = await target.query(".tab");
    expect(tab.count).toBeGreaterThan(0);
    const data = await target.send("rules", { node: tab.nodes[0].id });
    expect(data.count).toBeGreaterThan(0);

    // Cascade order: `(specificity, order)`, ascending — the LAST rule that
    // declares a property is the one in effect, which is what lets a panel
    // strike the earlier ones through without re-implementing the cascade.
    const keys = data.rules.map((r: any) => r.specificity * 1e6 + r.order);
    expect(keys).toEqual([...keys].sort((a: number, b: number) => a - b));

    // The app's own rule is in there, with its declarations as written.
    const own = data.rules.find((r: any) => r.selector.includes(".tab"));
    expect(own).toBeDefined();
    expect(own.declarations.length).toBeGreaterThan(0);
    expect(own.declarations.some((d: any) => d.property === "border-radius")).toBe(true);
    // The `:hover` rung is a separate rule, tagged with the bucket it folds into.
    expect(data.rules.some((r: any) => r.state === "hover")).toBe(true);
  }, T);

  test("`highlight` outlines a node and clears again", async () => {
    const tab = await target.query(".tab");
    const on = await target.send("highlight", { node: tab.nodes[1].id, ttlMs: 4000 });
    expect(on.id).toBe(tab.nodes[1].id);
    // It has a rectangle to draw, because the node was painted last frame.
    expect(on.painted).toBe(true);
    expect(on.bounds.w).toBeGreaterThan(0);

    const off = await target.send("highlight", { clear: true });
    expect(off.cleared).toBe(true);
  }, T);

  test("`pick` names the node under the pointer", async () => {
    const tab = await target.query(".tab");
    const wanted = tab.nodes[2];
    // A REAL pointer event through gpui, at the node's centre — the same
    // gesture `locator.hover()` makes.
    await target.locator(".tab").nth(2).hover();

    const picked = await target.send("pick");
    expect(picked.node).not.toBeNull();
    expect(picked.node.id).toBe(wanted.id);
    // The path is root-first and ends at the node, which is what the tree view
    // expands along.
    expect(picked.path[picked.path.length - 1]).toBe(wanted.id);
    expect(picked.path.length).toBeGreaterThan(1);
    // `insideWindow` is gpui's WINDOW-level hover flag, and a synthesised
    // pointer event does not set it — only a real cursor over the window does.
    // It is the inspector's latch signal, and it is reported as what it is.
    expect(typeof picked.insideWindow).toBe("boolean");
  }, T);
});
