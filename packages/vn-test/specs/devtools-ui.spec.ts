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
// target is rendering, that selecting a row produces that node's real rules and
// computed style, that the console's `eval` runs in the TARGET, and — new in
// v2 — that the keyboard, the search and the panels are wired to the same
// state a click reaches.
//
// The protocol additions v1 needed — `rules`, `highlight`, `pick` — are pinned
// against the target directly at the bottom, where a failure says which
// COMMAND broke rather than which panel looked wrong. The v2 additions
// (`windows`, `perf`, `storage`, `net`, the push channel, F12) have a file of
// their own: `devtools-v2.spec.ts`.

import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch } from "../src/index";

const target = await launch({ app: "hello", window: [900, 640], label: "dt-target" });
// `VN_DEVTOOLS_TARGET` is what `--target` sets: the slot the inspector should
// prefer out of everything it can see. Launched second, so the handshake file
// it is pointed at already exists.
const inspector = await launch({
  app: "devtools",
  window: [1180, 820],
  label: "dt-ui",
  env: { VN_DEVTOOLS_TARGET: target.slotDir, VN_DEVTOOLS_RAISE: "off" },
});

afterAll(async () => {
  await inspector.close();
  await target.close();
});

const T = 40_000;

describe("the inspector attaches to another instance", () => {
  test("it finds the target it was pointed at and says so", async () => {
    // The titlebar chip is the attachment readout: it only names a slot once
    // `ping` has come back from the OTHER process.
    await expect(inspector.locator(".what .slot")).toBeVisible();
    await expect(inspector.locator(".what .pid")).toContainText(String(target.pid));
    // …and the push channel is live, which is what makes the console immediate
    // rather than 700 ms behind (docs/DEVTOOLS.md, the push channel).
    await expect(inspector.locator(".feed.live")).toBeVisible();
  }, T);

  test("the element tree is the target's tree", async () => {
    // `hello`'s own markup, read out of `hello`'s retained tree, rendered in a
    // different process's window.
    await expect(inspector.locator(".list").first()).toContainText("frame");
    await expect(inspector.locator(".list").first()).toContainText("titlebar");
    // …and the same tree the wire reports directly.
    const tree = await target.tree(undefined, 3);
    bunExpect(tree.id).toBeGreaterThan(0);
  }, T);

  test("selecting a row shows the RULES that produced it, winner first", async () => {
    // Row 2 is `<header class="titlebar">` with the default expansion: root,
    // .frame, then the header. It used to be row 3, because the fragment
    // anchor between them was drawn as a `<root>` row — the tag-id-0 bug that
    // the m21 addendum fixed (`Tree::new` now reserves interned name 0, and
    // the inspector decides row kind from `kind` rather than `tag`), so the
    // anchor is a `<!---->` row hidden behind a toggle and no longer counts.
    await inspector.locator(".row").nth(2).click();
    await expect(inspector.locator(".who")).toContainText("titlebar");
    // hello's styles are `<style scoped>`, so every selector the cascade
    // matched carries the scope attribute the SFC compiler stamped.
    await expect(inspector.locator(".selector").first()).toContainText("data-v-");
    // The declarations are the author's own text, and the panel shows them as
    // property/value pairs rather than as an engine dump.
    await expect(inspector.locator(".decl").first()).toBeVisible();
    await expect(inspector.locator(".prop").first()).toBeVisible();
  }, T);

  test("the computed tab shows the engine's own vocabulary", async () => {
    await inspector.locator(".stab").nth(1).click();
    // The dump is gpui's `StyleRefinement` (docs/DEVTOOLS.md) — `display` is a
    // gpui `Display`, and the titlebar is a flex row in hello's stylesheet.
    await expect(inspector.locator(".bucket-name").first()).toHaveText("live");
    await expect(inspector.locator(".decls").first()).toContainText("display");
    await expect(inspector.locator(".decls").first()).toContainText("Flex");
    await inspector.locator(".stab").nth(0).click();
  }, T);

  test("the console evaluates in the TARGET", async () => {
    const marker = `vn-${Date.now()}`;
    // Set from here, in hello's JSC…
    await target.eval(`globalThis.__DT_SPEC__ = ${JSON.stringify(marker)}`);
    // …read from there, through the inspector's own prompt.
    await inspector.locator(".entry").type("globalThis.__DT_SPEC__");
    await inspector.locator(".entry").press("Enter");
    await expect(inspector.locator(".line.result")).toContainText(marker);
  }, T);

  test("a console line from the target arrives without anybody polling", async () => {
    const marker = `push-${Date.now()}`;
    await target.eval(`console.log(${JSON.stringify(marker)})`);
    // 2 s, not 10: the whole point of the push channel is that this is one
    // frame away, not one poll interval. v1's console could take 700 ms just to
    // ASK, and the tree 2.5 s.
    await expect(inspector.locator(".console .list")).toContainText(marker, { timeout: 2500 });
  }, T);
});

