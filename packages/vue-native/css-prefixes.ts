// Vendor-prefixed declarations, dropped from the CSS the builders ship. Mirrors
// crates/vn-css/src/prefix.rs (only the `-webkit-` spellings in `WEBKIT_APPLIED` apply, the rest are
// dropped); test/css-prefixes.test.ts fails when the lists differ. Declarations only.

import postcss, { type Root } from "postcss";

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

export function isDroppedPrefix(prop: string): boolean {
  const m = PREFIX.exec(prop);
  if (!m) return false;
  if (m[1]!.toLowerCase() !== "webkit") return true;
  return !APPLIED.has(prop.slice(m[0].length).toLowerCase());
}

const MAYBE = /(^|[;{\s])-(webkit|moz|ms|o)-[a-z-]+\s*:/i;

/** `css` without the vendor-prefixed declarations; unchanged when there are none or postcss cannot parse it. */
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
