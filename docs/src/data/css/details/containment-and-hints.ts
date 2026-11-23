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
        note: "Give the box an explicit width and height: under `strict` its content does not size it.",
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
        title: "Slide a toast in without its text stepping",
        lang: "css",
        code: `.toast {
  will-change: transform;
  transition: transform 300ms ease-out;
}`,
        note: "Without the hint each frame places the text on whole pixels, so it moves in 1px steps against the toast's border.",
      },
    ],
    edgeCases: [
      "`will-change: transform` or `opacity` does not make a stacking context, unlike in browsers.",
      '`getComputedStyle(el).getPropertyValue("will-change")` reads `auto` whatever you declare.',
    ],
  },
  "container-type": {
    examples: [
      {
        title: "A card that lays itself out by the width it is given",
        lang: "css",
        code: `.card-slot {
  container-type: inline-size;
}
.card {
  display: flex;
  flex-direction: column;
}
@container (min-width: 480px) {
  .card {
    flex-direction: row;
  }
}`,
        note: "The same card stacks in a narrow sidebar and sits in a row in the main column, whatever the window's width.",
      },
    ],
    edgeCases: [
      "A `size` container with no `height` is as tall as its padding: its content does not size it.",
      "An `inline-size` container that is a flex item and does not grow is as narrow as its padding; give it a `width` or `flex: 1`.",
      '`getComputedStyle(el).containerType` reads `normal`, `inline-size` or `size`; `CSS.supports("container-type", "inline-size")` is `true`.',
    ],
  },
  "container-name": {
    edgeCases: [
      "A name with no `container-type` makes a style container only: `@container sidebar style(--x: 1)` finds it, a size query does not.",
    ],
  },
  container: {
    edgeCases: ["`container: card` alone sets the name and resets the type to `normal`."],
  },
  "content-visibility": {
    edgeCases: [
      "The hidden value skips the content of an ordinary box while preserving its border and background. An explicit size continues to apply. The auto value is unsupported.",
    ],
  },
  "contain-intrinsic-size": {
    edgeCases: ["A declaration prints an advisory and changes nothing."],
  },
};
