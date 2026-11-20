import { sfcCompiler } from "../runtime-path";
import type { AST } from "svelte/compiler";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import postcss, { type ChildNode, type Root } from "postcss";
import { inspectCss, type CssNode } from "./css-engine";
import { diagnosticsOf, ZERO_RANGE, type Finding, type FindingRelated } from "./finding";
import { Packages } from "./packages";
import type { Diagnostic, Range } from "./schema";

interface Block {
  css: string;
  /** Offset of `css` in the file. */
  offset: number;
  /** Declarations only (a `style` attribute). */
  inline: boolean;
  tag?: string;
  scoped?: boolean;
}

/** The CSS of one file at its own offsets: a stylesheet, the `<style>` blocks of a component or page, its static `style` attributes. */
export function styleBlocks(path: string, source: string): Block[] {
  if (/\.css$/i.test(path)) return [{ css: source, offset: 0, inline: false }];
  if (/\.vue$/i.test(path)) return vueBlocks(path, source);
  if (/\.(svelte|html?)$/i.test(path)) return markupBlocks(source);
  return [];
}

const plainCss = (lang: string | undefined) => !lang || lang === "css" || lang === "postcss";

function vueBlocks(path: string, source: string): Block[] {
  const { descriptor } = sfcCompiler(dirname(path)).parse(source, { filename: path });
  const blocks: Block[] = descriptor.styles
    .filter((s) => !s.src && plainCss(s.lang))
    .map((s) => ({
      css: s.content,
      offset: s.loc.start.offset,
      inline: false,
      scoped: s.scoped ?? false,
    }));
  type Node = { type: number; tag?: string; props?: unknown[]; children?: unknown[] };
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
        ...(node.tag && { tag: node.tag }),
      });
    }
    for (const child of (node.children ?? []) as Node[]) if (child && typeof child === "object") visit(child);
  };
  if (descriptor.template?.ast) visit(descriptor.template.ast as unknown as Node);
  return blocks;
}

const blank = (text: string) => text.replace(/[^\r\n]/g, " ");

