// Stills of apps/blog-0-10 for the 0.10 post. Run with --shots.
import type { SkApp } from "../../../packages/test/src/index";
import type { AppPlans, Clip } from "../record";
import { sleep } from "../win32";

function still(show: (app: SkApp) => Promise<void>): Clip {
  return { poster: 0, show, plan: async () => ({ start: 0, end: 0 }) };
}
const view = (id: string) => still(async (app) => void (await app.eval(`__shots.show(${JSON.stringify(id)})`)));

export const plans: AppPlans = {
  app: "blog-0-10",
  group: "",
  window: [560, 360],
  clips: {
    "kbd-keys": view("keys"),
    "kbd-keyboard": view("keyboard"),
    "kbd-layouts": view("layouts"),
    "kbd-mouse": view("mouse"),
    "kbd-gamepad": view("gamepad"),
    "context-menu": still(async (app) => {
      await app.eval(`__shots.show("menu")`);
      // A synthetic key reaches the menu only once the window has had pointer input.
      for (const kind of ["move", "down", "up"] as const) await app.send("mouse", { x: 540, y: 350, kind });
      await sleep(300);
      await app.eval(`__shots.openMenu()`);
      await sleep(500);
      for (const key of ["ArrowDown", "ArrowDown", "ArrowRight"]) {
        await app.send("key", { key });
        await sleep(250);
      }
    }),
  },
};
