import {
  transformerMetaHighlight,
  transformerNotationDiff,
  transformerNotationFocus,
  transformerNotationHighlight,
} from "@shikijs/transformers";
import type { ShikiTransformer } from "shiki";

const lineClass = (name: string, lines: ReadonlySet<number>, preClass: string): ShikiTransformer => ({
  name: `skathi:${name}`,
  line(node, line) {
    if (lines.has(line)) this.addClassToHast(node, name);
  },
  pre(node) {
    if (lines.size) this.addClassToHast(node, preClass);
  },
});

export function parseLines(spec: string | undefined, regions: ReadonlyMap<string, Range> = new Map()): Set<number> {
  const lines = new Set<number>();
  for (const part of (spec ?? "")
    .split(",")
    .map((p) => p.trim())
    .filter(Boolean)) {
    const range = /^(\d+)(?:-(\d+))?$/.exec(part);
    if (range) {
      const from = Number(range[1]);
      const to = Number(range[2] ?? range[1]);
      for (let line = from; line <= to; line++) lines.add(line);
      continue;
    }
    const region = regions.get(part);
    if (!region) throw new Error(`Unknown line range or region "${part}".`);
    for (let line = region.start + 1; line <= region.end; line++) lines.add(line);
  }
  return lines;
}

const metaValue = (meta: string, key: string): string | undefined =>
  new RegExp(`(?:^|\\s)${key}=(?:"([^"]*)"|(\\S+))`)
    .exec(meta)
    ?.slice(1)
    .find((v) => v !== undefined);

const focusCache = new Map<string, Set<number>>();
const metaFocus = (meta: string): Set<number> => {
  let lines = focusCache.get(meta);
  if (!lines) focusCache.set(meta, (lines = parseLines(metaValue(meta, "focus"))));
  return lines;
};

const metaTransformer: ShikiTransformer = {
  name: "skathi:meta",
  line(node, line) {
    if (metaFocus(this.options.meta?.__raw ?? "").has(line)) this.addClassToHast(node, "focused");
  },
  pre(node) {
    const meta = this.options.meta?.__raw ?? "";
    if (metaFocus(meta).size) this.addClassToHast(node, "has-focused");
    const title = metaValue(meta, "title");
    if (title) node.properties["data-title"] = title;
    for (const key of ["os", "framework"]) {
      const value = metaValue(meta, key);
      if (value) node.properties[`data-${key}`] = value;
    }
  },
};

export const codeTransformers: ShikiTransformer[] = [
  transformerNotationFocus(),
  transformerNotationHighlight(),
  transformerNotationDiff(),
  transformerMetaHighlight(),
  metaTransformer,
];

export const lineTransformers = (focus: ReadonlySet<number>, highlight: ReadonlySet<number>): ShikiTransformer[] => [
  lineClass("focused", focus, "has-focused"),
  lineClass("highlighted", highlight, "has-highlighted"),
];

export interface Range {
  start: number;
  end: number;
}

const marker = /^\s*(?:\/\/|<!--|\/\*|#)\s*#(end)?region\b\s*([\w-]*)\s*(?:-->|\*\/)?\s*$/;

/** Drops `#region name` / `#endregion` marker lines and returns where each region lands, 0-based, end exclusive. */
export function splitRegions(source: string): { lines: string[]; regions: Map<string, Range> } {
  const lines: string[] = [];
  const regions = new Map<string, Range>();
  const open: { name: string; start: number }[] = [];
  for (const line of source.replace(/\r\n/g, "\n").replace(/\n$/, "").split("\n")) {
    const match = marker.exec(line);
    if (!match) {
      lines.push(line);
      continue;
    }
    if (match[1]) {
      const index = match[2] ? open.findLastIndex((r) => r.name === match[2]) : open.length - 1;
      const region = index >= 0 ? open.splice(index, 1)[0] : undefined;
      if (!region) throw new Error(`#endregion ${match[2] ?? ""} has no matching #region.`);
      regions.set(region.name, { start: region.start, end: lines.length });
    } else open.push({ name: match[2] ?? "", start: lines.length });
  }
  return { lines, regions };
}

export function dedent(lines: string[]): string[] {
  const indent = Math.min(...lines.filter((l) => l.trim()).map((l) => /^\s*/.exec(l)?.[0].length ?? 0));
  return Number.isFinite(indent) ? lines.map((l) => l.slice(indent)) : lines;
}
