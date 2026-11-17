// Fills src/data/css/bcd.ts (MDN link, experimental, deprecated per entry) from @mdn/browser-compat-data.
// Usage: bun scripts/css-bcd.ts <path to BCD data.json or its package folder> [--check]
import { existsSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { entries, type CssEntry } from "../src/data/css/entries";

const OUT = join(import.meta.dir, "../src/data/css/bcd.ts");

const KEYS: Readonly<Record<string, string | null>> = {
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
  "webkit-font-smoothing": "css.properties.font-smooth",
  "app-region": "css.properties.-webkit-app-region",
  "webkit-mask": null,
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

type Json = { readonly [key: string]: unknown };
const isObject = (x: unknown): x is Json => typeof x === "object" && x !== null && !Array.isArray(x);

function bareName(e: CssEntry): string {
  return e.name.replace(/^[:@]+/, "").replace(/\(\)$/, "");
}

function candidates(e: CssEntry): string[] {
  if (e.slug in KEYS) {
    const k = KEYS[e.slug];
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

interface Found {
  key: string;
  mdn?: string;
  experimental?: true;
  deprecated?: true;
}

function lookup(data: Json, key: string): Found | undefined {
  const parts = key.split(".");
  const chain: Json[] = [];
  let node: unknown = data;
  for (const p of parts) {
    if (!isObject(node)) return undefined;
    node = node[p];
    if (!isObject(node)) return undefined;
    chain.push(node);
  }
  const compat = (n: Json | undefined): Json | undefined =>
    n !== undefined && isObject(n["__compat"]) ? n["__compat"] : undefined;
  const own = compat(chain.at(-1));
  if (own === undefined) return undefined;
  const found: Found = { key };
  // A sub-feature without its own page links to the nearest feature that has one (stopping above css.types and the like).
  for (let i = chain.length - 1; i >= 2; i--) {
    const url = compat(chain[i])?.["mdn_url"];
    if (typeof url === "string") {
      found.mdn = url;
      break;
    }
  }
  const status = own["status"];
  if (isObject(status)) {
    if (status["experimental"] === true) found.experimental = true;
    if (status["deprecated"] === true) found.deprecated = true;
  }
  return found;
}

function render(found: ReadonlyMap<string, Found>): string {
  // Expanded objects with unquoted identifier keys, so `bun fmt` leaves the file as written.
  const key = (k: string) => (/^[A-Za-z_$][\w$]*$/.test(k) ? k : JSON.stringify(k));
  const lines = [...found].flatMap(([slug, f]) => [
    `  ${key(slug)}: {`,
    ...Object.entries(f).map(([k, v]) => `    ${k}: ${JSON.stringify(v)},`),
    "  },",
  ]);
  return [
    "// Generated by scripts/css-bcd.ts from @mdn/browser-compat-data; do not edit by hand.",
    "export interface BcdInfo {",
    "  key: string;",
    "  mdn?: string;",
    "  experimental?: true;",
    "  deprecated?: true;",
    "}",
    "",
    "export const bcd: Readonly<Record<string, BcdInfo>> = {",
    ...lines,
    "};",
    "",
  ].join("\n");
}

const [source, ...flags] = process.argv.slice(2);
if (source === undefined) {
  console.error("usage: bun scripts/css-bcd.ts <@mdn/browser-compat-data folder or data.json> [--check]");
  process.exit(2);
}
const file = existsSync(source) && statSync(source).isDirectory() ? join(source, "data.json") : source;
const parsed: unknown = JSON.parse(readFileSync(file, "utf8"));
if (!isObject(parsed)) throw new Error(`${file} is not BCD data`);
const meta = parsed["__meta"];
const version = isObject(meta) && typeof meta["version"] === "string" ? meta["version"] : "unknown";

const found = new Map<string, Found>();
const unknown: CssEntry[] = [];
for (const e of entries) {
  const hit = candidates(e)
    .map((k) => lookup(parsed, k))
    .find((f) => f !== undefined);
  if (hit === undefined) {
    if (!e.skathiOnly) unknown.push(e);
    continue;
  }
  found.set(e.slug, hit);
}

const stale = Object.keys(KEYS).filter((slug) => !entries.some((e) => e.slug === slug));
const text = render(found);
console.log(`BCD ${version}: ${found.size} of ${entries.length} entries matched`);
console.log(`not in BCD (no mdn): ${unknown.map((e) => e.slug).join(", ")}`);
console.log(
  `experimental: ${[...found]
    .filter(([, f]) => f.experimental)
    .map(([s]) => s)
    .join(", ")}`,
);
console.log(
  `deprecated on the web: ${[...found]
    .filter(([, f]) => f.deprecated)
    .map(([s]) => s)
    .join(", ")}`,
);
console.log(
  `matched without an MDN page: ${[...found]
    .filter(([, f]) => f.mdn === undefined)
    .map(([s]) => s)
    .join(", ")}`,
);
if (stale.length > 0) console.log(`key overrides for missing slugs: ${stale.join(", ")}`);

if (flags.includes("--check")) {
  const current = existsSync(OUT) ? readFileSync(OUT, "utf8") : "";
  if (current !== text) {
    console.error("src/data/css/bcd.ts is out of date; run without --check");
    process.exit(1);
  }
} else {
  writeFileSync(OUT, text);
  console.log(`wrote ${OUT}`);
}
