import type { MirrorRect } from "./types";
import { theGlobal } from "./interfaces";
import { defineGlobal } from "./globals";

export function asDomRect(r: MirrorRect): MirrorRect {
  const C = theGlobal("DOMRect");
  return typeof C === "function"
    ? new (C as new (x: number, y: number, w: number, h: number) => MirrorRect)(r.x, r.y, r.width, r.height)
    : r;
}

// getComputedStyle: a CSSStyleDeclaration over the host's cascade (`__vn_computed_object`), one property per read. Anything
// that is not a mirror node reads the root's.
defineGlobal("getComputedStyle", (node: unknown) => __vn_computed_object(node));
