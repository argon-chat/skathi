// A `flex: 1 1 0%` item with `height: 100%` in a definite flex column is held
// to the column, so the scroller inside it scrolls.
//
// Argon's settings drawer (SettingsWindow.vue in vaul-vue's DrawerContent) is
// a fixed flex column whose `.settings-layout` row is `flex-1` (`flex: 1 1
// 0%`) and `height: 100%`, around the `overflow-y: auto` page. The row's
// automatic minimum size is the smaller of its content and its specified
// height, so in Chromium it is the drawer's height and the page scrolls.
// vue-native dropped the specified height whenever a percentage basis was in
// force (a workaround for taffy reading the height when the percentage cannot
// be resolved), so the minimum was the content: the row and the page were
// 3424px tall in an 800px window, with nothing to scroll. taffy now treats an
// unresolvable percentage basis as `content` itself (CSS Flexbox §7.2.3,
// vendor/taffy `determine_flex_base_size`) and the size stays.
//
//   bun test packages/vn-test/specs/drawer-scroll.spec.ts
//
// The numbers are Chromium's for the same markup and sheet
// (fixtures/drawer-scroll/src/drawer.css) at a 1100x800 viewport, except the
// header, whose text is 6px taller here; the row's top moves with it.

import { afterAll, describe, expect, test } from "bun:test";
import { launch } from "../src/index";

const app = await launch({
  app: "packages/vn-test/fixtures/drawer-scroll/src/main.ts",
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
      const drawer = await box("drawer");
      expect(drawer).toEqual({ top: 40, height: 760 });
      // `height: 100%` of the drawer's content box (760 - 2 x 16): Chromium's 728.
      expect((await box("layout")).height).toBe(728);
      expect((await box("content")).height).toBe(728);
      const [scrollHeight, clientHeight] = await scroll();
      expect(clientHeight).toBe(728);
      expect(scrollHeight).toBe(3424);

      // And it does scroll: to the end, where the last row is.
      await app.eval(`document.getElementById("content").scrollTop = 1e6`);
      await sleep(300);
      const [, , top] = await scroll();
      expect(top).toBe(3424 - 728);
    },
    T,
  );

  test(
    "a percentage basis in an auto-height column is still the content (reka's Select viewport)",
    async () => {
      // `flex: 1` and `height: 40px` inside an auto-height column, around
      // 103px of items: Edge lays it out at 103.
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
