// The geometry the virtual lists need, and the one place it is written down. ## Why the inspector
// computes its own heights A virtual list has to know how tall its viewport is.

import { reactive } from "vue";

export const TITLEBAR_H = 34;
export const TOOLBAR_H = 30;
export const STATUS_H = 22;
export const PANE_HEAD_H = 26;
export const CRUMBS_H = 22;
export const PROMPT_H = 26;
export const SPLIT = 5;
export const ROW_H = 18;
export const LOG_H = 17;
export const NET_H = 20;
export const NET_COLS_H = 22;
export const DETAIL_TABS_H = 26;
export const BODY_BAR_H = 24;
export const BODY_LINE_H = 16;
export const MONO_W = 6.62;
export const MENU_ITEM_H = 22;

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

export function mainHeight(consoleOpen: boolean, consoleHeight: number, banner: boolean): number {
  const drawer = consoleOpen ? consoleHeight + SPLIT : 0;
  return Math.max(80, viewport.h - TITLEBAR_H - TOOLBAR_H - STATUS_H - drawer - (banner ? 22 : 0));
}

/**
 * Up, not down: a list that renders only whole rows leaves a strip of empty background at the
 * bottom of the pane, which reads as "the list ends here" when it does not.
 */
export function rowsFor(available: number, height: number): number {
  return Math.max(1, Math.ceil(available / height) + 1);
}
