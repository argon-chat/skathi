import { bcd } from "./bcd";

export type Status = "supported" | "partial" | "planned" | "on-hold" | "not-planned" | "deprecated";
export type Platform = "windows" | "macos" | "linux";
export type Kind =
  | "property"
  | "shorthand"
  | "selector"
  | "pseudo-class"
  | "pseudo-element"
  | "at-rule"
  | "function"
  | "media-feature"
  | "unit"
  | "feature";
export type Release = "0.2.0" | "0.3.0" | "0.4.0" | "0.5.0" | "0.6.0";
/** The release a platform's support arrived in, for the platforms where it differs from the entry's `since`. */
export type PlatformSince = Partial<Record<Platform, Release>>;

export const categories = [
  "Stylesheets",
  "Box model and sizing",
  "Layout",
  "Spacing and logical properties",
  "Typography and text",
  "Color",
  "Backgrounds and borders",
  "Effects",
  "Transforms",
  "Transitions and animations",
  "Scrolling",
  "Interaction",
  "SVG properties in CSS",
  "Containment and hints",
  "Generated content and lists",
  "Selectors",
  "At-rules",
  "Media features",
  "Values and units",
  "Vendor prefixes",
] as const;
export type Category = (typeof categories)[number];

/** Section prose from the coverage page that applies to the whole category. Inline code is Markdown. */
export const categoryNotes: Readonly<Partial<Record<Category, string>>> = {
  "Transitions and animations":
    "These properties animate (transitions, `@keyframes`, `element.animate()`): `opacity`, `color`, `background-color`, `border-color`, `text-decoration-color`, `text-shadow`, `fill`, `stroke`, `width`, `height`, `min-*`, `max-*`, `padding-*`, `margin-*`, the `border-*-radius` corners, `corner-shape` and its corners, `border-*-width`, `row-gap`, `column-gap`, `font-size`, `object-position`, `aspect-ratio`, `transform` and `--shading`. `@property` custom properties with the `<percentage>` syntax animate in `@keyframes` only. `overflow` animates discretely in `@keyframes` and `element.animate()`. Any other property snaps to its end value.",
  "SVG properties in CSS":
    "App stylesheets use Skathi's cascade. An inline `<svg>` also has its own renderer, which reads presentation attributes, element styles and a `<style>` inside the SVG.",
  "Media features": "A query that uses an unsupported feature drops its whole `@media` block, with an advisory.",
  "Vendor prefixes":
    "Write unprefixed properties. Skathi reads a set of `-webkit-` spellings as the unprefixed property, which then has that property's status. Every other vendor-prefixed property is dropped without an advisory, as Chromium drops what it does not know.",
};

export interface ValueSupport {
  value: string;
  status: Status;
  note?: string;
  /** Only when it differs from the entry's `since`. */
  since?: Release;
}

export interface CssEntry {
  slug: string;
  name: string;
  kind: Kind;
  category: Category;
  summary: string;
  status: Status;
  platforms: Record<Platform, Status>;
  since?: Release;
  platformSince?: PlatformSince;
  experimental?: boolean;
  /** Deprecated on the web (BCD), as opposed to `status: "deprecated"` in Skathi. */
  webDeprecated?: boolean;
  skathiOnly?: boolean;
  mdn?: string;
  values?: ValueSupport[];
  /** What works and how it behaves. */
  notes?: string[];
  /** What differs from browsers. */
  limitations?: string[];
  /** What to write instead. */
  workaround?: string;
  /** Interpolated by transitions, `@keyframes` and `element.animate()`; anything else snaps. */
  animates?: "interpolated" | "discrete";
  related?: string[];
  aliases?: string[];
}

type Extra = Partial<
  Pick<
    CssEntry,
    | "values"
    | "notes"
    | "limitations"
    | "workaround"
    | "animates"
    | "related"
    | "aliases"
    | "skathiOnly"
    | "experimental"
    | "since"
    | "platformSince"
    | "platforms"
  >
> & { slug?: string };

interface Draft extends Extra {
  name: string;
  kind: Kind;
  status: Status;
  summary: string;
}

const OK = "supported";
const PART = "partial";
const PLAN = "planned";
const NO = "not-planned";

const FIRST_RELEASE: Release = "0.2.0";

const draft =
  (kind: Kind) =>
  (name: string, status: Status, summary: string, x: Extra = {}): Draft => ({ name, kind, status, summary, ...x });
const prop = draft("property");
const sh = draft("shorthand");
const pc = draft("pseudo-class");
const pe = draft("pseudo-element");
const at = draft("at-rule");
const fn = draft("function");
const mf = draft("media-feature");
const unit = draft("unit");
const named =
  (kind: Kind) =>
  (slug: string, name: string, status: Status, summary: string, x: Extra = {}): Draft => ({
    slug,
    name,
    kind,
    status,
    summary,
    ...x,
  });
const kw = named("feature");
const feat = named("feature");
const sel = named("selector");

function v(value: string, status: Status, note?: string, since?: Release): ValueSupport {
  return { value, status, ...(note === undefined ? {} : { note }), ...(since === undefined ? {} : { since }) };
}

function slugFor(d: Draft): string {
  if (d.slug !== undefined) return d.slug;
  const bare = d.name
    .replace(/^[:@-]+/, "")
    .replace(/\(\)$/, "")
    .toLowerCase();
  switch (d.kind) {
    case "property":
    case "shorthand":
      return bare;
    case "at-rule":
      return `at-${bare}`;
    case "pseudo-class":
    case "pseudo-element":
      return `pseudo-${bare}`;
    case "function":
      return `function-${bare}`;
    case "media-feature":
      return `media-${bare}`;
    case "unit":
      return `unit-${bare}`;
    default:
      throw new Error(`${d.name}: a ${d.kind} entry needs an explicit slug`);
  }
}

function section(category: Category, drafts: readonly Draft[]): CssEntry[] {
  return drafts.map((d) => {
    const slug = slugFor(d);
    const status = d.status === OK && d.values?.some((value) => value.status !== OK) ? PART : d.status;
    const e: CssEntry = {
      ...d,
      slug,
      name: d.kind === "media-feature" ? `@media (${d.name})` : d.name,
      category,
      status,
      platforms: d.platforms ?? { windows: status, macos: status, linux: status },
    };
    if (d.status === OK || d.status === PART) e.since = d.since ?? FIRST_RELEASE;
    const info = bcd[slug];
    if (info?.mdn !== undefined) e.mdn = info.mdn;
    if (info?.experimental) e.experimental = true;
    if (info?.deprecated) e.webDeprecated = true;
    return e;
  });
}

const SIZE_NOTE = "Lengths, percentages, `auto`, `none` and `calc()` are supported.";
const SIZING_KEYWORDS = [
  v("min-content", NO, "Ignored without an advisory; use flex or grid sizing."),
  v("max-content", NO, "Ignored without an advisory; use flex or grid sizing."),
  v("fit-content", NO, "Ignored without an advisory; use flex or grid sizing."),
  v("stretch", NO, "Ignored without an advisory; use flex or grid sizing."),
];
const SIZING_VALUES = [
  ...SIZING_KEYWORDS,
  v(
    "calc-size()",
    OK,
    "Sizes the box from its `auto`, `min-content`, `max-content`, `fit-content` or `content` size; see `calc-size()`.",
    "0.4.0",
  ),
];
const SIZING = ["width", "height", "min-width", "min-height", "max-width", "max-height"];
const LOGICAL_SIZING = [
  "inline-size",
  "block-size",
  "min-inline-size",
  "max-inline-size",
  "min-block-size",
  "max-block-size",
];
const FLOW_RELATIVE = "Mapped left-to-right, top-to-bottom.";
const INLINE_DISPLAY_NOTE =
  "No line boxes: a run of inline content flows as a wrapping row, and an inline box among blocks takes its own line.";
const MASK_LIMITS = [
  "SVG and raster `url()` images and gradients work.",
  "`url(#id)` references are not applied.",
  "A `position: fixed` descendant is not masked.",
];
const MASKS = [
  "mask",
  "mask-image",
  "mask-position",
  "mask-repeat",
  "mask-size",
  "mask-mode",
  "mask-clip",
  "mask-origin",
  "mask-composite",
];
const GRADIENT_NOTE = "Color hints and two-position color stops are supported.";
const GRADIENT_LIMIT = "An `in <colorspace>` interpolation is not supported.";
const GRADIENTS = [
  "function-linear-gradient",
  "function-radial-gradient",
  "function-conic-gradient",
  "function-repeating-linear-gradient",
  "function-repeating-radial-gradient",
  "function-repeating-conic-gradient",
];
const COLOR_FUNCTIONS = [
  "function-rgb",
  "function-hsl",
  "function-hwb",
  "function-lab",
  "function-lch",
  "function-oklab",
  "function-oklch",
  "function-color",
  "function-color-mix",
];
const OUTLINES = ["outline", "outline-width", "outline-color", "outline-style", "outline-offset"];
const TRANSITIONS = [
  "transition",
  "transition-property",
  "transition-duration",
  "transition-timing-function",
  "transition-delay",
  "transition-behavior",
];
const ANIMATIONS = [
  "animation",
  "animation-name",
  "animation-duration",
  "animation-timing-function",
  "animation-delay",
  "animation-iteration-count",
  "animation-direction",
  "animation-fill-mode",
  "animation-play-state",
];
const SVG_FILTER =
  "Works on a shape inside SVG through its own style or filter attribute. For an app box, use `--shading`.";
const STATE_LIMIT = "Not supported inside `:not()`, `:is()`, `:where()` or `:has()`.";
const STATES = ["pseudo-hover", "pseudo-active", "pseudo-focus", "pseudo-focus-visible", "pseudo-focus-within"];
const FORM_STATE_ALT = "Key the style on an attribute or a class.";
const SHADOW_DOM_LIMIT = "There is no shadow DOM, so it matches nothing.";
const SVG_STYLE_ONLY =
  "The SVG renderer reads this property from the element's inline style or a stylesheet inside the SVG. An app stylesheet cannot set it.";
const CONST_MATH = "Supported with constant arguments.";
const MEDIA_SIZE_LIMIT =
  "Only absolute lengths (`px`, `in`, `cm`, `mm`, `Q`, `pt`, `pc`) work; an `em` length drops the whole `@media` block.";
const VIEWPORT_NOTE =
  "The window is the viewport, so the small (`s`), large (`l`) and dynamic (`d`) variants are equal.";
const GAP_NORMAL_LIMIT = "`normal` changes nothing, so it cannot undo an earlier gap; write `0`.";
const SVG_STATE_LIMIT =
  "Not applied from a `:hover`, `:focus`, `:active` or ancestor-state rule, with an advisory; toggle a class instead.";
const ROOT_HALF_EM = (unit: string) =>
  `Not measured from the root element's font: \`1${unit}\` is always half of \`1rem\`.`;
const ABSOLUTE_UNIT = {
  notes: [
    "Viewport size queries accept physical lengths. Container size queries accept them from 0.4. A constant expression such as `calc(1in + 1px)` also resolves to pixels.",
  ],
  limitations: ["A bare physical length in a declaration such as `width: 1in` leaves the previous width in effect."],
  workaround: "Use CSS pixels for a size declaration. One CSS inch equals 96 CSS pixels.",
  related: ["unit-px", "media-width", "at-container", "function-calc"],
} satisfies Extra;

const stylesheets = section("Stylesheets", [
  feat(
    "feature-style-blocks",
    "`<style>` blocks and global stylesheets",
    OK,
    "Styles written in a `<style>` block of a Vue single-file component or a Svelte component, or in a global `.css` file.",
    {
      aliases: ["<style>", "global css", "vue sfc style", "svelte style"],
      related: ["feature-scoped-styles"],
    },
  ),
  feat(
    "feature-scoped-styles",
    "Scoped styles (`<style scoped>`)",
    OK,
    "Styles that apply only to the component that declares them: `<style scoped>` in Vue, component styles in Svelte.",
    {
      notes: ["`:deep()`, `:slotted()` and `:global()` work as the framework compiles them."],
      aliases: ["<style scoped>", ":deep()", ":slotted()", ":global()", "component-scoped styles"],
      related: ["feature-style-blocks", "at-scope"],
    },
  ),
  feat(
    "feature-v-bind-in-css",
    "`v-bind()` in `<style>`",
    OK,
    "Binds a CSS value in a Vue `<style>` block to component state.",
    {
      aliases: ["v-bind", "vue css variables"],
    },
  ),
  feat(
    "feature-inline-style",
    "Inline styles and `element.style`",
    OK,
    "Declarations in an element's `style` attribute or set from script through `element.style`.",
    {
      aliases: ["style attribute", "element.style", "inline style"],
    },
  ),
  feat(
    "feature-cascade",
    "Cascade: `!important`, specificity, source order",
    OK,
    "The rules that decide which declaration wins when several apply to the same element.",
    {
      aliases: ["!important", "specificity", "source order", "cascade"],
    },
  ),
  feat(
    "feature-css-nesting",
    "CSS nesting",
    PLAN,
    "Writes a rule inside another rule (`.a { .b { } }`, `&`) so the inner selector is relative to the outer one.",
    {
      limitations: ["Nested rules are dropped without an advisory."],
      workaround: "Write the rules flat.",
      aliases: ["nesting", "&", "nesting selector"],
    },
  ),
  feat(
    "feature-css-modules",
    "CSS modules (`<style module>`)",
    PLAN,
    "Exposes a Vue component's class names as locally scoped, generated names through `$style` or `useCssModule()`.",
    {
      limitations: ["`useCssModule()` returns `{}`."],
      aliases: ["<style module>", "useCssModule", "css modules", "$style"],
    },
  ),
  feat(
    "feature-preprocessors",
    'Preprocessors (`<style lang="scss">`, `less`, `stylus`)',
    NO,
    "Style languages that compile to CSS, selected with the `lang` attribute of a `<style>` block.",
    {
      limitations: ["The content is not preprocessed: it is compiled as plain CSS."],
      workaround: "Compile it to CSS yourself beforehand.",
      aliases: ["scss", "sass", "less", "stylus", 'lang="scss"'],
    },
  ),
  feat(
    "feature-tailwind-3",
    "Tailwind CSS 3.4",
    OK,
    "A utility-first CSS framework that generates the classes an app uses from a `tailwind.config.*` file.",
    {
      notes: [
        "`sk dev` and `sk build` run it when the app has a `tailwind.config.*` file, with the app's own `tailwindcss` 3: `bun add -d tailwindcss@3`.",
      ],
      aliases: ["tailwind", "tailwindcss"],
      related: ["feature-tailwind-4"],
    },
  ),
  feat(
    "feature-tailwind-4",
    "Tailwind CSS 4",
    PLAN,
    "The CSS-first version of the Tailwind utility framework, configured from the stylesheet.",
    {
      limitations: [
        "The built-in Tailwind compiler requires version 3. Cascade layers emitted by version 4 are dropped by the renderer.",
      ],
      workaround: "Use Tailwind CSS 3.4.",
      aliases: ["tailwind", "tailwindcss", "tailwind v4"],
      related: ["feature-tailwind-3", "at-layer", "at-property"],
    },
  ),
]);

