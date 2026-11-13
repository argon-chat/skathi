import { afterAll, describe, test } from "bun:test";
import { expect, launch, uiaFormat, uiaSupported, uiaTree } from "../src/index";
import type { UiaNode } from "../src/index";

const T = 60_000;

if (!uiaSupported()) {
  describe("a11y (UI Automation)", () => {
    test.skip("Windows only", () => {});
  });
} else {
  const app = await launch({ app: "shadcn", window: [1400, 900], label: "a11y" });
  afterAll(() => app.close());

  const tree = () => uiaTree(app.pid, { minNodes: 20, timeout: 30_000 });

  const one = (nodes: UiaNode[], pred: (n: UiaNode) => boolean, what: string): UiaNode => {
    const hits = nodes.filter(pred);
    if (hits.length !== 1) {
      throw new Error(`expected exactly one ${what}, found ${hits.length}\n\n${uiaFormat(nodes)}`);
    }
    return hits[0]!;
  };

  describe("a11y (UI Automation)", () => {
    test(
      "the window hands UI Automation a populated tree",
      async () => {
        const nodes = await tree();
        // Before the lowering existed this was 1: the window, and nothing in it.
        expect(nodes.length).toBeGreaterThan(30);
        expect(nodes[0]!.type).toBe("Window");
        expect(nodes[0]!.depth).toBe(0);
      },
      T,
    );

    test(
      "HTML's own semantics arrive without a single aria attribute",
      async () => {
        const nodes = await tree();
        for (const label of ["Secondary", "Destructive", "Outline", "Ghost", "Link"]) {
          const node = one(nodes, (n) => n.name === label && n.type === "Button", `Button ${label}`);
          expect(node.enabled).toBe(true);
          expect(node.w).toBeGreaterThan(0);
        }
        expect(one(nodes, (n) => n.name === "Disabled", "disabled button").enabled).toBe(false);
        const edit = one(nodes, (n) => n.type === "Edit" && n.name === "Name", "the named text field");
        expect(edit.value ?? "").toContain("native gpui text input");
        expect(nodes.some((n) => n.type === "ProgressBar")).toBe(true);
      },
      T,
    );

    test(
      "a checkbox exposes its Toggle pattern, and a click moves it",
      async () => {
        const before = await tree();
        const box = one(before, (n) => n.type === "CheckBox", "checkbox");
        expect(box.toggle).toBe("On");
        expect(box.name).toContain("Accept terms");

        await app.locator("[role=checkbox]").first().click();
        await new Promise((r) => setTimeout(r, 400));

        const after = await tree();
        expect(one(after, (n) => n.type === "CheckBox", "checkbox").toggle).toBe("Off");

        // Put it back, so the file's tests do not depend on their own order.
        await app.locator("[role=checkbox]").first().click();
        await new Promise((r) => setTimeout(r, 400));
      },
      T,
    );

    test(
      "the tab list reports which tab is selected",
      async () => {
        const nodes = await tree();
        const tabs = nodes.filter((n) => n.type === "TabItem");
        expect(tabs.length).toBe(2);
        expect(tabs.map((t) => t.name)).toEqual(["Account", "Password"]);
        expect(tabs.map((t) => t.selected)).toEqual([true, false]);
        expect(nodes.filter((n) => n.type === "Pane").map((p) => p.name)).toEqual(["Account"]);
      },
      T,
    );

    test(
      "the dialog is a dialog, and its buttons are inside it",
      async () => {
        const before = await tree();
        expect(before.filter((n) => n.name === "Portalled content").length).toBe(0);

        await app.locator("[aria-haspopup=dialog]").first().click();
        await new Promise((r) => setTimeout(r, 500));

        const nodes = await tree();
        const at = nodes.findIndex((n) => n.name === "Portalled content" && n.depth === 1);
        expect(at).toBeGreaterThanOrEqual(0);
        const dialog = nodes[at]!;
        expect(dialog.type).toBe("Window");

        const inside = nodes.slice(at + 1).filter((n) => n.depth > dialog.depth);
        const stop = inside.findIndex((n) => n.depth <= dialog.depth);
        const subtree = stop < 0 ? inside : inside.slice(0, stop);
        const names = subtree.filter((n) => n.type === "Button").map((n) => n.name);
        expect(names).toContain("Confirm");
        expect(names).toContain("Close");

        const active = await app.eval(
          "document.activeElement ? [document.activeElement.tagName, document.activeElement.textContent.trim()] : null",
        );
        expect(active).toEqual(["BUTTON", "Confirm"]);

        expect(one(nodes, (n) => n.name === "Open dialog", "the dialog trigger").expand).toBe("Expanded");

        await app.press("Escape");
        await new Promise((r) => setTimeout(r, 500));
        expect((await tree()).filter((n) => n.name === "Portalled content").length).toBe(0);
      },
      T,
    );
  });
}
