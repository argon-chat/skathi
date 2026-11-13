// The `styles` command answers with a pretty `Debug` of gpui's `StyleRefinement`, split one JSON
// string per source line.

/** One `key: value` from the dump, with its nesting depth and its block folded flat. */
export type StyleEntry = { indent: number; key: string; value: string };

const KEY_LINE = /^(\s*)([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$/;

/** Folds a bucket's lines into `key: value` entries. */
export function flattenStyleDump(lines: string[]): StyleEntry[] {
  const out: StyleEntry[] = [];
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

/**
 * A handful of CSS names that map 1:1 onto an engine field, accepted so that a spec written in
 * browser vocabulary does not silently match nothing.
 */
const ALIASES: Record<string, string> = {
  "background-color": "background",
  "border-color": "border_color",
  "border-style": "border_style",
  "text-color": "color",
  "font-size": "font_size",
  "font-weight": "font_weight",
  "mouse-cursor": "mouse_cursor",
};

export const resolveProperty = (prop: string) => ALIASES[prop] ?? prop;

/** The value of `prop` in a bucket, or `undefined`. */
export function styleValue(lines: string[], prop: string): string | undefined {
  const key = resolveProperty(prop);
  const hits = flattenStyleDump(lines).filter((e) => e.key === key);
  if (!hits.length) return undefined;
  return hits.reduce((a, b) => (b.indent < a.indent ? b : a)).value;
}

/** Every property name a bucket declares, shallowest level first — for failure messages. */
export function styleKeys(lines: string[]): string[] {
  const entries = flattenStyleDump(lines);
  const shallowest = Math.min(...entries.map((e) => e.indent));
  return entries.filter((e) => e.indent === shallowest).map((e) => e.key);
}

/** `Hsla { h: .., s: .., l: .., a: .. }` out of a flattened value, when there is one. */
export function parseHsla(value: string): { h: number; s: number; l: number; a: number } | null {
  const m = /Hsla\s*\{\s*h:\s*([-\d.eE]+),\s*s:\s*([-\d.eE]+),\s*l:\s*([-\d.eE]+),\s*a:\s*([-\d.eE]+)/.exec(value);
  if (!m) return null;
  return { h: +m[1], s: +m[2], l: +m[3], a: +m[4] };
}
