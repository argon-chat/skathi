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