describe("the panels", () => {
  test("a `%c` line renders as coloured runs on ONE line", async () => {
    // The bug this pins, found by hand on apps/meet-probe: a console line built
    // out of several styled spans was drawn as a COLUMN, and — because the row
    // is one line tall — the second span landed on top of the first. Two
    // engine fixes and one declared row later, they sit side by side.
    //
    // The formatter is the other half: `%c` must not reach the panel as text,
    // and the CSS argument (which apps write across several lines) must not
    // reach it as a message.
    const marker = `seg-${Date.now()}`;
    await target.eval(
      `console.log("%cBADGE%c ${marker}", "color:#fff;background:#00BCD4", ""); 0`,
    );
    await expect(inspector.locator(".console .list")).toContainText(marker, { timeout: 3000 });

    // No filtering: `%c` is the only thing that produces a `.seg`, and
    // `hello` logs none of its own — so these two ARE the marker line, and
    // the console follows the tail, so it is on screen.
    const segs = await inspector.query(".line .seg");
    bunExpect(segs.count).toBeGreaterThan(1);
    const [badge, rest] = segs.nodes;
    bunExpect(badge.text).toBe("BADGE");
    // Same line, and the message starts to the RIGHT of the badge. Stacked,
    // the tops would differ and both x's would be the column's.
    bunExpect(badge.bounds!.y).toBe(rest.bounds!.y);
    bunExpect(rest.bounds!.x).toBeGreaterThan(badge.bounds!.x);
    // …and no `%c` anywhere in what the panel shows.
    bunExpect(await inspector.locator(".console .list").textContent()).not.toContain("%c");
  }, T);

  test("Ctrl+F searches the whole tree and opens what is collapsed over a hit", async () => {
    await inspector.press("Ctrl+f");
    await expect(inspector.locator(".search .input")).toBeVisible();
    await inspector.locator(".search .input").type("card");
    // The hit counter is the search's own answer, and a highlighted row is the
    // tree's.
    await expect(inspector.locator(".row.hit").first()).toBeVisible();
    bunExpect(Number(await inspector.locator(".hits").textContent())).toBeGreaterThan(0);
    await inspector.press("Escape");
  }, T);

  test("the Performance panel reads the target's rolling frame clock", async () => {
    await inspector.locator(".tab").nth(2).click();
    await expect(inspector.locator(".stat.big .v")).toBeVisible();
    // fps is a real number from the target's own ring, not a placeholder.
    const fps = Number(await inspector.locator(".stat.big .v").textContent());
    bunExpect(fps).toBeGreaterThan(1);
    // …and the chart drew bars from the pushed samples.
    await expect(inspector.locator(".bar").first()).toBeVisible();
  }, T);

  test("the Storage panel shows the target's own localStorage", async () => {
    const key = `vn.ui.${Date.now()}`;
    await target.eval(`localStorage.setItem(${JSON.stringify(key)}, "from the spec")`);
    await inspector.locator(".tab").nth(3).click();
    await expect(inspector.locator(".rows")).toContainText(key);
    await expect(inspector.locator(".rows")).toContainText("from the spec");
    await target.eval(`localStorage.removeItem(${JSON.stringify(key)})`);
  }, T);

  test("the Targets panel lists the instance it is attached to, and its windows", async () => {
    await inspector.locator(".tab").nth(4).click();
    await expect(inspector.locator(".row.on").first()).toContainText(String(target.pid));
    await expect(inspector.locator(".c-slot").first()).toBeVisible();
    await inspector.locator(".tab").nth(0).click();
  }, T);

  test("Escape toggles the console drawer", async () => {
    await expect(inspector.locator(".console")).toHaveCount(1);
    await inspector.press("Escape");
    await expect(inspector.locator(".console")).toHaveCount(0);
    await inspector.press("Escape");
    await expect(inspector.locator(".console")).toHaveCount(1);
  }, T);
});