const boxModel = section("Box model and sizing", [
  prop("width", OK, "Sets the width of an element's box.", {
    notes: [SIZE_NOTE],
    values: SIZING_VALUES,
    animates: "interpolated",
    related: SIZING.filter((s) => s !== "width"),
  }),
  prop("height", OK, "Sets the height of an element's box.", {
    notes: [SIZE_NOTE],
    values: SIZING_VALUES,
    animates: "interpolated",
    related: SIZING.filter((s) => s !== "height"),
  }),
  prop("min-width", OK, "Sets the smallest width an element's box can shrink to.", {
    notes: [SIZE_NOTE],
    values: SIZING_VALUES,
    animates: "interpolated",
    related: ["width", "max-width"],
  }),
  prop("min-height", OK, "Sets the smallest height an element's box can shrink to.", {
    notes: [SIZE_NOTE],
    values: SIZING_VALUES,
    animates: "interpolated",
    related: ["height", "max-height"],
  }),
  prop("max-width", OK, "Sets the largest width an element's box can grow to.", {
    notes: [SIZE_NOTE],
    values: SIZING_VALUES,
    animates: "interpolated",
    related: ["width", "min-width"],
  }),
  prop("max-height", OK, "Sets the largest height an element's box can grow to.", {
    notes: [SIZE_NOTE],
    values: SIZING_VALUES,
    animates: "interpolated",
    related: ["height", "min-height"],
  }),
  prop("box-sizing", PART, "Sets whether an element's declared width and height include its padding and border.", {
    notes: ["Every box is `border-box`."],
    values: [v("border-box", OK), v("content-box", NO, "Padding and border stay inside the declared size.")],
    aliases: ["-webkit-box-sizing"],
  }),
  prop("aspect-ratio", OK, "Sets a preferred width-to-height ratio for an element's box.", {
    values: [
      v("<ratio>", OK, "A number or a pair such as `16 / 9` sets the preferred ratio when a dimension is automatic."),
      v("auto", OK, "Undoes an earlier ratio; a replaced element keeps its natural ratio.", "0.4.0"),
      v(
        "auto <ratio>",
        OK,
        "`<img>`, `<video>`, `<canvas>` and Lottie stickers keep their natural ratio; any other box takes the ratio.",
        "0.4.0",
      ),
    ],
    notes: [
      "A `min-*` or `max-*` size on one axis is carried through the ratio to the other, as in browsers.",
      "An `<img>` with a relative width, such as `width: 100%`, takes its height from the ratio (since 0.4.0).",
    ],
    animates: "interpolated",
    related: ["width", "height", "at-container", "media-aspect-ratio"],
  }),
  ...LOGICAL_SIZING.map((name) =>
    prop(
      name,
      OK,
      `The flow-relative counterpart of \`${name.replace("inline-size", "width").replace("block-size", "height")}\`: sets the box's size along the ${name.includes("inline") ? "inline" : "block"} axis.`,
      {
        since: "0.4.0",
        notes: [`${FLOW_RELATIVE} ${SIZE_NOTE}`],
        values: SIZING_VALUES.map(({ since: _, ...value }) => value),
        related: LOGICAL_SIZING.filter((s) => s !== name),
      },
    ),
  ),
  sh(
    "overflow",
    OK,
    "Sets what happens to content that does not fit in an element's box: shown, clipped, or scrolled.",
    {
      notes: [
        "Values: `visible`, `hidden`, `clip`, `auto`, `scroll`.",
        "Animates discretely in `@keyframes` and `element.animate()`.",
      ],
      values: [v("auto", OK, "Overlay scrollbars."), v("scroll", OK, "Overlay scrollbars.")],
      animates: "discrete",
      related: ["overflow-x", "overflow-y", "scrollbar-width", "scrollbar-color"],
    },
  ),
  prop("overflow-x", OK, "Sets what happens to content that overflows an element's left and right edges.", {
    notes: ["Values: `visible`, `hidden`, `clip`, `auto`, `scroll`."],
    related: ["overflow", "overflow-y"],
  }),
  prop("overflow-y", OK, "Sets what happens to content that overflows an element's top and bottom edges.", {
    notes: ["Values: `visible`, `hidden`, `clip`, `auto`, `scroll`."],
    related: ["overflow", "overflow-x"],
  }),
  prop(
    "overflow-clip-margin",
    NO,
    "Sets how far outside its box an element with `overflow: clip` may paint before it is clipped.",
    { related: ["overflow"] },
  ),
  prop("visibility", OK, "Shows or hides an element without changing the layout.", {
    limitations: [
      "`collapse` behaves as `hidden`.",
      "A child with `visibility: visible` inside a `visibility: hidden` box stays hidden and takes no pointer events.",
    ],
  }),
]);

const layout = section("Layout", [
  prop("display", OK, "Sets the kind of box an element generates and how its children are laid out.", {
    values: [
      v("block", OK),
      v("flex", OK),
      v("grid", OK),
      v("none", OK),
      v(
        "flow-root",
        PART,
        "Lays out as `block`, so a child's margins still collapse through it; add `padding` or `overflow: hidden`.",
      ),
      v("inline", PART, INLINE_DISPLAY_NOTE),
      v("inline-block", PART, INLINE_DISPLAY_NOTE),
      v("inline-flex", PART, INLINE_DISPLAY_NOTE),
      v("inline-grid", PART, INLINE_DISPLAY_NOTE),
      v("list-item", PART, "Laid out as a block; markers are drawn on `<li>` only."),
      v("contents", NO, "Laid out as `block` without an advisory."),
      v("table and the other table values", NO, "Laid out as blocks; build tables with grid or flex rows."),
    ],
    aliases: [
      "display: inline",
      "display: contents",
      "display: table",
      "display: list-item",
      "inline-block",
      "flow-root",
    ],
  }),
  sh("flex", OK, "Sets how a flex item grows, shrinks and what size it starts from, in one declaration.", {
    limitations: ["`flex-basis: content` and `flex-basis: min-content` are not supported."],
    related: ["flex-grow", "flex-shrink", "flex-basis"],
    aliases: ["-webkit-flex"],
  }),
  prop("flex-grow", OK, "Sets how much of the free space in a flex container an item takes.", {
    related: ["flex", "flex-shrink", "flex-basis"],
    aliases: ["-webkit-flex-grow"],
  }),
  prop("flex-shrink", OK, "Sets how much a flex item shrinks when the container is too small.", {
    related: ["flex", "flex-grow", "flex-basis"],
    aliases: ["-webkit-flex-shrink"],
  }),
  prop("flex-basis", OK, "Sets the starting size of a flex item before free space is distributed.", {
    values: [
      v("content", NO),
      v("min-content", NO),
      v("calc-size()", OK, "Starts from the item's measured size, such as `calc-size(content, size * 2)`.", "0.4.0"),
    ],
    related: ["flex", "flex-grow", "flex-shrink"],
    aliases: ["-webkit-flex-basis"],
  }),
  prop("flex-direction", OK, "Sets the direction flex items are placed in: row or column, normal or reversed.", {
    related: ["flex-wrap", "flex-flow"],
    aliases: ["-webkit-flex-direction"],
  }),
  prop("flex-wrap", OK, "Sets whether flex items wrap onto several lines.", {
    related: ["flex-direction", "flex-flow"],
    aliases: ["-webkit-flex-wrap"],
  }),
  sh("flex-flow", OK, "Sets `flex-direction` and `flex-wrap` in one declaration.", {
    since: "0.4.0",
    notes: ["A half left out resets to its initial value: `flex-flow: wrap` is `row wrap`."],
    related: ["flex-direction", "flex-wrap"],
    aliases: ["-webkit-flex-flow"],
  }),
  prop("order", OK, "Sets the order an item is laid out in within a flex or grid container.", {
    notes: ["Works in flex and grid containers."],
    aliases: ["-webkit-order"],
  }),
  prop(
    "align-items",
    OK,
    "Sets the default alignment of the items of a flex or grid container on the cross or block axis.",
    {
      values: [v("normal", NO, "Prints an advisory; `stretch` is already the default.")],
      related: ["align-self", "align-content", "place-items"],
      aliases: ["-webkit-align-items"],
    },
  ),
  prop("align-self", OK, "Overrides `align-items` for one flex or grid item.", {
    limitations: ["`align-self: auto` and `align-self: normal` cannot undo an earlier value."],
    related: ["align-items", "place-self"],
    aliases: ["-webkit-align-self"],
  }),
  prop(
    "align-content",
    OK,
    "Distributes the lines of a flex container or the tracks of a grid along the cross or block axis.",
    {
      values: [
        v("normal", NO, "Prints an advisory and is not applied."),
        v("baseline values", NO, "Print an advisory and are not applied."),
      ],
      related: ["align-items", "place-content"],
      aliases: ["-webkit-align-content"],
    },
  ),
  prop("justify-content", OK, "Distributes flex items or grid tracks along the main or inline axis.", {
    values: [v("left", NO), v("right", NO), v("normal", NO)],
    related: ["justify-items", "justify-self", "place-content"],
    aliases: ["-webkit-justify-content"],
  }),
  prop("justify-items", OK, "Sets the default inline-axis alignment of the items of a grid container.", {
    related: ["justify-self", "place-items"],
  }),
  prop("justify-self", OK, "Overrides `justify-items` for one grid item.", {
    limitations: ["`auto` and `normal` compute to `stretch` instead of taking the container's `justify-items`."],
    related: ["justify-items", "place-self"],
  }),
  sh("place-items", OK, "Sets `align-items` and `justify-items` in one declaration.", {
    related: ["align-items", "justify-items"],
  }),
  sh("place-content", PLAN, "Sets `align-content` and `justify-content` in one declaration.", {
    workaround: "Use the `align-*` / `justify-*` longhands.",
    related: ["align-content", "justify-content"],
  }),
  sh("place-self", PLAN, "Sets `align-self` and `justify-self` in one declaration.", {
    workaround: "Use the `align-*` / `justify-*` longhands.",
    related: ["align-self", "justify-self"],
  }),
  sh("gap", OK, "Sets the gaps between rows and columns in flex and grid layouts.", {
    limitations: [GAP_NORMAL_LIMIT],
    related: ["row-gap", "column-gap"],
    aliases: ["grid-gap"],
  }),
  prop("row-gap", OK, "Sets the gap between rows in flex and grid layouts.", {
    limitations: [GAP_NORMAL_LIMIT],
    animates: "interpolated",
    related: ["gap", "column-gap"],
    aliases: ["grid-row-gap"],
  }),
  prop("column-gap", OK, "Sets the gap between columns in flex and grid layouts.", {
    limitations: [GAP_NORMAL_LIMIT],
    animates: "interpolated",
    related: ["gap", "row-gap"],
    aliases: ["grid-column-gap"],
  }),
  ...(["columns", "rows"] as const).map((axis) =>
    prop(`grid-template-${axis}`, PART, `Defines the sizes of a grid's ${axis}.`, {
      notes: [
        "Lengths, `%`, `fr`, `auto`, `min-content`, `max-content`, `minmax()` and `repeat()` including `auto-fill` / `auto-fit` are supported.",
      ],
      values: [
        v("fit-content()", NO),
        v("named lines", PART, "Track sizes are applied. The line names are ignored, so place items by line number."),
        v("subgrid", NO),
        v("masonry", NO),
      ],
      limitations: ["`none` cannot undo an earlier track list."],
      related: [
        axis === "columns" ? "grid-template-rows" : "grid-template-columns",
        "grid-template",
        "grid-auto-" + axis,
      ],
      aliases: ["fit-content()", "minmax()", "repeat()", "fr", "subgrid", "masonry"],
    }),
  ),
  sh("grid-column", OK, "Places a grid item between column lines.", {
    notes: ["Line numbers and `span` are supported."],
    values: [v("named lines and areas", NO, "Place items by line number.")],
    related: ["grid-row", "grid-area"],
    aliases: ["grid-column-start", "grid-column-end"],
  }),
  sh("grid-row", OK, "Places a grid item between row lines.", {
    notes: ["Line numbers and `span` are supported."],
    values: [v("named lines and areas", NO, "Place items by line number.")],
    related: ["grid-column", "grid-area"],
    aliases: ["grid-row-start", "grid-row-end"],
  }),
  sh("grid-area", OK, "Places a grid item by its row and column lines, or by a named area.", {
    notes: ["Line numbers and `span` are supported."],
    values: [v("named areas", NO, "Place items by line number.")],
    related: ["grid-column", "grid-row", "grid-template-areas"],
  }),
  prop("grid-template-areas", NO, "Names areas of a grid with an ASCII-art template that items can be placed into.", {
    workaround: "Place items by line number.",
    related: ["grid-area"],
  }),
  sh("grid", NO, "Sets every explicit and implicit grid property in one declaration.", {
    workaround: "Use the longhands.",
    related: ["grid-template", "grid-template-columns", "grid-template-rows", "grid-auto-flow"],
  }),
  sh(
    "grid-template",
    NO,
    "Sets `grid-template-rows`, `grid-template-columns` and `grid-template-areas` in one declaration.",
    {
      workaround: "Use the longhands.",
      related: ["grid", "grid-template-columns", "grid-template-rows", "grid-template-areas"],
    },
  ),
  prop(
    "grid-auto-flow",
    OK,
    "Sets how grid items without an explicit position are placed, including the `dense` packing.",
    { notes: ["`dense` is supported."], related: ["grid-auto-rows", "grid-auto-columns"] },
  ),
  prop("grid-auto-rows", OK, "Sets the size of grid rows that are created implicitly.", {
    related: ["grid-auto-columns", "grid-auto-flow"],
  }),
  prop("grid-auto-columns", OK, "Sets the size of grid columns that are created implicitly.", {
    related: ["grid-auto-rows", "grid-auto-flow"],
  }),
  prop("position", OK, "Sets how an element is positioned: in the flow, offset, or taken out of the flow.", {
    values: [
      v("static", OK),
      v("relative", OK, "With `z-index: auto` or `0`, the box paints above in-flow siblings (since 0.6)."),
      v("absolute", OK),
      v("fixed", PART, 'Placed against the parent: exact at the app root or in a `<Teleport to="body">`.'),
      v(
        "sticky",
        OK,
        "Held at `top`, `right`, `bottom` and `left` from the nearest scroll container's edges, without leaving its parent.",
        "0.3.0",
      ),
    ],
    notes: [
      "A `sticky` box's offset is applied when the frame is painted, from that frame's scroll position: no scroll listener or re-render is involved.",
      "`getBoundingClientRect()`, `offsetTop`, `elementFromPoint()` and `IntersectionObserver` see a stuck box where it is painted.",
    ],
    limitations: [
      "Under an ancestor with a `transform` that sits between a `sticky` box and its scroll container, the box is held against the untransformed edges.",
    ],
    related: ["top", "right", "bottom", "left", "inset", "z-index"],
    aliases: ["position: fixed", "position: sticky", "position: absolute"],
  }),
  prop("top", OK, "Sets the offset of a positioned element from the top.", {
    notes: ["Including `calc()` with a percentage."],
    related: ["inset", "bottom", "position"],
  }),
  prop("right", OK, "Sets the offset of a positioned element from the right.", {
    notes: ["Including `calc()` with a percentage."],
    related: ["inset", "left", "position"],
  }),
  prop("bottom", OK, "Sets the offset of a positioned element from the bottom.", {
    notes: ["Including `calc()` with a percentage."],
    related: ["inset", "top", "position"],
  }),
  prop("left", OK, "Sets the offset of a positioned element from the left.", {
    notes: ["Including `calc()` with a percentage."],
    related: ["inset", "right", "position"],
  }),
  sh("inset", OK, "Sets `top`, `right`, `bottom` and `left` in one declaration.", {
    notes: ["Including `calc()` with a percentage."],
    related: ["top", "right", "bottom", "left", "inset-inline", "inset-block"],
  }),
  prop("z-index", OK, "Sets the stacking order of positioned elements and flex or grid items.", {
    related: ["position"],
  }),
  prop("float", NO, "Takes an element out of the flow and lets text and inline content wrap around it.", {
    workaround: "Use flex or grid.",
    related: ["clear"],
  }),
  prop("clear", NO, "Moves an element below the floats that precede it.", {
    workaround: "Use flex or grid.",
    related: ["float"],
  }),
  sh("columns", NO, "Sets the number and width of the columns of a multi-column layout.", {
    workaround: "Use grid.",
    related: ["column-count", "column-width", "column-rule"],
    aliases: ["-webkit-columns", "multi-column"],
  }),
  prop("column-count", NO, "Sets the number of columns of a multi-column layout.", {
    workaround: "Use grid.",
    related: ["columns"],
    aliases: ["-webkit-column-count"],
  }),
  prop("column-width", NO, "Sets the ideal column width of a multi-column layout.", {
    workaround: "Use grid.",
    related: ["columns"],
    aliases: ["-webkit-column-width"],
  }),
  sh("column-rule", NO, "Draws a line between the columns of a multi-column layout.", {
    workaround: "Use grid.",
    related: ["columns"],
    aliases: ["column-rule-width", "column-rule-style", "column-rule-color", "-webkit-column-rule"],
  }),
  prop("break-before", NO, "Sets whether a page, column or region break happens before an element.", {
    related: ["break-after", "break-inside"],
    aliases: ["page-break-before"],
  }),
  prop("break-after", NO, "Sets whether a page, column or region break happens after an element.", {
    related: ["break-before", "break-inside"],
    aliases: ["page-break-after"],
  }),
  prop("break-inside", NO, "Sets whether a page, column or region break may happen inside an element.", {
    related: ["break-before", "break-after"],
    aliases: ["page-break-inside"],
  }),
  prop("border-collapse", NO, "Sets whether the borders of table cells are shared or separate.", {
    related: ["border-spacing", "table-layout"],
  }),
  prop("border-spacing", NO, "Sets the distance between the borders of adjacent table cells.", {
    related: ["border-collapse", "table-layout"],
  }),
  prop("table-layout", NO, "Sets the algorithm used to size a table's cells, rows and columns.", {
    related: ["border-collapse", "border-spacing"],
  }),
  prop("anchor-name", NO, "Names an element as an anchor that other positioned elements can be placed against.", {
    workaround: "Position from script with `getBoundingClientRect()`.",
    related: ["position-anchor", "function-anchor"],
    aliases: ["anchor positioning"],
  }),
  prop("position-anchor", NO, "Sets the default anchor element of a positioned element.", {
    workaround: "Position from script with `getBoundingClientRect()`.",
    related: ["anchor-name", "function-anchor"],
    aliases: ["anchor positioning"],
  }),
  fn(
    "anchor()",
    NO,
    "Returns a position on an anchor element's edges, for the inset properties of a positioned element.",
    {
      workaround: "Position from script with `getBoundingClientRect()`.",
      related: ["anchor-name", "position-anchor", "function-anchor-size"],
      aliases: ["anchor positioning"],
    },
  ),
]);

