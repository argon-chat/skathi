// The geometry the virtual lists need, and the one place it is written down.
//
// ## Why the inspector computes its own heights
//
// A virtual list has to know how tall its viewport is. In a browser that is
// `el.clientHeight`; here it is not available — `getBoundingClientRect()` is on
// the compat roadmap (docs/DEVTOOLS.md, "Bounds") and nothing in the app can
// read a laid-out box back yet.
//
// That is not a blocker, because this app OWNS its layout: every band in the
// shell has a height this file declares, the CSS uses the same numbers, and the
// window's own size is a first-class fact (`innerHeight`, kept live by the
// window prelude's `resize` event — docs/WINDOWING.md). So the height of the
// element list is arithmetic, not measurement.
//
// The cost is that these constants and the `<style>` blocks have to agree. They
// are named for the band they measure and used from exactly one place each, and
// `apps/devtools` is one app — but it IS a duplication, and it is the reason a
// virtual list is used only where it earns its keep (the element tree and the
// console, both of which can be thousands of rows and both of which need
// "scroll the selection into view", which nothing else here can do either).

import { reactive } from "vue";

/** The custom titlebar (`vn.config.json`'s `titleBarHeight`). */
export const TITLEBAR_H = 34;
/** The tab strip + picker row under it. */
export const TOOLBAR_H = 30;
/** The bottom status strip. */
export const STATUS_H = 22;
/** A pane's own header row (`.pane-head`). */
export const PANE_HEAD_H = 26;
/** The breadcrumb strip under the element tree. */
export const CRUMBS_H = 22;
/** The console's prompt row. */
export const PROMPT_H = 26;
/** A draggable divider. */
export const SPLIT = 5;
/** One element-tree row. */
export const ROW_H = 18;
/** One console line, and the extra a wrapped continuation line adds. */
export const LOG_H = 17;
/** One network row. */
export const NET_H = 20;

/** The window, as the host reports it. Kept live by the `resize` listener. */
export const viewport = reactive({ w: 1180, h: 820 });

export function trackViewport(): () => void {
  const read = () => {
    viewport.w = Math.max(320, Number(globalThis.innerWidth) || viewport.w);
    viewport.h = Math.max(240, Number(globalThis.innerHeight) || viewport.h);
  };
  read();
  addEventListener("resize", read);
  return () => removeEventListener("resize", read);
}

/**
 * How tall the main area is — everything between the toolbar and the status
 * strip, minus the console drawer when it is open.
 */
export function mainHeight(consoleOpen: boolean, consoleHeight: number, banner: boolean): number {
  const drawer = consoleOpen ? consoleHeight + SPLIT : 0;
  return Math.max(
    80,
    viewport.h - TITLEBAR_H - TOOLBAR_H - STATUS_H - drawer - (banner ? 22 : 0),
  );
}

/**
 * How many rows of `height` fit in `available`, rounded UP.
 *
 * Up, not down: a list that renders only whole rows leaves a strip of empty
 * background at the bottom of the pane, which reads as "the list ends here"
 * when it does not. A clipped last row reads as "there is more", which is true.
 */
export function rowsFor(available: number, height: number): number {
  return Math.max(1, Math.ceil(available / height) + 1);
}