describe("the protocol the inspector runs on", () => {
  test("`rules` reports provenance in cascade order", async () => {
    const tab = await target.query(".tab");
    bunExpect(tab.count).toBeGreaterThan(0);
    const data = await target.send("rules", { node: tab.nodes[0].id });
    bunExpect(data.count).toBeGreaterThan(0);

    // Cascade order: `(specificity, order)`, ascending — the LAST rule that
    // declares a property is the one in effect, which is what lets a panel
    // strike the earlier ones through without re-implementing the cascade.
    const keys = data.rules.map((r: any) => r.specificity * 1e6 + r.order);
    bunExpect(keys).toEqual([...keys].sort((a: number, b: number) => a - b));

    // The app's own rule is in there, with its declarations as written.
    const own = data.rules.find((r: any) => r.selector.includes(".tab"));
    bunExpect(own).toBeDefined();
    bunExpect(own.declarations.length).toBeGreaterThan(0);
    bunExpect(own.declarations.some((d: any) => d.property === "border-radius")).toBe(true);
    // The `:hover` rung is a separate rule, tagged with the bucket it folds into.
    bunExpect(data.rules.some((r: any) => r.state === "hover")).toBe(true);
  }, T);

  test("`highlight` outlines a node and clears again", async () => {
    const tab = await target.query(".tab");
    const on = await target.send("highlight", { node: tab.nodes[1].id, ttlMs: 4000 });
    bunExpect(on.id).toBe(tab.nodes[1].id);
    // It has a rectangle to draw, because the node was painted last frame.
    bunExpect(on.painted).toBe(true);
    bunExpect(on.bounds.w).toBeGreaterThan(0);

    const off = await target.send("highlight", { clear: true });
    bunExpect(off.cleared).toBe(true);
  }, T);

  test("`pick` names the node under the pointer, and the window it is in", async () => {
    const tab = await target.query(".tab");
    const wanted = tab.nodes[2];
    // A REAL pointer event through gpui, at the node's centre — the same
    // gesture `locator.hover()` makes.
    await target.locator(".tab").nth(2).hover();

    const picked = await target.send("pick");
    bunExpect(picked.node).not.toBeNull();
    bunExpect(picked.node.id).toBe(wanted.id);
    // The path is root-first and ends at the node, which is what the tree view
    // expands along.
    bunExpect(picked.path[picked.path.length - 1]).toBe(wanted.id);
    bunExpect(picked.path.length).toBeGreaterThan(1);
    // v2: the reply names the window it answered for, so a client with several
    // is never guessing (docs/DEVTOOLS.md, the `window` argument).
    bunExpect(typeof picked.window).toBe("number");
    // `insideWindow` is gpui's WINDOW-level hover flag, and a synthesised
    // pointer event does not set it — only a real cursor over the window does.
    // It is the inspector's latch signal, and it is reported as what it is.
    bunExpect(typeof picked.insideWindow).toBe("boolean");
  }, T);
});
