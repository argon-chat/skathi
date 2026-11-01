// Vendor-prefixed declarations, dropped from the CSS the builders ship.
//
// The runtime half is crates/vn-css/src/prefix.rs: vn-css applies a prefixed
// declaration only when it is a `-webkit-` spelling in its WEBKIT_APPLIED list
// and drops every other one (`-moz-`, `-ms-`, `-o-`, and the `-webkit-` ones
// Chromium ignores or that do nothing here) without an advisory. This module
// drops the same declarations at build time, so they are neither shipped nor
// compiled. The list below is a copy of the Rust one;
// test/css-prefixes.test.ts reads prefix.rs and fails when the two differ.
//
// Declarations only: prefixed at-rules (`@-webkit-keyframes`), selectors
// (`::-webkit-scrollbar`) and values (`display: -webkit-box`) are left alone.

import postcss, { type Root } from "postcss";

/** crates/vn-css/src/prefix.rs `WEBKIT_APPLIED`, in the same order. */
export const WEBKIT_APPLIED: readonly string[] = [
  "align-content",
  "align-items",
  "align-self",
  "animation",
  "animation-delay",
  "animation-direction",
  "animation-duration",
  "animation-fill-mode",
  "animation-iteration-count",
  "animation-name",
  "animation-play-state",
  "animation-timing-function",
  "app-region",
  "appearance",
  "backdrop-filter",
  "backface-visibility",
  "background-clip",
  "background-origin",
  "background-size",
  "border-bottom-left-radius",
  "border-bottom-right-radius",
  "border-radius",
  "border-top-left-radius",
  "border-top-right-radius",
  "box-align",
  "box-decoration-break",
  "box-direction",
  "box-flex",
  "box-ordinal-group",
  "box-orient",
  "box-pack",
  "box-reflect",
  "box-shadow",
  "box-sizing",
  "clip-path",
  "column-count",
  "column-gap",
  "column-rule",
  "column-span",
  "column-width",
  "columns",
  "filter",
  "flex",
  "flex-basis",
  "flex-direction",
  "flex-flow",
  "flex-grow",
  "flex-shrink",
  "flex-wrap",
  "font-feature-settings",
  "hyphens",
  "justify-content",
  "line-clamp",
  "mask",
  "mask-clip",
  "mask-composite",
  "mask-image",
  "mask-origin",
  "mask-position",
  "mask-repeat",
  "mask-size",
  "order",
  "perspective",
  "perspective-origin",
  "text-fill-color",
  "text-size-adjust",
  "text-stroke",
  "text-stroke-color",
  "text-stroke-width",
  "transform",
  "transform-origin",
  "transform-style",
  "transition",
  "transition-delay",
  "transition-duration",
  "transition-property",
  "transition-timing-function",
  "user-select",
  "writing-mode",
];

const APPLIED = new Set(WEBKIT_APPLIED);
const PREFIX = /^-(webkit|moz|ms|o)-/i;

/** Is `prop` a vendor-prefixed property vue-native drops? (prefix.rs `dropped`) */
export function isDroppedPrefix(prop: string): boolean {
  const m = PREFIX.exec(prop);
  if (!m) return false;
  if (m[1].toLowerCase() !== "webkit") return true;
  return !APPLIED.has(prop.slice(m[0].length).toLowerCase());
}

/** A prefixed declaration anywhere in the text: the cheap test before parsing. */
const MAYBE = /(^|[;{\s])-(webkit|moz|ms|o)-[a-z-]+\s*:/i;

/**
 * `css` without the vendor-prefixed declarations vue-native drops. The text is
 * returned as it came when there is none, or when postcss cannot parse it
 * (vn-css then drops them itself, at runtime).
 */
export function pruneVendorPrefixes(css: string): string {
  if (!MAYBE.test(css)) return css;
  let root: Root;
  try {
    root = postcss.parse(css);
  } catch {
    return css;
  }
  let removed = 0;
  root.walkDecls((decl) => {
    if (isDroppedPrefix(decl.prop)) {
      decl.remove();
      removed++;
    }
  });
  return removed ? root.toString() : css;
}
