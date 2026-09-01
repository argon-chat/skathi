// Reading the engine's computed-style dump, and the two colour helpers the
// Styles panel paints swatches with.
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

import type { TreeNode } from "./protocol";

const KEY_LINE = /^(\s*)([A-Za-z_][A-Za-z0-9_]*):\s*(.*)$/;

/** One folded line of the dump: how deep it sat, its key, and its value. */
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

/** Only the outermost level — an element's own `color`, not the one inside its `text` block. */
export function topLevel(lines: string[] | undefined): DumpEntry[] {
  const entries = flattenDump(lines);
  if (!entries.length) return [];
  const shallowest = Math.min(...entries.map((e) => e.indent));
  return entries.filter((e) => e.indent === shallowest);
}

/** `Hsla { h, s, l, a }` out of a flattened value — what paints the swatches. */
export function parseHsla(value: string | undefined): Hsla | null {
  const m = /Hsla\s*\{\s*h:\s*([-\d.eE]+),\s*s:\s*([-\d.eE]+),\s*l:\s*([-\d.eE]+),\s*a:\s*([-\d.eE]+)/.exec(
    value || "",
  );
  if (!m) return null;
  return { h: +m[1], s: +m[2], l: +m[3], a: +m[4] };
}

/** An `Hsla` as a CSS colour this app's own stylesheet can render. */
export function hslaCss(hsla: Hsla): string {
  return `hsla(${(hsla.h * 360).toFixed(1)}, ${(hsla.s * 100).toFixed(1)}%, ${(hsla.l * 100).toFixed(
    1,
  )}%, ${hsla.a.toFixed(3)})`;
}

/**
 * A swatch colour for an AUTHORED declaration value (`#1f6feb`, `rgb(9 105 218
 * / .4)`, `hsl(212 92% 45%)`, `white`).
 *
 * Deliberately conservative: it recognises the notations CSS actually writes
 * colours in and returns `null` for everything else, including things that
 * merely contain one (`1px solid #333`, `linear-gradient(...)`). A swatch beside
 * a value that is not a colour is worse than no swatch — it invites you to read
 * the number as the paint.
 *
 * The named list is the sixteen CSS 1 keywords plus the handful an app actually
 * types. It is not `<named-color>`'s 148, because this is a rendering hint and
 * a missing swatch costs nothing.
 */
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

// ---------------------------------------------------------------------------
// how a tree row is NAMED
// ---------------------------------------------------------------------------
//
// The retained tree holds three kinds of node (`vn_tree::devtools::NodeInfo`):
// elements, text, and Vue's FRAGMENT ANCHORS — the zero-size placeholder a
// `v-if` / `v-for` / `<component>` leaves behind so a later patch knows where
// to splice. Only an element has a tag; text and anchors carry `tag: 0`, which
// the wire sends as `""`.
//
// This panel used to draw every non-text node as `<{tag}>`, which made a run of
// anchors read as `<root> <root> <root>` — the host's name table handed id 0 to
// the first name interned (`"root"`), so `tag: 0` came back as `"root"`. That
// is fixed at the source (crates/vn-tree/src/tree.rs reserves name 0), but the
// mapping below is written to be right against an OLD host too: it decides by
// KIND and never by tag, so an inspector attached to a pre-fix binary still
// labels an anchor as an anchor.

/** The three row shapes the Elements tree draws. */
export type RowKind = "element" | "text" | "anchor";

/**
 * A node's row shape. An unknown `kind` — a host newer than this inspector —
 * is drawn as an element, which is the shape that shows the most and hides
 * nothing.
 */
export function rowKind(node: TreeNode | null | undefined): RowKind {
  if (node?.kind === "text") return "text";
  if (node?.kind === "anchor") return "anchor";
  return "element";
}

/**
 * Does this node get a row at all?
 *
 * Anchors are hidden by default (there is one per `v-if` and one per `v-for`,
 * and they paint nothing), behind the panel's "anchors" toggle — Chrome makes
 * the same call for the comment nodes Vue's browser build uses instead.
 * A whitespace-only text node is dropped outright: it renders nothing and no
 * toggle would make it worth a row.
 */
export function rowVisible(node: TreeNode | null | undefined, showAnchors: boolean): boolean {
  if (!node) return false;
  const kind = rowKind(node);
  if (kind === "anchor") return showAnchors;
  if (kind === "text") return !!(node.text || "").trim();
  return true;
}

/** `div.a.b` — how a node is named in the breadcrumbs and in panel headers. */
export function nodeLabel(node: TreeNode | null | undefined): string {
  if (!node) return "—";
  const kind = rowKind(node);
  // What Chrome shows for the comment node Vue's browser build anchors with.
  if (kind === "anchor") return "<!---->";
  if (kind === "text") return "#text";
  const classes = (node.classes || []).map((c) => `.${c}`).join("");
  return `${node.tag || "?"}${classes}`;
}

/**
 * The engine's key names, as close to CSS as they honestly get.
 *
 * gpui's `StyleRefinement` is not CSS and this does not pretend it is — it
 * un-camel-cases (`border_widths` -> `border-widths`) so the dump reads like a
 * stylesheet, and stops there. Anything more (mapping `background` onto
 * `background-color`, splitting `size` into `width`/`height`) would be the
 * panel inventing a vocabulary the engine does not have, and the first time the
 * two disagreed the panel would be the liar.
 */
export function cssish(key: string): string {
  return key.replace(/_/g, "-");
}
