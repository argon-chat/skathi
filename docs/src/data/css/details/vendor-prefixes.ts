import type { Details } from "../../details";

export const details: Details = {
  "feature-vendor-prefixes": {
    examples: [
      {
        title: "Keep a ported three-line clamp",
        lang: "css",
        code: `.preview-text {
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
  overflow: hidden;
}`,
        note: "The clamp comes from `-webkit-line-clamp` alone; `display: -webkit-box` lays out as `block`.",
      },
      {
        title: "Draw a gradient border with the prefixed mask idiom",
        lang: "css",
        code: `.gradient-border::after {
  content: "";
  position: absolute;
  inset: 0;
  padding: 1px;
  border-radius: inherit;
  background: linear-gradient(135deg, #7aa2ff, #c084fc);
  -webkit-mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0);
  -webkit-mask-composite: xor;
}`,
        note: "This two-layer ring is the one `-webkit-mask` pattern that applies, without an advisory.",
      },
      {
        title: "Feature-test a prefixed property",
        lang: "ts",
        code: `const blurs = CSS.supports("-webkit-backdrop-filter", "blur(8px)");
const callout = CSS.supports("-webkit-touch-callout", "none");

console.log(blurs, callout);`,
        note: "Prints `true false`: a spelling Skathi reads answers as its property does, and a dropped one answers `false`.",
      },
    ],
    edgeCases: [
      "`display: -webkit-flex` and `display: -ms-flexbox` lay out as `flex`.",
      "`-webkit-appearance: none` removes a control's native look like `appearance: none`; `-moz-appearance: none` is dropped.",
      "`-webkit-text-fill-color` takes a color or `currentColor`; any other value is dropped with an advisory.",
      "The legacy `-webkit-gradient()` function is not painted and prints an advisory; write `linear-gradient()`.",
    ],
  },
};