const spacing = section("Spacing and logical properties", [
  sh("margin", OK, "Sets the space outside an element's border, on all four sides or each side.", {
    notes: ["Negative margins and `auto` margins are supported."],
    animates: "interpolated",
    related: ["margin-inline", "margin-block", "padding"],
    aliases: ["margin-top", "margin-right", "margin-bottom", "margin-left"],
  }),
  sh("padding", OK, "Sets the space between an element's content and its border, on all four sides or each side.", {
    animates: "interpolated",
    related: ["padding-inline", "padding-block", "margin"],
    aliases: ["padding-top", "padding-right", "padding-bottom", "padding-left"],
  }),
  sh("margin-inline", OK, "Sets the margins at the start and end of the inline axis.", {
    notes: [FLOW_RELATIVE],
    related: ["margin", "margin-block"],
    aliases: ["margin-inline-start", "margin-inline-end"],
  }),
  sh("margin-block", OK, "Sets the margins at the start and end of the block axis.", {
    notes: [FLOW_RELATIVE],
    related: ["margin", "margin-inline"],
    aliases: ["margin-block-start", "margin-block-end"],
  }),
  sh("padding-inline", OK, "Sets the padding at the start and end of the inline axis.", {
    notes: [FLOW_RELATIVE],
    related: ["padding", "padding-block"],
    aliases: ["padding-inline-start", "padding-inline-end"],
  }),
  sh("padding-block", OK, "Sets the padding at the start and end of the block axis.", {
    notes: [FLOW_RELATIVE],
    related: ["padding", "padding-inline"],
    aliases: ["padding-block-start", "padding-block-end"],
  }),
  sh("inset-inline", OK, "Sets the offsets of a positioned element at the start and end of the inline axis.", {
    notes: [FLOW_RELATIVE],
    related: ["inset", "inset-block"],
    aliases: ["inset-inline-start", "inset-inline-end"],
  }),
  sh("inset-block", OK, "Sets the offsets of a positioned element at the start and end of the block axis.", {
    notes: [FLOW_RELATIVE],
    related: ["inset", "inset-inline"],
    aliases: ["inset-block-start", "inset-block-end"],
  }),
  sh("border-inline", PLAN, "Sets the borders at the start and end of the inline axis.", {
    workaround: "Use the physical sides.",
    related: ["border", "border-block"],
    aliases: [
      "border-inline-start",
      "border-inline-end",
      "border-inline-width",
      "border-inline-style",
      "border-inline-color",
    ],
  }),
  sh("border-block", PLAN, "Sets the borders at the start and end of the block axis.", {
    workaround: "Use the physical sides.",
    related: ["border", "border-inline"],
    aliases: [
      "border-block-start",
      "border-block-end",
      "border-block-width",
      "border-block-style",
      "border-block-color",
    ],
  }),
  prop("border-start-start-radius", PLAN, "Rounds the corner where the block-start and inline-start edges meet.", {
    workaround: "Use the physical sides.",
    related: ["border-radius"],
  }),
]);

const typography = section("Typography and text", [
  prop("font-family", OK, "Sets a prioritized list of font families for an element's text.", {
    notes: [
      "Installed families, `@font-face` families, `serif`, `sans-serif`, `monospace` and `system-ui` are supported.",
    ],
    values: [
      v("ui-monospace", PART, "Not resolved on its own; list a fallback after it."),
      v("ui-sans-serif", PART, "Not resolved on its own; list a fallback after it."),
      v("emoji", PART, "Not resolved on its own; list a fallback after it."),
      v("other newer generic families", PART, "Not resolved on their own; list a fallback after them."),
    ],
    related: ["at-font-face", "font"],
  }),
  prop("font-size", OK, "Sets the size of an element's text.", {
    notes: ["Lengths, `%`, keywords, `smaller` / `larger` and `calc()` are supported."],
    animates: "interpolated",
    related: ["font"],
  }),
  prop("font-weight", OK, "Sets how bold an element's text is.", {
    limitations: ["`lighter` gives 100."],
    related: ["font", "font-variation-settings"],
  }),
  prop("font-style", OK, "Selects a normal, italic or oblique face of the font.", {
    limitations: ["The angle of `oblique <angle>` is ignored."],
    related: ["font"],
  }),
  sh("font", PART, "Sets the font's style, weight, size, line height and family in one declaration.", {
    notes: ["A stylesheet inside an SVG can set `font` on SVG text."],
    workaround: "In app stylesheets, set the font longhands.",
    related: ["font-family", "font-size", "font-weight", "font-style", "line-height"],
  }),
  prop("font-stretch", PART, "Selects a condensed or expanded face of the font.", {
    notes: ["On SVG text, `font-stretch: condensed` selects a condensed face when the family has one.", SVG_STYLE_ONLY],
    related: ["font-width"],
  }),
  prop("font-width", NO, "Selects a condensed or expanded face of the font; the newer name of `font-stretch`.", {
    related: ["font-stretch"],
  }),
  sh(
    "font-variant",
    PART,
    "Selects alternate glyphs such as small capitals, numeral styles and ligatures, in one declaration.",
    {
      notes: ["SVG text supports `small-caps` when the font provides small capitals.", SVG_STYLE_ONLY],
      workaround: 'In app stylesheets, use `font-feature-settings: "smcp"` where the font has it.',
      related: ["font-variant-caps", "font-variant-numeric", "font-variant-ligatures", "font-feature-settings"],
    },
  ),
  prop("font-variant-caps", NO, "Selects small-capital and other capital glyphs.", {
    workaround: 'Use `font-feature-settings: "smcp"` where the font has it.',
    related: ["font-variant", "font-feature-settings"],
  }),
  prop("font-variant-numeric", OK, "Selects numeral glyphs such as tabular, old-style or slashed-zero figures.", {
    related: ["font-variant", "font-feature-settings"],
  }),
  prop("font-variant-ligatures", OK, "Turns ligatures and contextual forms on or off.", {
    related: ["font-variant", "font-feature-settings"],
  }),
  prop("font-kerning", OK, "Turns the font's kerning on or off.", { related: ["font-feature-settings"] }),
  prop("font-feature-settings", OK, "Turns OpenType features of the font on or off by tag.", {
    aliases: ["-webkit-font-feature-settings", "opentype features"],
    related: ["font-variant-numeric", "font-variant-ligatures"],
  }),
  prop(
    "font-variation-settings",
    PART,
    "Accepts `normal`. Set the weight and style of a variable font with their longhands.",
    {
      values: [v("normal", OK, "Uses the font's usual axis selection."), v("axis tag and value pairs", NO)],
      notes: ["`font-weight` and `font-style` reach the matching axes, and `opsz` follows the font size."],
      related: ["font-weight", "font-style", "font-optical-sizing"],
    },
  ),
  prop("font-optical-sizing", PART, "Sets whether a variable font's optical-size axis follows the font size.", {
    limitations: ["Always `auto`."],
    related: ["font-variation-settings"],
  }),
  sh(
    "font-synthesis",
    NO,
    "Sets whether the browser may synthesize bold, italic or small-capital faces the font lacks.",
  ),
  prop("font-size-adjust", NO, "Scales fallback fonts so their x-height matches the first choice."),
  prop(
    "font-variant-east-asian",
    NO,
    "Selects East Asian glyph variants such as simplified, traditional or ruby forms.",
  ),
  prop("line-height", OK, "Sets the height of a line of text.", { related: ["font"] }),
  prop("letter-spacing", OK, "Sets extra space between characters.", {
    limitations: ["A percentage is not supported."],
    related: ["word-spacing"],
  }),
  prop("word-spacing", OK, "Sets extra space between words.", {
    limitations: ["A percentage is not supported."],
    related: ["letter-spacing"],
  }),
  prop("color", OK, "Sets the color of an element's text and the value of `currentColor`.", {
    animates: "interpolated",
    related: ["keyword-currentcolor"],
  }),
  prop("text-align", OK, "Sets the horizontal alignment of the lines of text in a block.", {
    values: [v("left", OK), v("right", OK), v("center", OK), v("start", OK), v("end", OK), v("justify", NO)],
    related: ["text-align-last", "text-justify"],
  }),
  prop("text-align-last", NO, "Sets the alignment of the last line of a block.", { related: ["text-align"] }),
  prop("text-justify", NO, "Sets how justified text distributes its spacing.", { related: ["text-align"] }),
  prop("text-indent", PART, "Accepts a zero indent. The first line starts at the block's edge.", {
    values: [v("0", OK, "Text starts at the edge of the block."), v("nonzero indent", NO)],
    workaround: "`padding-left` indents the whole block.",
  }),
  prop("text-transform", OK, "Changes the case of text: uppercase, lowercase or capitalized."),
  sh(
    "text-decoration",
    PART,
    "Draws lines on text, setting their kind, style, color and thickness in one declaration.",
    {
      limitations: [
        "Underline and line-through are drawn; overline is not.",
        "A style such as `wavy` or a thickness written in the shorthand is ignored without an advisory; set `text-decoration-style` and `text-decoration-thickness` as longhands.",
      ],
      notes: ["Use `text-decoration-line: none` to undo a decoration."],
      related: ["text-decoration-line", "text-decoration-style", "text-decoration-color", "text-decoration-thickness"],
    },
  ),
  prop("text-decoration-line", PART, "Sets which lines are drawn on text: underline, overline or line-through.", {
    values: [
      v("underline", OK),
      v("line-through", OK),
      v("none", OK, "Undoes an earlier decoration."),
      v("overline", NO),
    ],
    limitations: ["Underline and line-through are drawn; overline is not."],
    related: ["text-decoration"],
  }),
  prop(
    "text-decoration-style",
    PART,
    "Sets the style of text decoration lines: solid, double, dotted, dashed or wavy.",
    {
      values: [
        v("solid", OK),
        v("wavy", OK),
        v("dotted", PART, "Drawn solid."),
        v("dashed", PART, "Drawn solid."),
        v("double", PART, "Drawn solid."),
      ],
      related: ["text-decoration"],
    },
  ),
  prop("text-decoration-color", OK, "Sets the color of text decoration lines.", {
    animates: "interpolated",
    related: ["text-decoration"],
  }),
  prop("text-decoration-thickness", OK, "Sets the thickness of text decoration lines.", {
    related: ["text-decoration", "text-underline-offset"],
  }),
  prop("text-underline-offset", OK, "Sets how far an underline sits from its text.", {
    related: ["text-decoration", "text-underline-position"],
  }),
  prop("text-decoration-skip-ink", PART, "Accepts `none`. Underlines run continuously across glyph ink.", {
    values: [v("none", OK), v("auto", NO)],
    limitations: ["The underline is one unbroken line."],
    related: ["text-decoration"],
  }),
  prop("text-underline-position", NO, "Sets where an underline is placed relative to its text.", {
    limitations: ["The underline is one unbroken line."],
    related: ["text-underline-offset"],
  }),
  prop("text-shadow", OK, "Adds shadows to text.", {
    notes: ["Several shadows, with blur, are supported."],
    limitations: ["A list of more than four shadows snaps instead of animating."],
    animates: "interpolated",
  }),
  sh("text-emphasis", NO, "Draws emphasis marks above or below characters."),
  prop("white-space", OK, "Sets how white space and line breaks in text are collapsed and wrapped.", {
    notes: ["All values are supported."],
    values: [v("break-spaces", PART, "Behaves as `pre-wrap`.")],
    related: ["white-space-collapse", "text-wrap"],
  }),
  prop("white-space-collapse", NO, "Sets how white space in text is collapsed.", {
    workaround: "Use `white-space: nowrap` for no wrapping.",
    related: ["white-space"],
  }),
  sh("text-wrap", NO, "Sets whether and how text wraps.", {
    workaround: "Use `white-space: nowrap` for no wrapping.",
    related: ["text-wrap-mode", "text-wrap-style", "white-space"],
  }),
  prop("text-wrap-mode", NO, "Sets whether text wraps.", {
    workaround: "Use `white-space: nowrap` for no wrapping.",
    related: ["text-wrap", "white-space"],
  }),
  prop("text-wrap-style", NO, "Sets how wrapped text is balanced across lines.", {
    workaround: "Use `white-space: nowrap` for no wrapping.",
    related: ["text-wrap"],
    aliases: ["text-wrap: balance", "text-wrap: pretty"],
  }),
  prop("word-break", OK, "Sets where lines may break inside words.", { related: ["overflow-wrap"] }),
  prop("overflow-wrap", OK, "Sets whether an unbreakable word may break to keep it from overflowing its box.", {
    aliases: ["word-wrap"],
    related: ["word-break"],
  }),
  prop("line-break", NO, "Sets the strictness of line-breaking rules for CJK text."),
  prop("hyphenate-character", NO, "Sets the character shown at the end of a hyphenated line.", {
    related: ["hyphens"],
  }),
  prop("hyphens", PART, "Sets whether words are hyphenated when text wraps.", {
    limitations: ["Breaks only at soft hyphens (`&shy;`): there is no dictionary."],
    aliases: ["-webkit-hyphens"],
  }),
  prop("text-overflow", OK, "Sets how text that overflows its box is signalled, for example with an ellipsis.", {
    values: [v("ellipsis", OK), v("clip", PART, "Cannot undo an earlier ellipsis."), v("a custom string", NO)],
    related: ["line-clamp", "white-space", "overflow"],
  }),
  sh("line-clamp", PART, "Limits a block's content to a number of lines.", {
    limitations: [
      "No ellipsis is drawn unless `text-overflow: ellipsis` is set too.",
      "`none` and `0` print an advisory and cannot undo an earlier clamp; put the clamp on a class and remove the class instead.",
    ],
    aliases: ["-webkit-line-clamp"],
    related: ["text-overflow"],
  }),
  prop("vertical-align", PART, "Accepts `baseline`. Other values cannot move an element up or down.", {
    values: [v("baseline", OK, "Accepted with the default layout."), v("middle, top, bottom and offsets", NO)],
    limitations: ["The renderer has no inline line boxes in which to shift an element."],
    workaround: "Use `align-items` / `align-self` on a flex parent.",
  }),
  prop("tab-size", NO, "Sets the width of a tab character.", { limitations: ["Ignored without an advisory."] }),
  prop("-webkit-text-fill-color", OK, "Sets the fill color of text, separately from `color`.", {
    notes: ["Use it with `background-clip: text` for gradient text."],
    related: ["background-clip", "webkit-text-stroke"],
  }),
  sh("-webkit-text-stroke", NO, "Draws an outline around the glyphs of text.", {
    aliases: ["-webkit-text-stroke-width", "-webkit-text-stroke-color"],
  }),
  prop("direction", NO, "Sets the direction of text and of the inline axis: left-to-right or right-to-left.", {
    limitations: ["Left-to-right, horizontal text only."],
    aliases: ["rtl"],
    related: ["unicode-bidi", "writing-mode"],
  }),
  prop("unicode-bidi", NO, "Sets how bidirectional text in an element is ordered.", {
    limitations: ["Left-to-right, horizontal text only."],
    related: ["direction"],
  }),
  prop("writing-mode", PART, "Sets whether lines of text are laid out horizontally or vertically.", {
    notes: ["SVG text supports vertical writing with `vertical-rl` and `vertical-lr`.", SVG_STYLE_ONLY],
    limitations: ["App layout uses horizontal writing."],
    aliases: ["-webkit-writing-mode", "vertical text"],
    related: ["text-orientation", "direction"],
  }),
  prop("text-orientation", NO, "Sets the orientation of characters in vertical text.", {
    limitations: ["Left-to-right, horizontal text only."],
    related: ["writing-mode"],
  }),
  prop(
    "text-rendering",
    PART,
    "Hints what to optimize for when rendering text: speed, legibility or geometric precision.",
    {
      notes: ["On SVG text, `optimizeSpeed` changes glyph rasterization.", SVG_STYLE_ONLY],
      limitations: ["App text uses the platform's text rendering."],
    },
  ),
  prop("-webkit-font-smoothing", NO, "Sets the antialiasing of text on macOS.", {
    limitations: ["Text uses the platform's text rendering."],
    aliases: ["font-smoothing", "-moz-osx-font-smoothing"],
  }),
  prop("quotes", NO, "Sets the quotation marks used by `open-quote` and `close-quote`."),
  prop("initial-letter", NO, "Sets the size and sink of drop caps and raised initial letters."),
  prop("ruby-position", NO, "Sets where ruby annotations are placed relative to their base text.", {
    aliases: ["ruby-*"],
    related: ["ruby-align"],
  }),
  prop("ruby-align", NO, "Sets how ruby annotations are distributed over their base text.", {
    aliases: ["ruby-*"],
    related: ["ruby-position"],
  }),
]);

