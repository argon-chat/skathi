---
title: CSS coverage
description: Supported CSS properties, layout, styling, and platform limitations in the native renderer.
section: Reference
order: 1
---

# CSS coverage

What a Skathi app can use from CSS, what is coming, and what is not. Stylesheets are compiled by
lightningcss, laid out by taffy and painted natively; there is no browser engine underneath. Most
unsupported declarations print an advisory while the app runs (`sk dev`, devtools); the few that are
accepted and ignored without one say so below.

| Mark | Meaning                                                     |
| ---- | ----------------------------------------------------------- |
| ✅   | Supported on Windows, macOS and Linux                       |
| 🪟   | Supported on Windows; macOS and Linux are planned           |
| 🍎   | Supported on Windows and macOS; Linux is planned            |
| 🟡   | Supported with a limitation (see the note)                  |
| 🗓️   | Planned                                                     |
| ⏸️   | On hold: waiting on something outside Skathi                |
| ⛔   | Not planned for now                                         |
| 🗑️   | Deprecated in Skathi: will not be added, or will be removed |

## Stylesheets

| Feature                                                    | Status | Notes                                                                                  |
| ---------------------------------------------------------- | ------ | -------------------------------------------------------------------------------------- |
| `<style>` in Vue SFCs and Svelte components, global `.css` | ✅     |                                                                                        |
| `<style scoped>` (Vue), component-scoped styles (Svelte)   | ✅     | `:deep()`, `:slotted()`, `:global()` as the framework compiles them                    |
| `v-bind()` in `<style>`                                    | ✅     |                                                                                        |
| Inline `style`, `element.style`                            | ✅     |                                                                                        |
| `!important`, specificity, source order                    | ✅     |                                                                                        |
| CSS nesting (`.a { .b { } }`, `&`)                         | 🗓️     | Nested rules are dropped without an advisory; write them flat                          |
| `<style module>` (CSS modules)                             | 🗓️     | `useCssModule()` answers `{}`                                                          |
| `<style lang="scss">`, `less`, `stylus`                    | ⛔     | Not preprocessed: the content is compiled as plain CSS. Compile it yourself beforehand |
| Tailwind CSS 3.4                                           | ✅     | Run by `sk dev` / `sk build` when the app has a `tailwind.config.*`                    |
| Tailwind CSS 4                                             | 🗓️     | Its `@layer` and `@property` output does not resolve yet; use 3.4                      |
| Vendor-prefixed properties                                 | 🟡     | See [Vendor prefixes](#vendor-prefixes)                                                |

## Box model and sizing

| Property                                                         | Status | Notes                                                |
| ---------------------------------------------------------------- | ------ | ---------------------------------------------------- |
| `width`, `height`, `min-*`, `max-*`                              | ✅     | Lengths, percentages, `auto`, `none`, `calc()`       |
| `width: min-content` / `max-content` / `fit-content` / `stretch` | ⛔     | Ignored without an advisory; use flex or grid sizing |
| `box-sizing: border-box`                                         | ✅     | Every box is `border-box`                            |
| `box-sizing: content-box`                                        | ⛔     | Padding and border stay inside the declared size     |
| `aspect-ratio`                                                   | ✅     | `auto` cannot undo an earlier ratio                  |
| `inline-size`, `block-size` and their `min-` / `max-`            | 🗓️     | Use `width` / `height`                               |
| `overflow`, `overflow-x`, `overflow-y`                           | ✅     | `visible`, `hidden`, `clip`, `auto`, `scroll`        |
| `overflow-clip-margin`                                           | ⛔     |                                                      |
| `visibility`                                                     | ✅     | `collapse` behaves as `hidden`                       |
| `opacity`                                                        | ✅     | Number or percentage                                 |

## Layout

| Property                                                          | Status | Notes                                                                                                         |
| ----------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------------- |
| `display: block`, `flex`, `grid`, `none`, `flow-root`             | ✅     |                                                                                                               |
| `display: inline`, `inline-block`, `inline-flex`, `inline-grid`   | 🟡     | No line boxes: a run of inline content flows as a wrapping row; an inline box among blocks takes its own line |
| `display: contents`                                               | ⛔     | Laid out as `block` without an advisory                                                                       |
| `display: table` and the other table values                       | ⛔     | Laid out as blocks; build tables with grid or flex rows                                                       |
| `display: list-item`                                              | 🟡     | A block; markers are drawn on `<li>` only                                                                     |
| `flex`, `flex-grow`, `flex-shrink`, `flex-basis`                  | ✅     | `flex-basis: content` / `min-content` are not                                                                 |
| `flex-direction`, `flex-wrap`                                     | ✅     |                                                                                                               |
| `flex-flow`                                                       | 🗓️     | Use `flex-direction` and `flex-wrap`                                                                          |
| `order`                                                           | ✅     | Flex and grid                                                                                                 |
| `align-items`, `align-self`, `align-content`                      | ✅     | `align-self: auto` / `normal` cannot undo an earlier value                                                    |
| `justify-content`, `justify-items`, `justify-self`                | ✅     | `justify-content: left` / `right` / `normal` are not                                                          |
| `place-items`                                                     | ✅     |                                                                                                               |
| `place-content`, `place-self`                                     | 🗓️     | Use the `align-*` / `justify-*` longhands                                                                     |
| `gap`, `row-gap`, `column-gap`                                    | ✅     |                                                                                                               |
| `grid-template-columns`, `grid-template-rows`                     | ✅     | Lengths, `%`, `fr`, `auto`, `min-content`, `max-content`, `minmax()`, `repeat()` including `auto-fill/fit`    |
| `fit-content()` tracks                                            | ⛔     |                                                                                                               |
| `grid-column`, `grid-row`, `grid-area` with numbers and `span`    | ✅     |                                                                                                               |
| `grid-template-areas`, named lines and areas                      | ⛔     | Place items by line number                                                                                    |
| `grid`, `grid-template` shorthands                                | ⛔     | Use the longhands                                                                                             |
| `grid-auto-flow` (with `dense`), `grid-auto-rows`, `-columns`     | ✅     |                                                                                                               |
| `subgrid`, `masonry`                                              | ⛔     |                                                                                                               |
| `position: static`, `relative`, `absolute`                        | ✅     | A `relative` box with `z-index: auto` paints in tree order                                                    |
| `position: fixed`                                                 | 🟡     | Placed against the parent: exact at the app root or in a `<Teleport to="body">`                               |
| `position: sticky`                                                | 🟡     | Laid out as `relative` inside a scroll container                                                              |
| `top`, `right`, `bottom`, `left`, `inset`                         | ✅     | Including `calc()` with a percentage                                                                          |
| `z-index`                                                         | ✅     |                                                                                                               |
| `float`, `clear`                                                  | ⛔     | Use flex or grid                                                                                              |
| `columns`, `column-count`, `column-width`, `column-rule`          | ⛔     | Use grid                                                                                                      |
| `break-*`, `page-break-*`                                         | ⛔     |                                                                                                               |
| `border-collapse`, `border-spacing`, `table-layout`               | ⛔     |                                                                                                               |
| Anchor positioning (`anchor-name`, `position-anchor`, `anchor()`) | ⛔     | Position from script with `getBoundingClientRect`                                                             |

## Spacing and logical properties

| Property                                                           | Status | Notes                               |
| ------------------------------------------------------------------ | ------ | ----------------------------------- |
| `margin`, `padding` and their sides                                | ✅     | Negative margins and `auto` margins |
| `margin-inline`, `margin-block`, `padding-inline`, `padding-block` | ✅     | Mapped left-to-right, top-to-bottom |
| `inset-inline`, `inset-block` and their `-start` / `-end`          | ✅     | Mapped left-to-right, top-to-bottom |
| `border-inline-*`, `border-block-*`, `border-start-start-radius`   | 🗓️     | Use the physical sides              |

## Typography and text

| Property                                                         | Status | Notes                                                                                |
| ---------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------ |
| `font-family`                                                    | ✅     | Installed and `@font-face` families, `serif`, `sans-serif`, `monospace`, `system-ui` |
| `ui-monospace`, `ui-sans-serif`, `emoji` and other new generics  | 🟡     | Not resolved on their own; list a fallback after them                                |
| `@font-face`                                                     | ✅     | TTF, OTF, WOFF, WOFF2; also `FontFace` from script                                   |
| `font-size`                                                      | ✅     | Lengths, `%`, keywords, `smaller` / `larger`, `calc()`                               |
| `font-weight`                                                    | ✅     | `lighter` gives 100                                                                  |
| `font-style`                                                     | ✅     | The angle of `oblique <angle>` is ignored                                            |
| `font` shorthand                                                 | 🗓️     | Use the longhands                                                                    |
| `font-stretch`, `font-width`                                     | ⛔     |                                                                                      |
| `font-variant`, `font-variant-caps`                              | ⛔     | `font-feature-settings: "smcp"` where the font has it                                |
| `font-variant-numeric`, `font-variant-ligatures`, `font-kerning` | ✅     |                                                                                      |
| `font-feature-settings`                                          | ✅     |                                                                                      |
| `font-variation-settings`                                        | ⛔     | `font-weight` and `font-style` reach the matching axes; `opsz` follows the size      |
| `font-optical-sizing`                                            | 🟡     | Always `auto`                                                                        |
| `font-synthesis`, `font-size-adjust`, `font-variant-east-asian`  | ⛔     |                                                                                      |
| `line-height`                                                    | ✅     |                                                                                      |
| `letter-spacing`, `word-spacing`                                 | ✅     | Not as a percentage                                                                  |
| `color`                                                          | ✅     |                                                                                      |
| `text-align`                                                     | ✅     | `left`, `right`, `center`, `start`, `end`; not `justify`                             |
| `text-align-last`, `text-justify`, `text-indent`                 | ⛔     | For `text-indent`, `padding-left` indents the whole block                            |
| `text-transform`                                                 | ✅     |                                                                                      |
| `text-decoration`, `text-decoration-line`                        | 🟡     | Underline and line-through; no overline. Use `text-decoration-line: none` to undo    |
| `text-decoration-style`                                          | 🟡     | `solid` and `wavy`; `dotted`, `dashed`, `double` draw solid                          |
| `text-decoration-color`, `-thickness`, `text-underline-offset`   | ✅     |                                                                                      |
| `text-decoration-skip-ink: auto`, `text-underline-position`      | ⛔     | The underline is one unbroken line                                                   |
| `text-shadow`                                                    | ✅     | Several shadows, with blur                                                           |
| `text-emphasis`                                                  | ⛔     |                                                                                      |
| `white-space`                                                    | ✅     | All values; `break-spaces` behaves as `pre-wrap`                                     |
| `white-space-collapse`, `text-wrap`, `text-wrap-mode`, `-style`  | ⛔     | `white-space: nowrap` for no wrapping                                                |
| `word-break`, `overflow-wrap`, `word-wrap`                       | ✅     |                                                                                      |
| `line-break`, `hyphenate-character`                              | ⛔     |                                                                                      |
| `hyphens`                                                        | 🟡     | Breaks only at soft hyphens (`&shy;`): no dictionary                                 |
| `text-overflow: ellipsis`                                        | ✅     | `clip` cannot undo an earlier ellipsis; a custom string is not supported             |
| `line-clamp`, `-webkit-line-clamp`                               | ✅     |                                                                                      |
| `vertical-align`                                                 | ⛔     | Use `align-items` / `align-self` on a flex parent                                    |
| `tab-size`                                                       | ⛔     | Ignored without an advisory                                                          |
| `-webkit-text-fill-color`                                        | ✅     | With `background-clip: text` for gradient text                                       |
| `-webkit-text-stroke`                                            | ⛔     |                                                                                      |
| `direction`, `unicode-bidi`, `writing-mode`, `text-orientation`  | ⛔     | Left-to-right, horizontal text only                                                  |
| `text-rendering`, `-webkit-font-smoothing`                       | ⛔     | The platform's text rendering                                                        |
| `quotes`, `initial-letter`, `ruby-*`                             | ⛔     |                                                                                      |

## Color

| Feature                                                     | Status | Notes                                     |
| ----------------------------------------------------------- | ------ | ----------------------------------------- |
| Named colors, hex, `rgb()`, `hsl()`, `transparent`          | ✅     |                                           |
| `currentColor`                                              | 🟡     | Not inside `box-shadow`                   |
| `hwb()`, `lab()`, `lch()`, `oklab()`, `oklch()`, `color()`  | ✅     | In every color property                   |
| `color-mix()`                                               | ✅     |                                           |
| Relative colors (`rgb(from red r g b / 50%)`)               | ✅     | With a literal origin color               |
| `light-dark()`                                              | ⛔     | Use `@media (prefers-color-scheme: dark)` |
| System colors (`Canvas`, `AccentColor`, …)                  | ⛔     |                                           |
| `color-scheme`, `forced-color-adjust`, `print-color-adjust` | ⛔     |                                           |

## Backgrounds and borders

| Property                                                               | Status | Notes                                                                                      |
| ---------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------ |
| `background-color`, `background` shorthand                             | ✅     | Several layers                                                                             |
| `background-image: url()`                                              | ✅     | Raster images and SVG                                                                      |
| `linear-`, `radial-`, `conic-gradient()` and `repeating-*`             | ✅     | Color hints and two-position stops; not an `in <colorspace>` interpolation                 |
| `image-set()`, `cross-fade()`, `element()`, `-webkit-gradient()`       | ⛔     | Write one `url()` or a standard gradient                                                   |
| `background-size`, `background-repeat`                                 | ✅     | `space` and `round` repeat as `repeat`                                                     |
| `background-position`, `-x`, `-y`                                      | 🟡     | An offset from the right or bottom edge (`right 10px`) sits flush; use `left`/`top` or `%` |
| `background-clip: text`                                                | ✅     |                                                                                            |
| `background-clip` (other values), `background-origin`                  | ⛔     | The background fills the padding box                                                       |
| `background-attachment`                                                | ⛔     | Ignored without an advisory                                                                |
| `background-blend-mode`                                                | ⛔     |                                                                                            |
| `border`, sides, `border-width`, `border-color`                        | ✅     |                                                                                            |
| `border-style: solid`, `dashed`, `none`, `hidden`                      | ✅     |                                                                                            |
| `border-style: dotted`, `double`, `groove`, `ridge`, `inset`, `outset` | 🟡     | Drawn solid                                                                                |
| `border-radius` and the corner longhands                               | 🟡     | Lengths and `%`; an elliptical corner is drawn as a circle of the smaller radius           |
| `corner-shape`                                                         | ⛔     |                                                                                            |
| `border-image` and its longhands                                       | ⛔     | A gradient border: `background` with `mask` (see Effects), or a wrapper with padding       |
| `outline`, `outline-width`, `-color`, `-style`, `-offset`              | ✅     | `dotted` draws dashed                                                                      |
| `box-decoration-break`                                                 | ⛔     |                                                                                            |

## Effects

| Property                                                          | Status | Notes                                                                               |
| ----------------------------------------------------------------- | ------ | ----------------------------------------------------------------------------------- |
| `box-shadow`                                                      | ✅     | Several shadows, `inset`, spread, `none`                                            |
| `opacity`                                                         | ✅     |                                                                                     |
| `filter: blur()`                                                  | 🟡     | On a box with no children and a solid background                                    |
| `filter: drop-shadow()`                                           | 🟡     | On a box with an opaque solid background, and on inline `<svg>`                     |
| `filter: url(#id)` to an SVG `feColorMatrix`                      | ✅     | One filter, in the same document                                                    |
| `filter`: `grayscale()`, `brightness()`, `contrast()`, `invert()` | ⛔     | Write a `--shading` effect                                                          |
| `filter`: `sepia()`, `saturate()`, `hue-rotate()`, `opacity()`    | ⛔     | Write a `--shading` effect                                                          |
| `backdrop-filter: blur()`                                         | ✅     | Other functions are dropped                                                         |
| `mix-blend-mode`                                                  | ⛔     |                                                                                     |
| `isolation`                                                       | ✅     |                                                                                     |
| `clip-path`                                                       | ✅     | `inset()`, `circle()`, `ellipse()`, `polygon()`, `path()`, a box keyword            |
| `clip-path: url()`, `xywh()`, `rect()`                            | ⛔     |                                                                                     |
| `clip: rect()`                                                    | 🟡     | Only the empty rect of the visually-hidden idiom                                    |
| `mask`, `mask-image`, `mask-position`, `mask-repeat`, `mask-size` | 🟡     | SVG and raster `url()`, gradients. Not `url(#id)`; a `fixed` descendant is unmasked |
| `mask-mode`                                                       | 🟡     | `alpha`, `luminance`, `match-source` (as alpha); one mode for all the layers        |
| `mask-clip`, `mask-origin`                                        | 🟡     | `border-box`; `mask-clip: content-box` only in the gradient-border ring             |
| `mask-composite`                                                  | 🟡     | `add`; `exclude` only in the gradient-border ring (two layers, the top one clipped) |
| `mask-type`, `mask-border`                                        | ⛔     |                                                                                     |
| `-webkit-mask-*`                                                  | ⛔     | Use the unprefixed property                                                         |
| `-webkit-box-reflect`                                             | ⛔     |                                                                                     |
| `--shading`                                                       | ✅     | Skathi's GPU effects: a custom property browsers ignore. Applies to its own element |

## Transforms

| Property                                                                            | Status | Notes                                                                    |
| ----------------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------ |
| `transform` with `translate()`, `rotate()`, `scale()`, `skew()`, `matrix()`         | ✅     | 2D; `translate3d()` / `scale3d()` with a zero Z                          |
| `translate()` with a `calc()` mixing `%` and a length                               | ⛔     | Use a length or a percentage                                             |
| `transform-origin`                                                                  | 🟡     | Lengths, percentages, keywords; not a keyword with an offset or `calc()` |
| `perspective`, `rotateX()`, `rotateY()`, `rotate3d()`, `translateZ()`, `matrix3d()` | 🗓️     |                                                                          |
| `transform-style`, `backface-visibility`, `perspective-origin`                      | 🗓️     |                                                                          |
| `translate`, `rotate`, `scale` properties                                           | 🗓️     | Use `transform`                                                          |
| `transform-box`, `offset-path`                                                      | ⛔     |                                                                          |

## Transitions and animations

| Feature                                                            | Status | Notes                                                                      |
| ------------------------------------------------------------------ | ------ | -------------------------------------------------------------------------- |
| `transition` and its longhands                                     | ✅     | Interrupted transitions retarget from the value on screen                  |
| A `transition` composed with `var()`                               | ⛔     | Write it literally                                                         |
| Negative `transition-delay` and `animation-delay`                  | ✅     | Start part-way through                                                     |
| `transition-behavior: allow-discrete`                              | ⛔     |                                                                            |
| `@keyframes`, `animation` and its longhands                        | ✅     | Per-property keyframe timelines, every fill mode, direction and play state |
| Easing: keywords, `cubic-bezier()`, `steps()`                      | ✅     |                                                                            |
| Easing: `linear(…)`                                                | ⛔     |                                                                            |
| `animation-timeline`, `animation-range`, scroll and view timelines | ⛔     | Runs on the document timeline                                              |
| `animation-composition`                                            | ⛔     |                                                                            |
| Web Animations (`element.animate()`)                               | ✅     | Same properties as `@keyframes`                                            |
| `@starting-style`                                                  | ⛔     | Use the framework's transitions (`<Transition>`, `transition:`)            |
| View transitions (`view-transition-name`, `::view-transition-*`)   | 🟡     | Same-document only; `@view-transition` (cross-document) is not             |

These properties animate (transitions, `@keyframes`, `element.animate()`): `opacity`, `color`,
`background-color`, `border-color`, `text-decoration-color`, `fill`, `stroke`, `width`, `height`,
`min-*`, `max-*`, `padding-*`, `margin-*`, the `border-*-radius` corners, `border-*-width`, `row-gap`,
`column-gap`, `font-size`, `transform`, `--shading` and `@property` custom properties with the
`<percentage>` syntax. `overflow` animates discretely in `@keyframes` and `element.animate()`. Any
other property snaps to its end value.

## Scrolling

| Property                                                    | Status | Notes                                       |
| ----------------------------------------------------------- | ------ | ------------------------------------------- |
| `overflow: auto` / `scroll`                                 | ✅     | Overlay scrollbars                          |
| `scrollbar-width`                                           | ✅     | `auto`, `thin`, `none`                      |
| `scrollbar-color`                                           | ✅     |                                             |
| `scrollbar-gutter`                                          | ⛔     |                                             |
| `::-webkit-scrollbar` and its parts                         | ⛔     | Use `scrollbar-width` and `scrollbar-color` |
| `overscroll-behavior` and its axes                          | ✅     | `auto`, `contain`, `none`                   |
| `scroll-behavior: smooth`                                   | ✅     |                                             |
| `scroll-snap-type`, `scroll-snap-align`, `scroll-snap-stop` | 🗓️     |                                             |
| `scroll-padding-*`, `scroll-margin-*`                       | 🗓️     | Accepted without an effect for now          |

## Interaction

| Property                                  | Status | Notes                                                                                                     |
| ----------------------------------------- | ------ | --------------------------------------------------------------------------------------------------------- |
| `cursor`                                  | 🟡     | Native cursors; `wait`, `progress`, `help`, `zoom-*` and `none` show the arrow; `url()` uses the fallback |
| `pointer-events`                          | ✅     | `auto` and `none`                                                                                         |
| `user-select`                             | ✅     | `contain` behaves as `text`                                                                               |
| `touch-action`                            | 🟡     | Accepted; the host does not deliver touch input yet                                                       |
| `appearance`                              | ✅     | `none` strips a form control's native look                                                                |
| `accent-color`                            | ✅     |                                                                                                           |
| `caret-color`, `caret-shape`              | ⛔     |                                                                                                           |
| `resize`                                  | ⛔     | Build a drag handle with pointer events                                                                   |
| `field-sizing`, `interactivity`           | ⛔     |                                                                                                           |
| `window-drag` / `app-region` (`-webkit-`) | ✅     | Drags the window. In Svelte use `-webkit-app-region`: svelte-check flags the other two                    |
| `--window-control`                        | ✅     | Skathi: `minimize`, `maximize`, `close` make the element that window button                               |

## SVG properties in CSS

These apply to inline `<svg>`.

| Property                                                                       | Status | Notes                                                                  |
| ------------------------------------------------------------------------------ | ------ | ---------------------------------------------------------------------- |
| `fill`, `stroke`                                                               | ✅     | Colors and `none`; a `url(#…)` from a CSS rule draws in `currentColor` |
| `stroke-width`, `stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit`       | ✅     |                                                                        |
| `stroke-dasharray`, `stroke-dashoffset`                                        | ✅     |                                                                        |
| `fill-opacity`, `stroke-opacity`, `fill-rule`, `shape-rendering`               | ✅     |                                                                        |
| `stop-color`, `stop-opacity`, `clip-rule`, `flood-color`, `lighting-color`     | ⛔     | Write them as SVG attributes, which render                             |
| `x`, `y`, `cx`, `cy`, `r`, `rx`, `ry`, `d` as CSS properties                   | ⛔     | Write them as SVG attributes                                           |
| `vector-effect`, `paint-order`, `marker-*`, `text-anchor`, `dominant-baseline` | ⛔     | Write them as SVG attributes                                           |

## Containment and hints

| Property                                        | Status | Notes                                |
| ----------------------------------------------- | ------ | ------------------------------------ |
| `contain`                                       | ✅     | `paint`, `content` and `strict` clip |
| `will-change`                                   | ✅     | A hint                               |
| `container-type`, `container-name`, `container` | 🗓️     | With `@container`                    |
| `content-visibility`, `contain-intrinsic-size`  | ⛔     |                                      |

## Generated content and lists

| Feature                                                          | Status | Notes                                                                                          |
| ---------------------------------------------------------------- | ------ | ---------------------------------------------------------------------------------------------- |
| `::before`, `::after` with a string `content`                    | ✅     | Including `content: ""` boxes                                                                  |
| `content: attr()`, `counter()`, `url()`, `open-quote`            | ⛔     | Render the value as a real element                                                             |
| `::before` / `::after` behind a state (`:hover::after`)          | ⛔     | Key it on an attribute instead (`[data-state=open]::after`)                                    |
| `counter-reset`, `counter-increment`, `counter-set`              | ⛔     |                                                                                                |
| `list-style-type`                                                | 🟡     | On `<li>`: `disc`, `circle`, `square`, `decimal`, `decimal-leading-zero`, alpha, roman, `none` |
| `list-style-position`, `list-style`                              | ✅     |                                                                                                |
| `list-style-image`, string markers, `@counter-style`, `::marker` | ⛔     | Use `::before { content: "…" }`                                                                |
| `object-fit`                                                     | ✅     |                                                                                                |
| `object-position`                                                | ✅     |                                                                                                |
| `image-rendering`, `image-orientation`                           | ⛔     |                                                                                                |

## Selectors

| Selector                                                                                                | Status | Notes                                                          |
| ------------------------------------------------------------------------------------------------------- | ------ | -------------------------------------------------------------- |
| Type, class, id, `*`, attribute selectors (every operator, `i`)                                         | ✅     |                                                                |
| Descendant, `>`, `+`, `~`                                                                               | ✅     |                                                                |
| `:hover`, `:active`, `:focus`, `:focus-visible`, `:focus-within`                                        | ✅     | Not inside `:not()`, `:is()`, `:where()` or `:has()`           |
| A state on an ancestor compound (`.menu:focus .item`)                                                   | ✅     | One ancestor per state; not left of `+` or `~`                 |
| `:disabled`, `:enabled`, `:checked`, `:fullscreen`                                                      | ✅     |                                                                |
| `:root`, `:empty`, `:first-child`, `:last-child`, `:only-child`, `:nth-*`, `*-of-type`                  | ✅     |                                                                |
| `:nth-child(… of S)`                                                                                    | ⛔     |                                                                |
| `:not()`, `:is()`, `:where()`                                                                           | ✅     |                                                                |
| `:has()`                                                                                                | 🟡     | Descendant, `>`, `+`, `~`; not nested, not with a state inside |
| `:scope`                                                                                                | ✅     | In `querySelector`                                             |
| `:placeholder-shown`, `:read-only`, `:required`, `:indeterminate`, `:default`, `:autofill`              | ⛔     | Key it on an attribute or a class                              |
| `:valid`, `:invalid`, `:user-valid`, `:user-invalid`                                                    | ⛔     | Key it on an attribute or a class                              |
| `:link`, `:visited`, `:any-link`, `:target`                                                             | ⛔     |                                                                |
| `:lang()`, `:dir()`, `:modal`, `:popover-open`, `:open`, `:defined`                                     | ⛔     |                                                                |
| `::placeholder`, `::selection`, `::file-selector-button`                                                | ✅     |                                                                |
| `::-webkit-slider-thumb`, `::-webkit-slider-runnable-track`, `::-moz-range-*`, `::-webkit-color-swatch` | ✅     | Style range and color inputs                                   |
| `::marker`, `::first-line`, `::first-letter`, `::backdrop`, `::highlight()`                             | ⛔     |                                                                |
| `:host`, `::part()`, `::slotted()`                                                                      | ⛔     | No shadow DOM; they match nothing                              |
| Namespaced selectors (`svg\|a`)                                                                         | ⛔     |                                                                |

## At-rules

| Rule                                                            | Status | Notes                                                                                                            |
| --------------------------------------------------------------- | ------ | ---------------------------------------------------------------------------------------------------------------- |
| `@media`                                                        | ✅     | See [Media features](#media-features)                                                                            |
| `@supports`                                                     | ✅     | Answers for Skathi: a declaration passes only if Skathi applies it; `selector()` too                             |
| `@container`                                                    | 🗓️     |                                                                                                                  |
| `@layer`                                                        | 🗓️     | The rules inside are dropped for now; Tailwind's `@layer` is resolved by its build pass                          |
| `@keyframes`                                                    | ✅     |                                                                                                                  |
| `@font-face`                                                    | ✅     |                                                                                                                  |
| `@import`                                                       | 🟡     | Local files are inlined by `sk dev` / `sk build`; remote URLs are not                                            |
| `@property`                                                     | 🟡     | `<percentage>` registrations, which then animate; other syntaxes are ignored and the property stays unregistered |
| `@scope`                                                        | ⛔     | Use `<style scoped>`                                                                                             |
| `@starting-style`                                               | ⛔     |                                                                                                                  |
| `@page`, `@namespace`, `@counter-style`, `@font-feature-values` | ⛔     |                                                                                                                  |

## Media features

A query that uses an unsupported feature drops its whole `@media` block, with an advisory.

| Feature                                                                                | Status | Notes                                                                          |
| -------------------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------ |
| `screen`, `all`                                                                        | ✅     |                                                                                |
| `print`                                                                                | ✅     | Never matches                                                                  |
| `width`, `height`, `min-*`, `max-*`, range syntax (`400px <= width < 800px`)           | 🟡     | `px` only; an `em` length drops the block                                      |
| `prefers-color-scheme`, `prefers-reduced-motion`                                       | ✅     | Follow the system settings                                                     |
| `resolution`, `min-resolution`, `max-resolution`                                       | ✅     | `dppx`, `dpi`, `dpcm`                                                          |
| `display-mode`                                                                         | ✅     | `standalone`, `window-controls-overlay`, `fullscreen`; `browser` never matches |
| `pointer`, `hover`, `any-pointer`, `any-hover`                                         | ✅     |                                                                                |
| `and`, `or`, `not`, `only`, comma lists                                                | ✅     |                                                                                |
| `orientation`, `aspect-ratio`                                                          | ⛔     | Use `width` and `height`                                                       |
| `prefers-contrast`, `prefers-reduced-transparency`, `forced-colors`, `inverted-colors` | ⛔     |                                                                                |
| `color`, `color-gamut`, `dynamic-range`, `scripting`, `update`                         | ⛔     |                                                                                |
| `-webkit-device-pixel-ratio`, `device-width`, `device-height`                          | ⛔     | Use `resolution` and `width`                                                   |
| Custom media queries (`--name`)                                                        | ⛔     |                                                                                |

## Values and units

| Feature                                                                                             | Status | Notes                                                                                      |
| --------------------------------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------ |
| `px`, `%`, `em`, `rem`, `ch`, `ex`, `rch`, `rex`                                                    | ✅     |                                                                                            |
| `vw`, `vh`, `vmin`, `vmax`, `vi`, `vb` and the `s` / `l` / `d` variants (`dvh`)                     | ✅     | The window is the viewport, so all three variants are equal                                |
| `in`, `cm`, `mm`, `pt`, `pc`, `Q`                                                                   | 🗓️     | Ignored without an advisory today; use `px`                                                |
| `lh`, `rlh`, container units (`cqw`, …)                                                             | ⛔     | Ignored without an advisory                                                                |
| `calc()`                                                                                            | 🟡     | Mixing `%` and a length works in sizes, insets, margins, padding, gaps and `border-radius` |
| `min()`, `max()`, `clamp()`                                                                         | 🟡     | Not over a percentage mixed with a length                                                  |
| `round()`, `mod()`, `abs()`, `sign()`, `pow()`, `sqrt()`, `hypot()`, `exp()`, `log()`, trigonometry | ✅     | With constant arguments                                                                    |
| `var()`, custom properties                                                                          | ✅     | With fallbacks, inherited                                                                  |
| `env()`                                                                                             | ✅     | `safe-area-inset-*`, `titlebar-area-*` (Window Controls Overlay)                           |
| `attr()` in property values                                                                         | ⛔     |                                                                                            |
| `inherit`, `initial`, `unset`, `revert`                                                             | ✅     |                                                                                            |
| `all`                                                                                               | ⛔     |                                                                                            |
| `calc-size()`, `anchor()`, `anchor-size()`                                                          | ⛔     |                                                                                            |

## Vendor prefixes

Write unprefixed properties. Skathi reads these `-webkit-` spellings as the unprefixed property,
which then has that property's status above: `-webkit-align-*`, `-webkit-animation*`,
`-webkit-app-region`, `-webkit-appearance`, `-webkit-backdrop-filter`, `-webkit-backface-visibility`,
`-webkit-background-clip`, `-webkit-background-origin`, `-webkit-background-size`,
`-webkit-border-*-radius`, `-webkit-box-*` (the legacy flexbox properties), `-webkit-box-reflect`,
`-webkit-box-shadow`, `-webkit-box-sizing`, `-webkit-clip-path`, `-webkit-column*`,
`-webkit-filter`, `-webkit-flex*`, `-webkit-font-feature-settings`, `-webkit-hyphens`,
`-webkit-justify-content`, `-webkit-line-clamp`, `-webkit-order`, `-webkit-perspective*`,
`-webkit-text-fill-color`, `-webkit-text-size-adjust`, `-webkit-text-stroke*`,
`-webkit-transform*`, `-webkit-transition*`, `-webkit-user-select`, `-webkit-writing-mode`.

Every other vendor-prefixed property (`-moz-`, `-ms-`, `-o-`, and the remaining `-webkit-` ones such
as `-webkit-font-smoothing` or `-webkit-tap-highlight-color`) is dropped without an advisory, as
Chromium drops what it does not know. `-webkit-mask-*` is the exception: it is dropped with an
advisory to use the unprefixed `mask-*`.
