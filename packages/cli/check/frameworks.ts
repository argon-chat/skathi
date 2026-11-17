import { readdirSync, readFileSync, statSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import type ts from "typescript";
import { sfcCompiler } from "../runtime-path";
import { diagnosticLine, diagnosticsOf, ZERO_RANGE, type Finding, type FindingRelated } from "./finding";
import type { BuildStyles } from "./graph";
import { Packages } from "./packages";
import { svelteScripts, vueScripts, type TypeScript } from "./program";
import type { Diagnostic, Range } from "./schema";

/** Files whose Vue or Svelte usage the pass reads. */
export const FRAMEWORK_FILE = /\.(vue|svelte|[cm]?[jt]sx?)$/i;
const SCRIPT_FILE = /\.[cm]?[jt]sx?$/i;
const JSX_FILE = /\.[jt]sx$/i;

interface Rule {
  code: string;
  message: string;
}

const VAPOR: Rule = {
  code: "SK5001",
  message: "Vapor mode is not supported: Skathi renders Vue through its virtual-DOM renderer",
};
const customElement = (what: string): Rule => ({
  code: "SK5002",
  message: `${what}: Skathi has no custom elements (no \`customElements\` registry, no shadow DOM)`,
});
const ssr = (what: string): Rule => ({
  code: "SK5009",
  message: `${what} has nothing to hydrate in a Skathi window: it renders from scratch`,
});
const history = (name: string): Rule => ({
  code: "SK5008",
  message: `\`${name}()\` navigates, but back and forward do nothing: a Skathi window keeps no session history`,
});

const VUE_IMPORTS: Record<string, Rule> = {
  createVaporApp: VAPOR,
  defineVaporComponent: VAPOR,
  defineVaporAsyncComponent: VAPOR,
  defineVaporCustomElement: VAPOR,
  vaporInteropPlugin: VAPOR,
  defineCustomElement: customElement("`defineCustomElement()` makes a class that throws when constructed"),
  defineSSRCustomElement: customElement("`defineSSRCustomElement()` makes a class that throws when constructed"),
  VueElement: customElement("`VueElement` throws when constructed"),
  useHost: customElement("`useHost()` answers null"),
  useShadowRoot: customElement("`useShadowRoot()` answers null"),
  useCssModule: {
    code: "SK5004",
    message: "`useCssModule()` answers `{}`: Skathi's builders do not compile `<style module>`",
  },
  createSSRApp: ssr("`createSSRApp()`"),
  hydrate: ssr("`hydrate()`"),
};

const IMPORTS: Record<string, Record<string, Rule>> = {
  vue: VUE_IMPORTS,
  "@skathi-native/runtime": VUE_IMPORTS,
  "@vue/runtime-core": VUE_IMPORTS,
  "@vue/runtime-dom": VUE_IMPORTS,
  "vue-router": {
    createWebHistory: history("createWebHistory"),
    createWebHashHistory: history("createWebHashHistory"),
  },
  svelte: { hydrate: ssr("Svelte's `hydrate()`") },
};

/** Cheap pre-filter: a script file without any of these names has nothing to report. */
const MENTIONS = new RegExp(
  `(?<![\\w$])(${[...new Set(Object.values(IMPORTS).flatMap(Object.keys)), "\\$host"].join("|")})(?![\\w$])`,
);

const PREPROCESSED = /^(scss|sass|less|stylus|styl)$/i;

class Lines {
  private readonly starts: number[] = [0];

  constructor(text: string) {
    for (let i = 0; i < text.length; i++) if (text[i] === "\n") this.starts.push(i + 1);
  }

  range(start: number, end: number): Range {
    return { start: this.position(start), end: this.position(Math.max(start, end)) };
  }

  private position(offset: number) {
    let lo = 0;
    let hi = this.starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (this.starts[mid]! <= offset) lo = mid;
      else hi = mid - 1;
    }
    return { line: lo, character: offset - this.starts[lo]! };
  }
}

class Collector {
  readonly out: Finding[] = [];
  private readonly lines: Lines;

  constructor(
    readonly file: string,
    readonly text: string,
  ) {
    this.lines = new Lines(text);
  }

  add(rule: Rule, start: number, end: number): void {
    this.out.push({
      code: rule.code,
      message: rule.message,
      file: this.file,
      range: this.lines.range(start, end),
      related: [],
    });
  }
}

function scriptKind(typescript: TypeScript, file: string): ts.ScriptKind {
  if (/\.tsx$/i.test(file)) return typescript.ScriptKind.TSX;
  if (/\.jsx$/i.test(file)) return typescript.ScriptKind.JSX;
  if (/\.[cm]?js$/i.test(file)) return typescript.ScriptKind.JS;
  return typescript.ScriptKind.TS;
}

