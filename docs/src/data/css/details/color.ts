import type { Details } from "../../details";

export const details: Details = {
  "named-colors": {
    examples: [
      {
        title: "Color presence dots by status",
        lang: "css",
        code: `.status-dot.online {
  background-color: limegreen;
}

.status-dot.busy {
  background-color: crimson;
}

.status-dot.away {
  background-color: goldenrod;
}`,
      },
    ],
    edgeCases: [
      "`getComputedStyle(el).color` reports a named color as `rgb()`: `rebeccapurple` reads back as `rgb(102, 51, 153)`.",
    ],
  },
  "hex-colors": {
    examples: [
      {
        title: "Define a palette with an alpha overlay",
        lang: "css",
        code: `:root {
  --accent: #0969da;
  --accent-hover: #0550ae;
  --overlay: #0d111780;
}

.modal-backdrop {
  background-color: var(--overlay);
}`,
      },
    ],
    edgeCases: [
      "The 4- and 8-digit forms carry alpha, and `getComputedStyle(el).color` reports them as `rgba()`: `#0d111780` reads back as `rgba(13, 17, 23, 0.5)`.",
    ],
  },
  "function-rgb": {
    examples: [
      {
        title: "Tint hovered and selected rows",
        lang: "css",
        code: `.list-row:hover {
  background-color: rgb(9 105 218 / 8%);
}

.list-row.selected {
  background-color: rgba(9, 105, 218, 0.16);
}`,
      },
    ],
    edgeCases: [
      "`getComputedStyle(el).color` rounds the alpha to two decimals: `rgb(0 0 0 / 0.125)` reads back as `rgba(0, 0, 0, 0.13)`.",
    ],
  },
  "function-hsl": {
    examples: [
      {
        title: "Derive hover and pressed shades by lightness",
        lang: "css",
        code: `.primary-button {
  background-color: hsl(212 92% 45%);
}

.primary-button:hover {
  background-color: hsl(212 92% 38%);
}

.primary-button:active {
  background-color: hsl(212 92% 32%);
}`,
      },
    ],
    edgeCases: ["The color is computed in sRGB: `getComputedStyle(el).color` reports `rgb()`, not `hsl()`."],
  },
  "keyword-transparent": {
    examples: [
      {
        title: "Fade a search hit's highlight in",
        lang: "css",
        code: `.search-hit {
  background-color: transparent;
  transition: background-color 300ms ease-out;
}

.search-hit.current {
  background-color: #fff8c5;
}`,
      },
    ],
    edgeCases: [
      "Transitions interpolate with premultiplied alpha, so a fade between `transparent` and a color has no dark midpoint.",
      "`color-mix(in srgb, red 25%, transparent)` keeps the red and lowers only the alpha, giving `rgb(255 0 0 / 25%)`.",
    ],
  },
  "keyword-currentcolor": {
    examples: [
      {
        title: "Make an icon button's border and icon follow its text color",
        lang: "css",
        code: `.icon-button {
  color: #656d76;
  border: 1px solid currentColor;
}

.icon-button.danger {
  color: #cf222e;
}

.icon-button svg {
  fill: currentColor;
}`,
      },
      {
        title: "Tint a badge's background from its text color",
        lang: "css",
        code: `.badge {
  color: #1a7f37;
  background-color: color-mix(in srgb, currentColor 12%, transparent);
}

.badge.warning {
  color: #9a6700;
}`,
      },
    ],
    edgeCases: [
      "In the `color` property itself, `currentColor` is the parent's color, so `color: currentColor` acts as `inherit`.",
      "Inside `color-mix()` and relative colors it resolves per element, and the result follows when the element's `color` changes.",
      "`text-decoration-color: currentColor` logs an advisory. Leave the property unset instead: the line then takes the text color.",
    ],
  },
  "function-hwb": {
    examples: [
      {
        title: "Mix a pale tag color from a hue",
        lang: "css",
        code: `.tag.neutral {
  background-color: hwb(210 90% 4%);
  color: hwb(210 30% 50%);
}`,
      },
    ],
    edgeCases: [
      "The color is computed in sRGB: `getComputedStyle(el).color` reports `rgb()`, so `hwb(120 0% 0%)` reads back as `rgb(0, 255, 0)`.",
    ],
  },
  "function-lab": {
    examples: [
      {
        title: "Match a brand color specified in CIE Lab",
        lang: "css",
        code: `.brand-mark {
  color: lab(54.29% 80.82 69.91);
  border-bottom: 2px solid lab(54.29% 80.82 69.91 / 40%);
}`,
      },
    ],
    edgeCases: [
      "It is converted to sRGB and clipped per channel: `lab(54.29% 80.82 69.91)` paints `rgb(255, 0, 0)`, and `getComputedStyle(el).color` reports that `rgb()`.",
    ],
  },
  "function-lch": {
    examples: [
      {
        title: "Color calendar events at one lightness and chroma",
        lang: "css",
        code: `.calendar-event.work {
  background-color: lch(85% 30 250);
}

.calendar-event.personal {
  background-color: lch(85% 30 140);
}

.calendar-event.travel {
  background-color: lch(85% 30 40);
}`,
      },
    ],
    edgeCases: [
      "An alpha after `/` is kept: `lch(54.29% 106.84 40.85 / 50%)` paints `rgb(255 0 0 / 50%)`.",
      "`getComputedStyle(el).color` reports the sRGB result as `rgb()` or `rgba()`.",
    ],
  },
  "function-oklab": {
    examples: [
      {
        title: "Step neutral surfaces by perceived lightness",
        lang: "css",
        code: `.surface-1 {
  background-color: oklab(0.97 0 0);
}

.surface-2 {
  background-color: oklab(0.93 0 0);
}`,
      },
    ],
    edgeCases: [
      "`oklab(0.5 0 0)` paints `rgb(99, 99, 99)`, and `getComputedStyle(el).color` reports the `rgb()` value.",
    ],
  },
  "function-oklch": {
    examples: [
      {
        title: "Build an accent scale with even perceived steps",
        lang: "css",
        code: `:root {
  --accent-300: oklch(80% 0.12 255);
  --accent-500: oklch(62% 0.18 255);
  --accent-700: oklch(45% 0.16 255);
}

.primary-button {
  background-color: var(--accent-500);
}

.primary-button:hover {
  background-color: var(--accent-700);
}`,
      },
    ],
    edgeCases: [
      "A chroma outside sRGB is clipped per channel, so a too-vivid `oklch(90% 0.4 145)` still paints a bright green.",
      "`getComputedStyle(el).color` reports the sRGB result as `rgb()` or `rgba()`, not `oklch()`.",
    ],
  },
  "function-color": {
    examples: [
      {
        title: "Write colors in named color spaces",
        lang: "css",
        code: `.live-indicator {
  background-color: color(display-p3 1 0.2 0.1);
}

.chart-line {
  stroke: color(srgb 0 0.5 1);
}`,
        note: "The Display P3 color paints as its clipped sRGB equivalent.",
      },
    ],
    edgeCases: [
      "Wide-gamut spaces are converted to sRGB and clipped per channel: `color(display-p3 1 0 0)` paints sRGB red.",
      "`srgb-linear` works: `color(srgb-linear 0.2158 0.2158 0.2158)` is `rgb(128, 128, 128)`.",
    ],
  },
  "function-color-mix": {
    examples: [
      {
        title: "Derive hover and border shades from one accent",
        lang: "css",
        code: `:root {
  --accent: #0969da;
}

.primary-button {
  background-color: var(--accent);
  border: 1px solid color-mix(in oklab, var(--accent), black 20%);
}

.primary-button:hover {
  background-color: color-mix(in oklab, var(--accent), white 15%);
}`,
      },
      {
        title: "Make a toast's background translucent",
        lang: "css",
        code: `.toast {
  color: #fff;
  background-color: color-mix(in srgb, #1f2328 85%, transparent);
}`,
      },
    ],
    edgeCases: [
      "Mixing with `transparent` keeps the other color's hue and lowers only the alpha: `color-mix(in srgb, red 25%, transparent)` is `rgb(255 0 0 / 25%)`.",
      "`currentColor` inside it resolves against the element's own `color` and follows changes to it; in the `color` property it is the parent's.",
      "`var()` arguments are substituted per element before the mix is computed.",
    ],
  },
  "feature-relative-colors": {
    examples: [
      {
        title: "Tint a drop target from its border color",
        lang: "css",
        code: `.drop-target {
  border: 2px dashed #0969da;
  background-color: rgb(from #0969da r g b / 10%);
}

.drop-target.active {
  background-color: oklch(from #0969da l c h / 20%);
}`,
      },
      {
        title: "Dim secondary text from the surrounding color",
        lang: "css",
        code: `.message-meta {
  font-size: 12px;
  color: rgb(from currentColor r g b / 60%);
}`,
        note: "In `color`, `currentColor` is the parent's color, so the meta line follows whatever color its message has.",
      },
    ],
    edgeCases: [
      "`currentColor` works as the origin: in `color` it is the parent's color, in any other property the element's own.",
    ],
  },
  "function-light-dark": {
    examples: [
      {
        title: "Instead: switch a panel's colors with the system theme",
        lang: "css",
        code: `.panel {
  background-color: #ffffff;
  color: #1f2328;
}

@media (prefers-color-scheme: dark) {
  .panel {
    background-color: #161b22;
    color: #e6edf3;
  }
}`,
      },
    ],
    edgeCases: [
      "`light-dark()` logs an advisory that the value does not resolve to a color, and the declaration is not applied.",
    ],
  },
  "system-colors": {
    edgeCases: [
      "A system color such as `Canvas` or `AccentColor` logs an advisory that the value does not resolve to a color, and the declaration is not applied.",
    ],
  },
  "color-scheme": {
    edgeCases: [
      '`color-scheme: light dark` logs an advisory and is not applied; `CSS.supports("color-scheme", "light dark")` returns `false`.',
    ],
  },
  "forced-color-adjust": {
    edgeCases: [
      '`forced-color-adjust: none` logs an advisory and is not applied; `CSS.supports("forced-color-adjust", "none")` returns `false`.',
    ],
  },
  "print-color-adjust": {
    edgeCases: [
      '`print-color-adjust: exact` logs an advisory and is not applied; `CSS.supports("print-color-adjust", "exact")` returns `false`.',
    ],
  },
};
