import { parse as parseSfc } from "@vue/compiler-sfc";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import postcss, { type AtRule, type ChildNode, type Container, type Declaration, type Root, type Rule } from "postcss";
import type { Finding, FindingRelated } from "./finding";
import type { Packages } from "./packages";
import type { Range } from "./schema";

interface CatalogEntry {
  name: string;
  status: "supported" | "partial" | "ignored" | "unsupported" | "dropped";
  note: string;
  applied: string[];
  refused: string[];
  webkit: boolean;
}

const catalog = JSON.parse(readFileSync(join(import.meta.dir, "css-catalog.json"), "utf8")) as CatalogEntry[];
const ENTRIES = new Map(catalog.map((e) => [e.name.toLowerCase(), e]));

const squash = (value: string) => value.trim().replace(/\s+/g, " ").toLowerCase();
const functionsOf = (value: string) =>
  [...value.matchAll(/(?<![\w-])([a-z][\w-]*)\(/gi)].map((m) => m[1]!.toLowerCase());

/** `name` when the value is one function call and nothing else. */
function singleCall(value: string): string | null {
  const text = value.trim();
  const m = /^([a-z][\w-]*)\(/i.exec(text);
  if (!m) return null;
  let depth = 0;
  for (let i = m[0].length - 1; i < text.length; i++) {
    if (text[i] === "(") depth++;
    else if (text[i] === ")" && --depth === 0) return i === text.length - 1 ? m[1]!.toLowerCase() : null;
  }
  return null;
}

/** Functions a partial property refuses whatever their arguments: a refused sample that is one call, in no applied sample. */
const REFUSED_FUNCTIONS = new Map(
  catalog.map((e) => {
    const applied = new Set(e.applied.flatMap(functionsOf));
    const refused = e.refused.map(singleCall).filter((f): f is string => f !== null && !applied.has(f));
    return [e.name.toLowerCase(), new Set(refused)];
  }),
);

const CSS_WIDE = new Set(["inherit", "initial", "unset", "revert", "revert-layer"]);
/** Unsupported properties set to what a browser would compute anyway: no difference to report. */
const INERT_DEFAULTS = new Set(["none", "normal", "auto"]);
const INERT_FOR: Record<string, string> = {
  direction: "ltr",
  "writing-mode": "horizontal-tb",
  "backface-visibility": "visible",
  "transform-style": "flat",
  "border-collapse": "separate",
  "text-orientation": "mixed",
};
const SUBSTITUTED = /(?<![\w-])(var|env|theme|screen)\(|#\{|\$[\w-]/i;

const UNSUPPORTED_AT_RULES: Record<string, string> = {
  container: "Use a `@media` query on the window size, or set a class from script.",
  layer: "Move the rules out of the `@layer` block; their order in the sheet decides the cascade.",
  scope: "Scope the rules with a class selector instead.",
  "starting-style": "Start the transition from script: set the start values, then change them on the next frame.",
  "counter-style": "Use one of the predefined list styles (`disc`, `decimal`, …) or draw the marker yourself.",
  "font-feature-values": "Use `font-feature-settings` with the feature tags.",
  namespace: "Drop the namespace and select by element name or class.",
};
const RECURSE_AT_RULES = new Set(["media", "keyframes", "-webkit-keyframes", "-moz-keyframes"]);

const STATE_FIX = "Set a class or data attribute from script and select on that.";
const PSEUDO_CLASSES: Record<string, string> = Object.fromEntries([
  ...[
    "placeholder-shown",
    "read-only",
    "read-write",
    "required",
    "optional",
    "indeterminate",
    "default",
    "autofill",
    "valid",
    "invalid",
    "user-valid",
    "user-invalid",
    "in-range",
    "out-of-range",
    "blank",
    "link",
    "visited",
    "any-link",
    "local-link",
    "target",
    "target-within",
    "lang",
    "dir",
    "modal",
    "popover-open",
    "open",
    "closed",
    "defined",
    "picture-in-picture",
    "playing",
    "paused",
    "seeking",
    "buffering",
    "stalled",
    "muted",
    "volume-locked",
    "current",
    "past",
    "future",
  ].map((name) => [name, STATE_FIX]),
]);
const LADDER_STATES = /:(hover|active|focus|focus-visible|focus-within)(?![\w-])/i;
const FUNCTIONAL = /:(not|is|where|matches|any|-webkit-any|-moz-any)\(/gi;

interface Block {
  css: string;
  /** Offset of `css` in the file. */
  offset: number;
  /** Declarations only (a `style` attribute). */
  inline: boolean;
}

/** The CSS of one file at its own offsets: a stylesheet, the `<style>` blocks of a component or page, its static `style` attributes. */
export function styleBlocks(path: string, source: string): Block[] {
  if (/\.css$/i.test(path)) return [{ css: source, offset: 0, inline: false }];
  if (/\.vue$/i.test(path)) return vueBlocks(path, source);
  if (/\.(svelte|html?)$/i.test(path)) return markupBlocks(source, /\.svelte$/i.test(path));
  return [];
}

const plainCss = (lang: string | undefined) => !lang || lang === "css" || lang === "postcss";

function vueBlocks(path: string, source: string): Block[] {
  const { descriptor } = parseSfc(source, { filename: path });
  const blocks: Block[] = descriptor.styles
    .filter((s) => !s.src && plainCss(s.lang))
    .map((s) => ({
      css: s.content,
      offset: s.loc.start.offset,
      inline: false,
    }));
  type Node = { type: number; props?: unknown[]; children?: unknown[] };
  type Attr = {
    type: number;
    name: string;
    value?: { loc: { start: { offset: number }; end: { offset: number } } };
  };
  const visit = (node: Node) => {
    for (const prop of (node.props ?? []) as Attr[]) {
      if (prop.type !== 6 || prop.name !== "style" || !prop.value) continue;
      let start = prop.value.loc.start.offset;
      let end = prop.value.loc.end.offset;
      if (/["']/.test(source[start] ?? "")) {
        start++;
        end--;
      }
      blocks.push({
        css: source.slice(start, end),
        offset: start,
        inline: true,
      });
    }
    for (const child of (node.children ?? []) as Node[]) if (child && typeof child === "object") visit(child);
  };
  if (descriptor.template?.ast) visit(descriptor.template.ast as unknown as Node);
  return blocks;
}

const blank = (text: string) => text.replace(/[^\r\n]/g, " ");

function markupBlocks(source: string, attributes: boolean): Block[] {
  const blocks: Block[] = [];
  let markup = source.replace(/<!--[\s\S]*?-->/g, blank);
  for (const m of markup.matchAll(/<style(\s[^>]*)?>([\s\S]*?)<\/style\s*>/gi)) {
    const lang = /\blang\s*=\s*["']?([\w-]+)/i.exec(m[1] ?? "")?.[1]?.toLowerCase();
    if (plainCss(lang))
      blocks.push({
        css: m[2]!,
        offset: m.index + m[0].indexOf(">") + 1,
        inline: false,
      });
  }
  if (!attributes) return blocks;
  markup = markup.replace(/<(script|style)\b[\s\S]*?<\/\1\s*>/gi, blank);
  for (const m of markup.matchAll(/<[a-zA-Z][^\s/>]*\s[^>]*?\bstyle\s*=\s*(["'])([^"'{}]*)\1/g)) {
    const offset = m.index + m[0].length - 1 - m[2]!.length;
    blocks.push({ css: m[2]!, offset, inline: true });
  }
  return blocks;
}

class Lines {
  private readonly starts: number[] = [0];

  constructor(text: string) {
    for (let i = 0; i < text.length; i++) if (text[i] === "\n") this.starts.push(i + 1);
  }

  position(offset: number) {
    let lo = 0;
    let hi = this.starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.starts[mid]! <= offset) lo = mid;
      else hi = mid - 1;
    }
    return { line: lo, character: offset - this.starts[lo]! };
  }

  range(start: number, end: number): Range {
    return {
      start: this.position(start),
      end: this.position(Math.max(start, end)),
    };
  }
}

const sentence = (text: string) => (text && !/[.!?)]$/.test(text) ? `${text}.` : text);

function entryOf(property: string): CatalogEntry | undefined {
  const own = ENTRIES.get(property);
  if (own) return own;
  const base = property.startsWith("-webkit-") ? ENTRIES.get(property.slice(8)) : undefined;
  return base?.webkit ? base : undefined;
}

/** What the engine does not apply in one declaration: a code, what is wrong and what to do. */
export function declarationProblem(
  property: string,
  value: string,
): { code: string; message: string; help: string } | null {
  const prop = property.trim().toLowerCase();
  if (prop.startsWith("--")) return null;
  const entry = entryOf(prop);
  if (!entry) return null;
  const v = squash(value);
  if (!v || CSS_WIDE.has(v) || SUBSTITUTED.test(value)) return null;
  const keep = `Remove it, or keep it for browsers inside \`@supports (${prop}: ${value.trim()})\`.`;
  if (entry.status === "unsupported") {
    if (INERT_DEFAULTS.has(v) || INERT_FOR[prop] === v) return null;
    return {
      code: "SK4001",
      message: `\`${prop}\` is not supported: this declaration has no effect`,
      help: [sentence(entry.note), keep].filter(Boolean).join(" "),
    };
  }
  if (entry.status !== "partial") return null;
  const refused = entry.refused.some((r) => squash(r) === v)
    ? v
    : functionsOf(v).find((f) => REFUSED_FUNCTIONS.get(entry.name.toLowerCase())?.has(f));
  if (!refused) return null;
  const what = refused === v ? `\`${prop}: ${value.trim()}\`` : `\`${refused}()\` in \`${prop}\``;
  return {
    code: "SK4002",
    message: `${what} is not applied`,
    help: [sentence(entry.note), keep].filter(Boolean).join(" "),
  };
}

/** Selector parts no rule can match here, each with what to do instead. */
export function selectorProblems(selector: string): { what: string; help: string }[] {
  const text = selector
    .replace(/\\./g, "  ")
    .replace(/\/\*[\s\S]*?\*\//g, blank)
    .replace(/"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'/g, blank)
    .replace(/\[[^\]]*\]/g, blank);
  const out = new Map<string, string>();
  for (const m of text.matchAll(/(?<!:):(?!:)(-webkit-)?([a-z][\w-]*)(\()?/gi)) {
    const name = m[2]!.toLowerCase();
    if (m[1] && name !== "autofill") continue;
    const help = PSEUDO_CLASSES[name];
    if (help) out.set(`the pseudo-class \`:${name}${m[3] ? "()" : ""}\``, help);
  }
  for (const m of text.matchAll(
    /::?(first-line|first-letter)(?![\w-])|::(marker|-webkit-scrollbar[\w-]*)(?![\w-])/gi,
  )) {
    const name = (m[1] ?? m[2]!).toLowerCase();
    out.set(
      `the pseudo-element \`::${name}\``,
      name.startsWith("-webkit-scrollbar")
        ? "Style scrollbars with `scrollbar-width` and `scrollbar-color`."
        : "Wrap the part in its own element and style that.",
    );
  }
  if (/:nth-(last-)?child\([^)]*\sof\s/i.test(text))
    out.set("`:nth-child(… of S)`", "Select the subset with a class and use `:nth-child()` without `of`.");
  for (const m of text.matchAll(FUNCTIONAL)) {
    let depth = 1;
    let i = m.index + m[0].length;
    const from = i;
    for (; i < text.length && depth > 0; i++) {
      if (text[i] === "(") depth++;
      else if (text[i] === ")") depth--;
    }
    const state = LADDER_STATES.exec(text.slice(from, i - 1));
    if (state)
      out.set(
        `\`:${state[1]!.toLowerCase()}\` inside \`:${m[1]!.toLowerCase()}()\``,
        `Put \`:${state[1]!.toLowerCase()}\` on the compound itself, outside the parentheses, or set a class from script.`,
      );
  }
  return [...out].map(([what, help]) => ({ what, help }));
}

interface Context {
  file: string;
  lines: Lines;
  offset: number;
  tailwind: boolean;
  out: Finding[];
}

function push(ctx: Context, code: string, message: string, help: string, start: number, end: number) {
  ctx.out.push({
    code,
    message,
    file: ctx.file,
    range: ctx.lines.range(ctx.offset + start, ctx.offset + end),
    help,
    related: [],
  });
}

const startOf = (node: ChildNode) => node.source?.start?.offset ?? 0;

function visit(ctx: Context, container: Container, keyframes: boolean) {
  for (const node of container.nodes ?? []) {
    if (node.type === "decl") declaration(ctx, node);
    else if (node.type === "rule") rule(ctx, node, keyframes);
    else if (node.type === "atrule") atRule(ctx, node);
  }
}

function declaration(ctx: Context, decl: Declaration) {
  const problem = declarationProblem(decl.prop, decl.value);
  if (!problem) return;
  const start = startOf(decl);
  push(ctx, problem.code, problem.message, problem.help, start, start + decl.toString().length);
}

function rule(ctx: Context, node: Rule, keyframes: boolean) {
  if (!keyframes) {
    const start = startOf(node);
    for (const { what, help } of selectorProblems(node.selector))
      push(
        ctx,
        "SK4003",
        `${what} never matches here, so this rule is dropped`,
        help,
        start,
        start + node.selector.length,
      );
  }
  visit(ctx, node, keyframes);
}

function atRule(ctx: Context, node: AtRule) {
  const name = node.name.toLowerCase();
  if (RECURSE_AT_RULES.has(name)) {
    visit(ctx, node, name !== "media");
    return;
  }
  // Tailwind consumes `@layer` blocks before the engine sees them; a bare `@layer a, b;` changes nothing.
  if (name === "layer" && ctx.tailwind) {
    visit(ctx, node, false);
    return;
  }
  const help = UNSUPPORTED_AT_RULES[name];
  if (!help || (name === "layer" && node.nodes === undefined)) return;
  const start = startOf(node);
  push(
    ctx,
    "SK4004",
    `\`@${name}\` is not supported: ${node.nodes === undefined ? "it is ignored" : "the rules inside it are dropped"}`,
    help,
    start,
    start + name.length + 1,
  );
}

/** One file's CSS findings. A block that does not parse is skipped: the build reports it. */
export function fileFindings(file: string, source: string, tailwind: boolean): Finding[] {
  const out: Finding[] = [];
  const lines = new Lines(source);
  for (const block of styleBlocks(file, source)) {
    let root: Root;
    try {
      root = postcss.parse(block.css);
    } catch {
      continue;
    }
    const ctx: Context = { file, lines, offset: block.offset, tailwind, out };
    if (block.inline) for (const node of root.nodes) node.type === "decl" && declaration(ctx, node);
    else visit(ctx, root, false);
  }
  return out;
}

const MAX_RELATED = 50;

/**
 * CSS the engine does not apply, in the app's components, pages and stylesheets. A dependency's
 * findings become one SK4005 per package, its places listed as related locations.
 */
export function cssFindings(input: {
  files: string[];
  packages: Packages;
  deps: boolean;
  tailwind: boolean;
}): Finding[] {
  const out: Finding[] = [];
  const byPackage = new Map<string, { name: string; found: Finding[] }>();
  for (const file of new Set(input.files)) {
    const owner = input.packages.ownerOf(file);
    if (owner && !input.deps) continue;
    let source: string;
    try {
      source = readFileSync(file, "utf8").replace(/^\uFEFF/, "");
    } catch {
      continue;
    }
    let found: Finding[];
    try {
      found = fileFindings(file, source, input.tailwind);
    } catch {
      continue;
    }
    if (!owner) {
      out.push(...found);
      continue;
    }
    const group = byPackage.get(owner.dir) ?? { name: owner.name, found: [] };
    group.found.push(...found);
    byPackage.set(owner.dir, group);
  }
  for (const { name, found } of byPackage.values()) {
    const first = found[0];
    if (!first) continue;
    const related: FindingRelated[] = found
      .slice(0, MAX_RELATED)
      .map((f) => ({ file: f.file, range: f.range, message: f.message }));
    out.push({
      code: "SK4005",
      message: `${found.length === 1 ? "1 rule or declaration" : `${found.length} rules and declarations`} in ${name}'s CSS ${found.length === 1 ? "is" : "are"} not applied by Skathi`,
      file: first.file,
      range: first.range,
      help: "Check that the parts of the package the app uses look right; allow SK4005 for the package when they do.",
      related,
    });
  }
  return out;
}
