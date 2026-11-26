import { afterAll, describe, expect as bunExpect, test } from "bun:test";
import { expect, launch, MACOS, WINDOWS } from "../src/index";

const target = await launch({ app: "hello", window: [900, 640], label: "dt-target" });
// Launched second, so the handshake file it is pointed at already exists.
const inspector = await launch({
  app: "devtools",
  window: [1180, 820],
  label: "dt-ui",
  env: { SK_DEVTOOLS_TARGET: target.slotDir, SK_DEVTOOLS_RAISE: "off" },
});

afterAll(async () => {
  await inspector.close();
  await target.close();
});

const T = 40_000;

describe("the inspector attaches to another instance", () => {
  test(
    "it finds the target it was pointed at and says so",
    async () => {
      await expect(inspector.locator(".what .slot")).toBeVisible();
      await expect(inspector.locator(".what .pid")).toContainText(String(target.pid));
      await expect(inspector.locator(".feed.live")).toBeVisible();
    },
    T,
  );

  test(
    "the element tree is the target's tree",
    async () => {
      await expect(inspector.locator(".list").first()).toContainText("frame");
      await expect(inspector.locator(".list").first()).toContainText("titlebar");
      const tree = await target.tree(undefined, 3);
      bunExpect(tree.id).toBeGreaterThan(0);
    },
    T,
  );

  // macOS CI: what the inspector adds after a click is not painted (bounds null).
  test.skipIf(MACOS)(
    "selecting a row shows the RULES that produced it, winner first",
    async () => {
      const rows = await inspector.query(".row");
      const at = rows.nodes.findIndex((n) => (n.text ?? "").includes("titlebar"));
      bunExpect(at).toBeGreaterThanOrEqual(0);
      await inspector.locator(".row").nth(at).click();
      await expect(inspector.locator(".who")).toContainText("titlebar");
      await expect(inspector.locator(".selector").first()).toContainText("data-v-");
      await expect(inspector.locator(".decl").first()).toBeVisible();
      await expect(inspector.locator(".prop").first()).toBeVisible();
    },
    T,
  );

  test.skipIf(MACOS)(
    "the computed tab shows the engine's own vocabulary",
    async () => {
      await inspector.locator(".stab").nth(1).click();
      await expect(inspector.locator(".bucket-name").first()).toHaveText("live");
      await expect(inspector.locator(".decls").first()).toContainText("display");
      await expect(inspector.locator(".decls").first()).toContainText("Flex");
      await inspector.locator(".stab").nth(0).click();
    },
    T,
  );

  test.skipIf(MACOS)(
    "the console evaluates in the TARGET",
    async () => {
      const marker = `sk-${Date.now()}`;
      await target.eval(`globalThis.__DT_SPEC__ = ${JSON.stringify(marker)}`);
      await inspector.locator(".entry").type("globalThis.__DT_SPEC__");
      await inspector.locator(".entry").press("Enter");
      await expect(inspector.locator(".line.result")).toContainText(marker);
    },
    T,
  );

  test(
    "a console line from the target arrives without anybody polling",
    async () => {
      const marker = `push-${Date.now()}`;
      await target.eval(`console.log(${JSON.stringify(marker)})`);
      await expect(inspector.locator(".console .list")).toContainText(marker, { timeout: 2500 });
    },
    T,
  );
});

