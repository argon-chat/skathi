// What a template has no recorder's handle for: `__demo` with a drawn cursor, as the statements of
// a function body to evaluate in the page.
import { CURSOR_FILL, CURSOR_HOTSPOT, CURSOR_OUTLINE, CURSOR_VIEWBOX } from "../../apps/elements/src/cursorShape";

const SIZE = 27;
const HX = (CURSOR_HOTSPOT.x / CURSOR_VIEWBOX) * SIZE;
const HY = (CURSOR_HOTSPOT.y / CURSOR_VIEWBOX) * SIZE;

export const DEMO_HANDLE = `
  const cursor = document.createElement("div");
  cursor.style.cssText =
    "position:fixed;left:0;top:0;width:${SIZE}px;height:${SIZE}px;z-index:1000;pointer-events:none;opacity:0;" +
    "transition-property:transform,opacity;transition-timing-function:cubic-bezier(0.32,0,0.18,1),ease";
  const NS = "http://www.w3.org/2000/svg";
  const arrow = document.createElementNS(NS, "svg");
  arrow.setAttribute("width", "${SIZE}");
  arrow.setAttribute("height", "${SIZE}");
  arrow.setAttribute("viewBox", "0 0 ${CURSOR_VIEWBOX} ${CURSOR_VIEWBOX}");
  arrow.style.cssText = "transition:transform 110ms ease;transform-origin:4.7px 4px";
  for (const [d, fill] of [["${CURSOR_FILL}", "#f7eafc"], ["${CURSOR_OUTLINE}", "#8a44a3"]]) {
    const path = document.createElementNS(NS, "path");
    path.setAttribute("d", d);
    path.setAttribute("fill", fill);
    path.setAttribute("fill-rule", "evenodd");
    arrow.append(path);
  }
  cursor.append(arrow);
  document.body.append(cursor);
  const at = (x, y, ms) => {
    cursor.style.transitionDuration = ms + "ms, 200ms";
    cursor.style.transform = "translate(" + (x - ${HX}) + "px, " + (y - ${HY}) + "px)";
  };
  globalThis.__demo = {
    rect(selector) {
      const el = document.querySelector(selector);
      if (!el) return null;
      const r = el.getBoundingClientRect();
      return JSON.stringify({ x: r.x, y: r.y, w: r.width, h: r.height });
    },
    cursor: {
      show(x, y) {
        at(x, y, 0);
        setTimeout(() => (cursor.style.opacity = "1"), 32);
      },
      hide: () => (cursor.style.opacity = "0"),
      move: at,
      press: (down) => (arrow.style.transform = down ? "scale(0.86)" : "scale(1)"),
      key() {},
    },
  };
`;