/** Imports of what does not work, and `$host()` in a Svelte component; `text` keeps the file's offsets. */
function scanScript(typescript: TypeScript, c: Collector, text: string, svelte: boolean): void {
  if (!MENTIONS.test(text)) return;
  const source = typescript.createSourceFile(
    c.file,
    text,
    typescript.ScriptTarget.Latest,
    true,
    scriptKind(typescript, c.file),
  );
  for (const statement of source.statements) {
    if (!typescript.isImportDeclaration(statement) || !typescript.isStringLiteral(statement.moduleSpecifier)) continue;
    const table = IMPORTS[statement.moduleSpecifier.text];
    const clause = statement.importClause;
    if (!table || !clause || clause.isTypeOnly) continue;
    const named = clause.namedBindings;
    if (!named || !typescript.isNamedImports(named)) continue;
    for (const element of named.elements) {
      const rule = element.isTypeOnly ? undefined : table[(element.propertyName ?? element.name).text];
      if (rule) c.add(rule, element.getStart(source), element.getEnd());
    }
  }
  if (!svelte || !text.includes("$host")) return;
  const visit = (node: ts.Node): void => {
    if (
      typescript.isCallExpression(node) &&
      typescript.isIdentifier(node.expression) &&
      node.expression.text === "$host"
    )
      c.add(customElement("`$host()` exists only in a custom element"), node.getStart(source), node.getEnd());
    typescript.forEachChild(node, visit);
  };
  visit(source);
}

interface TemplateNode {
  type: number;
  tag?: string;
  props?: TemplateProp[];
  children?: TemplateNode[];
  loc: { start: { offset: number }; end: { offset: number } };
}

interface TemplateProp {
  type: number;
  name: string;
  value?: { content: string } | undefined;
  arg?: { content: string; isStatic: boolean } | undefined;
  modifiers?: (string | { content: string })[];
  loc: { start: { offset: number }; end: { offset: number } };
}

const ELEMENT = 1;
const ATTRIBUTE = 6;
const DIRECTIVE = 7;

const htmlString = (what: string): Rule => ({
  code: "SK5003",
  message: `${what} renders nothing: Skathi's DOM does not parse HTML strings`,
});
const toggleInput = (type: string): Rule => ({
  code: "SK5005",
  message: `\`<input type="${type}">\` renders as a text field: Skathi has no native ${type === "radio" ? "radio buttons" : "checkboxes"}`,
});
const MOVES: Rule = {
  code: "SK5007",
  message: "moves are not animated: items that stay in the list jump to their new place",
};

const modifierNames = (prop: TemplateProp) =>
  (prop.modifiers ?? []).map((m) => (typeof m === "string" ? m : m.content));
const staticAttr = (node: TemplateNode, name: string) =>
  node.props?.find((p) => p.type === ATTRIBUTE && p.name === name);
const boundArg = (prop: TemplateProp, ...names: string[]) =>
  prop.type === DIRECTIVE && prop.name === "bind" && !!prop.arg?.isStatic && names.includes(prop.arg.content);

const TRANSITION_TAGS = new Set(["Transition", "transition", "TransitionGroup", "transition-group"]);
const ANIMATED: Rule = {
  code: "SK5014",
  message: "a CSS animation in the transition classes is not waited for: the element enters and leaves at once",
};

/** The component's own styles give this transition's enter or leave class a CSS `animation`. */
function animated(node: TemplateNode, styles: string): boolean {
  const dynamic = node.props?.some((p) => boundArg(p, "name"));
  const name = dynamic ? "[\\w-]+" : (staticAttr(node, "name")?.value?.content ?? "v").replace(/[^\w-]/g, "");
  const phase = new RegExp(`\\.${name}-(enter|leave|appear)-active(?![\\w-])`);
  for (const m of styles.matchAll(/([^{}]*)\{([^{}]*)\}/g))
    if (phase.test(m[1]!) && /(^|[\s;])animation(-name)?\s*:/.test(m[2]!)) return true;
  return false;
}