const color = section("Color", [
  kw("named-colors", "Named colors", OK, "Keywords such as `red` or `rebeccapurple` that name a color.", {
    related: ["hex-colors", "function-rgb"],
  }),
  kw(
    "hex-colors",
    "Hex colors",
    OK,
    "A color written as hexadecimal red, green, blue and optional alpha digits, such as `#ff8800`.",
    { aliases: ["#rgb", "#rrggbb", "hex"], related: ["function-rgb"] },
  ),
  fn("rgb()", OK, "Defines a color by its red, green and blue channels and an optional alpha.", {
    aliases: ["rgba()"],
    related: COLOR_FUNCTIONS.filter((s) => s !== "function-rgb"),
  }),
  fn("hsl()", OK, "Defines a color by its hue, saturation and lightness and an optional alpha.", {
    aliases: ["hsla()"],
    related: ["function-rgb", "function-hwb"],
  }),
  kw("keyword-transparent", "transparent", OK, "A fully transparent color."),
  kw("keyword-currentcolor", "currentColor", PART, "Uses the element's `color` value as a color in another property.", {
    limitations: ["Not supported inside `box-shadow`."],
    related: ["color", "box-shadow"],
  }),
  ...(
    [
      ["hwb()", "Defines a color by its hue, whiteness and blackness."],
      ["lab()", "Defines a color in the CIE Lab color space."],
      ["lch()", "Defines a color in the CIE LCH color space by lightness, chroma and hue."],
      ["oklab()", "Defines a color in the perceptual Oklab color space."],
      ["oklch()", "Defines a color in the perceptual Oklab color space by lightness, chroma and hue."],
      ["color()", "Defines a color in a named color space such as `display-p3` or `srgb`."],
    ] as const
  ).map(([name, summary]) =>
    fn(name, OK, summary, {
      notes: ["Works in every color property."],
      related: COLOR_FUNCTIONS.filter((s) => s !== `function-${name.slice(0, -2)}`),
    }),
  ),
  fn("color-mix()", OK, "Mixes two colors in a given color space.", { related: ["feature-relative-colors"] }),
  feat(
    "feature-relative-colors",
    "Relative colors",
    OK,
    "Derives a new color from an origin color by adjusting its channels, as in `rgb(from red r g b / 50%)`.",
    {
      notes: ["The origin can be a literal color, `currentColor` or a `var()`."],
      aliases: ["from", "relative color syntax"],
      related: ["function-color-mix", "function-rgb", "function-oklch"],
    },
  ),
  fn("light-dark()", OK, "Returns one of two colors depending on whether a light or dark color scheme is in use.", {
    since: "0.4.0",
    notes: [
      "Resolves per element from its used `color-scheme`: the dark color when the scheme lists `dark` and the window theme is dark (`appWindow.theme`, or the system setting under `system`), or when it lists only `dark`.",
      "Under `color-scheme: normal`, the initial value, it is the light color, as in browsers: declare `color-scheme: light dark` on `:root` to follow the theme.",
      "Updates live when the theme changes. Works in any color, inside `var()`, `color-mix()`, gradients and shadows.",
    ],
    limitations: ["In `@keyframes`, the light color is used."],
    related: ["media-prefers-color-scheme", "color-scheme"],
  }),
  kw(
    "system-colors",
    "System colors",
    NO,
    "Keywords such as `Canvas` or `AccentColor` that resolve to colors of the user's system.",
    { aliases: ["Canvas", "CanvasText", "AccentColor", "ButtonFace"] },
  ),
  prop("color-scheme", OK, "Declares which color schemes, light or dark, an element can be rendered in.", {
    since: "0.4.0",
    values: [
      v("normal", OK, "Light."),
      v("light", OK),
      v("dark", OK),
      v("light dark", OK, "Follows the window theme."),
      v("only", OK, "Accepted; Skathi never forces colors, so it changes nothing."),
    ],
    notes: [
      "Inherited: set it on `:root` for the whole app, or on any element for its subtree.",
      "Drives `light-dark()` and the controls Skathi draws: checkboxes, radio buttons, range inputs, color inputs, the file input's button and the `<select>` popup take dark colors under a dark scheme.",
      "Under `normal`, those controls keep their defaults: the `<select>` popup follows the window theme and the others are light.",
      "`getComputedStyle(el).colorScheme` reads the computed value.",
    ],
    limitations: [
      "Scrollbars and text inputs take their colors from the text color, not from the scheme.",
      "The window's background and title bar follow `appWindow.theme`, not `color-scheme`.",
    ],
    related: ["function-light-dark", "media-prefers-color-scheme"],
  }),
  prop("forced-color-adjust", NO, "Sets whether an element's colors may be overridden in forced-colors mode.", {
    related: ["media-forced-colors"],
  }),
  prop("print-color-adjust", NO, "Sets whether the browser may adjust colors to save ink when printing.", {
    aliases: ["-webkit-print-color-adjust"],
  }),
  prop("dynamic-range-limit", PART, "Limits how bright high dynamic range (HDR) content in an element may be.", {
    since: "0.4.0",
    values: [
      v("standard", OK, "No brighter than SDR white."),
      v("constrained", OK, "Up to twice SDR white, one stop of headroom."),
      v("no-limit", OK, "The display's full headroom; the initial value."),
      v("dynamic-range-limit-mix()", OK),
    ],
    notes: [
      "Parsed, computed and inherited; `getComputedStyle(el).dynamicRangeLimit` reads it, with a mix written as `dynamic-range-limit-mix()`.",
      "An `<sk-surface>` producer reads it from `SurfaceInfo::dynamic_range_limit`, and `SurfaceInfo::content_headroom()` turns it into the brightest value a frame should reach on the surface's display, such as a video player's target peak.",
      "On Windows and macOS, a window on an HDR display shows an `<sk-surface>`'s half-float frames above SDR white, each element's clamped to the headroom its `dynamic-range-limit` allows (new in 0.4.0).",
    ],
    limitations: [
      "It has no effect on `<video>`, images or the rest of the page, which are SDR.",
      "Linux windows draw SDR, so there only a surface's producer acts on it.",
      "Transitions and animations of it snap to the end value.",
    ],
    aliases: ["hdr", "high dynamic range"],
    related: ["function-dynamic-range-limit-mix", "media-dynamic-range"],
  }),
  fn("dynamic-range-limit-mix()", OK, "Mixes `dynamic-range-limit` keywords by percentage.", {
    since: "0.4.0",
    notes: [
      "`dynamic-range-limit-mix(standard 30%, no-limit 70%)` takes two or more limits, each a keyword or another mix, with percentages that are normalized when they do not sum to 100%.",
      "The limits mix in stops of brightness: half `standard` and half `no-limit` on a display with four stops of headroom allows two.",
    ],
    related: ["dynamic-range-limit"],
  }),
]);

const backgrounds = section("Backgrounds and borders", [
  prop("background-color", OK, "Sets the background color of an element.", {
    notes: ["Several layers are supported."],
    animates: "interpolated",
    related: ["background"],
  }),
  sh("background", OK, "Sets every background property, for one or several layers, in one declaration.", {
    notes: ["Several layers are supported."],
    related: [
      "background-color",
      "background-image",
      "background-size",
      "background-repeat",
      "background-position",
      "background-clip",
    ],
  }),
  prop("background-image", OK, "Sets one or more images or gradients as the background of an element.", {
    values: [
      v("url()", OK, "Raster images and SVG."),
      v("gradients", OK),
      v("image-set()", NO, "Write one `url()` or a standard gradient."),
      v("cross-fade()", NO, "Write one `url()` or a standard gradient."),
      v("element()", NO, "Write one `url()` or a standard gradient."),
      v("-webkit-gradient()", NO, "Write one `url()` or a standard gradient."),
    ],
    limitations: ["An animated GIF shows only its first frame."],
    related: ["background", ...GRADIENTS],
  }),
  ...(
    [
      ["linear-gradient()", "Creates an image of colors that transition along a straight line."],
      ["radial-gradient()", "Creates an image of colors that radiate from an origin."],
      ["conic-gradient()", "Creates an image of colors that rotate around a center point."],
      ["repeating-linear-gradient()", "Creates a linear gradient that repeats to fill its area."],
      ["repeating-radial-gradient()", "Creates a radial gradient that repeats to fill its area."],
      ["repeating-conic-gradient()", "Creates a conic gradient that repeats to fill its area."],
    ] as const
  ).map(([name, summary]) =>
    fn(name, OK, summary, {
      notes: [GRADIENT_NOTE],
      limitations: [GRADIENT_LIMIT],
      related: GRADIENTS.filter((s) => s !== `function-${name.slice(0, -2)}`),
      aliases: ["gradient"],
    }),
  ),
  fn("image-set()", NO, "Picks the most suitable image from a set, for example by screen resolution.", {
    workaround: "Write one `url()` or a standard gradient.",
    aliases: ["-webkit-image-set()"],
    related: ["background-image"],
  }),
  fn("cross-fade()", NO, "Blends two or more images at given transparencies.", {
    workaround: "Write one `url()` or a standard gradient.",
    related: ["background-image"],
  }),
  fn("element()", NO, "Uses the rendering of another element as an image.", {
    workaround: "Write one `url()` or a standard gradient.",
    related: ["background-image"],
  }),
  fn("-webkit-gradient()", NO, "WebKit's legacy syntax for linear and radial gradients.", {
    workaround: "Write a standard gradient.",
    related: ["function-linear-gradient", "function-radial-gradient"],
  }),
  prop("background-size", OK, "Sets the size of background images.", {
    aliases: ["-webkit-background-size"],
    related: ["background"],
  }),
  prop("background-repeat", OK, "Sets how background images repeat.", {
    values: [v("space", PART, "Repeats as `repeat`."), v("round", PART, "Repeats as `repeat`.")],
    related: ["background"],
  }),
  ...["background-position", "background-position-x", "background-position-y"].map((name) =>
    (name === "background-position" ? sh : prop)(
      name,
      PART,
      name === "background-position"
        ? "Sets the starting position of background images."
        : `Sets the ${name.endsWith("x") ? "horizontal" : "vertical"} starting position of background images.`,
      {
        limitations: ["An offset from the right or bottom edge (`right 10px`) sits flush against that edge."],
        workaround: "Use `left` / `top` offsets or a percentage.",
        related: ["background-position", "background-position-x", "background-position-y", "background"].filter(
          (s) => s !== name,
        ),
      },
    ),
  ),
  prop(
    "background-clip",
    PART,
    "Sets which box a background extends under: border, padding, content, or the text itself.",
    {
      values: [
        v("text", OK),
        v("border-box", NO, "The background fills the padding box."),
        v("padding-box", NO, "The background fills the padding box."),
        v("content-box", NO, "The background fills the padding box."),
      ],
      limitations: ["Only `text` is supported; with other values the background fills the padding box."],
      aliases: ["-webkit-background-clip", "background-clip: text", "gradient text"],
      related: ["webkit-text-fill-color", "background-origin"],
    },
  ),
  prop("background-origin", NO, "Sets which box background images are positioned from.", {
    limitations: ["The background fills the padding box."],
    aliases: ["-webkit-background-origin"],
    related: ["background-clip"],
  }),
  prop(
    "background-attachment",
    NO,
    "Sets whether a background scrolls with its element or stays fixed to the viewport.",
    { limitations: ["Ignored without an advisory."] },
  ),
  prop("background-blend-mode", NO, "Sets how an element's background layers blend with each other.", {
    related: ["mix-blend-mode"],
  }),
  sh("border", OK, "Sets the width, style and color of an element's border, on all sides or one side.", {
    related: ["border-width", "border-style", "border-color", "border-radius"],
    aliases: ["border-top", "border-right", "border-bottom", "border-left"],
  }),
  sh("border-width", OK, "Sets the width of an element's border.", {
    animates: "interpolated",
    related: ["border"],
    aliases: ["border-top-width", "border-right-width", "border-bottom-width", "border-left-width"],
  }),
  sh("border-color", OK, "Sets the color of an element's border.", {
    animates: "interpolated",
    related: ["border"],
    aliases: ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"],
  }),
  sh("border-style", PART, "Sets the line style of an element's border.", {
    limitations: [
      "All four sides share one line style: in `border-style: dashed solid` every side draws dashed, and across the side longhands the last one declared wins.",
    ],
    values: [
      v("solid", OK),
      v("dashed", OK),
      v("none", OK),
      v("hidden", OK),
      v("dotted", PART, "Drawn solid."),
      v("double", PART, "Drawn solid."),
      v("groove", PART, "Drawn solid."),
      v("ridge", PART, "Drawn solid."),
      v("inset", PART, "Drawn solid."),
      v("outset", PART, "Drawn solid."),
    ],
    related: ["border"],
    aliases: ["border-top-style", "border-right-style", "border-bottom-style", "border-left-style"],
  }),
  sh("border-radius", PART, "Rounds the corners of an element's border box.", {
    notes: ["Lengths and `%` are supported, on the shorthand and the corner longhands."],
    limitations: [
      "An elliptical corner is drawn as a circle of the smaller radius.",
      "Each corner is capped at half the box's shorter side, even when its neighboring corners are `0`.",
    ],
    animates: "interpolated",
    related: ["corner-shape", "border-start-start-radius"],
    aliases: [
      "border-top-left-radius",
      "border-top-right-radius",
      "border-bottom-right-radius",
      "border-bottom-left-radius",
      "-webkit-border-radius",
    ],
  }),
  sh("corner-shape", PART, "Sets the shape of rounded corners, such as bevel, scoop or squircle.", {
    since: "0.4.0",
    notes: [
      "All six keywords and `superellipse()` with a number, `infinity` or `-infinity` are supported, on the shorthand, the corner longhands and the side shorthands.",
      "Shapes interpolate along the curve's half corner, as browsers do.",
      "The `corner` and `corner-top-left`-style shorthands that set radius and shape together are not supported, as in browsers.",
    ],
    limitations: [
      "Corners stay circular: an elliptical radius is drawn as a circle of the smaller radius, capped at half the box's shorter side. The cap also keeps opposite concave corners from overlapping.",
      "A blurred `box-shadow` around a shaped corner is approximated. It is exact for straight edges and round corners.",
      "Between `scoop` and `bevel` (`superellipse()` from -1 to 0), the inner border edge and the overflow clip approximate the curve with a superellipse.",
      "Dashed borders space their dashes along an estimated length of a shaped corner.",
      "`<img>`, `<video>` and other replaced content are clipped with the same shape at the padding box's radius.",
      "A `--shading` effect that draws its element's content clips that content to round corners under a shaped `overflow` ancestor.",
      "The flow-relative longhands are mapped left-to-right, top-to-bottom.",
    ],
    animates: "interpolated",
    related: ["border-radius"],
    aliases: [
      "corner-top-left-shape",
      "corner-top-right-shape",
      "corner-bottom-right-shape",
      "corner-bottom-left-shape",
      "corner-start-start-shape",
      "corner-start-end-shape",
      "corner-end-start-shape",
      "corner-end-end-shape",
      "corner-top-shape",
      "corner-right-shape",
      "corner-bottom-shape",
      "corner-left-shape",
      "corner-block-start-shape",
      "corner-block-end-shape",
      "corner-inline-start-shape",
      "corner-inline-end-shape",
      "superellipse()",
      "squircle",
    ],
  }),
  sh("border-image", NO, "Draws an image or gradient in place of an element's border.", {
    workaround: "For a gradient border, use `background` with `mask` (see Effects), or a wrapper with padding.",
    aliases: [
      "border-image-source",
      "border-image-slice",
      "border-image-width",
      "border-image-outset",
      "border-image-repeat",
      "gradient border",
    ],
    related: ["mask", "border"],
  }),
  ...OUTLINES.map((name) =>
    (name === "outline" ? sh : prop)(
      name,
      OK,
      {
        outline: "Draws a line outside an element's border, setting its width, style and color in one declaration.",
        "outline-width": "Sets the width of an element's outline.",
        "outline-color": "Sets the color of an element's outline.",
        "outline-style": "Sets the line style of an element's outline.",
        "outline-offset": "Sets the space between an element's outline and its border.",
      }[name] ?? "",
      { limitations: ["`dotted` draws dashed."], related: OUTLINES.filter((s) => s !== name) },
    ),
  ),
  prop(
    "box-decoration-break",
    NO,
    "Sets how an element's decorations are drawn where it is split across lines, columns or pages.",
    { aliases: ["-webkit-box-decoration-break"] },
  ),
]);

