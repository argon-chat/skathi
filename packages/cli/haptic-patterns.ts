import postcss, { type AtRule } from "postcss";

/** Vue and Svelte scope CSS selectors. A time inside a haptic pattern is a device timestamp. */
export function protectHapticPatterns(css: string) {
  const originals: AtRule[] = [];
  if (/@haptic-pattern\b/i.test(css)) {
    const root = postcss.parse(css);
    root.walkAtRules(/^haptic-pattern$/i, (rule) => {
      if (!rule.nodes) return;
      const index = originals.push(rule.clone()) - 1;
      rule.removeAll().append(postcss.decl({ prop: "--skathi-pattern-index", value: String(index) }));
    });
    css = root.toString();
  }
  const restore = (compiled: string) => {
    if (!originals.length) return compiled;
    const root = postcss.parse(compiled);
    root.walkAtRules(/^haptic-pattern$/i, (rule) => {
      const marker = rule.nodes?.[0];
      if (marker?.type !== "decl" || marker.prop !== "--skathi-pattern-index") return;
      const original = originals[Number(marker.value)];
      if (original) rule.replaceWith(original.clone());
    });
    return root.toString();
  };
  return { css, restore };
}