function vueTemplate(c: Collector, root: TemplateNode, styles: string): void {
  const visit = (node: TemplateNode): void => {
    if (node.type === ELEMENT && node.tag) {
      const tagEnd = node.loc.start.offset + 1 + node.tag.length;
      for (const prop of node.props ?? []) {
        if (prop.type === DIRECTIVE && prop.name === "html")
          c.add(htmlString("`v-html`"), prop.loc.start.offset, prop.loc.end.offset);
        if (boundArg(prop, "innerHTML", "inner-html"))
          c.add(htmlString("A bound `innerHTML`"), prop.loc.start.offset, prop.loc.end.offset);
        if (prop.type === DIRECTIVE && prop.name === "model" && modifierNames(prop).includes("lazy"))
          c.add(
            {
              code: "SK5010",
              message: "`v-model.lazy` updates on every keystroke: text fields here do not fire `change`",
            },
            prop.loc.start.offset,
            prop.loc.end.offset,
          );
      }
      const type = node.tag === "input" ? staticAttr(node, "type")?.value?.content.toLowerCase() : undefined;
      if (type === "checkbox" || type === "radio") c.add(toggleInput(type), node.loc.start.offset, tagEnd);
      if (TRANSITION_TAGS.has(node.tag) && animated(node, styles)) c.add(ANIMATED, node.loc.start.offset, tagEnd);
      if (node.tag === "TransitionGroup" || node.tag === "transition-group") {
        const moveClass = node.props?.some(
          (p) => (p.type === ATTRIBUTE && /^move-?class$/i.test(p.name)) || boundArg(p, "moveClass", "move-class"),
        );
        const name = staticAttr(node, "name")?.value?.content;
        const dynamicName = node.props?.some((p) => boundArg(p, "name"));
        const moveRule = dynamicName
          ? /-move(?![\w-])/.test(styles)
          : new RegExp(`\\.${(name ?? "v").replace(/[^\w-]/g, "")}-move(?![\\w-])`).test(styles);
        if (moveClass || moveRule) c.add(MOVES, node.loc.start.offset, tagEnd);
      }
    }
    for (const child of node.children ?? []) if (child && typeof child === "object") visit(child);
  };
  visit(root);
}

const tagStart = (text: string, tag: string, contentStart: number) => {
  const at = text.lastIndexOf(`<${tag}`, contentStart);
  return at < 0 ? contentStart : at;
};

function vueFile(typescript: TypeScript | null, c: Collector): void {
  const { descriptor } = sfcCompiler(dirname(c.file)).parse(c.text, { filename: c.file });
  const vapor =
    [descriptor.scriptSetup, descriptor.script].find((b) => b && b.attrs.vapor !== undefined) ??
    ((descriptor as { vapor?: boolean }).vapor ? (descriptor.scriptSetup ?? descriptor.script) : null);
  if (vapor) {
    const start = tagStart(c.text, "script", vapor.loc.start.offset);
    c.add(VAPOR, start, vapor.loc.start.offset);
  }
  for (const style of descriptor.styles) {
    const start = tagStart(c.text, "style", style.loc.start.offset);
    if (style.module)
      c.add(
        { code: "SK5004", message: "`<style module>` is not compiled: `$style` and `useCssModule()` have no classes" },
        start,
        style.loc.start.offset,
      );
    if (style.lang && PREPROCESSED.test(style.lang)) c.add(preprocessed(style.lang), start, style.loc.start.offset);
  }
  const styles = descriptor.styles.map((s) => s.content).join("\n");
  if (descriptor.template?.ast) vueTemplate(c, descriptor.template.ast as unknown as TemplateNode, styles);
  if (!typescript) return;
  const scripts = vueScripts(c.text, c.file);
  if (scripts) scanScript(typescript, c, scripts.text, false);
}

const preprocessed = (lang: string): Rule => ({
  code: "SK5011",
  message: `\`<style lang="${lang}">\` is not preprocessed: it is compiled as plain CSS`,
});

interface SvelteNode {
  type?: string;
  name?: string;
  start?: number;
  end?: number;
  value?: unknown;
  attributes?: SvelteNode[];
  [key: string]: unknown;
}

interface SvelteRoot {
  options?: { start?: number; end?: number; customElement?: unknown } | null;
  fragment?: unknown;
}

type SvelteParse = (source: string, options: { modern: true; filename?: string }) => SvelteRoot;

const svelteParsers = new Map<string, SvelteParse | null>();

/** `svelte/compiler`'s parser as the file's own app resolves it, or null without Svelte. */
function svelteParser(dir: string): SvelteParse | null {
  let path: string;
  const from = createRequire(join(dir, "package.json"));
  try {
    path = from.resolve("svelte/compiler");
  } catch {
    return null;
  }
  if (!svelteParsers.has(path)) {
    try {
      svelteParsers.set(path, (from(path) as { parse: SvelteParse }).parse);
    } catch {
      svelteParsers.set(path, null);
    }
  }
  return svelteParsers.get(path) ?? null;
}