const effects = section("Effects", [
  prop("box-shadow", PART, "Adds shadows around or inside an element's box.", {
    notes: ["Several shadows, `inset`, spread and `none` are supported."],
    limitations: ["`currentColor` is not supported inside `box-shadow`."],
    aliases: ["-webkit-box-shadow"],
    related: ["text-shadow", "keyword-currentcolor"],
  }),
  prop("opacity", OK, "Sets how transparent an element is, together with its content.", {
    notes: ["A number or a percentage."],
    animates: "interpolated",
  }),
  prop("filter", PART, "Applies graphical effects such as blur, drop shadows or color changes to an element.", {
    values: [
      v("blur()", PART, "On a box with no children and a solid background."),
      v("drop-shadow()", PART, "On a box with an opaque solid background, and on inline `<svg>`."),
      v("url(#id)", OK, "To an SVG `feColorMatrix`: one filter, in the same document."),
      ...[
        "grayscale()",
        "brightness()",
        "contrast()",
        "invert()",
        "sepia()",
        "saturate()",
        "hue-rotate()",
        "opacity()",
      ].map((f) => v(f, PART, SVG_FILTER)),
    ],
    limitations: [
      "App stylesheets support `blur()`, `drop-shadow()` and a local `url(#id)` referencing an SVG `feColorMatrix`. Color filters on SVG shapes must be set inside the SVG.",
    ],
    aliases: [
      "-webkit-filter",
      "blur()",
      "drop-shadow()",
      "grayscale()",
      "brightness()",
      "contrast()",
      "invert()",
      "sepia()",
      "saturate()",
      "hue-rotate()",
    ],
    related: ["backdrop-filter", "shading"],
  }),
  prop("backdrop-filter", PART, "Applies graphical effects such as blur to the area behind an element.", {
    values: [v("blur()", OK)],
    limitations: ["Only `blur()` applies; other functions are dropped."],
    aliases: ["-webkit-backdrop-filter", "frosted glass"],
    related: ["filter"],
  }),
  prop("mix-blend-mode", PART, "Sets how an element's content blends with what is behind it.", {
    values: [
      v("normal", OK),
      v(
        "plus-lighter",
        PART,
        "Adds the colors of the snapshots painted by `::view-transition-old()` and `::view-transition-new()`.",
      ),
      v(
        "multiply",
        PART,
        "A stylesheet inside an SVG can blend overlapping shapes. App stylesheets reject this value.",
      ),
      v("other modes in app stylesheets", NO, "Print an advisory and change nothing."),
    ],
    related: ["background-blend-mode", "isolation", "pseudo-view-transition"],
  }),
  prop("isolation", OK, "Creates a new stacking context, so blending inside it does not reach what is behind it.", {
    related: ["mix-blend-mode"],
  }),
  prop("clip-path", PART, "Clips an element to a shape, hiding what is outside it.", {
    limitations: ["App stylesheets cannot use a `url()` clip reference, `xywh()` or `rect()`."],
    values: [
      v("inset()", OK),
      v("circle()", OK),
      v("ellipse()", OK),
      v("polygon()", OK),
      v("path()", OK),
      v("a box keyword", OK),
      v(
        "url(#id)",
        PART,
        "On an SVG shape, its inline style, presentation attribute or a stylesheet inside the SVG can reference a local `<clipPath>`.",
      ),
      v("xywh()", NO),
      v("rect()", NO),
    ],
    aliases: ["-webkit-clip-path"],
    related: ["clip", "mask"],
  }),
  prop("clip", PART, "Clips an absolutely positioned element to a rectangle; superseded by `clip-path`.", {
    values: [v("rect()", PART, "Only the empty rect of the visually-hidden idiom.")],
    limitations: ["Only the empty `rect()` of the visually-hidden idiom works."],
    related: ["clip-path"],
  }),
  ...(
    [
      ["mask", "Hides parts of an element with images or gradients, setting every mask property in one declaration."],
      ["mask-image", "Sets the images or gradients used as an element's mask layers."],
      ["mask-position", "Sets the position of each mask image."],
      ["mask-repeat", "Sets how each mask image repeats."],
      ["mask-size", "Sets the size of each mask image."],
    ] as const
  ).map(([name, summary]) =>
    (name === "mask" ? sh : prop)(name, PART, summary, {
      limitations: MASK_LIMITS,
      related: [...MASKS.filter((s) => s !== name), "webkit-mask"],
    }),
  ),
  prop("mask-mode", PART, "Sets whether a mask image uses its alpha channel or its luminance.", {
    values: [v("alpha", OK), v("luminance", OK), v("match-source", PART, "Treated as `alpha`.")],
    limitations: ["One mode applies to all the layers."],
    related: ["mask", "mask-type"],
  }),
  prop("mask-clip", PART, "Sets the area a mask applies to.", {
    values: [v("border-box", OK), v("content-box", PART, "Only in the gradient-border ring.")],
    related: ["mask", "mask-origin"],
  }),
  prop("mask-origin", PART, "Sets the box mask images are positioned from.", {
    values: [v("border-box", OK)],
    limitations: ["Only `border-box` is supported."],
    related: ["mask", "mask-clip"],
  }),
  prop("mask-composite", PART, "Sets how mask layers combine with each other.", {
    values: [
      v("add", OK),
      v("exclude", PART, "Only in the gradient-border ring: two layers, the top one clipped."),
      v("subtract", NO, "Prints an advisory; the layers are added."),
      v("intersect", NO, "Prints an advisory; the layers are added."),
    ],
    related: ["mask"],
  }),
  prop("mask-type", PART, "Sets whether an SVG `<mask>` element uses luminance or alpha.", {
    notes: ["Set `mask-type: alpha` on the SVG mask to use opacity, including opaque black.", SVG_STYLE_ONLY],
    related: ["mask-mode"],
  }),
  sh("mask-border", NO, "Uses an image as a mask for an element's border area.", {
    aliases: ["mask-border-source", "mask-border-slice"],
  }),
  prop("-webkit-mask", NO, "The `-webkit-` spellings of the mask properties.", {
    slug: "webkit-mask",
    limitations: ["Dropped with an advisory."],
    workaround: "Use the unprefixed property.",
    aliases: [
      "-webkit-mask-*",
      "-webkit-mask-image",
      "-webkit-mask-size",
      "-webkit-mask-position",
      "-webkit-mask-repeat",
      "-webkit-mask-composite",
    ],
    related: ["mask", "mask-image"],
  }),
  prop("-webkit-box-reflect", NO, "Draws a reflection of an element next to it."),
  prop("--shading", OK, "Applies one of Skathi's GPU shading effects to an element.", {
    skathiOnly: true,
    notes: ["A custom property that browsers ignore.", "It applies to its own element."],
    animates: "interpolated",
    aliases: ["shading", "effects", "gpu effects"],
    related: ["filter"],
  }),
]);

const transforms = section("Transforms", [
  prop("transform", PART, "Moves, rotates, scales or skews an element without changing the layout.", {
    notes: ["2D transforms are supported."],
    values: [
      v("translate()", OK),
      v("rotate()", OK),
      v("scale()", OK),
      v("skew()", OK),
      v("matrix()", OK),
      v("translate3d()", PART, "Applies the X and Y offsets. A nonzero Z offset emits an advisory."),
      v("scale3d()", PART, "Applies the X and Y scale factors. The Z factor has no effect."),
      v("translate() with a calc() mixing % and a length", NO, "Use a length or a percentage."),
      v("perspective()", PLAN),
      v("rotateX()", PLAN),
      v("rotateY()", PLAN),
      v("rotate3d()", PLAN),
      v("translateZ()", PLAN),
      v("matrix3d()", PLAN),
    ],
    animates: "interpolated",
    aliases: ["-webkit-transform", "translate()", "rotate()", "scale()", "skew()", "matrix()", "3d transforms"],
    related: ["transform-origin", "translate", "rotate", "scale", "perspective"],
  }),
  prop("transform-origin", PART, "Sets the point a transform is applied around.", {
    notes: ["Lengths, percentages and keywords are supported."],
    limitations: ["A keyword with an offset and `calc()` are not supported."],
    aliases: ["-webkit-transform-origin"],
    related: ["transform"],
  }),
  prop(
    "perspective",
    PLAN,
    "Sets the distance between the viewer and the z=0 plane, giving 3D-transformed children depth.",
    {
      aliases: ["-webkit-perspective"],
      related: ["perspective-origin", "transform-style", "transform"],
    },
  ),
  prop("transform-style", PLAN, "Sets whether an element's children are positioned in 3D space or flattened.", {
    aliases: ["-webkit-transform-style"],
    related: ["perspective"],
  }),
  prop(
    "backface-visibility",
    PLAN,
    "Sets whether the back face of an element is visible when it is turned away from the viewer.",
    { aliases: ["-webkit-backface-visibility"], related: ["transform-style"] },
  ),
  prop("perspective-origin", PLAN, "Sets the viewer's position for the `perspective` property.", {
    aliases: ["-webkit-perspective-origin"],
    related: ["perspective"],
  }),
  prop("translate", PLAN, "Moves an element, independently of `transform`.", {
    workaround: "Use `transform`.",
    related: ["transform", "rotate", "scale"],
  }),
  prop("rotate", PLAN, "Rotates an element, independently of `transform`.", {
    workaround: "Use `transform`.",
    related: ["transform", "translate", "scale"],
  }),
  prop("scale", PLAN, "Scales an element, independently of `transform`.", {
    workaround: "Use `transform`.",
    related: ["transform", "translate", "rotate"],
  }),
  prop("transform-box", NO, "Sets the box that `transform` and `transform-origin` relate to.", {
    related: ["transform-origin"],
  }),
  prop("offset-path", NO, "Sets a path for an element to follow, for motion along a path.", {
    aliases: ["motion path"],
  }),
]);

const transitions = section("Transitions and animations", [
  sh(
    "transition",
    OK,
    "Animates changes of property values over time, setting every transition property in one declaration.",
    {
      notes: ["Interrupted transitions retarget from the value on screen."],
      values: [
        v("composed with var()", OK, "The shorthand and longhands resolve custom properties on each element.", "0.6.0"),
      ],
      aliases: ["-webkit-transition"],
      related: TRANSITIONS.filter((s) => s !== "transition"),
    },
  ),
  prop("transition-property", OK, "Sets which properties transition when their values change.", {
    aliases: ["-webkit-transition-property"],
    related: ["transition"],
  }),
  prop("transition-duration", OK, "Sets how long a transition takes.", {
    aliases: ["-webkit-transition-duration"],
    related: ["transition"],
  }),
  prop("transition-timing-function", OK, "Sets the easing curve of a transition.", {
    aliases: ["-webkit-transition-timing-function"],
    related: ["transition", "easing-functions"],
  }),
  prop("transition-delay", OK, "Sets how long to wait before a transition starts.", {
    values: [v("negative values", OK, "The transition starts part-way through.", "0.3.0")],
    aliases: ["-webkit-transition-delay"],
    related: ["transition", "animation-delay"],
  }),
  prop("transition-behavior", NO, "Sets whether properties with discrete values, such as `display`, transition.", {
    values: [v("allow-discrete", NO)],
    related: ["transition", "at-starting-style"],
  }),
  prop(
    "interpolate-size",
    OK,
    "Lets sizes animate to and from intrinsic keywords such as `auto`, `min-content` and `fit-content`.",
    {
      since: "0.4.0",
      notes: [
        "Inherited: set `interpolate-size: allow-keywords` on `:root` and `height: auto` transitions, in `@keyframes` and in `element.animate()`, as if it were `calc-size(auto, size)`.",
        "Under `numeric-only`, the initial value, a keyword snaps.",
      ],
      limitations: [
        "Only `auto` interpolates: `min-content`, `max-content` and `fit-content` are not sizes Skathi applies.",
      ],
      related: ["function-calc-size", "transition", "height"],
    },
  ),
  sh(
    "animation",
    OK,
    "Runs `@keyframes` animations on an element, setting every animation property in one declaration.",
    {
      notes: ["Per-property keyframe timelines, every fill mode, direction and play state are supported."],
      aliases: ["-webkit-animation"],
      related: [...ANIMATIONS.filter((s) => s !== "animation"), "at-keyframes"],
    },
  ),
  ...(
    [
      ["animation-name", "Sets the `@keyframes` rules an element runs."],
      ["animation-duration", "Sets how long one cycle of an animation takes."],
      ["animation-timing-function", "Sets the easing curve of an animation."],
      ["animation-iteration-count", "Sets how many times an animation repeats."],
      ["animation-direction", "Sets whether an animation runs forwards, backwards or alternates."],
      ["animation-fill-mode", "Sets which styles an animation applies before it starts and after it ends."],
      ["animation-play-state", "Pauses or resumes an animation."],
    ] as const
  ).map(([name, summary]) =>
    prop(name, OK, summary, {
      aliases: [`-webkit-${name}`],
      related: name === "animation-timing-function" ? ["animation", "easing-functions"] : ["animation"],
    }),
  ),
  prop("animation-delay", OK, "Sets how long to wait before an animation starts.", {
    values: [v("negative values", OK, "The animation starts part-way through.", "0.3.0")],
    aliases: ["-webkit-animation-delay"],
    related: ["animation", "transition-delay"],
  }),
  prop("animation-timeline", NO, "Drives an animation from a scroll or view timeline instead of time.", {
    limitations: ["Animations run on the document timeline."],
    aliases: [
      "scroll-driven animations",
      "scroll timeline",
      "view timeline",
      "scroll()",
      "view()",
      "scroll-timeline",
      "view-timeline",
    ],
    related: ["animation-range"],
  }),
  sh("animation-range", NO, "Sets where on its timeline a scroll-driven animation starts and ends.", {
    limitations: ["Animations run on the document timeline."],
    aliases: ["animation-range-start", "animation-range-end"],
    related: ["animation-timeline"],
  }),
  prop(
    "animation-composition",
    NO,
    "Sets how an animation's values combine with the underlying value and other animations.",
    { related: ["animation"] },
  ),
  feat("easing-functions", "Easing functions", OK, "Curves that set how an animated value progresses over time.", {
    values: [
      v("keywords (ease, linear, ease-in, …)", OK),
      v("cubic-bezier()", OK),
      v("steps()", OK),
      v("linear(…)", NO),
    ],
    aliases: ["cubic-bezier()", "steps()", "linear()", "ease", "ease-in-out", "timing function"],
    related: ["transition-timing-function", "animation-timing-function"],
  }),
  feat(
    "feature-web-animations",
    "Web Animations (`element.animate()`)",
    OK,
    "Runs keyframe animations from script with `element.animate()`.",
    {
      notes: ["Animates the same properties as `@keyframes`."],
      aliases: ["element.animate()", "waapi", "web animations api"],
      related: ["at-keyframes", "animation"],
    },
  ),
  feat(
    "feature-view-transitions",
    "View transitions",
    PART,
    "Animates between two states of a document by snapshotting the old and new views (`document.startViewTransition()`).",
    {
      limitations: ["Same-document view transitions only; `@view-transition` (cross-document) is not supported."],
      aliases: ["startViewTransition", "view-transition"],
      related: ["view-transition-name", "pseudo-view-transition", "at-view-transition"],
    },
  ),
  prop("view-transition-name", PART, "Gives an element its own snapshot group in a view transition.", {
    limitations: ["Same-document view transitions only."],
    related: ["feature-view-transitions", "pseudo-view-transition", "view-transition-class"],
  }),
  prop(
    "view-transition-class",
    PART,
    "Gives an element's view transition group one or more class names, so one `::view-transition-group(*.name)` rule styles them all.",
    {
      limitations: ["Same-document view transitions only."],
      related: ["view-transition-name", "pseudo-view-transition"],
    },
  ),
  pc(":active-view-transition", OK, "Matches the root element while a view transition is running.", {
    related: ["pseudo-active-view-transition-type", "feature-view-transitions"],
  }),
  pc(
    ":active-view-transition-type()",
    OK,
    "Matches the root element while a view transition of one of the given types is running.",
    {
      notes: ["The types come from `document.startViewTransition({ update, types })`."],
      related: ["pseudo-active-view-transition", "feature-view-transitions"],
    },
  ),
  pe(
    "::view-transition",
    PART,
    "The pseudo-element tree a view transition renders: groups, image pairs and old and new snapshots.",
    {
      limitations: ["Same-document view transitions only."],
      aliases: [
        "::view-transition-*",
        "::view-transition-group()",
        "::view-transition-image-pair()",
        "::view-transition-old()",
        "::view-transition-new()",
      ],
      related: ["feature-view-transitions", "view-transition-name"],
    },
  ),
  at("@view-transition", NO, "Opts a document into view transitions on navigation between documents.", {
    related: ["feature-view-transitions"],
  }),
]);

const scrolling = section("Scrolling", [
  prop("scrollbar-width", OK, "Sets the thickness of an element's scrollbars, or hides them.", {
    values: [v("auto", OK), v("thin", OK), v("none", OK)],
    related: ["scrollbar-color", "overflow"],
  }),
  prop("scrollbar-color", OK, "Sets the colors of an element's scrollbar thumb and track.", {
    related: ["scrollbar-width"],
  }),
  prop("scrollbar-gutter", NO, "Reserves space for a scrollbar so the layout does not shift when it appears.", {
    related: ["scrollbar-width"],
  }),
  pe("::-webkit-scrollbar", NO, "Styles a scrollbar and its parts in WebKit and Blink browsers.", {
    workaround: "Use `scrollbar-width` and `scrollbar-color`.",
    aliases: [
      "::-webkit-scrollbar-thumb",
      "::-webkit-scrollbar-track",
      "::-webkit-scrollbar-button",
      "::-webkit-scrollbar-corner",
      "::-webkit-scrollbar-track-piece",
    ],
    related: ["scrollbar-width", "scrollbar-color"],
  }),
  sh(
    "overscroll-behavior",
    OK,
    "Sets what happens when scrolling reaches the edge of a scroll container: chain to the parent, bounce, or stop.",
    {
      values: [v("auto", OK), v("contain", OK), v("none", OK)],
      aliases: [
        "overscroll-behavior-x",
        "overscroll-behavior-y",
        "overscroll-behavior-inline",
        "overscroll-behavior-block",
      ],
    },
  ),
  prop("scroll-behavior", OK, "Sets whether programmatic and navigation scrolling is instant or smooth.", {
    values: [v("smooth", OK)],
  }),
  prop("scroll-snap-type", OK, "Makes a scroll container snap to positions on its children.", {
    since: "0.4.0",
    values: [
      v("x, y, block, inline, both", OK),
      v("mandatory", OK),
      v("proximity", OK, "Snaps within a third of the snapport of a snap position."),
    ],
    notes: [
      "Wheel, touchpad and touch scrolling snap once the gesture and its momentum stop; a notched wheel moves at least to the next snap position.",
      "A scrollbar drag or track click snaps on release.",
      "Arrow keys, Page Up, Page Down and Space scroll the focused element's scroll container to a snap position in that direction; Home and End to the nearest one at the edge.",
      "`scrollTo()`, `scrollBy()`, `scrollIntoView()` and writes to `scrollTop` and `scrollLeft` land on a snap position, gliding there when smooth.",
      "After a layout change, such as a resize or new content, the container goes back to the element it was snapped to.",
    ],
    limitations: [
      "With `both`, each axis picks its snap position on its own, so the two can come from different elements.",
      "The `scrollsnapchange` and `scrollsnapchanging` events are not fired.",
    ],
    related: ["scroll-snap-align", "scroll-snap-stop", "scroll-padding", "scroll-margin"],
  }),
  prop("scroll-snap-align", OK, "Sets the snap position of an element inside a snapping scroll container.", {
    since: "0.4.0",
    values: [v("none, start, end, center", OK)],
    notes: ["The two-value form is block (y) then inline (x)."],
    related: ["scroll-snap-type", "scroll-snap-stop"],
  }),
  prop("scroll-snap-stop", OK, "Sets whether a snapping scroll container may scroll past an element's snap position.", {
    since: "0.4.0",
    values: [v("normal", OK), v("always", OK)],
    notes: [
      "`always` stops wheel, touchpad, arrow-key and paging scrolls; Home, End, scrollbar drags and scripts pass it.",
    ],
    related: ["scroll-snap-type", "scroll-snap-align"],
  }),
  sh("scroll-padding", OK, "Sets the inset of a scroll container's snap and scroll-into-view area.", {
    since: "0.4.0",
    notes: ["Shrinks the area snap positions and `scrollIntoView()` align to. `auto` is 0."],
    aliases: ["scroll-padding-*", "scroll-padding-top", "scroll-padding-inline", "scroll-padding-block"],
    related: ["scroll-margin", "scroll-snap-type"],
  }),
  sh("scroll-margin", OK, "Sets the outset of an element's snap and scroll-into-view area.", {
    since: "0.4.0",
    notes: ["Grows the box snap positions and `scrollIntoView()` align."],
    aliases: ["scroll-margin-*", "scroll-margin-top", "scroll-margin-inline", "scroll-margin-block"],
    related: ["scroll-padding", "scroll-snap-align"],
  }),
]);

