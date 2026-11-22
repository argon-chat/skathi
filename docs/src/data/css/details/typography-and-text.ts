import type { Details } from "../../details";

export const details: Details = {
  "font-family": {
    examples: [
      {
        title: "Ship a brand font with a system fallback",
        lang: "css",
        code: `@font-face {
  font-family: "Inter";
  src: url("./fonts/Inter.woff2") format("woff2");
}

:root {
  font-family: "Inter", system-ui, sans-serif;
}

.log-line {
  font-family: "Cascadia Code", ui-monospace, monospace;
}`,
      },
      {
        title: "Draw toolbar icons from an icon font and labels from the text font",
        lang: "css",
        code: `@font-face {
  font-family: "App Icons";
  src: url("./fonts/app-icons.woff2") format("woff2");
}

.toolbar-button {
  font-family: "App Icons", "Inter", system-ui;
}`,
        note: "The icon family comes first: it draws the private-use icon glyphs and leaves the Latin label to the next family.",
      },
    ],
    edgeCases: [
      "`sans-serif` resolves to the first installed of Arial, Helvetica and Liberation Sans; `serif` to Times New Roman, Times or Liberation Serif; `monospace` to Consolas, Menlo, DejaVu Sans Mono, Liberation Mono or Courier New; `system-ui` to the platform UI font.",
      "When the first family in the list is neither installed nor declared by `@font-face` or `FontFace`, a `missing-font` advisory names the family that draws the text instead.",
      "A `@font-face` family listed after an installed family is not used for the characters the installed one lacks: the system fallback font draws them, and an advisory says so. Put icon and subset families first.",
    ],
  },
  "font-size": {
    examples: [
      {
        title: "Scale a panel's type from one base size",
        lang: "css",
        code: `.settings {
  font-size: 14px;
}

.settings h2 {
  font-size: 1.25em;
}

.settings .hint {
  font-size: smaller;
}`,
      },
      {
        title: "Animate a density toggle",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

const compact = ref(false);
</script>

<template>
  <div class="inbox" :class="{ compact }">
    <button @click="compact = !compact">Density</button>
    <div class="subject">Quarterly report draft</div>
  </div>
</template>

<style scoped>
.subject {
  font-size: 15px;
  transition: font-size 150ms ease-out;
}

.compact .subject {
  font-size: 13px;
}
</style>`,
      },
    ],
    edgeCases: [
      "`em` and `%` resolve against the parent's font size; `smaller` and `larger` divide or multiply the parent's size by 1.2.",
      "Keywords are fixed sizes, independent of the parent: `small` is `13px`, `medium` `16px`, `large` `18px`, `x-large` `24px`, `xx-large` `32px`.",
      "A `calc()` that comes out negative is clamped to `0`.",
      "`getComputedStyle(el).fontSize` reports the resolved size in px.",
    ],
  },
  "font-weight": {
    examples: [
      {
        title: "Weight a dialog's title, body and emphasis",
        lang: "css",
        code: `.dialog-title {
  font-weight: 600;
}

.dialog-body {
  font-weight: 400;
}

.dialog-body strong {
  font-weight: bold;
}`,
      },
    ],
    edgeCases: [
      "`bolder` always gives `700`, whatever the parent's weight.",
      "A number is not rounded to a multiple of 100: `450` reaches the font as `450`.",
    ],
  },
  "font-style": {
    examples: [
      {
        title: "Italicize an empty-state message",
        lang: "css",
        code: `.empty-state {
  font-style: italic;
  color: #8a8f98;
}

.empty-state .file-name {
  font-style: normal;
}`,
      },
    ],
  },
  font: {
    examples: [
      {
        title: "Instead: set a label's font with the longhands",
        lang: "css",
        code: `.status-label {
  font-style: italic;
  font-weight: 600;
  font-size: 12px;
  line-height: 16px;
  font-family: "Inter", system-ui, sans-serif;
}`,
      },
    ],
    edgeCases: [
      "In an app stylesheet, `font: 600 12px/16px Inter` logs an advisory and the element keeps its inherited font. A stylesheet inside an SVG can set the shorthand on its text.",
      '`CSS.supports("font", "12px serif")` returns `false`.',
    ],
  },
  "font-stretch": {
    edgeCases: [
      'App stylesheets drop `font-stretch: condensed`, and `CSS.supports("font-stretch", "condensed")` returns `false`. On SVG text, set the stretch directly on the element or in a stylesheet inside the SVG.',
    ],
  },
  "font-width": {
    edgeCases: [
      '`font-width: condensed` logs an advisory and is not applied; `CSS.supports("font-width", "condensed")` returns `false`.',
    ],
  },
  "font-variant": {
    examples: [
      {
        title: "Instead: show a keyboard hint in small capitals",
        lang: "css",
        code: `.shortcut-hint {
  font-feature-settings: "smcp";
  letter-spacing: 0.02em;
  color: #656d76;
}`,
        note: "The font must have the `smcp` feature; without it the text stays lowercase.",
      },
    ],
    edgeCases: [
      '`CSS.supports("font-variant", "small-caps")` returns `false` for app stylesheets. An SVG `<text style="font-variant: small-caps">` uses the font’s small capitals when available.',
    ],
  },
  "font-variant-caps": {
    examples: [
      {
        title: "Instead: set an acronym in all small capitals",
        lang: "css",
        code: `.plan-name {
  font-feature-settings: "smcp", "c2sc";
  letter-spacing: 0.03em;
}`,
        note: "`c2sc` turns the capitals into small capitals too, which the font must support.",
      },
    ],
    edgeCases: [
      '`font-variant-caps: small-caps` logs an advisory and is not applied; `CSS.supports("font-variant-caps", "small-caps")` returns `false`.',
    ],
  },
  "font-variant-numeric": {
    examples: [
      {
        title: "Keep a call timer's digits from jumping",
        lang: "css",
        code: `.call-timer {
  font-size: 13px;
  font-variant-numeric: tabular-nums;
}

.invoice-total {
  font-variant-numeric: tabular-nums slashed-zero;
}`,
      },
    ],
    edgeCases: [
      "Each keyword turns on one OpenType feature: `tabular-nums` is `tnum`, `proportional-nums` `pnum`, `oldstyle-nums` `onum`, `lining-nums` `lnum`, `slashed-zero` `zero`, `diagonal-fractions` `frac`, `stacked-fractions` `afrc`, `ordinal` `ordn`.",
      '`normal` sets nothing, so it does not undo `tabular-nums` from an earlier rule; write `font-feature-settings: "tnum" 0` instead.',
      "An unknown keyword drops the declaration with an advisory.",
      "It shares one feature list with `font-feature-settings`, `font-variant-ligatures` and `font-kerning`: a later rule or a child that sets any of them replaces the whole list.",
    ],
  },
  "font-variant-ligatures": {
    examples: [
      {
        title: "Turn ligatures off in a code view",
        lang: "css",
        code: `.code-view {
  font-family: "Fira Code", monospace;
  font-variant-ligatures: none;
}

.code-view.ligatures {
  font-variant-ligatures: common-ligatures contextual;
}`,
      },
    ],
    edgeCases: [
      "`none` turns off `liga`, `clig`, `dlig`, `hlig` and `calt`; `no-common-ligatures` turns off only `liga` and `clig`, so a code font's contextual alternates stay.",
      "`normal` sets nothing and does not undo `none` from an earlier rule.",
    ],
  },
  "font-kerning": {
    examples: [
      {
        title: "Disable kerning in a letter-spaced caption",
        lang: "css",
        code: `.section-caption {
  text-transform: uppercase;
  letter-spacing: 0.08em;
  font-kerning: none;
}`,
      },
    ],
    edgeCases: [
      "`none` sends `kern` off to the shaper. `auto` and `normal` set nothing, so they cannot undo `none` from an earlier rule.",
    ],
  },
  "font-feature-settings": {
    examples: [
      {
        title: "Pick a font's alternate glyphs",
        lang: "css",
        code: `.brand-heading {
  font-family: "Inter", sans-serif;
  font-feature-settings: "ss01", "cv11";
}

.account-id {
  font-feature-settings: "zero" on, "tnum" 1;
}`,
      },
    ],
    edgeCases: [
      "A tag without a value is on; `on`, `off` and numbers are accepted. A tag that is not four characters long drops the declaration with an advisory.",
      "`font-feature-settings`, `font-variant-numeric`, `font-variant-ligatures` and `font-kerning` write one feature list. In one rule they combine; a later rule or a child element that sets any of them replaces the whole list, so a child that sets `font-variant-numeric` loses the parent's `ss01`.",
      "`normal` sets nothing and does not clear features turned on by an earlier rule.",
    ],
  },
  "font-variation-settings": {
    edgeCases: ["Any value other than `normal` logs an advisory and is not applied."],
  },
  "font-optical-sizing": {
    examples: [
      {
        title: "Let a variable font's optical size follow the heading size",
        lang: "css",
        code: `@font-face {
  font-family: "Roboto Flex";
  src: url("./fonts/RobotoFlex.woff2") format("woff2-variations");
}

.hero-title {
  font-family: "Roboto Flex", sans-serif;
  font-size: 40px;
  font-optical-sizing: auto;
}`,
      },
    ],
    edgeCases: [
      "`none` logs an advisory, and the `opsz` axis keeps following the font size.",
      '`CSS.supports("font-optical-sizing", "none")` returns `false`.',
    ],
  },
  "font-synthesis": {
    edgeCases: [
      '`font-synthesis: none` logs an advisory and is not applied; `CSS.supports("font-synthesis", "none")` returns `false`.',
    ],
  },
  "font-size-adjust": {
    edgeCases: [
      '`font-size-adjust: 0.5` logs an advisory and is not applied; `CSS.supports("font-size-adjust", "0.5")` returns `false`.',
    ],
  },
  "font-variant-east-asian": {
    edgeCases: [
      '`font-variant-east-asian: ruby` logs an advisory and is not applied; `CSS.supports("font-variant-east-asian", "ruby")` returns `false`.',
    ],
  },
  "line-height": {
    examples: [
      {
        title: "Set a readable rhythm for message text",
        lang: "css",
        code: `.message-list {
  font-size: 14px;
  line-height: 1.45;
}

.message-list .timestamp {
  font-size: 11px;
}`,
      },
      {
        title: "Center a one-line label in a fixed-height tag",
        lang: "css",
        code: `.tag {
  height: 22px;
  padding: 0 8px;
  font-size: 12px;
  line-height: 22px;
}`,
      },
    ],
    edgeCases: [
      "`normal` is `1.2` times the font size, not the font's own line gap.",
      "A number inherits as the number, so each child multiplies its own font size. An `em` or `%` value is resolved to px on the element that declares it, and children inherit that px.",
    ],
  },
  "letter-spacing": {
    examples: [
      {
        title: "Space out an uppercase sidebar heading",
        lang: "css",
        code: `.sidebar-section {
  font-size: 11px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 0.06em;
}`,
      },
    ],
    edgeCases: [
      "`em` resolves against the element's own font size, so `0.06em` at `11px` is `0.66px`.",
      "`normal` is `0`.",
    ],
  },
  "word-spacing": {
    examples: [
      {
        title: "Loosen the words of a tracked-out banner",
        lang: "css",
        code: `.update-banner {
  text-transform: uppercase;
  letter-spacing: 0.1em;
  word-spacing: 0.25em;
}`,
      },
    ],
    edgeCases: ["`normal` is `0`, and `em` resolves against the element's own font size."],
  },
  color: {
    examples: [
      {
        title: "Theme text with custom properties",
        lang: "css",
        code: `:root {
  --text: #1f2328;
  --text-muted: #656d76;
}

@media (prefers-color-scheme: dark) {
  :root {
    --text: #e6edf3;
    --text-muted: #8d96a0;
  }
}

body {
  color: var(--text);
}

.caption {
  color: var(--text-muted);
}`,
      },
      {
        title: "Fade a navigation link's color on hover",
        lang: "css",
        code: `.nav-link {
  color: #656d76;
  transition: color 120ms ease-out;
}

.nav-link:hover {
  color: #0969da;
}`,
      },
    ],
    edgeCases: [
      "`color: currentColor` behaves as `inherit`.",
      "`getComputedStyle(el).color` returns `rgb(r, g, b)`, or `rgba(r, g, b, a)` with the alpha rounded to two decimals, whatever color syntax you wrote.",
      "A value that does not resolve to a color, such as a system color, logs an advisory, and the element keeps its inherited color.",
    ],
  },
  "text-align": {
    examples: [
      {
        title: "Right-align the amounts column of a table",
        lang: "css",
        code: `.amount {
  text-align: end;
  font-variant-numeric: tabular-nums;
}

.empty-state {
  text-align: center;
}`,
      },
    ],
    edgeCases: [
      "`start` is `left` and `end` is `right`, since text is always left-to-right.",
      "`justify` logs an advisory, and the text keeps the alignment it inherited.",
    ],
  },
  "text-align-last": {
    edgeCases: [
      '`text-align-last: center` logs an advisory and is not applied; `CSS.supports("text-align-last", "center")` returns `false`.',
    ],
  },
  "text-justify": {
    edgeCases: [
      '`text-justify: inter-word` logs an advisory and is not applied; `CSS.supports("text-justify", "inter-word")` returns `false`.',
    ],
  },
  "text-indent": {
    examples: [
      {
        title: "Instead: indent a quoted reply",
        lang: "css",
        code: `.quoted-reply {
  padding-left: 12px;
  border-left: 3px solid #d0d7de;
  color: #656d76;
}`,
      },
    ],
    edgeCases: ["`text-indent: 0` is accepted without an advisory; any other value logs one."],
  },
  "text-transform": {
    examples: [
      {
        title: "Uppercase a button label without changing its text",
        lang: "css",
        code: `.primary-button {
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.contact-name {
  text-transform: capitalize;
}`,
      },
    ],
    edgeCases: [
      "`capitalize` uppercases the first letter after white space and leaves the rest of each word as written: `mcDonald re-entry` shows as `McDonald Re-entry`.",
      "Copied text and `getSelection().toString()` have the transformed case; `textContent` keeps the text as written.",
    ],
  },
  "text-decoration": {
    examples: [
      {
        title: "Underline links on hover and strike through removed items",
        lang: "css",
        code: `.inline-link {
  color: #0969da;
}

.inline-link:hover {
  text-decoration: underline;
}

.removed-item {
  text-decoration: line-through #8c959f;
}`,
      },
    ],
    edgeCases: [
      "The shorthand resets the color to `currentColor`, which ends a `text-decoration-color` from an earlier rule.",
      '`CSS.supports("text-decoration", "none")` returns `false`.',
    ],
  },
  "text-decoration-line": {
    examples: [
      {
        title: "Strike through completed tasks",
        lang: "css",
        code: `.task.done .task-title {
  text-decoration-line: line-through;
  color: #8c959f;
}

.task.done:hover .task-title {
  text-decoration-line: none;
}`,
      },
    ],
    edgeCases: [
      "`underline line-through` draws both lines.",
      "`none` cancels both lines, and a color, style or thickness declared later does not bring them back.",
    ],
  },
  "text-decoration-style": {
    examples: [
      {
        title: "Mark a misspelled word with a wavy underline",
        lang: "css",
        code: `.misspelled {
  text-decoration-line: underline;
  text-decoration-style: wavy;
  text-decoration-color: #cf222e;
}`,
      },
    ],
    edgeCases: [
      "It styles underlines only; a `line-through` stays solid.",
      "`dotted`, `dashed` and `double` log an advisory.",
      "A style in another rule than the line still applies, in either order.",
    ],
  },
  "text-decoration-color": {
    examples: [
      {
        title: "Fade a link's underline in on hover",
        lang: "css",
        code: `.doc-link {
  text-decoration-line: underline;
  text-decoration-color: transparent;
  transition: text-decoration-color 150ms ease-out;
}

.doc-link:hover {
  text-decoration-color: #0969da;
}`,
      },
    ],
    edgeCases: [
      "Declared in a rule without a line, it colors a line from another rule, in either order, as Tailwind's separate `underline` and `decoration-*` classes need.",
      "`currentColor` logs an advisory. Leave the property unset instead: the line then takes the text color.",
    ],
  },
  "text-decoration-thickness": {
    examples: [
      {
        title: "Draw a heavier underline under the active tab",
        lang: "css",
        code: `.tab.active {
  text-decoration-line: underline;
  text-decoration-thickness: 2px;
  text-underline-offset: 6px;
}`,
      },
    ],
    edgeCases: [
      "It applies to underlines only; a `line-through` keeps its default thickness.",
      "Only a length applies: `auto`, `from-font` and percentages are ignored without an advisory.",
    ],
  },
  "text-underline-offset": {
    examples: [
      {
        title: "Lower a link's underline clear of descenders",
        lang: "css",
        code: `.article a {
  color: #0969da;
  text-decoration-line: underline;
  text-underline-offset: 3px;
}`,
      },
    ],
    edgeCases: [
      "`auto` keeps the default position; a percentage logs an advisory.",
      "Declared without a line in the same rule, it moves an underline that another rule draws.",
    ],
  },
  "text-decoration-skip-ink": {
    edgeCases: [
      "`none` is accepted without an advisory, since that is what is drawn; `auto` logs one.",
      '`CSS.supports("text-decoration-skip-ink", "auto")` returns `false`.',
    ],
  },
  "text-underline-position": {
    edgeCases: [
      '`text-underline-position: under` logs an advisory and is not applied; `CSS.supports("text-underline-position", "under")` returns `false`.',
    ],
  },
  "text-shadow": {
    examples: [
      {
        title: "Keep a caption legible over video",
        lang: "css",
        code: `.video-caption {
  color: #fff;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.8);
}`,
      },
      {
        title: "Glow the active channel name",
        lang: "css",
        code: `.channel.active .channel-name {
  color: #fff;
  text-shadow:
    0 0 4px #58a6ff,
    0 0 12px rgba(88, 166, 255, 0.6);
}`,
      },
      {
        title: "Fade a glow in on hover",
        lang: "css",
        code: `.play-label {
  text-shadow: 0 0 8px rgba(255, 196, 0, 0);
  transition: text-shadow 200ms ease-out;
}

.play-label:hover {
  text-shadow: 0 0 8px rgba(255, 196, 0, 0.9);
}`,
      },
    ],
    edgeCases: [
      "Shadows paint in list order with the first on top, all under the text.",
      "A shadow without a color takes each element's own `color`, so a child with another `color` casts a shadow in that color.",
      "A spread (a fourth length) or a negative blur drops the whole declaration with an advisory.",
      "A blur above `32px` logs an advisory: glyph shadows blur at most 64 device pixels. Use `filter: drop-shadow()` for a softer shadow.",
      "When two animated lists differ in length, a missing shadow fades from transparent.",
    ],
  },
  "text-emphasis": {
    edgeCases: [
      '`text-emphasis: dot` logs an advisory and is not applied; `CSS.supports("text-emphasis", "dot")` returns `false`.',
    ],
  },
  "white-space": {
    examples: [
      {
        title: "Keep a menu shortcut on one line",
        lang: "css",
        code: `.menu-item {
  display: flex;
  justify-content: space-between;
  gap: 24px;
}

.menu-item .shortcut {
  white-space: nowrap;
}`,
      },
      {
        title: "Show a log message with its line breaks",
        lang: "vue",
        code: `<script setup lang="ts">
defineProps<{ message: string }>();
</script>

<template>
  <div class="log-entry">{{ message }}</div>
</template>

<style scoped>
.log-entry {
  white-space: pre-wrap;
  font-family: ui-monospace, monospace;
}
</style>`,
      },
    ],
    edgeCases: [
      "`nowrap` and `pre` never break a line, even with `overflow-wrap: anywhere`; `pre-wrap` does break a long word under it.",
      "`pre-line` collapses runs of spaces and keeps line breaks.",
    ],
  },
  "white-space-collapse": {
    examples: [
      {
        title: "Instead: keep a breadcrumb on one line",
        lang: "css",
        code: `.breadcrumb {
  display: flex;
  gap: 4px;
  white-space: nowrap;
}`,
      },
    ],
    edgeCases: [
      '`white-space-collapse: preserve` logs an advisory and is not applied; `CSS.supports("white-space-collapse", "preserve")` returns `false`.',
    ],
  },
  "text-wrap": {
    examples: [
      {
        title: "Instead: stop a button label from wrapping",
        lang: "css",
        code: `.dialog-actions button {
  padding: 6px 12px;
  white-space: nowrap;
}`,
      },
    ],
    edgeCases: [
      '`text-wrap: balance` logs an advisory and is not applied; `CSS.supports("text-wrap", "balance")` returns `false`, so an `@supports (text-wrap: balance)` block is skipped.',
    ],
  },
  "text-wrap-mode": {
    examples: [
      {
        title: "Instead: keep a status pill on one line",
        lang: "css",
        code: `.status-pill {
  padding: 2px 8px;
  border-radius: 999px;
  white-space: nowrap;
}`,
      },
    ],
    edgeCases: [
      '`text-wrap-mode: nowrap` logs an advisory and is not applied; `CSS.supports("text-wrap-mode", "nowrap")` returns `false`.',
    ],
  },
  "text-wrap-style": {
    examples: [
      {
        title: "Instead: keep a short card title on one line",
        lang: "css",
        code: `.card-title {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}`,
      },
    ],
    edgeCases: [
      '`text-wrap-style: balance` logs an advisory and is not applied; `CSS.supports("text-wrap-style", "balance")` returns `false`.',
    ],
  },
  "word-break": {
    examples: [
      {
        title: "Break commit hashes anywhere in a narrow column",
        lang: "css",
        code: `.commit-hash {
  width: 120px;
  font-family: ui-monospace, monospace;
  word-break: break-all;
}`,
      },
      {
        title: "Keep Japanese phrases together in a tooltip",
        lang: "css",
        code: `.tooltip-ja {
  max-width: 200px;
  word-break: keep-all;
}`,
      },
    ],
    edgeCases: [
      "`break-all` breaks between any two letters to fill each line, also in text that has spaces to break at.",
      "`keep-all` keeps CJK text from breaking between characters.",
      "`break-word`, the legacy value, breaks a long word as `overflow-wrap: break-word` does.",
    ],
  },
  "overflow-wrap": {
    examples: [
      {
        title: "Wrap long links inside a chat bubble",
        lang: "css",
        code: `.bubble {
  max-width: 520px;
  padding: 8px 12px;
  overflow-wrap: break-word;
}

.bubble a {
  overflow-wrap: anywhere;
}`,
      },
      {
        title: "Let a flex item holding a long path shrink",
        lang: "css",
        code: `.file-row {
  display: flex;
  width: 240px;
}

.file-row .path {
  overflow-wrap: anywhere;
}`,
        note: "With `break-word` instead, the path would hold the row open at the width of its longest token.",
      },
    ],
    edgeCases: [
      "`anywhere` lowers a flex item's min-content width to one letter, so the item shrinks and wraps; `break-word` does not, so the item keeps the width of its longest word.",
      "It has no effect under `white-space: nowrap` or `pre`.",
    ],
  },
  "line-break": {
    edgeCases: [
      '`line-break: strict` logs an advisory and is not applied; `CSS.supports("line-break", "strict")` returns `false`.',
    ],
  },
  "hyphenate-character": {
    edgeCases: ['`hyphenate-character: "-"` logs an advisory and is not applied.'],
  },
  hyphens: {
    examples: [
      {
        title: "Let a long German label break at a soft hyphen",
        lang: "vue",
        code: `<script setup lang="ts">
const label = "Benachrichtigungs\\u00ADeinstellungen";
</script>

<template>
  <div class="setting-label">{{ label }}</div>
</template>

<style scoped>
.setting-label {
  width: 140px;
  hyphens: manual;
}
</style>`,
      },
      {
        title: "Never hyphenate product names",
        lang: "css",
        code: `.product-name {
  font-weight: 600;
  hyphens: none;
}`,
      },
    ],
    edgeCases: [
      "A line that breaks at a soft hyphen ends with a visible hyphen. With `none`, a soft hyphen is no break opportunity at all.",
      "`<wbr>` is a break opportunity too.",
      '`CSS.supports("hyphens", "auto")` returns `false`, though `auto` applies as `manual`.',
    ],
  },
  "text-overflow": {
    examples: [
      {
        title: "Truncate a long file name in a list row",
        lang: "css",
        code: `.file-row {
  display: flex;
  gap: 8px;
}

.file-row .name {
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}`,
      },
    ],
    edgeCases: [
      "On a single line, spaces and ASCII punctuation just before the cut are dropped, so the line ends in `draft…`, not `draft,…`.",
      "`text-shadow` shadows the ellipsis too.",
      "With `line-clamp`, the ellipsis ends the last allowed line.",
    ],
  },
  "line-clamp": {
    examples: [
      {
        title: "Expand a clamped message preview on click",
        lang: "vue",
        code: `<script setup lang="ts">
import { ref } from "vue";

defineProps<{ body: string }>();
const expanded = ref(false);
</script>

<template>
  <div class="preview" :class="{ clamped: !expanded }" @click="expanded = !expanded">
    {{ body }}
  </div>
</template>

<style scoped>
.clamped {
  line-clamp: 3;
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>`,
        note: "The clamp lives on its own class because `line-clamp: none` cannot undo it.",
      },
    ],
    edgeCases: [
      "A fractional count is truncated: `2.5` clamps to two lines.",
      "`-webkit-line-clamp` works on its own; `display: -webkit-box` and `-webkit-box-orient: vertical` are not needed.",
    ],
  },
  "vertical-align": {
    examples: [
      {
        title: "Center custom emoji on the text of a chat composer",
        lang: "css",
        code: `sk-editor .emoji {
  width: 20px;
  height: 20px;
  vertical-align: middle;
}`,
      },
      {
        title: "Footnote marks and an icon in a sentence",
        lang: "html",
        code: `<p>E = mc<sup>2</sup>, H<sub>2</sub>O
  <img src="info.svg" width="16" height="16" style="vertical-align: middle" />
</p>`,
      },
    ],
    edgeCases: [
      "An atom's baseline is its bottom edge, as for an image, even when the atom element holds text.",
      "In an editor, a `bottom` atom taller than its row pushes the row's baseline down and a `top` atom the row's bottom. In other lines, the text of a line with a tall `bottom` box stays at the top.",
      "`overflow: hidden` on an `inline-block` does not move its baseline to its bottom edge as CSS does; it still aligns by its first line.",
      "Tailwind's preflight sets `vertical-align: middle` together with `display: block` on images, so it changes nothing, as in a browser.",
    ],
  },
  "tab-size": {
    edgeCases: ['`CSS.supports("tab-size", "4")` returns `true`, though the value changes nothing.'],
  },
  "webkit-text-fill-color": {
    examples: [
      {
        title: "Fill a welcome heading with a gradient",
        lang: "css",
        code: `.welcome-title {
  font-size: 32px;
  font-weight: 700;
  background: linear-gradient(90deg, #8957e5, #0969da);
  background-clip: text;
  -webkit-text-fill-color: transparent;
}`,
      },
    ],
    edgeCases: [
      "A `text-shadow` without a color takes the element's `color`, not the fill color.",
      "`currentColor` draws the text in `color` again, undoing a fill from an earlier rule.",
      "A value that is not a color logs an advisory.",
    ],
  },
  "webkit-text-stroke": {
    edgeCases: [
      "`-webkit-text-stroke: 1px red` logs an advisory and is not applied, and so do `-webkit-text-stroke-width` and `-webkit-text-stroke-color`.",
    ],
  },
  direction: {
    edgeCases: [
      '`direction: rtl` logs an advisory and is not applied; `CSS.supports("direction", "rtl")` returns `false`.',
      '`getComputedStyle(el).direction` is always `"ltr"`.',
    ],
  },
  "unicode-bidi": {
    edgeCases: [
      '`unicode-bidi: isolate` logs an advisory and is not applied; `CSS.supports("unicode-bidi", "isolate")` returns `false`.',
    ],
  },
  "writing-mode": {
    edgeCases: [
      '`CSS.supports("writing-mode", "vertical-rl")` returns `false` for app stylesheets. SVG text with `style="writing-mode: vertical-rl"` runs vertically.',
    ],
  },
  "text-orientation": {
    edgeCases: [
      '`text-orientation: upright` logs an advisory and is not applied; `CSS.supports("text-orientation", "upright")` returns `false`.',
    ],
  },
  "text-rendering": {
    edgeCases: [
      '`CSS.supports("text-rendering", "optimizeLegibility")` returns `false` for app stylesheets. The SVG renderer reads text rendering hints set on its text elements.',
    ],
  },
  "webkit-font-smoothing": {
    edgeCases: [
      "`-webkit-font-smoothing` and `-moz-osx-font-smoothing` are dropped without an advisory, as Chromium drops them; `CSS.supports()` returns `false` for both.",
    ],
  },
  quotes: {
    edgeCases: [
      '`quotes: none` logs an advisory and is not applied; `CSS.supports("quotes", "none")` returns `false`.',
    ],
  },
  "initial-letter": {
    edgeCases: [
      '`initial-letter: 2` logs an advisory and is not applied; `CSS.supports("initial-letter", "2")` returns `false`.',
    ],
  },
};