function staticText(value: unknown): string | null {
  if (!Array.isArray(value)) return null;
  const parts = value as SvelteNode[];
  return parts.every((p) => p.type === "Text") ? parts.map((p) => String(p.data ?? "")).join("") : null;
}

const SKIP_KEYS = new Set(["metadata", "parent", "loc", "expression"]);

function svelteMarkup(c: Collector, fragment: unknown): void {
  const seen = new WeakSet<object>();
  const visit = (value: unknown): void => {
    if (!value || typeof value !== "object" || seen.has(value)) return;
    seen.add(value);
    if (Array.isArray(value)) {
      for (const item of value) visit(item);
      return;
    }
    const node = value as SvelteNode;
    const at = (n: SvelteNode, end = n.end) => [n.start ?? 0, end ?? n.start ?? 0] as const;
    if (node.type === "HtmlTag") c.add(htmlString("`{@html}`"), ...at(node));
    if (node.type === "AnimateDirective") c.add(MOVES, ...at(node));
    if (node.type === "RegularElement" && node.start !== undefined) {
      const tagEnd = node.start + 1 + String(node.name).length;
      const attrs = node.attributes ?? [];
      if (node.name === "input") {
        const type = attrs.find((a) => a.type === "Attribute" && a.name === "type");
        const text = type ? staticText(type.value)?.toLowerCase() : undefined;
        if (text === "checkbox" || text === "radio") c.add(toggleInput(text), node.start, tagEnd);
      }
      if (node.name === "select") {
        const bind = attrs.find((a) => a.type === "BindDirective" && a.name === "value");
        if (bind)
          c.add(
            {
              code: "SK5006",
              message: "`bind:value` on a `<select>` reads the first option, not the one picked",
            },
            ...at(bind),
          );
      }
    }
    for (const [key, child] of Object.entries(node)) if (!SKIP_KEYS.has(key)) visit(child);
  };
  visit(fragment);
}

const blank = (text: string) => text.replace(/[^\r\n]/g, " ");