function markupBlocks(source: string): Block[] {
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
  markup = markup.replace(/<(script|style)\b[\s\S]*?<\/\1\s*>/gi, blank);
  for (const element of markup.matchAll(/<([a-zA-Z][\w:-]*)\b((?:[^>"']|"[^"]*"|'[^']*')*)>/g)) {
    const tag = element[1]!;
    for (const attr of element[2]!.matchAll(/\s+([^\s="'<>/]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>]+)))?/g)) {
      const css = attr[2] ?? attr[3];
      if (attr[1]?.toLowerCase() !== "style" || css === undefined || /[{}]/.test(css)) continue;
      const offset = element.index + 1 + tag.length + attr.index + attr[0].length - 1 - css.length;
      blocks.push({ css, offset, inline: true, tag: tag.toLowerCase() });
    }
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

function frameworkSelector(file: string, block: Block, selector: string): string {
  if (block.scoped && /:(?:v-)?(?:deep|global|slotted)\b|>>>|\/deep\//.test(selector)) {
    const result = sfcCompiler(dirname(file)).compileStyle({
      filename: file,
      id: "data-v-sk",
      scoped: true,
      source: `${selector}{color:red}`,
    });
    const rule = postcss.parse(result.code).nodes.find((node) => node.type === "rule");
    return rule?.selector ?? selector;
  }
  if (!/\.svelte$/i.test(file) || !selector.includes(":global")) return selector;
  const compiler = createRequire(resolve(file))("svelte/compiler") as typeof import("svelte/compiler");
  const prefix = "<style>";
  const source = prefix + selector + "{}</style>";
  const rule = compiler.parse(source, { modern: true }).css?.children[0];
  if (rule?.type !== "Rule") return selector;
  const remove: [number, number][] = [];
  const visit = (list: AST.CSS.SelectorList) => {
    for (const compound of list.children.flatMap((s) => s.children)) {
      for (const token of compound.selectors) {
        if (token.type !== "PseudoClassSelector" && token.type !== "PseudoElementSelector") continue;
        if (token.name === "global") {
          if (token.args) remove.push([token.start, token.args.start], [token.args.end, token.end]);
          else remove.push([token.start, token.end]);
        }
        if (token.args) visit(token.args);
      }
    }
  };
  visit(rule.prelude);
  for (const [start, end] of remove.sort(([a], [b]) => b - a))
    selector = selector.slice(0, start - prefix.length) + selector.slice(end - prefix.length);
  return selector.trim() || "*";
}

function syntax(nodes: ChildNode[], file: string, block: Block): CssNode[] {
  const result: CssNode[] = [];
  for (const node of nodes) {
    const start = node.source?.start?.offset ?? 0;
    if (node.type === "decl") {
      result.push({
        kind: "declaration",
        name: node.prop.startsWith("--") ? node.prop : node.prop.toLowerCase(),
        value: node.value,
        important: node.important === true,
        start,
        end: start + node.toString().length,
      });
    } else if (node.type === "rule" && !block.inline) {
      result.push({
        kind: "rule",
        selector: frameworkSelector(file, block, node.selector),
        nodes: syntax(node.nodes, file, block),
        start,
        end: start + node.selector.length,
      });
    } else if (node.type === "atrule" && !block.inline) {
      if (node.name.toLowerCase() === "haptic-pattern") {
        const text = node.toString();
        result.push({
          kind: "at",
          name: "haptic-pattern",
          params: node.params,
          body: node.nodes ? text.slice(text.indexOf("{") + 1, text.lastIndexOf("}")) : null,
          nodes: [],
          start,
          end: start + node.name.length + 1,
        });
        continue;
      }
      const body =
        node.nodes
          ?.filter((n) => n.type === "decl")
          .map((n) => n.toString())
          .join(";") ?? null;
      result.push({
        kind: "at",
        name: node.name.toLowerCase(),
        params: node.params,
        body,
        nodes: syntax(node.nodes ?? [], file, block),
        start,
        end: start + node.name.length + 1,
      });
    }
  }
  return result;
}

/** CSS findings at source locations before Vue/Svelte scope rewriting. */
export function fileFindings(
  file: string,
  source: string,
  tailwind: boolean,
  emittedMessages?: ReadonlySet<string>,
): Finding[] {
  const out: Finding[] = [];
  const lines = new Lines(source);
  for (const block of styleBlocks(file, source)) {
    let root: Root;
    try {
      root = postcss.parse(block.css);
    } catch {
      continue;
    }
    const nodes = syntax(root.nodes, file, block);
    if (!nodes.length) continue;
    for (const { start, end, ...diagnostic } of inspectCss({ nodes, tag: block.tag ?? null, tailwind })) {
      if (!block.inline && emittedMessages && !emittedMessages.has(diagnostic.message)) continue;
      out.push({ ...diagnostic, file, range: lines.range(block.offset + start, block.offset + end), related: [] });
    }
  }
  return out;
}

const MAX_RELATED = 50;

/** A stylesheet, component or page and its text; offsets inside it come from `styleBlocks`. */
export interface StyleSource {
  /** Absolute. */
  file: string;
  text: string;
}

/** Each readable file once; one that cannot be read is left out. */
export function readStyleSources(files: Iterable<string>): StyleSource[] {
  const out: StyleSource[] = [];
  for (const file of new Set(files)) {
    try {
      out.push({ file, text: readFileSync(file, "utf8").replace(/^﻿/, "") });
    } catch {}
  }
  return out;
}

export interface CssPassOptions {
  root: string;
  /** sk.config.json `check.allow`. */
  allow: Map<string, string[]>;
  tailwind: boolean;
  compiled?: { css: string; entry: string } | undefined;
}

/** The CSS pass of `sk check` on its own, dependencies included, as sk build and sk dev report it. */
export function cssDiagnostics(
  options: CssPassOptions & { sources: StyleSource[]; packages?: Packages | undefined },
): Diagnostic[] {
  const root = resolve(options.root);
  const packages = options.packages ?? new Packages(root);
  const findings = cssFindings({
    sources: options.sources,
    packages,
    deps: true,
    tailwind: options.tailwind,
    compiled: options.compiled,
  });
  return diagnosticsOf(findings, root, packages, options.allow).diagnostics;
}

/** sk dev: the pass over the files whose text changed since the last call, each warning once per session. */
export class CssWarnings {
  private readonly texts = new Map<string, string>();
  private readonly shown = new Set<string>();
  private readonly packages: Packages;

  constructor(private readonly options: CssPassOptions) {
    this.packages = new Packages(resolve(options.root));
  }

  update(files: Iterable<string>): Diagnostic[] {
    const sources = readStyleSources(files).filter((s) => this.texts.get(s.file) !== s.text);
    if (!sources.length) return [];
    for (const s of sources) this.texts.set(s.file, s.text);
    // Keyed by place in order rather than line, so an edit above a warning does not repeat it.
    const nth = new Map<string, number>();
    return cssDiagnostics({ ...this.options, sources, packages: this.packages }).filter((d) => {
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
 * CSS the engine does not apply, in the app's components, pages and stylesheets. A dependency's
 * findings become one SK4005 per package, its places listed as related locations.
 */
export function cssFindings(input: {
  sources: StyleSource[];
  packages: Packages;
  deps: boolean;
  tailwind: boolean;
  compiled?: CssPassOptions["compiled"];
}): Finding[] {
  const out: Finding[] = [];
  const byPackage = new Map<string, { name: string; found: Finding[] }>();
  // Scope and PostCSS transforms can remove source warnings or introduce new ones.
  const emitted = input.compiled && fileFindings(input.compiled.entry + ".css", input.compiled.css, false);
  const messages = emitted && new Set(emitted.map((finding) => finding.message));
  const located = new Set<string>();
  let skippedDependency = false;
  for (const { file, text } of input.sources) {
    const owner = input.packages.ownerOf(file);
    if (owner && !input.deps) {
      skippedDependency = true;
      continue;
    }
    const found = fileFindings(file, text, input.tailwind, messages);
    if (emitted) for (const finding of found) located.add(finding.message);
    if (!owner) {
      out.push(...found);
      continue;
    }
    const group = byPackage.get(owner.dir) ?? { name: owner.name, found: [] };
    group.found.push(...found);
    byPackage.set(owner.dir, group);
  }
  if (emitted && input.compiled && !skippedDependency) {
    for (const finding of emitted) {
      if (located.has(finding.message)) continue;
      out.push({
        ...finding,
        file: input.compiled.entry,
        range: ZERO_RANGE,
        message: `In the compiled stylesheet: ${finding.message}`,
      });
      located.add(finding.message);
    }
  }
  for (const { name, found } of byPackage.values()) {
    const first = found[0];
    if (!first) continue;
    const related: FindingRelated[] = found
      .slice(0, MAX_RELATED)
      .map((f) => ({ file: f.file, range: f.range, message: f.message }));
    out.push({
      code: "SK4005",
      message: `${name}'s CSS has ${found.length} rendering or host compatibility warning${found.length === 1 ? "" : "s"}`,
      file: first.file,
      range: first.range,
      help: "Check that the parts of the package the app uses look right; allow SK4005 for the package when they do.",
      related,
    });
  }
  return out;
}