const interaction = section("Interaction", [
  prop("cursor", PART, "Sets the mouse cursor shown over an element.", {
    notes: ["Native cursors are used."],
    values: [
      v(
        "help, progress, wait, move, all-scroll",
        PART,
        "The system cursor on Windows and Linux; macOS has no public cursor for them and shows the arrow.",
        "0.4.0",
      ),
      v("cell", OK, "The system `cell` cursor on Linux, the crosshair on Windows and macOS.", "0.4.0"),
      v("no-drop", OK, "The system cursor on Linux, `not-allowed`'s on Windows and macOS.", "0.4.0"),
      v("zoom-in, zoom-out", PART, "The system cursor on Linux; Windows and macOS show the arrow.", "0.4.0"),
      v("none", PART, "Hides the cursor on Windows; macOS and Linux show the arrow.", "0.4.0"),
      v(
        "grab, grabbing, copy, context-menu",
        PART,
        "Windows has no system cursor for these and shows the arrow; macOS and Linux show theirs.",
      ),
      v("url()", PART, "The image is not loaded: the first fallback keyword shows."),
    ],
  }),
  prop("pointer-events", OK, "Sets whether an element can be the target of pointer events.", {
    values: [v("auto", OK), v("none", OK)],
  }),
  prop("user-select", OK, "Sets whether the user can select an element's text.", {
    values: [v("contain", PART, "Behaves as `text`.")],
    aliases: ["-webkit-user-select"],
  }),
  prop(
    "touch-action",
    PART,
    "Sets which touch gestures, such as panning or pinch-zoom, the browser handles on an element.",
    { limitations: ["Accepted, but the host does not deliver touch input yet."] },
  ),
  prop("appearance", OK, "Sets whether a form control is drawn with its native look.", {
    notes: ["`none` strips a form control's native look."],
    aliases: ["-webkit-appearance"],
  }),
  prop("accent-color", PART, "Sets the accent color of form controls, such as the filled part of a range input.", {
    limitations: ["Range inputs, checkboxes and radio buttons read it; other controls keep their default color."],
  }),
  prop("caret-color", NO, "Sets the color of the text insertion caret.", { related: ["caret-shape"] }),
  prop("caret-shape", NO, "Sets the shape of the text insertion caret: bar, block or underscore.", {
    related: ["caret-color"],
  }),
  prop("resize", PART, "Accepts `none`. Native boxes have no resize handle.", {
    values: [v("none", OK, "Boxes have no resize handle."), v("horizontal, vertical or both", NO)],
    workaround: "Build a drag handle with pointer events.",
  }),
  prop("field-sizing", NO, "Lets form fields such as `<textarea>` size to their content."),
  prop("interactivity", NO, "Makes an element and its subtree inert."),
  prop("app-region", OK, "Marks an element as a region that drags the window, as in an installed web app.", {
    values: [v("drag", OK), v("no-drag", OK), v("move", OK, "Same as `drag`."), v("none", OK, "Same as `no-drag`.")],
    notes: [
      "`app-region: drag` drags the window, as in an installed web app.",
      "`window-drag` is read as the same property.",
      'On Windows, the OS moves the window. On macOS and Linux, the host does, in the main window and in a popout with `chrome="hidden"`.',
    ],
    platformSince: { macos: "0.3.0" },
    aliases: ["-webkit-app-region", "window-drag", "drag region", "titlebar"],
    related: ["window-control"],
  }),
  prop("--window-control", OK, "Turns an element into a window button: `minimize`, `maximize` or `close`.", {
    skathiOnly: true,
    values: [v("minimize", OK), v("maximize", OK), v("close", OK)],
    notes: [
      "On Windows, the OS performs the action. On macOS and Linux, the host performs it when the mouse button is released.",
      "In the main window, `close` dispatches `close-requested`, `beforeunload` and `pagehide`, as the OS close button does.",
    ],
    platformSince: { macos: "0.3.0" },
    aliases: ["window buttons", "titlebar buttons"],
    related: ["app-region"],
  }),
]);

const svgSection = section("SVG properties in CSS", [
  prop("fill", OK, "Sets the paint used to fill an SVG shape or text.", {
    notes: ["Colors and `none` are supported."],
    limitations: [
      "A `url(#…)` paint server from a CSS rule draws in `currentColor`.",
      "Not applied from a `:hover`, `:focus`, `:active` or ancestor-state rule, with an advisory; toggle a class, or use `fill: currentColor` and change `color` in the state rule.",
    ],
    animates: "interpolated",
    related: ["stroke", "fill-opacity", "fill-rule"],
  }),
  prop("stroke", OK, "Sets the paint used to outline an SVG shape or text.", {
    notes: ["Colors and `none` are supported."],
    limitations: [
      "A `url(#…)` paint server from a CSS rule draws in `currentColor`.",
      "Not applied from a `:hover`, `:focus`, `:active` or ancestor-state rule, with an advisory; toggle a class, or use `stroke: currentColor` and change `color` in the state rule.",
    ],
    animates: "interpolated",
    related: ["fill", "stroke-width", "stroke-opacity"],
  }),
  ...(
    [
      ["stroke-width", "Sets the width of an SVG shape's outline."],
      ["stroke-linecap", "Sets the shape drawn at the ends of open SVG paths."],
      ["stroke-linejoin", "Sets the shape drawn where two segments of an SVG outline meet."],
      ["stroke-miterlimit", "Sets the limit on the length of a mitered join before it is beveled."],
      ["stroke-dasharray", "Sets the pattern of dashes and gaps of an SVG outline."],
      ["stroke-dashoffset", "Sets where the dash pattern of an SVG outline starts."],
      ["fill-opacity", "Sets the opacity of an SVG shape's fill."],
      ["stroke-opacity", "Sets the opacity of an SVG shape's outline."],
      ["fill-rule", "Sets which parts of a self-intersecting SVG shape are inside and filled."],
      [
        "shape-rendering",
        "Hints the trade-off between speed, crisp edges and geometric precision when rendering SVG shapes.",
      ],
    ] as const
  ).map(([name, summary]) =>
    prop(name, OK, summary, {
      limitations: [SVG_STATE_LIMIT],
      related: [name.startsWith("fill") ? "fill" : name === "shape-rendering" ? "fill" : "stroke"],
    }),
  ),
  ...(
    [
      ["stop-color", "Sets the color of an SVG gradient stop."],
      ["stop-opacity", "Sets the opacity of an SVG gradient stop."],
      ["x", "Sets the x coordinate of an SVG element."],
      ["y", "Sets the y coordinate of an SVG element."],
      ["cx", "Sets the x coordinate of the center of an SVG circle or ellipse."],
      ["cy", "Sets the y coordinate of the center of an SVG circle or ellipse."],
      ["r", "Sets the radius of an SVG circle."],
      ["rx", "Sets the horizontal radius of an SVG ellipse or rounded rectangle."],
      ["ry", "Sets the vertical radius of an SVG ellipse or rounded rectangle."],
    ] as const
  ).map(([name, summary]) => prop(name, OK, summary, { since: "0.6.0", limitations: [SVG_STATE_LIMIT] })),
  ...(
    [
      ["clip-rule", "Sets which parts of a self-intersecting SVG clip path are inside."],
      ["flood-color", "Sets the color of the SVG `<feFlood>` and `<feDropShadow>` filter primitives."],
      ["lighting-color", "Sets the light color of SVG lighting filter primitives."],
      ["paint-order", "Sets the order the fill, stroke and markers of SVG shapes and text are painted in."],
      ["text-anchor", "Aligns SVG text to its start, middle or end point."],
      ["dominant-baseline", "Sets the baseline used to align SVG text."],
    ] as const
  ).map(([name, summary]) =>
    prop(name, PART, summary, {
      notes: [SVG_STYLE_ONLY, `The SVG presentation attribute \`${name}\` also works.`],
    }),
  ),
  sh("marker", PART, "Sets the markers drawn at the vertices of SVG paths, lines and polygons.", {
    notes: [SVG_STYLE_ONLY],
    workaround: "For presentation attributes, use `marker-start`, `marker-mid` and `marker-end` on the path.",
    aliases: ["marker-*", "marker-start", "marker-mid", "marker-end"],
  }),
  prop("d", NO, "Sets the path data of an SVG `<path>`.", {
    workaround: 'Set the SVG attribute with path data, such as `d="M0 0L10 10"`.',
  }),
  prop("vector-effect", NO, "Sets how an SVG element's stroke reacts to transforms.", {
    limitations: ["The stroke scales with the shape even when `non-scaling-stroke` is set as an SVG attribute."],
  }),
]);

const containment = section("Containment and hints", [
  prop("contain", PART, "Isolates an element's layout, paint or size from the rest of the page, for performance.", {
    values: [
      v("paint", OK, "Clips."),
      v("content", OK, "Clips."),
      v("strict", OK, "Clips, and sizes the box as if it were empty.", "0.4.0"),
      v("size", OK, "Sizes the box as if it were empty on both axes.", "0.4.0"),
      v("inline-size", OK, "Sizes the box's width as if it were empty.", "0.4.0"),
    ],
    notes: [
      "Size containment sizes the box from its own `width`, `height`, `min-*` and `max-*`, its padding and border, and what its parent gives it (a stretched block or a flex item that grows), never from its content.",
    ],
    limitations: ["`getComputedStyle()` reads `contain` as `none`."],
    related: ["content-visibility", "container-type"],
  }),
  prop("will-change", OK, "Hints which properties of an element are expected to change.", {
    notes: ["Treated as a hint."],
  }),
  prop("container-type", OK, "Makes an element a query container that `@container` rules can test.", {
    since: "0.4.0",
    values: [
      v("normal", OK, "Not a size container; still a style container."),
      v("inline-size", OK, "Queries on the width, with inline-size containment."),
      v("size", OK, "Queries on both axes, with size containment."),
      v("scroll-state", NO, "Prints an advisory; the element is not a query container."),
    ],
    notes: [
      "The element gets size containment on the queried axes, so its size there does not depend on its content: give a `size` container a height, and an `inline-size` container in a shrink-to-fit place (a flex item that does not grow, an absolutely positioned box) a width.",
      "A container whose size changes restyles the elements that queried it and lays them out again in the same frame, before `ResizeObserver` callbacks and paint.",
    ],
    limitations: ["Layout containment is not applied: a container still has a baseline and floats can cross it."],
    related: ["container-name", "container", "at-container", "unit-container"],
  }),
  prop("container-name", OK, "Names a query container for `@container` rules.", {
    since: "0.4.0",
    notes: ["A list of names; `@container <name>` picks the nearest ancestor that has the name."],
    related: ["container-type", "container", "at-container"],
  }),
  sh("container", OK, "Sets `container-name` and `container-type` in one declaration.", {
    since: "0.4.0",
    related: ["container-type", "container-name", "at-container"],
  }),
  prop("content-visibility", NO, "Lets the browser skip rendering an element's content until it is needed.", {
    related: ["contain-intrinsic-size", "contain"],
  }),
  sh(
    "contain-intrinsic-size",
    NO,
    "Sets the size used for an element whose size is contained, such as one skipped by `content-visibility`.",
    { related: ["content-visibility"] },
  ),
]);

const generated = section("Generated content and lists", [
  pe("::before", OK, "Creates a pseudo-element as the first child of an element, rendered from `content`.", {
    notes: ['A string `content` is supported, including `content: ""` boxes.'],
    values: [
      v(":hover / :focus-visible before ::before", OK, "Matches the originating element or an ancestor (since 0.6)."),
    ],
    related: ["pseudo-after", "content"],
  }),
  pe("::after", OK, "Creates a pseudo-element as the last child of an element, rendered from `content`.", {
    notes: ['A string `content` is supported, including `content: ""` boxes.'],
    values: [
      v(":hover / :focus-visible before ::after", OK, "Matches the originating element or an ancestor (since 0.6)."),
    ],
    related: ["pseudo-before", "content"],
  }),
  prop("content", PART, "Sets the content of `::before` and `::after` pseudo-elements.", {
    values: [
      v("a string", OK),
      v("attr()", NO, "Render the value as a real element."),
      v("counter()", NO, "Render the value as a real element."),
      v("url()", NO, "Render the value as a real element."),
      v("open-quote", NO, "Render the value as a real element."),
    ],
    limitations: ["Only a string `content` is supported."],
    related: ["pseudo-before", "pseudo-after"],
  }),
  prop("counter-reset", NO, "Creates or resets CSS counters.", { related: ["counter-increment", "counter-set"] }),
  prop("counter-increment", NO, "Increments CSS counters.", { related: ["counter-reset", "counter-set"] }),
  prop("counter-set", NO, "Sets CSS counters to a value.", { related: ["counter-reset", "counter-increment"] }),
  prop("list-style-type", PART, "Sets the marker of a list item, such as a disc or a number.", {
    values: [
      v("disc", OK),
      v("circle", OK),
      v("square", OK),
      v("decimal", OK),
      v("decimal-leading-zero", OK),
      v("alphabetic (lower-alpha, upper-alpha, …)", OK),
      v("roman (lower-roman, upper-roman)", OK),
      v("none", OK),
      v("a string", NO, 'Use `::before { content: "…" }`.'),
    ],
    limitations: ["Markers are drawn on `<li>` only."],
    related: ["list-style", "list-style-position", "list-style-image", "pseudo-marker"],
  }),
  prop("list-style-position", OK, "Sets whether a list item's marker sits inside or outside its box.", {
    related: ["list-style", "list-style-type"],
  }),
  sh("list-style", OK, "Sets a list item's marker type, position and image in one declaration.", {
    related: ["list-style-type", "list-style-position", "list-style-image"],
  }),
  prop("list-style-image", PART, "Accepts `none`. List markers use the shape or numbering from `list-style-type`.", {
    values: [
      v("none", OK, "The marker comes from `list-style-type`."),
      v("<image>", NO, "Marker images are not drawn."),
    ],
    workaround: 'Use `::before { content: "…" }`.',
    related: ["list-style-type"],
  }),
  pe("::marker", NO, "Selects the marker box of a list item.", {
    workaround: 'Use `::before { content: "…" }`.',
    related: ["list-style-type", "pseudo-before"],
  }),
  prop("object-fit", OK, "Sets how an image or video is resized to fit its box.", { related: ["object-position"] }),
  prop("object-position", OK, "Sets where an image or video sits inside its box.", { related: ["object-fit"] }),
  prop("image-rendering", PART, "Sets the scaling algorithm used for images, such as smooth or pixelated.", {
    notes: ["A stylesheet inside an SVG can set `image-rendering: pixelated` on an SVG `<image>`."],
    limitations: ["App stylesheets cannot change the sampling of an `<img>`."],
  }),
  prop("image-orientation", NO, "Sets whether an image is rotated according to its EXIF orientation."),
]);

