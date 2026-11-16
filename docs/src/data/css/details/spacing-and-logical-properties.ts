import type { Details } from "../../details";

const advisory =
  "A declaration prints an advisory in `sk dev` and changes nothing, and `CSS.supports()` answers `false` for it.";

export const details: Details = {
  margin: {
    examples: [
      {
        title: "Push a toolbar's last buttons to the right",
        lang: "css",
        code: `.toolbar {
  display: flex;
  gap: 4px;
}
.toolbar .share {
  margin-left: auto;
}`,
        note: "An `auto` margin in a flex row takes all the free space, so `.share` and every button after it move to the right edge.",
      },
      {
        title: "Overlap avatars in a participant stack",
        lang: "css",
        code: `.participants {
  display: flex;
}
.participants .avatar {
  width: 28px;
  height: 28px;
  border-radius: 50%;
  border: 2px solid #1e1e22;
}
.participants .avatar + .avatar {
  margin-left: -8px;
}`,
      },
    ],
    edgeCases: [
      "In a `display: block` container, the bottom margin of one child and the top margin of the next collapse into one, and a first or last child's margin passes through a parent that has no padding, border or `overflow` on that side (and, at the bottom, no fixed height).",
      "Margins of flex and grid items never collapse.",
    ],
  },
  padding: {
    examples: [
      {
        title: "Give a button room around its label",
        lang: "css",
        code: `.button {
  padding: 6px 12px;
  border-radius: 6px;
}`,
      },
      {
        title: "Scale a badge's padding with its text",
        lang: "css",
        code: `.badge {
  font-size: 11px;
  padding: 0.2em 0.6em;
  border-radius: 9999px;
}
.badge.large {
  font-size: 14px;
}`,
        note: "The `em` padding follows each badge's own `font-size`, so `.large` grows its padding too.",
      },
    ],
    edgeCases: [
      "A percentage resolves against the containing block's width, for `padding-top` and `padding-bottom` too.",
      "`em` resolves against the element's own `font-size`, and a `:hover` rule that changes `font-size` also changes the `em` padding of the base rule.",
    ],
  },
  "margin-inline": {
    examples: [
      {
        title: "Center a fixed-width column",
        lang: "css",
        code: `.onboarding {
  width: 100%;
  max-width: 480px;
  margin-inline: auto;
}`,
      },
    ],
  },
  "margin-block": {
    examples: [
      {
        title: "Space a settings section from its neighbours",
        lang: "css",
        code: `.settings-section {
  margin-block: 24px 12px;
}`,
        note: "The first value is the top margin, the second the bottom one.",
      },
    ],
  },
  "padding-inline": {
    examples: [
      {
        title: "Pad a list row on its sides only",
        lang: "css",
        code: `.list-row {
  height: 32px;
  padding-inline: 12px;
}`,
      },
    ],
  },
  "padding-block": {
    examples: [
      {
        title: "Pad a menu above its first item and below its last",
        lang: "css",
        code: `.menu {
  display: flex;
  flex-direction: column;
  padding-block: 4px;
}`,
      },
    ],
  },
  "inset-inline": {
    examples: [
      {
        title: "Stretch a banner across the top of a pane",
        lang: "css",
        code: `.pane {
  position: relative;
}
.update-banner {
  position: absolute;
  top: 0;
  inset-inline: 0;
  height: 32px;
}`,
      },
    ],
    edgeCases: ["`initial` and `unset` reset both offsets to `auto`."],
  },
  "inset-block": {
    examples: [
      {
        title: "Run a side panel the full height of its parent",
        lang: "css",
        code: `.editor {
  position: relative;
}
.outline-panel {
  position: absolute;
  inset-block: 0;
  right: 0;
  width: 260px;
}`,
      },
    ],
    edgeCases: ["`initial` and `unset` reset both offsets to `auto`."],
  },
  "border-inline": {
    examples: [
      {
        title: "Instead: draw dividers on a tab's left and right sides",
        lang: "css",
        code: `.tab {
  border-left: 1px solid #ffffff1a;
  border-right: 1px solid #ffffff1a;
}`,
      },
    ],
    edgeCases: [advisory],
  },
  "border-block": {
    examples: [
      {
        title: "Instead: rule a list section above and below",
        lang: "css",
        code: `.list-section {
  border-top: 1px solid #ffffff1a;
  border-bottom: 1px solid #ffffff1a;
}`,
      },
    ],
    edgeCases: [advisory],
  },
  "border-start-start-radius": {
    examples: [
      {
        title: "Instead: round a chat bubble's top-left corner",
        lang: "css",
        code: `.bubble.theirs {
  border-radius: 12px;
  border-top-left-radius: 4px;
}`,
      },
    ],
    edgeCases: [advisory],
  },
};
