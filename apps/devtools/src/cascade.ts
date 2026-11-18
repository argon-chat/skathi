import type { Rule } from "./protocol";

const SIDES = ["top", "right", "bottom", "left"];
const sides = (prefix: string, suffix = "") => SIDES.map((s) => `${prefix}-${s}${suffix}`);
const RADII = ["top-left", "top-right", "bottom-right", "bottom-left"].map((c) => `border-${c}-radius`);

const SHORTHANDS: Record<string, string[]> = {
  margin: sides("margin"),
  padding: sides("padding"),
  inset: SIDES,
  "scroll-margin": sides("scroll-margin"),
  "scroll-padding": sides("scroll-padding"),
  "border-width": sides("border", "-width"),
  "border-style": sides("border", "-style"),
  "border-color": sides("border", "-color"),
  "border-radius": RADII,
  ...Object.fromEntries(
    SIDES.map((s) => [`border-${s}`, [`border-${s}-width`, `border-${s}-style`, `border-${s}-color`]]),
  ),
  border: [...sides("border", "-width"), ...sides("border", "-style"), ...sides("border", "-color")],
  outline: ["outline-width", "outline-style", "outline-color"],
  background: [
    "background-color",
    "background-image",
    "background-position",
    "background-size",
    "background-repeat",
    "background-origin",
    "background-clip",
    "background-attachment",
  ],
  font: ["font-style", "font-variant", "font-weight", "font-stretch", "font-size", "line-height", "font-family"],
  flex: ["flex-grow", "flex-shrink", "flex-basis"],
  "flex-flow": ["flex-direction", "flex-wrap"],
  gap: ["row-gap", "column-gap"],
  "grid-gap": ["row-gap", "column-gap"],
  overflow: ["overflow-x", "overflow-y"],
  "place-items": ["align-items", "justify-items"],
  "place-content": ["align-content", "justify-content"],
  "place-self": ["align-self", "justify-self"],
  "text-decoration": [
    "text-decoration-line",
    "text-decoration-style",
    "text-decoration-color",
    "text-decoration-thickness",
  ],
  transition: ["transition-property", "transition-duration", "transition-timing-function", "transition-delay"],
  animation: [
    "animation-name",
    "animation-duration",
    "animation-timing-function",
    "animation-delay",
    "animation-iteration-count",
    "animation-direction",
    "animation-fill-mode",
    "animation-play-state",
  ],
  "list-style": ["list-style-type", "list-style-position", "list-style-image"],
  "grid-row": ["grid-row-start", "grid-row-end"],
  "grid-column": ["grid-column-start", "grid-column-end"],
  "grid-area": ["grid-row-start", "grid-column-start", "grid-row-end", "grid-column-end"],
  "grid-template": ["grid-template-rows", "grid-template-columns", "grid-template-areas"],
  mask: [
    "mask-image",
    "mask-mode",
    "mask-position",
    "mask-size",
    "mask-repeat",
    "mask-origin",
    "mask-clip",
    "mask-composite",
  ],
  "margin-inline": ["margin-inline-start", "margin-inline-end"],
  "margin-block": ["margin-block-start", "margin-block-end"],
  "padding-inline": ["padding-inline-start", "padding-inline-end"],
  "padding-block": ["padding-block-start", "padding-block-end"],
  "inset-inline": ["inset-inline-start", "inset-inline-end"],
  "inset-block": ["inset-block-start", "inset-block-end"],
};

/** The longhands a property sets: itself, or every longhand of a shorthand. */
export function longhands(property: string): string[] {
  const name = property.trim();
  if (name.startsWith("--")) return [name];
  const lower = name.toLowerCase();
  const parts = SHORTHANDS[lower];
  return parts ? parts.flatMap(longhands) : [lower];
}

/** Whether a rule applies now: a rule for a state the node is not in (`:hover`, a group's state) does not compete. */
export type RuleActive = (rule: Rule) => boolean;

/**
 * The declarations the cascade discards, as `"rule:declaration"` indices into `rules` (cascade
 * order, last wins). A declaration is overridden when a later or `!important` one sets every
 * longhand it sets; pseudo-element rules compete only with rules for the same pseudo-element.
 */
export function overriddenDeclarations(rules: Rule[], active: RuleActive = (r) => r.state === "base"): Set<string> {
  const winner = new Map<string, { key: string; important: boolean }>();
  rules.forEach((rule, ri) => {
    if (!active(rule)) return;
    const scope = rule.pseudo ?? "";
    rule.declarations.forEach((decl, di) => {
      const important = !!decl.important;
      for (const longhand of longhands(decl.property)) {
        const slot = `${scope}\u0000${longhand}`;
        const held = winner.get(slot);
        if (!held || important || !held.important) winner.set(slot, { key: `${ri}:${di}`, important });
      }
    });
  });
  const won = new Set([...winner.values()].map((w) => w.key));
  const dead = new Set<string>();
  rules.forEach((rule, ri) => {
    if (!active(rule)) return;
    rule.declarations.forEach((_decl, di) => {
      const key = `${ri}:${di}`;
      if (!won.has(key)) dead.add(key);
    });
  });
  return dead;
}