const selectors = section("Selectors", [
  sel("selector-type", "Type selector", OK, "Matches elements by tag name, such as `p` or `button`.", {
    aliases: ["element selector", "tag selector"],
  }),
  sel("selector-class", "Class selector", OK, "Matches elements by class, such as `.card`."),
  sel("selector-id", "ID selector", OK, "Matches the element with a given `id`, such as `#main`."),
  sel("selector-universal", "Universal selector (`*`)", OK, "Matches elements of any type.", { aliases: ["*"] }),
  sel(
    "selector-attribute",
    "Attribute selectors",
    OK,
    'Match elements by the presence or value of an attribute, such as `[type="checkbox"]`.',
    {
      notes: ["Every operator and the `i` flag are supported."],
      aliases: [
        "[attr]",
        "[attr=value]",
        "[attr^=value]",
        "[attr$=value]",
        "[attr*=value]",
        "[attr~=value]",
        "[attr|=value]",
      ],
    },
  ),
  sel(
    "selector-descendant",
    "Descendant combinator",
    OK,
    "Matches elements nested anywhere inside another, written as a space.",
    { related: ["selector-child"] },
  ),
  sel("selector-child", "Child combinator (`>`)", OK, "Matches elements that are direct children of another.", {
    aliases: [">"],
    related: ["selector-descendant"],
  }),
  sel(
    "selector-next-sibling",
    "Next-sibling combinator (`+`)",
    OK,
    "Matches an element that directly follows another sibling.",
    { aliases: ["+", "adjacent sibling"], related: ["selector-subsequent-sibling"] },
  ),
  sel(
    "selector-subsequent-sibling",
    "Subsequent-sibling combinator (`~`)",
    OK,
    "Matches elements that follow another sibling, not necessarily directly.",
    {
      aliases: ["~", "general sibling"],
      related: ["selector-next-sibling"],
    },
  ),
  ...(
    [
      [":hover", "Matches an element the pointer is over."],
      [":active", "Matches an element that is being activated, for example while it is pressed."],
      [":focus", "Matches the element that has focus."],
      [":focus-visible", "Matches the focused element when focus should be shown, such as after keyboard navigation."],
      [":focus-within", "Matches an element that has focus or contains the focused element."],
    ] as const
  ).map(([name, summary]) =>
    pc(name, OK, summary, {
      limitations: [STATE_LIMIT],
      related: [...STATES.filter((s) => s !== `pseudo-${name.slice(1)}`), "selector-ancestor-state"],
    }),
  ),
  sel(
    "selector-ancestor-state",
    "State on an ancestor compound",
    OK,
    "A user-action pseudo-class on an ancestor in a selector, such as `.menu:focus .item`.",
    {
      values: [
        v(":hover, :active on an ancestor", OK),
        v(":focus, :focus-visible, :focus-within on an ancestor", OK, undefined, "0.3.0"),
      ],
      limitations: ["Only one ancestor per state.", "Not supported left of `+` or `~`."],
      related: STATES,
    },
  ),
  ...(
    [
      [":disabled", "Matches form controls that are disabled."],
      [":enabled", "Matches form controls that are enabled."],
      [":checked", "Matches checkboxes, radio buttons and options that are checked or selected."],
      [":indeterminate", "Matches a checkbox in an indeterminate state, partly checked."],
      [":fullscreen", "Matches the element that is in fullscreen mode."],
      [":root", "Matches the root element of the document."],
      [":empty", "Matches elements that have no children."],
      [":first-child", "Matches an element that is the first of its siblings."],
      [":last-child", "Matches an element that is the last of its siblings."],
      [":only-child", "Matches an element that has no siblings."],
      [":nth-last-child()", "Matches elements by their position among their siblings, counting from the end."],
      [":nth-of-type()", "Matches elements by their position among siblings of the same type."],
      [
        ":nth-last-of-type()",
        "Matches elements by their position among siblings of the same type, counting from the end.",
      ],
      [":first-of-type", "Matches an element that is the first of its type among its siblings."],
      [":last-of-type", "Matches an element that is the last of its type among its siblings."],
      [":only-of-type", "Matches an element that has no siblings of the same type."],
    ] as const
  ).map(([name, summary]) => pc(name, OK, summary)),
  pc(":nth-child()", OK, "Matches elements by their position among their siblings.", {
    values: [v(":nth-child(… of S)", NO)],
    related: ["pseudo-nth-last-child", "pseudo-nth-of-type"],
  }),
  pc(":not()", OK, "Matches elements that do not match any of the selectors given.", {
    related: ["pseudo-is", "pseudo-where"],
  }),
  pc(":is()", OK, "Matches elements that match any of the selectors given.", {
    related: ["pseudo-where", "pseudo-not"],
  }),
  pc(":where()", OK, "Matches elements that match any of the selectors given, with zero specificity.", {
    related: ["pseudo-is", "pseudo-not"],
  }),
  pc(":has()", PART, "Matches elements that have a descendant or sibling matching the selector given.", {
    notes: ["Descendant, `>`, `+` and `~` relations are supported."],
    limitations: [
      "`:has()` cannot be nested.",
      "A user-action state inside `:has()` is not supported.",
      "It does not work in `querySelector()`, `querySelectorAll()`, `matches()` or `closest()`: the query matches nothing and prints an advisory. Filter the results in script instead.",
    ],
    aliases: ["parent selector"],
  }),
  pc(":scope", OK, "Matches the element a selector is scoped to, such as the element `querySelector()` is called on.", {
    notes: ["Supported in `querySelector()`."],
  }),
  ...(
    [
      [":placeholder-shown", "Matches an input or textarea that is showing its placeholder."],
      [":read-only", "Matches elements the user cannot edit."],
      [":required", "Matches form fields that are required."],
      [":default", "Matches the default option of a group of form controls."],
      [":autofill", "Matches an input whose value the browser has autofilled."],
      [":valid", "Matches form fields whose content validates."],
      [":invalid", "Matches form fields whose content does not validate."],
      [":user-valid", "Matches form fields that validate after the user has interacted with them."],
      [":user-invalid", "Matches form fields that do not validate after the user has interacted with them."],
    ] as const
  ).map(([name, summary]) => pc(name, NO, summary, { workaround: FORM_STATE_ALT })),
  ...(
    [
      [":link", "Matches links that have not been visited."],
      [":visited", "Matches links that have been visited."],
      [":any-link", "Matches every element that is the source anchor of a link."],
      [":target", "Matches the element whose id matches the URL fragment."],
      [":lang()", "Matches elements by their content language."],
      [":dir()", "Matches elements by their text direction."],
      [":modal", "Matches an element in a modal state, such as a dialog opened with `showModal()`."],
      [":popover-open", "Matches a popover element that is showing."],
      [":open", "Matches elements in an open state, such as an open `<details>` or `<dialog>`."],
      [":defined", "Matches elements that are defined, including registered custom elements."],
    ] as const
  ).map(([name, summary]) => pc(name, NO, summary)),
  pe("::placeholder", OK, "Selects the placeholder text of an input or textarea.", {
    related: ["pseudo-placeholder-shown"],
  }),
  pe("::selection", OK, "Selects the part of the document the user has selected."),
  pe("::file-selector-button", OK, 'Selects the button of an `<input type="file">`.'),
  ...(
    [
      ["::-webkit-slider-thumb", "Selects the thumb of a range input in WebKit and Blink browsers."],
      ["::-webkit-slider-runnable-track", "Selects the track of a range input in WebKit and Blink browsers."],
      ["::-moz-range-thumb", "Selects the thumb of a range input in Firefox."],
      ["::-moz-range-track", "Selects the track of a range input in Firefox."],
      ["::-webkit-color-swatch", "Selects the color swatch of a color input in WebKit and Blink browsers."],
    ] as const
  ).map(([name, summary]) =>
    pe(name, OK, summary, {
      notes: ["Styles range and color inputs."],
      ...(name.startsWith("::-moz-range") ? { aliases: ["::-moz-range-*"] } : {}),
    }),
  ),
  pe("::-moz-range-progress", NO, "Selects the filled part of a range input's track in Firefox.", {
    limitations: ["Matches nothing, without an advisory."],
    workaround: "Color the filled part with `accent-color`.",
    aliases: ["::-moz-range-*"],
    related: ["accent-color"],
  }),
  pe("::first-line", NO, "Selects the first line of a block of text."),
  pe("::first-letter", NO, "Selects the first letter of a block of text."),
  pe("::backdrop", NO, "Selects the box rendered behind a fullscreen element or a modal dialog."),
  pe("::highlight()", NO, "Selects text ranges registered with the CSS Custom Highlight API."),
  pc(":host", NO, "Matches the shadow host from inside its shadow DOM.", { limitations: [SHADOW_DOM_LIMIT] }),
  pe("::part()", NO, "Selects an element inside a shadow tree that has a matching `part` attribute.", {
    limitations: [SHADOW_DOM_LIMIT],
  }),
  pe("::slotted()", NO, "Selects elements placed into a slot of a shadow DOM.", {
    limitations: [SHADOW_DOM_LIMIT],
    notes: ["Vue's `:slotted()` in `<style scoped>` is a different feature and works."],
    related: ["feature-scoped-styles"],
  }),
  sel("selector-namespace", "Namespaced selectors", PART, "Match elements in a given XML namespace, such as `svg|a`.", {
    values: [
      v("*|div", OK, "Matches elements named div."),
      v("|div", PART, "Matches by tag without checking the namespace."),
      v("named prefixes and namespaced attributes", NO),
    ],
    aliases: ["svg|a", "|"],
    related: ["at-namespace"],
  }),
]);

const atRules = section("At-rules", [
  at("@media", OK, "Applies styles only when a media query matches, such as a minimum window width.", {
    notes: ["See the media features for what a query can test."],
    limitations: ["A query that uses an unsupported feature drops its whole `@media` block, with an advisory."],
    related: ["media-width", "media-prefers-color-scheme", "media-types"],
  }),
  at("@supports", OK, "Applies styles only when the engine supports the declarations or selectors tested.", {
    notes: [
      "Uses the same declaration and selector checks as `CSS.supports()`. Some accepted values leave the native layout unchanged.",
    ],
    aliases: ["CSS.supports()", "feature queries"],
  }),
  at("@container", OK, "Applies styles based on the size of an ancestor query container.", {
    since: "0.4.0",
    notes: [
      "Size queries accept `width`, `height`, `inline-size`, `block-size`, `aspect-ratio` and `orientation`. Combine conditions with `and`, `or` and `not`. Lengths and ratios also support range comparisons. Lengths can use CSS pixels, physical units, `rem` or the container's `em`. Use `container-type: size` to query an ancestor's ratio or orientation.",
      "An optional name picks the nearest ancestor with that `container-name`; otherwise the nearest container that can answer every feature in the query.",
      "Style queries on custom properties: `style(--theme: dark)` and `style(--dense)`.",
      "Nests inside `@media`, `@supports` and another `@container`, and holds `@media` blocks.",
      "The query sees the container's content box as the frame lays it out, including while a transition or animation resizes it; `getBoundingClientRect()` and `ResizeObserver` see the restyled layout.",
    ],
    limitations: [
      "A query that uses an unsupported feature (scroll-state queries, style queries on standard properties, `%`, viewport units or `calc()` lengths) drops its whole block, with an advisory.",
      "A query container in a pop-out window settles a frame late after that window's own layout moves it.",
      "Rules are not ordered by `@layer`, which is not supported.",
    ],
    aliases: ["container queries", "@container aspect-ratio", "@container orientation"],
    related: ["container-type", "container-name", "container", "unit-container", "aspect-ratio", "media-aspect-ratio"],
  }),
  at("@layer", PLAN, "Declares cascade layers that control the order of precedence of groups of rules.", {
    limitations: ["The rules inside are dropped for now."],
    notes: ["Tailwind's `@layer` is resolved by its build pass."],
    aliases: ["cascade layers"],
    related: ["feature-tailwind-4"],
  }),
  at("@keyframes", OK, "Defines the intermediate steps of a CSS animation.", {
    aliases: ["-webkit-keyframes"],
    related: ["animation", "feature-web-animations"],
  }),
  at("@font-face", OK, "Defines a custom font from a font file that `font-family` can then use.", {
    notes: ["TTF, OTF, WOFF and WOFF2 fonts are supported.", "`FontFace` from script works too."],
    aliases: ["FontFace", "web fonts"],
    related: ["font-family"],
  }),
  at("@import", PART, "Imports the rules of another stylesheet.", {
    notes: ["`sk dev` and `sk build` inline local files."],
    limitations: ["Remote URLs are not imported."],
  }),
  at("@property", PART, "Registers a custom property with a syntax, inheritance and initial value.", {
    notes: ["`<percentage>` registrations are supported, and those custom properties then animate in `@keyframes`."],
    limitations: [
      "Other syntaxes are ignored and the property stays unregistered.",
      "Transitions and `element.animate()` do not interpolate a registered property; it snaps to its end value.",
    ],
    related: ["custom-properties", "feature-tailwind-4"],
  }),
  at("@function", OK, "Defines a custom function that returns a value computed from its arguments.", {
    since: "0.4.0",
    notes: [
      "Parameters take an optional type (`<length>`, `<color>`, `type(<number> | auto)`, `*`) and an optional default, which may read an earlier parameter. `returns` sets the result's type.",
      "The body holds `result` and local custom properties, which can read the parameters and the calling element's custom properties, and `@media`, `@supports` and `@container` blocks around them. Containers are the calling element's.",
      "An argument or result that does not match its type, an unknown function, a missing argument and a function that calls itself make the declaration invalid at computed-value time, as with `var()`.",
      "With several definitions of one name, the last one whose `@media` condition holds wins.",
    ],
    limitations: [
      "Arguments and results are substituted as written once their type is checked, not as computed values: an `em` in an argument resolves where the result is used.",
      "A definition inside a top-level `@container` block applies regardless of the container.",
      "Definitions are not ordered by `@layer`, which is not supported.",
    ],
    related: ["dashed-function", "function-var", "custom-properties", "at-property"],
  }),
  at("@scope", NO, "Scopes styles to a subtree of the document, with an optional lower boundary.", {
    workaround: "Use `<style scoped>`.",
    related: ["feature-scoped-styles"],
  }),
  at("@starting-style", NO, "Defines the starting values a transition runs from when an element is first rendered.", {
    workaround: "Use the framework's transitions (`<Transition>`, `transition:`).",
    related: ["transition", "transition-behavior"],
  }),
  at("@page", NO, "Sets the margins and size of pages when printing."),
  at("@namespace", NO, "Declares an XML namespace for the selectors of a stylesheet.", {
    related: ["selector-namespace"],
  }),
  at("@counter-style", NO, "Defines a custom counter style for list markers and counters.", {
    workaround: 'Use `::before { content: "…" }`.',
    related: ["list-style-type"],
  }),
  at("@font-feature-values", NO, "Names a font's alternate glyph sets for use with `font-variant-alternates`."),
]);

const mediaFeatures = section("Media features", [
  feat("media-types", "Media types", OK, "The kind of device a media query targets: `screen`, `print` or `all`.", {
    values: [v("screen", OK), v("all", OK), v("print", OK, "Never matches.")],
    aliases: ["screen", "print", "all"],
    related: ["at-media"],
  }),
  mf("width", PART, "Tests the width of the viewport.", {
    limitations: [MEDIA_SIZE_LIMIT],
    aliases: ["min-width", "max-width"],
    related: ["media-height", "media-range-syntax", "at-media"],
  }),
  mf("height", PART, "Tests the height of the viewport.", {
    limitations: [MEDIA_SIZE_LIMIT],
    aliases: ["min-height", "max-height"],
    related: ["media-width", "media-range-syntax", "at-media"],
  }),
  feat(
    "media-range-syntax",
    "Range syntax",
    PART,
    "Writes a media query as a comparison, such as `(400px <= width < 800px)`.",
    {
      limitations: [MEDIA_SIZE_LIMIT],
      aliases: ["<=", ">=", "media query range"],
      related: ["media-width", "media-height"],
    },
  ),
  mf("prefers-color-scheme", OK, "Tests whether the user prefers a light or dark color scheme.", {
    notes: [
      "Follows `appWindow.theme` when it is `light` or `dark`, and the system setting under `system`, the default.",
    ],
    aliases: ["dark mode"],
    related: ["function-light-dark", "color-scheme"],
  }),
  mf("prefers-reduced-motion", OK, "Tests whether the user asked for less motion.", {
    notes: ["Follows the system settings."],
  }),
  mf("resolution", OK, "Tests the pixel density of the output device.", {
    notes: ["`dppx`, `dpi` and `dpcm` are supported."],
    aliases: ["min-resolution", "max-resolution", "dppx"],
    related: ["media-webkit-device-pixel-ratio"],
  }),
  mf("display-mode", OK, "Tests how the app is displayed: in a browser tab, standalone, fullscreen, and so on.", {
    values: [
      v("standalone", OK),
      v("window-controls-overlay", OK),
      v("fullscreen", OK),
      v("browser", OK, "Never matches."),
    ],
  }),
  mf("pointer", OK, "Tests the accuracy of the primary pointing device.", {
    related: ["media-any-pointer", "media-hover"],
  }),
  mf("hover", OK, "Tests whether the primary pointing device can hover over elements.", {
    related: ["media-any-hover", "media-pointer"],
  }),
  mf("any-pointer", PART, "Tests the accuracy of any available pointing device.", {
    platforms: { windows: OK, macos: PART, linux: PART },
    values: [
      v("fine", OK, "Always matches."),
      v("coarse", PART, "Matches on Windows when a touch screen or pen digitizer is present."),
      v("none", OK, "Never matches."),
    ],
    limitations: ["`coarse` never matches on macOS and Linux, even with a touch screen."],
    related: ["media-pointer"],
  }),
  mf("any-hover", OK, "Tests whether any available pointing device can hover over elements.", {
    related: ["media-hover"],
  }),
  feat(
    "media-logical-operators",
    "Logical operators in media queries",
    OK,
    "Combine, negate or list media queries with `and`, `or`, `not`, `only` and commas.",
    {
      aliases: ["and", "or", "not", "only", "comma"],
      related: ["at-media"],
    },
  ),
  mf("orientation", NO, "Tests whether the viewport is in portrait or landscape orientation.", {
    workaround:
      "Use viewport width and height breakpoints, or test a size container with `@container (orientation: landscape)`.",
    related: ["media-width", "media-height", "at-container"],
  }),
  mf("aspect-ratio", NO, "Tests the width-to-height ratio of the viewport.", {
    notes: [
      "The viewport query drops the `@media` block. The `aspect-ratio` property sizes an element, and `@container (aspect-ratio > 1/1)` tests an ancestor with `container-type: size`.",
    ],
    workaround: "For an exact window ratio, compare `innerWidth` and `innerHeight` on resize.",
    aliases: ["min-aspect-ratio", "max-aspect-ratio"],
    related: ["aspect-ratio", "at-container", "media-width", "media-height"],
  }),
  mf("prefers-contrast", NO, "Tests whether the user asked for more or less contrast."),
  mf("prefers-reduced-transparency", PART, "Tests whether the user asked for less transparency.", {
    since: "0.4.0",
    platforms: { windows: OK, macos: PART, linux: PART },
    notes: [
      "On Windows, `reduce` matches while Settings > Personalization > Colors > Transparency effects is off, and updates as soon as it changes.",
    ],
    limitations: ["macOS and Linux always answer `no-preference`."],
    related: ["media-prefers-reduced-motion", "media-prefers-color-scheme"],
  }),
  mf("forced-colors", NO, "Tests whether the system enforces a limited color palette, such as a high-contrast mode."),
  mf("inverted-colors", NO, "Tests whether the system inverts colors."),
  mf("color", NO, "Tests the number of bits per color component of the output device."),
  mf("color-gamut", NO, "Tests the range of colors the output device can show."),
  mf("dynamic-range", PART, "Tests whether the output device supports high dynamic range.", {
    since: "0.4.0",
    platforms: { windows: OK, macos: OK, linux: PART },
    notes: [
      "`high` matches while the window's display is in HDR (Windows' HDR mode, a Mac screen with EDR headroom); `standard` always matches. It follows the window to another display.",
    ],
    limitations: ["Linux always answers `standard`."],
    related: ["media-video-dynamic-range", "dynamic-range-limit"],
  }),
  mf("video-dynamic-range", PART, "Tests whether the output device supports high dynamic range for video.", {
    since: "0.4.0",
    platforms: { windows: OK, macos: OK, linux: PART },
    notes: ["Answers as `dynamic-range` does: a window draws video and the page alike."],
    limitations: ["Linux always answers `standard`."],
    related: ["media-dynamic-range"],
  }),
  mf("scripting", NO, "Tests whether scripting is available."),
  mf("update", NO, "Tests how quickly the output device can change what it shows."),
  mf("-webkit-device-pixel-ratio", NO, "Tests the device pixel ratio; the non-standard predecessor of `resolution`.", {
    workaround: "Use `resolution` and `width`.",
    aliases: ["-webkit-min-device-pixel-ratio", "-webkit-max-device-pixel-ratio"],
    related: ["media-resolution"],
  }),
  mf("device-width", OK, "Tests the width of the output device's screen.", {
    since: "0.4.0",
    notes: [
      "The width of the monitor the window is on, in CSS pixels: `screen.width`. It updates when the window moves to another monitor.",
    ],
    limitations: [MEDIA_SIZE_LIMIT],
    aliases: ["min-device-width", "max-device-width"],
    related: ["media-width", "media-device-height"],
  }),
  mf("device-height", OK, "Tests the height of the output device's screen.", {
    since: "0.4.0",
    notes: ["The height of the monitor the window is on, in CSS pixels: `screen.height`."],
    limitations: [MEDIA_SIZE_LIMIT],
    aliases: ["min-device-height", "max-device-height"],
    related: ["media-height", "media-device-width"],
  }),
  feat(
    "media-custom-media",
    "Custom media queries",
    NO,
    "Name a media query with `@custom-media --name` and reuse it as `(--name)`.",
    { aliases: ["@custom-media", "--name"] },
  ),
]);

