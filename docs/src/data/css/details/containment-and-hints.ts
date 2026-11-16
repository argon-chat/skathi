import type { Details } from "../../details";

export const details: Details = {
  contain: {
    examples: [
      {
        title: "Keep a widget's overflowing content inside its tile",
        lang: "css",
        code: `.dashboard-tile {
  width: 280px;
  height: 160px;
  contain: paint;
}`,
        note: "Content that spills past the tile is clipped at its edges, as `overflow: clip` would.",
      },
      {
        title: "Contain a fixed-size sparkline",
        lang: "css",
        code: `.sparkline {
  width: 120px;
  height: 32px;
  contain: strict;
}`,
        note: "Give the box an explicit width and height: `strict` sizes nothing for you.",
      },
    ],
    edgeCases: [
      "`contain: paint` turns an axis with `overflow: visible` into `clip`; an axis already `auto` or `scroll` keeps scrolling.",
      "`layout` and `style` are accepted and change nothing visible.",
      "An unknown keyword drops the whole declaration with an advisory.",
    ],
  },
  "will-change": {
    examples: [
      {
        title: "Mark a panel that a drag moves",
        lang: "css",
        code: `.split-pane-handle {
  will-change: transform;
}`,
        note: "Keep it for browser builds of the same code; Skathi needs no hint to animate `transform`.",
      },
    ],
    edgeCases: [
      "`will-change: transform` or `opacity` does not make a stacking context, unlike in browsers.",
      '`getComputedStyle(el).getPropertyValue("will-change")` reads `auto` whatever you declare.',
    ],
  },
  "container-type": {
    edgeCases: [
      'A declaration prints an advisory and changes nothing; `CSS.supports("container-type", "inline-size")` returns `false`.',
    ],
  },
  "container-name": {
    edgeCases: ["A declaration prints an advisory and changes nothing."],
  },
  container: {
    edgeCases: ["A declaration prints an advisory and changes nothing."],
  },
  "content-visibility": {
    edgeCases: [
      "A declaration prints an advisory and changes nothing: the content renders as if the property were absent.",
    ],
  },
  "contain-intrinsic-size": {
    edgeCases: ["A declaration prints an advisory and changes nothing."],
  },
};