function svelteFile(typescript: TypeScript | null, c: Collector): void {
  const markup = c.text.replace(/<!--[\s\S]*?-->/g, blank);
  for (const m of markup.matchAll(/<style(\s[^>]*)?>/gi)) {
    const lang = /\blang\s*=\s*["']?([\w-]+)/i.exec(m[1] ?? "")?.[1];
    if (lang && PREPROCESSED.test(lang)) c.add(preprocessed(lang), m.index, m.index + m[0].length);
  }
  const parse = svelteParser(dirname(c.file));
  if (parse) {
    let root: SvelteRoot | null = null;
    try {
      root = parse(c.text, { modern: true, filename: c.file });
    } catch {}
    if (root?.options?.customElement !== undefined && root.options.customElement !== null)
      c.add(
        customElement("`<svelte:options customElement>` defines an element that cannot be registered"),
        root.options.start ?? 0,
        root.options.end ?? 0,
      );
    if (root) svelteMarkup(c, root.fragment);
  }
  if (!typescript) return;
  const scripts = svelteScripts(c.text);
  if (scripts) scanScript(typescript, c, scripts.text, true);
}

/** One file's findings; a file that does not parse has none (the build reports it). */
export function fileFrameworkFindings(file: string, text: string, typescript: TypeScript | null): Finding[] {
  const c = new Collector(file, text);
  try {
    if (/\.vue$/i.test(file)) vueFile(typescript, c);
    else if (/\.svelte$/i.test(file)) svelteFile(typescript, c);
    else if (typescript && SCRIPT_FILE.test(file)) scanScript(typescript, c, text, false);
  } catch {
    return [];
  }
  return c.out;
}

export interface FrameworkSource {
  /** Absolute. */
  file: string;
  text: string;
}

/** Each readable file once. */
export function readFrameworkSources(files: Iterable<string>): FrameworkSource[] {
  const out: FrameworkSource[] = [];
  for (const file of new Set(files)) {
    if (!FRAMEWORK_FILE.test(file)) continue;
    try {
      out.push({ file, text: readFileSync(file, "utf8").replace(/^﻿/, "") });
    } catch {}
  }
  return out;
}

const MAX_RELATED = 50;

/**
 * Vue and Svelte features that do not work, or work differently, in the app's files. A dependency's findings
 * become one SK5013 per package, its places listed as related locations.
 */
export function frameworkFindings(input: {
  sources: FrameworkSource[];
  packages: Packages;
  deps: boolean;
  typescript: TypeScript | null;
}): Finding[] {
  const out: Finding[] = [];
  const byPackage = new Map<string, { name: string; found: Finding[] }>();
  for (const { file, text } of input.sources) {
    const owner = input.packages.ownerOf(file);
    if (owner && !input.deps) continue;
    const found = fileFrameworkFindings(file, text, input.typescript);
    if (!owner) {
      if (JSX_FILE.test(file))
        found.push({
          code: "SK5012",
          message: "`sk dev` does not load .tsx and .jsx files: importing this one fails in development",
          file,
          range: ZERO_RANGE,
          related: [],
        });
      out.push(...found);
      continue;
    }
    if (!found.length) continue;
    const group = byPackage.get(owner.dir) ?? { name: owner.name, found: [] };
    group.found.push(...found);
    byPackage.set(owner.dir, group);
  }
  for (const { name, found } of byPackage.values()) {
    const first = found[0]!;
    const related: FindingRelated[] = found
      .slice(0, MAX_RELATED)
      .map((f) => ({ file: f.file, range: f.range, message: `${f.code}: ${f.message}` }));
    out.push({
      code: "SK5013",
      message: `${name} uses ${found.length === 1 ? "a Vue or Svelte feature" : `${found.length} Vue or Svelte features`} that Skathi does not support`,
      file: first.file,
      range: first.range,
      help: "Check that the parts of the package the app uses work; allow SK5013 for the package when they do.",
      related,
    });
  }
  return out;
}

let bundledTypeScript: TypeScript | null | undefined;

/** The CLI's own TypeScript, for the import scan of sk build and sk dev; null if it cannot load. */
export function cliTypeScript(): TypeScript | null {
  if (bundledTypeScript === undefined) {
    try {
      bundledTypeScript = createRequire(join(import.meta.dir, "..", "package.json"))("typescript") as TypeScript;
    } catch {
      bundledTypeScript = null;
    }
  }
  return bundledTypeScript;
}

export interface FrameworkPassOptions {
  root: string;
  /** sk.config.json `check.allow`. */
  allow: Map<string, string[]>;
}

/** The framework pass of `sk check` on its own, as sk build and sk dev report it. */
export function frameworkDiagnostics(
  options: FrameworkPassOptions & { sources: FrameworkSource[]; packages?: Packages | undefined },
): Diagnostic[] {
  const root = resolve(options.root);
  const packages = options.packages ?? new Packages(root);
  const findings = frameworkFindings({ sources: options.sources, packages, deps: true, typescript: cliTypeScript() });
  return diagnosticsOf(findings, root, packages, options.allow).diagnostics;
}

/** sk dev: the pass over the files whose text changed since the last call, each diagnostic once per session. */
export class FrameworkWarnings {
  private readonly texts = new Map<string, string>();
  private readonly shown = new Set<string>();
  private readonly packages: Packages;

  constructor(private readonly options: FrameworkPassOptions) {
    this.packages = new Packages(resolve(options.root));
  }

  update(files: Iterable<string>): Diagnostic[] {
    const sources = readFrameworkSources(files).filter((s) => this.texts.get(s.file) !== s.text);
    if (!sources.length) return [];
    for (const s of sources) this.texts.set(s.file, s.text);
    const nth = new Map<string, number>();
    return frameworkDiagnostics({ ...this.options, sources, packages: this.packages }).filter((d) => {
      const what = `${d.file}\0${d.code}\0${d.message}`;
      const n = (nth.get(what) ?? 0) + 1;
      nth.set(what, n);
      const key = `${what}\0${n}`;
      if (this.shown.has(key)) return false;
      this.shown.add(key);
      return true;
    });
  }
}

/**
 * sk build and sk publish: the pass over the components the bundle compiled (styles.json) and the script files
 * beside the entry, one line each.
 */
export function frameworkBuildLines(options: {
  appRoot: string;
  entry: string;
  stylesJson: string;
  allow: Map<string, string[]>;
}): string[] {
  let components: string[] = [];
  try {
    components = (JSON.parse(readFileSync(options.stylesJson, "utf8")) as BuildStyles).files;
  } catch {}
  const sources = readFrameworkSources([...components, ...appScripts(dirname(options.entry))]);
  return frameworkDiagnostics({ root: options.appRoot, allow: options.allow, sources }).map(diagnosticLine);
}

function appScripts(dir: string, out: string[] = []): string[] {
  let names: string[];
  try {
    names = readdirSync(dir);
  } catch {
    return out;
  }
  for (const name of names) {
    if (name === "node_modules" || name === "dist" || name.startsWith(".")) continue;
    const path = join(dir, name);
    try {
      if (statSync(path).isDirectory()) appScripts(path, out);
      else if (SCRIPT_FILE.test(name) && !name.endsWith(".d.ts")) out.push(path);
    } catch {}
  }
  return out;
}