const valuesAndUnits = section("Values and units", [
  unit("px", OK, "An absolute length unit: one CSS pixel.", { related: ["unit-em", "unit-rem"] }),
  unit("%", OK, "A percentage of a reference value, usually a size of the containing block.", {
    slug: "unit-percent",
    aliases: ["percentage"],
  }),
  unit("em", OK, "A length relative to the element's font size.", { related: ["unit-rem"] }),
  unit("rem", OK, "A length relative to the root element's font size.", { related: ["unit-em"] }),
  unit("ch", OK, "A length equal to the advance width of the `0` glyph in the element's font.", {
    related: ["unit-rch"],
  }),
  unit("ex", OK, "A length equal to the x-height of the element's font.", { related: ["unit-rex"] }),
  unit("rch", PART, "A length equal to the advance width of the `0` glyph in the root element's font.", {
    limitations: [ROOT_HALF_EM("rch")],
    related: ["unit-ch"],
  }),
  unit("rex", PART, "A length equal to the x-height of the root element's font.", {
    limitations: [ROOT_HALF_EM("rex")],
    related: ["unit-ex"],
  }),
  ...(
    [
      ["vw", "1% of the viewport's width.", ["svw", "lvw", "dvw"]],
      ["vh", "1% of the viewport's height.", ["svh", "lvh", "dvh"]],
      ["vmin", "1% of the viewport's smaller dimension.", ["svmin", "lvmin", "dvmin"]],
      ["vmax", "1% of the viewport's larger dimension.", ["svmax", "lvmax", "dvmax"]],
      ["vi", "1% of the viewport's size along the inline axis.", ["svi", "lvi", "dvi"]],
      ["vb", "1% of the viewport's size along the block axis.", ["svb", "lvb", "dvb"]],
    ] as const
  ).map(([name, summary, variants]) =>
    unit(name, OK, `A viewport length unit: ${summary}`, { notes: [VIEWPORT_NOTE], aliases: [...variants] }),
  ),
  ...(
    [
      ["in", "An absolute length unit: one inch, 96px."],
      ["cm", "An absolute length unit: one centimeter."],
      ["mm", "An absolute length unit: one millimeter."],
      ["pt", "An absolute length unit: one point, 1/72 of an inch."],
      ["pc", "An absolute length unit: one pica, 12 points."],
      ["Q", "An absolute length unit: a quarter of a millimeter."],
    ] as const
  ).map(([name, summary]) => unit(name, PART, summary, ABSOLUTE_UNIT)),
  unit("lh", NO, "A length equal to the element's computed line height.", {
    limitations: ["Ignored without an advisory."],
    related: ["unit-rlh"],
  }),
  unit("rlh", NO, "A length equal to the root element's computed line height.", {
    limitations: ["Ignored without an advisory."],
    related: ["unit-lh"],
  }),
  unit("Container query units", OK, "Lengths relative to the size of a query container, such as `cqw` and `cqh`.", {
    slug: "unit-container",
    since: "0.4.0",
    notes: [
      "`cqw` and `cqi` are a percent of the nearest `inline-size` or `size` container's content width, `cqh` and `cqb` of the nearest `size` container's content height; `cqmin` and `cqmax` take the smaller or larger. With no such container they use the window.",
      "Work in stylesheets, inline styles, `calc()` and `var()`.",
    ],
    limitations: ["Inside `@keyframes` they resolve against the window."],
    aliases: ["cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax"],
    related: ["at-container"],
  }),
  fn("calc()", PART, "Computes a value from an arithmetic expression, which can mix units.", {
    limitations: [
      "Mixing `%` and a length works in sizes, `flex-basis`, insets, margins, padding, gaps and `border-radius`.",
      "In `translate()`, `translateX()` and `translateY()`, a `calc()` that mixes `%` or font-relative units with `px`, such as `calc(-100% - 8px)`, is skipped with an advisory while the other transform functions apply. Chain functions instead: `translateX(-100%) translateX(-8px)`.",
    ],
    related: ["function-min", "function-max", "function-clamp"],
  }),
  fn("min()", PART, "Returns the smallest of several values.", {
    notes: ["Width, height and their min/max bounds accept mixed percentages and lengths (since 0.6)."],
    related: ["function-max", "function-clamp", "function-calc"],
  }),
  fn("max()", PART, "Returns the largest of several values.", {
    notes: ["Width, height and their min/max bounds accept mixed percentages and lengths (since 0.6)."],
    related: ["function-min", "function-clamp", "function-calc"],
  }),
  fn("clamp()", PART, "Clamps a value between a minimum and a maximum.", {
    notes: ["Width, height and their min/max bounds accept mixed percentages and lengths (since 0.6)."],
    related: ["function-min", "function-max", "function-calc"],
  }),
  ...(
    [
      ["round()", "Rounds a value to a multiple of an interval, using a rounding strategy."],
      ["mod()", "Returns the modulus of two values, with the sign of the divisor."],
      ["abs()", "Returns the absolute value of its argument."],
      ["sign()", "Returns -1, 0 or 1 depending on the sign of its argument."],
      ["pow()", "Raises a number to a power."],
      ["sqrt()", "Returns the square root of a number."],
      ["hypot()", "Returns the square root of the sum of the squares of its arguments."],
      ["exp()", "Returns e raised to the power of a number."],
      ["log()", "Returns the logarithm of a number, in base e or a given base."],
      ["sin()", "Returns the sine of an angle."],
      ["cos()", "Returns the cosine of an angle."],
      ["tan()", "Returns the tangent of an angle."],
      ["asin()", "Returns the arcsine of a number, as an angle."],
      ["acos()", "Returns the arccosine of a number, as an angle."],
      ["atan()", "Returns the arctangent of a number, as an angle."],
      ["atan2()", "Returns the angle of a point from its y and x coordinates."],
    ] as const
  ).map(([name, summary]) =>
    fn(name, OK, summary, {
      notes: [CONST_MATH],
      ...(/^(a?(sin|cos|tan)|atan2)\(/.test(name) ? { aliases: ["trigonometry"] } : {}),
    }),
  ),
  fn("var()", OK, "Inserts the value of a custom property.", {
    notes: ["Fallbacks are supported."],
    related: ["custom-properties"],
  }),
  prop("Custom properties (`--*`)", OK, "Properties whose names start with `--`, holding values that `var()` reads.", {
    slug: "custom-properties",
    notes: [
      "Unregistered custom properties inherit. `var(--name, fallback)` uses its fallback when the property is missing or invalid.",
      'A matching state rule can change a custom property for the element and its descendants. `getComputedStyle(element).getPropertyValue("--name")` returns the value for the current state (since 0.6).',
    ],
    aliases: ["--*", "css variables"],
    related: ["function-var", "pseudo-hover", "at-property"],
  }),
  kw(
    "dashed-function",
    "<dashed-function>",
    OK,
    "A call of a custom function defined with `@function`, such as `--pad(8px)`.",
    {
      since: "0.4.0",
      notes: [
        "Works anywhere a value goes, including inside `var()` fallbacks, `calc()`, other functions, custom properties, `@keyframes` and `element.animate()`.",
        "Arguments may hold `var()` and other calls. An argument with a comma in it is wrapped in braces: `--first({a, b})`.",
      ],
      aliases: ["custom functions", "css functions"],
      related: ["at-function", "function-var"],
    },
  ),
  fn("env()", OK, "Inserts a value defined by the user agent, such as safe-area insets.", {
    values: [v("safe-area-inset-*", OK), v("titlebar-area-*", OK, "Window Controls Overlay.")],
    aliases: ["safe-area-inset", "titlebar-area", "window controls overlay"],
  }),
  fn("attr()", NO, "Uses the value of an attribute of the element in a property value.", { related: ["content"] }),
  kw("keyword-inherit", "inherit", PART, "Makes a property take its value from the parent element.", {
    notes: [
      "Works on `border-radius` and its corners, on custom properties, and on properties that inherit anyway, such as `color` and `font-size`.",
    ],
    limitations: [
      "It does not undo a value that an earlier rule sets on the same element, and on other properties that do not inherit, such as `background-color`, it changes nothing. Neither case prints an advisory.",
    ],
    related: ["keyword-initial", "keyword-unset", "keyword-revert"],
  }),
  kw("keyword-initial", "initial", PART, "Sets a property to its initial value.", {
    notes: [
      "Works on the insets (`top`, `inset` and the others, where it is `auto`), on `text-shadow` and on custom properties.",
    ],
    limitations: ["On other properties it is not applied, with an advisory."],
    workaround: "Write the initial value itself, such as `none` or `auto`.",
    related: ["keyword-inherit", "keyword-unset", "keyword-revert"],
  }),
  kw("keyword-unset", "unset", PART, "Resets a property to `inherit` if it inherits, or to `initial` if it does not.", {
    notes: ["Works on the insets (`top`, `inset` and the others, where it is `auto`) and on custom properties."],
    limitations: ["On other properties it is not applied, with an advisory."],
    workaround: "Write the initial value itself, such as `none` or `auto`.",
    related: ["keyword-inherit", "keyword-initial", "keyword-revert"],
  }),
  kw(
    "keyword-revert",
    "revert",
    PART,
    "Rolls a property back to the value of the previous cascade origin, such as the user-agent stylesheet.",
    {
      notes: ["Works on custom properties."],
      limitations: ["On other properties it is not applied, with an advisory."],
      related: ["keyword-inherit", "keyword-initial", "keyword-unset"],
    },
  ),
  sh("all", NO, "Resets every property of an element, except `direction` and `unicode-bidi`, at once.", {
    related: ["keyword-initial", "keyword-unset", "keyword-revert"],
  }),
  fn("calc-size()", OK, "Performs calculations on intrinsic sizes such as `auto` or `fit-content`.", {
    since: "0.4.0",
    notes: [
      "Works in `width`, `height`, their `min-*` and `max-*`, the logical sizes and `flex-basis`: `calc-size(auto, size + 20px)`, `calc-size(fit-content, size * .5)`.",
      "The basis is `auto`, `min-content`, `max-content`, `fit-content`, `content` (in `flex-basis`), `any`, a length or percentage, or another `calc-size()`. Layout measures the box at its basis and `size` is that measured size.",
      "Interpolates with lengths and with another `calc-size()` of the same basis, so `height: 0` to `calc-size(auto, size)` transitions; `interpolate-size: allow-keywords` does the same for `auto`.",
    ],
    limitations: [
      "The calculation is linear in `size`: `min()`, `max()`, `clamp()` and the stepped functions around `size`, and a percentage multiplied by `size`, make the declaration invalid.",
      "A box is measured with every other `calc-size()` box in the same layout at its basis, so a `calc-size()` box inside another sees its parent's basis size.",
      "`stretch` is not a basis.",
    ],
    related: ["function-calc", "interpolate-size", "width", "height", "flex-basis"],
  }),
  fn("anchor-size()", NO, "Returns a size of an anchor element, for sizing a positioned element.", {
    related: ["function-anchor", "anchor-name"],
  }),
]);

const vendorPrefixes = section("Vendor prefixes", [
  feat(
    "feature-vendor-prefixes",
    "Vendor-prefixed properties",
    PART,
    "Browser-specific spellings of properties, such as `-webkit-transform`, kept for older engines.",
    {
      notes: [
        "Skathi reads these `-webkit-` spellings as the unprefixed property, which then has that property's status: `-webkit-align-*`, `-webkit-animation*`, `-webkit-app-region`, `-webkit-appearance`, `-webkit-backdrop-filter`, `-webkit-backface-visibility`, `-webkit-background-clip`, `-webkit-background-origin`, `-webkit-background-size`, `-webkit-border-*-radius`, `-webkit-box-reflect`, `-webkit-box-shadow`, `-webkit-box-sizing`, `-webkit-clip-path`, `-webkit-column*`, `-webkit-filter`, `-webkit-flex*`, `-webkit-font-feature-settings`, `-webkit-hyphens`, `-webkit-justify-content`, `-webkit-line-clamp`, `-webkit-order`, `-webkit-perspective*`, `-webkit-text-fill-color`, `-webkit-text-size-adjust`, `-webkit-text-stroke*`, `-webkit-transform*`, `-webkit-transition*`, `-webkit-user-select`, `-webkit-writing-mode`.",
      ],
      limitations: [
        "Every other vendor-prefixed property (`-moz-`, `-ms-`, `-o-`, and the remaining `-webkit-` ones such as `-webkit-font-smoothing` or `-webkit-tap-highlight-color`) is dropped without an advisory, as Chromium drops what it does not know.",
        "`-webkit-mask-*` is the exception: it is dropped with an advisory to use the unprefixed `mask-*`.",
        "The legacy flexbox does not lay out: `display: -webkit-box` lays out as `block`, `-webkit-box-orient`, `-webkit-box-align` and `-webkit-box-pack` are accepted and change nothing, and the other `-webkit-box-*` properties are not applied. Use `display: flex`.",
      ],
      workaround: "Write unprefixed properties.",
      aliases: [
        "-webkit-",
        "-moz-",
        "-ms-",
        "-o-",
        "prefixes",
        "-webkit-box-*",
        "-webkit-text-size-adjust",
        "-webkit-tap-highlight-color",
      ],
      related: [
        "webkit-mask",
        "webkit-font-smoothing",
        "webkit-text-fill-color",
        "webkit-text-stroke",
        "webkit-box-reflect",
      ],
    },
  ),
]);

export const entries: readonly CssEntry[] = [
  ...stylesheets,
  ...boxModel,
  ...layout,
  ...spacing,
  ...typography,
  ...color,
  ...backgrounds,
  ...effects,
  ...transforms,
  ...transitions,
  ...scrolling,
  ...interaction,
  ...svgSection,
  ...containment,
  ...generated,
  ...selectors,
  ...atRules,
  ...mediaFeatures,
  ...valuesAndUnits,
  ...vendorPrefixes,
];

export const entriesBySlug: ReadonlyMap<string, CssEntry> = new Map(entries.map((e) => [e.slug, e]));
