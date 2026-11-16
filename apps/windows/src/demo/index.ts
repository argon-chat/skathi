// The recorder's handle: `globalThis.__demo`. A clip is `reset()`, then a scripted `play(id)`.
// Demo mode (the drawn cursor) is on with the `--demo` launch argument or `__demo.enable()`.
import { appLaunch, appWindow } from "@skathi-native/runtime";
import { scenarioById, scenarios } from "../scenarios";
import { background, go, leaveCompact, nav, resetDraft, titleBar } from "../store";
import { demo, hide, pointer, sleep } from "./cursor";
import { scripts, targets } from "./scripts";

type Phase = "idle" | "running" | "done" | "error";
const status = { id: "", phase: "idle" as Phase, error: "" };
/** Set by `show(id)`: the page is already fresh, so `play(id)` does not remount it in the clip. */
let shownFor = "";

/** Puts the app back to its launch state and mounts `page` afresh. */
async function reset(page = nav.id): Promise<void> {
  pointer.shown = false;
  pointer.key = "";
  demo.running = "";
  if (document.fullscreenElement) await document.exitFullscreen().catch(() => {});
  if (appWindow.isFullscreen()) appWindow.setFullscreen(false);
  if (appWindow.isMinimized() || appWindow.isMaximized()) appWindow.unmaximize();
  if (!appWindow.isVisible()) appWindow.show();
  leaveCompact();
  titleBar.tabs = false;
  if (appWindow.chrome.hostControls) appWindow.setTitleBarOverlay(false);
  appWindow.backdrop = "acrylic";
  resetDraft();
  background.keepRunning = false;
  go(page, true);
  await sleep(250);
}

async function play(id: string): Promise<void> {
  const scenario = scenarioById.get(id);
  if (!scenario)
    throw new TypeError(`no scenario ${JSON.stringify(id)}; have ${scenarios.map((s) => s.id).join(", ")}`);
  if (!demo.enabled && scenario.cursor === "drawn") {
    throw new Error("demo mode is off: launch with --demo or call __demo.enable() first");
  }
  status.id = id;
  status.phase = "running";
  status.error = "";
  try {
    if (shownFor !== id) await reset(id);
    shownFor = "";
    demo.running = id;
    await sleep(600);
    await scripts[id]!();
    await hide();
    status.phase = "done";
  } catch (e) {
    status.phase = "error";
    status.error = String((e as Error)?.stack ?? e);
    throw e;
  } finally {
    demo.running = "";
  }
}

export function installDemo(): void {
  if (appLaunch.argv.includes("--demo")) demo.enabled = true;
  (globalThis as Record<string, unknown>).__demo = {
    ids: scenarios.map((s) => s.id),
    cursor: Object.fromEntries(scenarios.map((s) => [s.id, s.cursor])),
    get enabled() {
      return demo.enabled;
    },
    enable() {
      demo.enabled = true;
    },
    disable() {
      demo.enabled = false;
      pointer.shown = false;
    },
    play,
    reset() {
      void reset();
    },
    /** Resets and opens a scenario's page without playing it, so a capture can start on it. */
    show(id: string) {
      shownFor = id;
      void reset(id);
    },
    /** For the recorder, which cannot await through `eval`: `{ id, phase, error }`. */
    status: () => ({ ...status }),
    targets,
  };
}
