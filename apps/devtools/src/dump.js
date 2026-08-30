// Reading the engine's computed-style dump.
//
// `styles` answers with a pretty `Debug` of gpui's `StyleRefinement`, one JSON
// string per source line. It is the ENGINE's vocabulary, not CSS's
// (`background`, not `background-color`; an `Hsla` struct, not `rgb(...)`), and
// the panel does not pretend otherwise — a hand-rolled `getComputedStyle()`
// serializer would agree with no browser and drift from the engine on the first
// field that gains a variant.
//
// The algorithm is the one `packages/vn-test/src/styles.ts` uses for the same
// dump, re-implemented here because an app bundle must not import the test
// harness. Keep the two in step.

const KEY_LINE = /^(\s*)([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$/;

/**
 * Folds a bucket's lines into `{indent, key, value}` entries, joining a nested
 * block back onto one line:
 *
 *     background: Some(          ->  background: Some( Color( Solid(Hsla { … }) ) )
 *         Color(
 *             Solid(Hsla { … }),
 *         ),
 *     ),
 */
export function flattenDump(lines) {
  const out = [];
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

/** Only the outermost level — an element's own `color`, not the one inside its `text` block. */
export function topLevel(lines) {
  const entries = flattenDump(lines);
  if (!entries.length) return [];
  const shallowest = Math.min(...entries.map((e) => e.indent));
  return entries.filter((e) => e.indent === shallowest);
}

/** `Hsla { h, s, l, a }` out of a flattened value — what paints the swatches. */
export function parseHsla(value) {
  const m = /Hsla\s*\{\s*h:\s*([-\d.eE]+),\s*s:\s*([-\d.eE]+),\s*l:\s*([-\d.eE]+),\s*a:\s*([-\d.eE]+)/.exec(
    value || "",
  );
  if (!m) return null;
  return { h: +m[1], s: +m[2], l: +m[3], a: +m[4] };
}

/** An `Hsla` as a CSS colour this app's own stylesheet can render. */
export function hslaCss(hsla) {
  return `hsla(${(hsla.h * 360).toFixed(1)}, ${(hsla.s * 100).toFixed(1)}%, ${(hsla.l * 100).toFixed(
    1,
  )}%, ${hsla.a.toFixed(3)})`;
}

/** `<div class="a b">` — how a node is named in the tree and in headers. */
export function nodeLabel(node) {
  if (!node) return "—";
  if (node.kind === "text") return "#text";
  const classes = (node.classes || []).map((c) => `.${c}`).join("");
  return `${node.tag || "?"}${classes}`;
}
