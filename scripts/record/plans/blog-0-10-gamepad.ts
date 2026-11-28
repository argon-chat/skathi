// The 0.10 post's gamepad clip: apps/blog-0-10's live readout while someone works a real controller.
// The clip runs for RECORD_SECONDS (default 25) and logs a countdown.
import type { AppPlans } from "../record";
import { sleep } from "../win32";

const SECONDS = Number(process.env.RECORD_SECONDS ?? 25);

export const plans: AppPlans = {
  app: "blog-0-10",
  group: "blog-0-10",
  window: [640, 440],
  clips: {
    controls: {
      poster: 0,
      async show(app) {
        await app.eval(`__shots.show("controls")`);
      },
      plan: async () => ({ start: 0, end: 0 }),
    },
    "controls-dark": {
      poster: 0,
      async show(app) {
        await app.eval(`__shots.show("controls"); document.documentElement.style.colorScheme = "dark"; "ok"`);
      },
      plan: async () => ({ start: 0, end: 0 }),
    },
    "controls-unset": {
      poster: 0,
      async show(app) {
        await app.eval(`__shots.show("controls"); document.documentElement.style.colorScheme = "";
          const s = document.createElement("style");
          s.textContent = "button, input, select, textarea { all: unset }";
          document.head.append(s);
          "ok"`);
      },
      plan: async () => ({ start: 0, end: 0 }),
    },
    gamepad: {
      poster: 0.4,
      settle: 800,
      async show(app) {
        await app.eval(`__shots.show("pad")`);
      },
      async plan() {
        const start = Date.now();
        for (let left = SECONDS; left > 0; left -= 5) {
          console.log(`[record] gamepad: ${left} s left`);
          await sleep(Math.min(5, left) * 1000);
        }
        return { start, end: Date.now() };
      },
    },
  },
};
