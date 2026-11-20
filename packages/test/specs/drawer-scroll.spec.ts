// A `flex: 1 1 0%` item with `height: 100%` in a definite flex column is held to the column, so
// the scroller inside it scrolls.

import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/test/fixtures/drawer-scroll/src/main.ts",
  window: [1100, 800],
  label: "drawer-scroll",
});
afterAll(() => app.close());

const T = 30_000;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
type Box = { top: number; height: number };

const box = async (id: string): Promise<Box> =>
  (await app.eval(
    `(() => { const b = document.getElementById(${JSON.stringify(id)}).getBoundingClientRect(); return { top: Math.round(b.top), height: Math.round(b.height) }; })()`,
  )) as Box;
const scroll = () =>
  app.eval(
    `(() => { const c = document.getElementById("content"); return [c.scrollHeight, c.clientHeight, c.scrollTop]; })()`,
  ) as Promise<[number, number, number]>;

describe("settings drawer", () => {
  test(
    "the row is the drawer's height, not its content's, and the page scrolls",
    async () => {
      await sleep(500);
      const viewport = (await box("root")).height;
      const drawerHeight = Math.round(viewport * 0.95);
      const contentHeight = drawerHeight - 32;
      const drawer = await box("drawer");
      expect(drawer).toEqual({ top: viewport - drawerHeight, height: drawerHeight });
      expect((await box("layout")).height).toBe(contentHeight);
      expect((await box("content")).height).toBe(contentHeight);
      const [scrollHeight, clientHeight] = await scroll();
      expect(clientHeight).toBe(contentHeight);
      expect(scrollHeight).toBe(3424);

      await app.eval(`document.getElementById("content").scrollTop = 1e6`);
      await sleep(300);
      const [, , top] = await scroll();
      expect(top).toBe(scrollHeight - contentHeight);
    },
    T,
  );

  test(
    "a percentage basis in an auto-height column is still the content (reka's Select viewport)",
    async () => {
      await app.eval(`(() => {
        const root = document.getElementById("root");
        const col = document.createElement("div");
        col.id = "listbox";
        col.setAttribute("style", "position: absolute; left: 0; top: 0; width: 200px; display: flex; flex-direction: column");
        const vp = document.createElement("div");
        vp.id = "viewport";
        vp.setAttribute("style", "flex: 1 1 0%; height: 40px; overflow: hidden");
        const items = document.createElement("div");
        items.setAttribute("style", "height: 103px");
        vp.appendChild(items);
        col.appendChild(vp);
        root.appendChild(col);
      })()`);
      await sleep(400);
      expect((await box("viewport")).height).toBe(103);
      expect((await box("listbox")).height).toBe(103);
    },
    T,
  );
});
