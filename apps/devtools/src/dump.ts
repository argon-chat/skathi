// `styles` answers with a pretty `Debug` of gpui's `StyleRefinement`, one JSON string per source
// line.

import type { TreeNode } from "./protocol";

const KEY_LINE = /^(\s*)([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$/;

export interface DumpEntry {
  indent: number;
  key: string;
  value: string;
}

/** `Hsla { h, s, l, a }`, in the 0..1 units gpui stores it in. */
export interface Hsla {
  h: number;
  s: number;
  l: number;
  a: number;
}

export function flattenDump(lines: string[] | undefined): DumpEntry[] {
  const out: DumpEntry[] = [];
  if (!Array.isArray(lines)) return out;
  for (let i = 0; i < lines.length; i++) {
    const m = KEY_LINE.exec(lines[i]);
    if (!m) continue;
    const indent = m[1].length;
    let value = m[3].trim();
    if (/[([{]$/.test(value)) {
      const parts = [value];
      for (let j = i + 1; j < lines.length; j++) {
        const inner = lines[j];
        const innerIndent = inner.length - inner.trimStart().length;
        parts.push(inner.trim());
        if (innerIndent <= indent && /^[)\]}]/.test(inner.trim())) break;
      }
      value = parts.join(" ");
    }
    out.push({ indent, key: m[2], value: value.replace(/\s+/g, " ").replace(/,$/, "").trim() });
  }
  return out;
}

export function topLevel(lines: string[] | undefined): DumpEntry[] {
  const entries = flattenDump(lines);
  if (!entries.length) return [];
  const shallowest = Math.min(...entries.map((e) => e.indent));
  return entries.filter((e) => e.indent === shallowest);
}

export function parseHsla(value: string | undefined): Hsla | null {
  const m = /Hsla\s*\{\s*h:\s*([-\d.eE]+),\s*s:\s*([-\d.eE]+),\s*l:\s*([-\d.eE]+),\s*a:\s*([-\d.eE]+)/.exec(
    value || "",
  );
  if (!m) return null;
  return { h: +m[1], s: +m[2], l: +m[3], a: +m[4] };
}

export function hslaCss(hsla: Hsla): string {
  return `hsla(${(hsla.h * 360).toFixed(1)}, ${(hsla.s * 100).toFixed(1)}%, ${(hsla.l * 100).toFixed(
    1,
  )}%, ${hsla.a.toFixed(3)})`;
}

const NAMED: Record<string, string> = {
  black: "#000000",
  white: "#ffffff",
  red: "#ff0000",
  green: "#008000",
  blue: "#0000ff",
  yellow: "#ffff00",
  cyan: "#00ffff",
  magenta: "#ff00ff",
  gray: "#808080",
  grey: "#808080",
  silver: "#c0c0c0",
  maroon: "#800000",
  olive: "#808000",
  lime: "#00ff00",
  aqua: "#00ffff",
  teal: "#008080",
  navy: "#000080",
  fuchsia: "#ff00ff",
  purple: "#800080",
  orange: "#ffa500",
  transparent: "#00000000",
};

export function cssColor(value: string | undefined): string | null {
  const v = (value || "").trim().toLowerCase();
  if (!v) return null;
  if (NAMED[v]) return NAMED[v];
  if (/^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/.test(v)) return v;
  if (/^(rgb|rgba|hsl|hsla)\([^()]*\)$/.test(v)) return v;
  return null;
}

export type RowKind = "element" | "text" | "anchor";

export function rowKind(node: TreeNode | null | undefined): RowKind {
  if (node?.kind === "text") return "text";
  if (node?.kind === "anchor") return "anchor";
  return "element";
}

/**
 * A whitespace-only text node is dropped outright: it renders nothing and no toggle would make it
 * worth a row.
 */
export function rowVisible(node: TreeNode | null | undefined, showAnchors: boolean): boolean {
  if (!node) return false;
  const kind = rowKind(node);
  if (kind === "anchor") return showAnchors;
  if (kind === "text") return !!(node.text || "").trim();
  return true;
}

export function nodeLabel(node: TreeNode | null | undefined): string {
  if (!node) return "—";
  const kind = rowKind(node);
  if (kind === "anchor") return "<!---->";
  if (kind === "text") return "#text";
  const classes = (node.classes || []).map((c) => `.${c}`).join("");
  return `${node.tag || "?"}${classes}`;
}

export function cssish(key: string): string {
  return key.replace(/_/g, "-");
}
