import type { CssEntry } from "./entries";
import { browserCompatData, findCompat, type BcdData } from "../browser-compat";

export const overrides: Readonly<Record<string, string | null>> = {
  "feature-style-blocks": null,
  "feature-scoped-styles": null,
  "feature-v-bind-in-css": null,
  "feature-inline-style": null,
  "feature-cascade": null,
  "feature-css-modules": null,
  "feature-preprocessors": null,
  "feature-tailwind-3": null,
  "feature-tailwind-4": null,
  "feature-vendor-prefixes": null,
  "feature-css-nesting": "css.selectors.nesting",
  "feature-relative-colors": "css.types.color.rgb.relative_syntax",
  "feature-web-animations": "api.Element.animate",
  "feature-view-transitions": "api.Document.startViewTransition",
  "named-colors": "css.types.color.named-color",
  "hex-colors": "css.types.color.rgb_hexadecimal_notation",
  "keyword-transparent": "css.types.color.transparent",
  "keyword-currentcolor": "css.types.color.currentcolor",
  "system-colors": "css.types.color.system-color",
  "keyword-inherit": "css.types.global_keywords.inherit",
  "keyword-initial": "css.types.global_keywords.initial",
  "keyword-unset": "css.types.global_keywords.unset",
  "keyword-revert": "css.types.global_keywords.revert",
  "easing-functions": "css.types.easing-function",
  "custom-properties": "css.properties.custom-property",
  "dashed-function": "css.types.dashed-function",
  "font-smoothing": "css.properties.font-smooth",
  "app-region": "css.properties.-webkit-app-region",
  "webkit-mask": null,
  "function-webkit-gradient": null,
  "pseudo-webkit-color-swatch": null,
  "pseudo-mark": null,
  "pseudo-line": null,
  "pseudo-decoration": null,
  "pseudo-gutter": null,
  "pseudo-line-number": null,
  "selector-type": "css.selectors.type",
  "selector-class": "css.selectors.class",
  "selector-id": "css.selectors.id",
  "selector-universal": "css.selectors.universal",
  "selector-attribute": "css.selectors.attribute",
  "selector-descendant": "css.selectors.descendant",
  "selector-child": "css.selectors.child",
  "selector-next-sibling": "css.selectors.next-sibling",
  "selector-subsequent-sibling": "css.selectors.subsequent-sibling",
  "selector-namespace": "css.selectors.namespace",
  "selector-ancestor-state": null,
  "media-types": "css.at-rules.media",
  "media-range-syntax": "css.at-rules.media.range_syntax",
  "media-logical-operators": "css.at-rules.media.or_syntax",
  "media-custom-media": "css.at-rules.custom-media",
  "unit-px": "css.types.length",
  "unit-percent": "css.types.percentage",
  "unit-in": "css.types.length",
  "unit-cm": "css.types.length",
  "unit-mm": "css.types.length",
  "unit-pt": "css.types.length",
  "unit-pc": "css.types.length",
  "unit-container": "css.types.length.container_query_length_units",
};

function bareName(e: CssEntry): string {
  if (e.kind === "media-feature") return /\(([^)]+)\)/.exec(e.name)?.[1] ?? e.name;
  return e.name.replace(/^[:@]+/, "").replace(/\(\)$/, "");
}

function candidates(e: CssEntry): string[] {
  if (e.slug in overrides) {
    const k = overrides[e.slug];
    return k === null || k === undefined ? [] : [k];
  }
  if (e.skathiOnly) return [];
  const b = bareName(e);
  switch (e.kind) {
    case "property":
    case "shorthand":
      return [`css.properties.${b}`];
    case "at-rule":
      return [`css.at-rules.${b}`];
    case "pseudo-class":
    case "pseudo-element":
      return [`css.selectors.${b}`];
    case "function":
      return [`css.types.color.${b}`, `css.types.gradient.${b}`, `css.types.image.${b}`, `css.types.${b}`];
    case "media-feature":
      return [`css.at-rules.media.${b}`];
    case "unit":
      return [`css.types.length.${b}`];
    default:
      return [];
  }
}

export function compat(entry: CssEntry, data: BcdData = browserCompatData) {
  return findCompat(data, candidates(entry), 2);
}