describe("the panels", () => {
  test.skipIf(MACOS)(
    "a `%c` line renders as coloured runs on ONE line",
    async () => {
      const marker = `seg-${Date.now()}`;
      await target.eval(`console.log("%cBADGE%c ${marker}", "color:#fff;background:#00BCD4", ""); 0`);
      await expect(inspector.locator(".console .list")).toContainText(marker, { timeout: 3000 });

      // No filtering: `%c` is the only thing that produces a `.seg`, and `hello` logs none of its
      // own — so these two ARE the marker line, and the console follows the tail, so it is on
      // screen.
      const segs = await inspector.query(".line .seg");
      bunExpect(segs.count).toBeGreaterThan(1);
      const badge = segs.nodes[0]!;
      const rest = segs.nodes[1]!;
      bunExpect(badge.text).toBe("BADGE");
      // Stacked, the tops would differ and both x's would be the column's.
      bunExpect(badge.bounds!.y).toBe(rest.bounds!.y);
      bunExpect(rest.bounds!.x).toBeGreaterThan(badge.bounds!.x);
      bunExpect(await inspector.locator(".console .list").textContent()).not.toContain("%c");
    },
    T,
  );

  test(
    "Ctrl+F searches the whole tree and opens what is collapsed over a hit",
    async () => {
      await inspector.press("Ctrl+f");
      await expect(inspector.locator(".search .input")).toBeVisible();
      await inspector.locator(".search .input").type("card");
      await expect(inspector.locator(".row.hit").first()).toBeVisible();
      bunExpect(Number(await inspector.locator(".hits").textContent())).toBeGreaterThan(0);
      await inspector.press("Escape");
    },
    T,
  );

  test.skipIf(MACOS)(
    "the search matches `.class` and `#id` as selectors, and text a selector cannot parse",
    async () => {
      await target.eval(
        `(() => { const d = document.createElement("div"); d.className = "dt-probe"; d.setAttribute("id", "dt-search"); d.textContent = "probe (dt)"; setTimeout(() => document.body.appendChild(d), 0); return 0; })()`,
      );
      await expect(inspector.locator(".list").first()).toContainText("probe (dt)");
      for (const query of [".dt-probe", "#dt-search", "(dt)"]) {
        await inspector.press("Ctrl+f");
        await inspector.locator(".search .input").type(query);
        await expect(inspector.locator(".row.hit").first()).toContainText("probe (dt)");
        bunExpect(Number(await inspector.locator(".hits").textContent())).toBe(1);
        await inspector.press("Escape");
        await expect(inspector.locator(".search .input")).toHaveCount(0);
      }
      await target.eval(`setTimeout(() => document.querySelector(".dt-probe").remove(), 0); 0`);
    },
    T,
  );

  test.skipIf(MACOS)(
    "an overridden declaration is struck through with a line that paints",
    async () => {
      await inspector.press("Ctrl+f");
      await inspector.locator(".search .input").type(".tab.router-link-active");
      await expect(inspector.locator(".hits")).toHaveText("1");
      await inspector.press("Enter");
      await expect(inspector.locator(".who")).toContainText("router-link-active");
      await inspector.press("Escape");
      await expect(inspector.locator(".decl.dead").first()).toBeVisible();
      const dead = await inspector.query(".decl.dead .prop");
      bunExpect(dead.nodes.map((n) => n.text).sort()).toEqual(["background", "color"]);
      const base = (await inspector.send("styles", { node: dead.nodes[0]!.id })).computed.base.join("\n");
      bunExpect(base).toMatch(/strikethrough: Some\(\s*StrikethroughStyle \{\s*thickness: (?!0px)[\d.]+px/);
    },
    T,
  );

  test.skipIf(MACOS)(
    "a text change made by a click reaches the tree without a refresh",
    async () => {
      await target.eval(
        `(() => { const b = document.createElement("div"); b.className = "dt-counter"; b.style.cssText = "position: fixed; left: 300px; top: 300px; width: 160px; height: 40px; z-index: 9999; background-color: white;"; let n = 0; b.textContent = "clicks 0"; b.addEventListener("click", () => { b.textContent = "clicks " + ++n; }); setTimeout(() => document.body.appendChild(b), 0); return 0; })()`,
      );
      await expect(inspector.locator(".list").first()).toContainText("clicks 0");
      await target.send("mouse", { selector: ".dt-counter", kind: "click" });
      await expect(inspector.locator(".list").first()).toContainText("clicks 1", { timeout: 2500 });
      await target.send("mouse", { selector: ".dt-counter", kind: "click" });
      await expect(inspector.locator(".list").first()).toContainText("clicks 2", { timeout: 2500 });
      await target.eval(`setTimeout(() => document.querySelector(".dt-counter").remove(), 0); 0`);
    },
    T,
  );

  // The macOS and Linux CI hosts read under 1 fps here.
  test.skipIf(!WINDOWS)(
    "the Performance panel reads the target's rolling frame clock",
    async () => {
      await inspector.locator(".tab").nth(2).click();
      const clock = inspector.locator('[data-metric="fps"]');
      await expect(clock).toBeVisible();
      const fps = Number.parseFloat(await clock.textContent());
      bunExpect(fps).toBeGreaterThan(1);
      await expect(inspector.locator(".bars .fbar, .bars .bar").first()).toBeVisible();
    },
    T,
  );

  test.skipIf(MACOS)(
    "the Storage panel shows the target's own localStorage",
    async () => {
      const key = `sk.ui.${Date.now()}`;
      await target.eval(`localStorage.setItem(${JSON.stringify(key)}, "from the spec")`);
      await inspector.locator(".tab").nth(3).click();
      await expect(inspector.locator(".rows")).toContainText(key);
      await expect(inspector.locator(".rows")).toContainText("from the spec");
      await target.eval(`localStorage.removeItem(${JSON.stringify(key)})`);
    },
    T,
  );

  test.skipIf(MACOS)(
    "the Targets panel lists the instance it is attached to, and its windows",
    async () => {
      await inspector.locator(".tab").nth(4).click();
      await expect(inspector.locator(".row.on").first()).toContainText(String(target.pid));
      await expect(inspector.locator(".c-slot").first()).toBeVisible();
      await inspector.locator(".tab").nth(0).click();
    },
    T,
  );

  test.skipIf(MACOS)(
    "Escape toggles the console drawer",
    async () => {
      await expect(inspector.locator(".console")).toHaveCount(1);
      await inspector.press("Escape");
      await expect(inspector.locator(".console")).toHaveCount(0);
      await inspector.press("Escape");
      await expect(inspector.locator(".console")).toHaveCount(1);
    },
    T,
  );
});

describe("the protocol the inspector runs on", () => {
  test(
    "`rules` reports provenance in cascade order",
    async () => {
      const tab = await target.query(".tab");
      bunExpect(tab.count).toBeGreaterThan(0);
      const data = await target.send("rules", { node: tab.nodes[0]!.id });
      bunExpect(data.count).toBeGreaterThan(0);

      const keys = data.rules.map((r: any) => r.specificity * 1e6 + r.order);
      bunExpect(keys).toEqual([...keys].sort((a: number, b: number) => a - b));

      const own = data.rules.find((r: any) => r.selector.includes(".tab"));
      bunExpect(own).toBeDefined();
      bunExpect(own.declarations.length).toBeGreaterThan(0);
      bunExpect(own.declarations.some((d: any) => d.property === "border-radius")).toBe(true);
      bunExpect(data.rules.some((r: any) => r.state === "hover")).toBe(true);
    },
    T,
  );

  test(
    "`highlight` outlines a node and clears again",
    async () => {
      const tab = await target.query(".tab");
      const on = await target.send("highlight", { node: tab.nodes[1]!.id, ttlMs: 4000 });
      bunExpect(on.id).toBe(tab.nodes[1]!.id);
      // It has a rectangle to draw, because the node was painted last frame.
      bunExpect(on.painted).toBe(true);
      bunExpect(on.bounds.w).toBeGreaterThan(0);

      const off = await target.send("highlight", { clear: true });
      bunExpect(off.cleared).toBe(true);
    },
    T,
  );

  test(
    "`pick` names the node under the pointer, and the window it is in",
    async () => {
      const tab = await target.query(".tab");
      const wanted = tab.nodes[2]!;
      // A REAL pointer event through gpui, at the node's centre — the same gesture `locator.hover()`
      // makes.
      await target.locator(".tab").nth(2).hover();

      const picked = await target.send("pick");
      bunExpect(picked.node).not.toBeNull();
      bunExpect(picked.node.id).toBe(wanted.id);
      bunExpect(picked.path[picked.path.length - 1]).toBe(wanted.id);
      bunExpect(picked.path.length).toBeGreaterThan(1);
      bunExpect(typeof picked.window).toBe("number");
      // `insideWindow` is gpui's WINDOW-level hover flag, and a synthesised pointer event does not
      // set it — only a real cursor over the window does.
      bunExpect(typeof picked.insideWindow).toBe("boolean");
    },
    T,
  );
});
