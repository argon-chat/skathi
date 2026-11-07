//! `lightningcss::Property` → `gpui::StyleRefinement`. The load-bearing match
//! statement: this file decides what CSS vue-native renders. Unsupported
//! declarations produce a DIAGNOSTIC, never silence (the redcss `Coverage`
//! philosophy) — a property that parses but does nothing is the worst DX bug
//! a styling system can have.

use std::rc::Rc;

use gpui::{
    px, relative, AbsoluteLength, DefiniteLength, Hsla, Length, Overflow, StyleRefinement,
    TransformLength, TransformSpec,
};
use lightningcss::declaration::DeclarationBlock;
use lightningcss::properties::align::{ContentPosition, SelfPosition};
use lightningcss::properties::border::LineStyle;
use lightningcss::properties::display::{DisplayInside, DisplayKeyword};
use lightningcss::properties::flex::FlexDirection as CssFlexDirection;
use lightningcss::properties::font::{AbsoluteFontWeight, FontFamily as CssFontFamily, FontWeight as CssFontWeight, LineHeight as CssLineHeight};
use lightningcss::properties::overflow::OverflowKeyword;
use lightningcss::properties::size::Size as CssSize;
use lightningcss::properties::text::TextAlign as CssTextAlign;
use lightningcss::properties::{Property, PropertyId};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions};
use lightningcss::properties::masking::ClipPath as CssClipPath;
use lightningcss::values::color::CssColor;
use lightningcss::values::length::{Length as CssLength, LengthPercentage, LengthPercentageOrAuto, LengthValue};
use lightningcss::values::percentage::DimensionPercentage;

use crate::keyframes::{AnimationCollector, AnimationPatch, AnimationSpec};
use crate::transition::{Collector, TransitionSpec};
use crate::vars::{self, Vars};

/// `(name, raw value text)` pairs — the transport for both channels that cannot
/// be mapped at sheet-compile time: `--x` declarations (which fold into the
/// element's [`Vars`] map) and known properties whose value referenced `var()`
/// (which are substituted and re-parsed per element).
pub type Decls = Vec<(String, String)>;

/// CSS `object-fit`, the sizing rule for a REPLACED element's content box.
///
/// It lives here rather than in a `StyleRefinement` because gpui keeps it out
/// of `Style` entirely: `ObjectFit` is a field of the `img()` element's own
/// `ImageStyle`, set through the `StyledImage` trait. A local mirror (rather
/// than `gpui::ObjectFit`) is what lets [`ComputedStates`](crate::ComputedStates)
/// stay `Clone + PartialEq` — gpui's enum derives neither.
///
/// The browser initial value is `fill`; gpui's `ImageStyle::default()` is
/// `Contain`, so the renderer must state `Fill` explicitly when no rule
/// declares one (see `vn-tree`'s `build_img`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
}

/// CSS `window-drag` — "this element drags the OS window" (docs/WINDOWING.md
/// §2.1).
///
/// The CSSWG resolved on 2026-01-21 to standardise Electron's
/// `-webkit-app-region` as `window-drag: none | move`, and separately that the
/// property is INHERITED. Chromium ships it with `app-region` and
/// `-webkit-app-region` as permanent aliases, so all three spellings are
/// accepted here and `window-drag` is canonical.
///
/// Inheritance is the one thing this type cannot express on its own: vn-css
/// has no inherited properties at all, so the fold happens at tree-build time
/// in `vn-tree`'s render pass, which carries the parent's effective value down
/// the recursion (docs/WINDOWING.md §5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowDrag {
    /// `window-drag: move` / `app-region: drag`.
    Move,
    /// `window-drag: none` / `app-region: no-drag` — the initial value, and
    /// the way a button opts out of an ancestor's drag region.
    None,
}

/// CSS `--window-control` — the deliberate extension (docs/WINDOWING.md §2.2).
///
/// The web has no word for "this element is the maximize button", and it needs
/// one because that single fact is what Windows 11 requires in order to offer
/// snap layouts (answer `WM_NCHITTEST` with `HTMAXBUTTON`).
///
/// Spelled as a CUSTOM property for the same reason `--shading` is: a browser
/// parses, stores and ignores a custom property, whereas it DROPS an unknown
/// value of a known property — and `window-drag: maximize` in a browser would
/// therefore fall back to the *inherited* `move`, making the button draggable.
/// A custom property degrades to "does nothing", which is correct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowControl {
    Minimize,
    Maximize,
    Close,
    /// `--window-control: none` — an explicit "not a caption button", so a
    /// rule can cancel one set by an earlier rule.
    None,
}

/// `user-select`, as declared. `auto`'s used value is the parent's, which
/// `vn_tree::selection` finds by walking up (CSS UI 4 §6.1). `contain`
/// selects as `text` does, with an advisory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserSelect {
    /// Declared `auto`: kept apart from "not declared" so that it can
    /// override an earlier rule's `none` in the cascade.
    Auto,
    /// No selection starts here, and a selection passing over the element
    /// neither paints on it nor copies its text.
    None,
    /// Selectable text: the explicit spelling of what `auto` usually means.
    Text,
    /// A selection that reaches into the element takes the element whole.
    All,
}

/// One `filter: drop-shadow()`, resolved.
///
/// CSS's `drop-shadow()` blurs the ALPHA of whatever the element painted and
/// lays it underneath — so for an element whose painted output is an opaque
/// rounded rect it is exactly gpui's shadow primitive, which rasterises a
/// Gaussian-blurred rounded rect with the element's own corner radii. That is
/// the same argument `filter: blur()` uses (m18), one function along.
///
/// The colour is optional because CSS defaults it to the element's `color`,
/// which the mapper does not resolve; the renderer substitutes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropShadow {
    /// Horizontal offset, px.
    pub dx: f32,
    /// Vertical offset, px.
    pub dy: f32,
    /// The Gaussian's **sigma**, px. CSS's third length and gpui's
    /// `blur_radius` are both sigmas, so it transfers 1:1.
    pub blur: f32,
    /// `None` = `currentColor`, resolved by the renderer.
    pub color: Option<gpui::Hsla>,
}

/// Which corners' `border-*-radius` is `inherit`, in cascade order.
///
/// `border-radius` does not inherit, so `inherit` asks for the PARENT's
/// computed radius, which the mapper cannot know. The value is filled in after
/// the cascade, from the parent (`vn_tree`'s restyle walk calls
/// [`crate::resolve_inherited_radii`]) or, for `::before`/`::after`, from the
/// element the box belongs to. `declared` keeps the cascade honest: a later
/// concrete radius on a corner cancels an earlier `inherit`, and a later
/// `inherit` beats an earlier length.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RadiusInherit {
    /// Corners some declaration wrote, length or `inherit`.
    pub declared: u8,
    /// Of those, the ones whose last declaration was `inherit`.
    pub inherit: u8,
}

impl RadiusInherit {
    pub const TOP_LEFT: u8 = 1;
    pub const TOP_RIGHT: u8 = 2;
    pub const BOTTOM_RIGHT: u8 = 4;
    pub const BOTTOM_LEFT: u8 = 8;
    pub const ALL: u8 = 15;

    pub(crate) fn write(&mut self, corners: u8, inherit: bool) {
        self.declared |= corners;
        if inherit {
            self.inherit |= corners;
        } else {
            self.inherit &= !corners;
        }
    }

    /// Later-wins per corner.
    pub(crate) fn refine(&mut self, later: &RadiusInherit) {
        self.inherit = (self.inherit & !later.declared) | (later.inherit & later.declared);
        self.declared |= later.declared;
    }
}

/// Computed values that a `gpui::StyleRefinement` has nowhere to carry.
///
/// The refinement is the cascade's whole payload for every other property, so
/// this is deliberately tiny and `Copy`: one folded value per node, refined in
/// the same (specificity, order) sweep as the refinements themselves.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    /// `object-fit` — only meaningful on a replaced element (`<img>`); parsed
    /// and folded for every element, exactly as a browser does.
    pub object_fit: Option<ObjectFit>,
    /// `window-drag` / `app-region` / `-webkit-app-region`. `None` here means
    /// "this element declared nothing", which is what lets the renderer
    /// inherit the parent's effective value — not the same as
    /// `Some(WindowDrag::None)`, which is an explicit opt-out.
    pub window_drag: Option<WindowDrag>,
    /// `--window-control`. Never inherited (D-14).
    pub window_control: Option<WindowControl>,
    /// `position: fixed`. gpui/taffy have one out-of-flow position (`Absolute`,
    /// resolved against the element's PARENT), so `fixed` maps onto it and the
    /// renderer additionally paints the node in a deferred layer so it lands
    /// above its siblings. See the advisory in the `Position` arm.
    pub fixed: Option<bool>,
    /// `filter: drop-shadow(<dx> <dy> <blur>? <color>?)`, recorded for the one
    /// shape gpui can draw exactly — see [`DropShadow`] and
    /// `vn_tree::render::apply_drop_shadow`.
    pub drop_shadow: Option<DropShadow>,
    /// `position: sticky`, which maps onto `relative` — taffy has no way to
    /// take a scroll offset as a LAYOUT input. Whether that is a divergence
    /// depends on whether anything above the element actually scrolls, which
    /// only the renderer knows (`vn_tree::render::advise_out_of_flow`).
    pub sticky: Option<bool>,
    /// `z-index`: `None` = not declared, `Some(None)` = `auto`, `Some(Some(n))`
    /// = an integer. [`Self::z`] is the integer, if there is one.
    ///
    /// `auto` and `0` are different values. Both paint in the same layer
    /// (CSS 2.1 Appendix E, step 8), but where it applies an integer — `0`
    /// included — makes the box a STACKING CONTEXT, which keeps its
    /// descendants' own `z-index`es inside it, and `auto` does not. The
    /// renderer turns the value into a paint order (`vn_tree::render::Stacking`).
    ///
    /// Recorded wherever it is declared. Whether it APPLIES is the renderer's
    /// question (`vn_tree::render::z_applies`). It needs [`Self::positioned`]
    /// and the parent's display, and the mapper sees neither.
    pub z_index: Option<Option<i32>>,
    /// `isolation`: `Some(true)` for `isolate`, `Some(false)` for `auto`. An
    /// isolated box is a stacking context (CSS Compositing 1 §3.4.1), the
    /// lever an author pulls to keep a `z-index: -1` layer from sinking behind
    /// the box's own background (Tailwind's `isolate`).
    pub isolate: Option<bool>,
    /// The element DECLARED a `position` other than `static`: `Some(true)` for
    /// `relative` / `absolute` / `fixed` / `sticky`, `Some(false)` for
    /// `static`, `None` for no declaration (static too). gpui's own `position`
    /// cannot say this, because `static` and `relative` both map onto its
    /// `Relative`, which is also its default.
    ///
    /// CSS needs the difference for `z-index`, which applies only to a
    /// positioned box or a flex or grid item (CSS 2.1 §9.9.1, css-flexbox
    /// §4.3, css-grid §9). `z-10` on a static block is inert in a browser.
    pub positioned: Option<bool>,
    /// `pointer-events: none` — the node takes no listeners and no hover/active
    /// rungs, which is how a hitbox stops existing in gpui.
    ///
    /// INHERITED, like the CSS property. The cascade folds only the element's
    /// own declaration; `vn_tree::tree::restyle_with` threads the effective
    /// value down the document-order walk and rewrites this to `Some(effective)`
    /// before the states are stored, so everything downstream (the renderer, the
    /// devtools' `getComputedStyle`) reads one answer.
    pub pointer_events_none: Option<bool>,
    /// The `visibility` the element DECLARED: `Some(true)` for `hidden` or
    /// `collapse`, `Some(false)` for `visible`, `None` when nothing declared it.
    ///
    /// The refinement's own `visibility` is gpui's paint switch, and the mapper
    /// also turns it off for the screen-reader-only idiom
    /// `clip: rect(0, 0, 0, 0)`. Its box paints nothing, as one under the
    /// other idiom, `clip-path: inset(50%)`, a real clip to nothing, does,
    /// but its CSS `visibility` is still `visible`, and that is what
    /// a browser asks before `el.focus()` and answers from `getComputedStyle`
    /// (`vn_tree::render::css_visible`). The property is inherited, so the
    /// nearest declaration on the element or an ancestor decides.
    pub visibility_hidden: Option<bool>,
    /// `user-select`; `None` when nothing declared it. NOT folded down the
    /// tree: `auto` defers to the parent at the moment a selection asks
    /// (`vn_tree::selection`), which is the used-value rule.
    pub user_select: Option<UserSelect>,
    /// The element's `display` had an INLINE outer type (`inline`,
    /// `inline-block`, `inline-flex`, `inline-grid`).
    ///
    /// taffy models the inner display type only, so the box is laid out as its
    /// block-level equivalent — but a block-level box ignores the `text-align`
    /// that would have centred it, which is exactly how the meet app's hero logo
    /// (an `inline-flex` badge in a `text-center` column) ended up flush left
    /// while its heading was centred. The renderer reads this flag together with
    /// the inherited `text-align` and centres the box with auto margins, which
    /// is the block-layout spelling of the same intent.
    pub inline_level: Option<bool>,
    /// `appearance: none` (`Some(true)`) or any other value (`Some(false)`).
    /// The one native skin here is `<select>`'s: its arrow, and the padding
    /// Chromium keeps for it, which `none` takes away (vn-tree `select.rs`).
    pub appearance_none: Option<bool>,
    /// Native control tint; Some(None) resets an earlier color to auto.
    pub accent_color: Option<Option<Hsla>>,
    /// SVG `fill` — see [`SvgPaint`].
    pub fill: Option<SvgPaint>,
    /// SVG `stroke` — see [`SvgPaint`].
    pub stroke: Option<SvgPaint>,
    /// The other SVG presentation properties a rule sets (`stroke-width`,
    /// `stroke-opacity`, `fill-opacity`, the dash and join properties,
    /// `fill-rule`, `shape-rendering`), by name, as CSS text. Only an `<svg>`
    /// root reads them: the serializer writes them on the document's root, in
    /// place of the markup's own attribute of that name, where the shapes
    /// inherit them as they inherit the computed value in a browser. Merged
    /// per name across rules ([`Extras::refine`]).
    pub svg_presentation: Option<Rc<Vec<(&'static str, Rc<str>)>>>,
    /// `text-transform`. gpui has no such text-style field: the shaper is
    /// handed a string, so the case change is applied to that string by the
    /// renderer. INHERITED, like the CSS property — the renderer threads the
    /// effective value down the same way it threads `window-drag`.
    pub text_transform: Option<TextTransform>,
    /// `white-space`, as CSS means it — the WHITESPACE PROCESSING half.
    ///
    /// `r.text.white_space` beside it is gpui's, and gpui's has exactly one
    /// job: may a line WRAP. The other half of the property — which of the
    /// source's spaces, tabs and newlines survive into the rendered text — is
    /// not something gpui models at all: it shapes whatever string it is
    /// handed, and it breaks a line at every `\n` in it regardless.
    ///
    /// So the collapsing is the renderer's, applied to the string, exactly
    /// like [`Extras::text_transform`] — and this is the value it needs. It
    /// INHERITS, like the CSS property.
    pub white_space: Option<WhiteSpaceMode>,
    /// `content`, the string a `::before` / `::after` rule generates its box
    /// with. Meaningful ONLY on a pseudo-element rule: on an ordinary element
    /// CSS ignores the property outright, and so does the renderer.
    ///
    /// `Rc<str>` is why [`Extras`] is `Clone` rather than `Copy`; the clone is
    /// a refcount bump and the field is `None` on all but a handful of rules.
    pub content: Option<Rc<str>>,
    /// `background-clip: text` (or its `-webkit-` spelling). The renderer
    /// retains the background stack as a brush and masks it with glyph alpha.
    pub background_clip_text: Option<bool>,
    /// `-webkit-text-fill-color` overrides foreground `color`, independently
    /// of the background clipped behind the text.
    pub text_fill_color: Option<TextFill>,
    /// Representative background metadata retained for style inspection.
    /// Text clipping uses the complete background stack, not this colour.
    pub background_key_color: Option<gpui::Hsla>,
    /// The `background-image` layers, in CSS order — layer 0 paints on TOP.
    ///
    /// Held here rather than written straight onto the `StyleRefinement`
    /// because the geometry that places them (`background-size`,
    /// `background-position`, `background-repeat`) is three SEPARATE
    /// properties, and Tailwind puts each in a rule of its own: the meet app's
    /// page grid is `bg-[linear-gradient(…),linear-gradient(…)]` on one class
    /// and `bg-[size:64px_64px]` on another. Neither declaration can see the
    /// other, so the fold happens once, afterwards, in
    /// [`Extras::resolve_background_layers`].
    pub background_images: Option<Rc<Vec<BackgroundLayer>>>,
    /// The `url()` layers of the same `background-image` list, next to their
    /// CSS layer indices. Always written together with
    /// [`Extras::background_images`] (an empty list when the declaration has no
    /// `url()`), so the two halves of one declaration cascade as one.
    pub background_urls: Option<Rc<Vec<BackgroundUrl>>>,
    /// `background-size`, per layer, in CSS order.
    pub background_size: Option<Rc<Vec<BackgroundSize>>>,
    /// `background-position`, per layer, in CSS order.
    pub background_position: Option<Rc<Vec<(TileLength, TileLength)>>>,
    /// `background-repeat`, per layer, in CSS order, as `(x, y)`.
    pub background_repeat: Option<Rc<Vec<(bool, bool)>>>,
    /// `list-style-type` — the marker an `<li>` under this element gets.
    ///
    /// INHERITED, like the CSS property, and folded the way `text-transform` is:
    /// vn-css folds no inherited property, so `vn_tree::render` reads the
    /// nearest ancestor that declared one when it builds an `<li>`. `None` means
    /// this element declared nothing; `Some(ListMarker::None)` is an explicit
    /// `list-style: none`, which is what stops a marker being generated.
    pub list_marker: Option<ListMarker>,
    /// `list-style-position: inside`. Also inherited.
    pub list_marker_inside: Option<bool>,
    /// The web fonts this element's `font-family` asks for that are not loaded
    /// yet: the families of the list, up to the one used, that
    /// [`set_web_font_families`] named, in order. The host loads the faces of
    /// the first one that covers the text drawn with them, then the next for
    /// what it leaves (fonts load lazily, as in a browser). INHERITED with
    /// `font-family`: `None` means this element declared no `font-family`, and
    /// an empty list that it declared one that asks for nothing to load.
    /// Always `None` while no web font is waiting.
    pub web_font: Option<Rc<[String]>>,
    /// `text-underline-offset`, in px.
    ///
    /// The offset lands on the decoration itself (gpui's `UnderlineStyle`
    /// carries it), but the two properties can arrive from different rules —
    /// Tailwind's `.underline` and `.underline-offset-4` are two classes — so
    /// the value is also kept here and applied to whatever underline the whole
    /// cascade ended up with.
    pub underline_offset: Option<f32>,
    /// Which properties on this element were written as `currentColor`.
    ///
    /// `currentColor` is the element's computed `color`, and vn-css folds no
    /// inherited text property (gpui's `TextStyleRefinement` cascades at paint
    /// time), so the value is not known while a declaration is being mapped —
    /// only after the whole cascade has run and the parent's colour is in hand.
    /// [`Engine::compute_node`] does the substitution; this records what to
    /// substitute into.
    pub current_color: CurrentColorTargets,
    /// Border declarations clear an earlier currentColor on the affected side.
    pub current_color_overrides: CurrentColorTargets,
    /// `filter: blur(<radius>)` — the radius, in px.
    ///
    /// Blur is not a general post-processing pass; SVG color matrices use
    /// an isolated compositor separately. This exists because ONE
    /// shape of blurred element is both extremely common and exactly
    /// expressible with what gpui already has: a leaf box with a solid
    /// background and no content — the decorative "glow blob" every modern
    /// landing page builds its ambient lighting out of
    /// (`bg-primary/5 rounded-full blur-3xl`).
    ///
    /// A Gaussian-blurred solid rounded rect is *precisely* what gpui's drop
    /// shadow primitive rasterises, down to the corner radii, so for that shape
    /// the renderer repaints the element as its own shadow and the result is
    /// not an approximation at all. `crate::render` decides; anything with
    /// children, text or a gradient keeps the advisory and stays sharp.
    pub blur: Option<f32>,
    /// A same-document SVG filter, resolved against the retained DOM at paint time.
    pub filter_url: Option<Rc<str>>,
    /// `scrollbar-width: auto | thin | none` — how wide the overlay scrollbar
    /// this element's `overflow` produces should be, or that it should not be
    /// drawn at all.
    ///
    /// NOT inherited here, where CSS says it is not either. `scrollbar-color`
    /// beside it IS inherited in CSS and is not here — see
    /// [`Extras::scrollbar_color`].
    pub scrollbar_width: Option<ScrollbarWidth>,
    /// `scrollbar-color: <thumb> <track>`.
    ///
    /// **Not inherited (v1 divergence.)** In CSS this is an inherited property,
    /// so `html { scrollbar-color: … }` recolours every scroller on the page;
    /// here it has to be declared on the scroll container itself. vn-css folds
    /// no inherited property — the two that behave as if they do
    /// (`pointer-events`, `window-drag`) are threaded down by the renderer —
    /// and a colour pair is not worth a third thread until something asks.
    pub scrollbar_color: Option<(gpui::Hsla, gpui::Hsla)>,
    /// `overscroll-behavior-x`, the horizontal half of what a wheel at this
    /// box's edge does to its ancestors.
    pub overscroll_x: Option<OverscrollBehavior>,
    /// `overscroll-behavior-y`.
    pub overscroll_y: Option<OverscrollBehavior>,
    /// The `overflow-x` keyword, which the refinement's `gpui::Overflow` cannot
    /// spell: it folds `auto` into `Scroll` and `clip` into `Hidden`. Only a
    /// QUALIFIER of the refinement's axis, which stays the truth: see
    /// [`compute_overflow`] and [`Extras::overflow_names`].
    pub overflow_x: Option<OverflowKeyword>,
    /// The `overflow-y` keyword, as [`Self::overflow_x`].
    pub overflow_y: Option<OverflowKeyword>,
    /// `mask-image` (or the `mask` shorthand's images). The three `mask_*`
    /// fields are kept apart because they cascade apart, and
    /// [`Extras::mask_ring`] reads them together.
    pub mask_images: Option<MaskImages>,
    /// `mask-clip` (or the shorthand's boxes).
    pub mask_clips: Option<MaskClips>,
    /// `mask-composite` / `-webkit-mask-composite` (or the shorthand's), for
    /// the top layer, which is the only one composited onto anything.
    pub mask_composite: Option<MaskCompositing>,
    /// `border-radius: inherit` and its longhands — see [`RadiusInherit`].
    pub radius_inherit: RadiusInherit,
}

/// `mask-image`, as far as the paint path can tell layers apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskImages {
    /// `none`: no mask.
    None,
    /// Exactly two layers, each a gradient whose every stop is opaque, so it
    /// covers its whole box: the `linear-gradient(#fff 0 0)` pair of the
    /// gradient-border idiom.
    TwoOpaque,
    /// Anything else (a `url()`, a fade, one layer, three).
    Other,
}

/// `mask-clip`, as far as the gradient-border idiom needs it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskClips {
    /// The top layer clipped to the content box, the second to the border box
    /// (the initial value).
    ContentThenBorder,
    /// Anything else.
    Other,
}

/// The top mask layer's compositing operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskCompositing {
    /// `add` / `source-over`, the initial value: the layers' union.
    Add,
    /// `exclude` / `xor`, and `destination-out` (the layers below, minus this
    /// one). Over the idiom's pair, all three leave the border box minus the
    /// content box.
    Exclude,
    /// Anything else.
    Other,
}

/// CSS `scrollbar-width`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarWidth {
    /// The platform's own width — 14 px here, which is what Windows 11's
    /// overlay scrollbar occupies when it is not hovered.
    Auto,
    /// A narrower bar. CSS gives no number; browsers pick something around
    /// half, and so do we (8 px).
    Thin,
    /// No scrollbar at all. The box still scrolls — this is the property that
    /// makes a custom scrollbar possible, and every carousel uses it.
    None,
}

/// CSS `overscroll-behavior`, per axis.
///
/// It answers one question: when a wheel arrives at a scroll box that is
/// already at its edge on that axis, does the ANCESTOR scroll?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverscrollBehavior {
    /// The initial value: the scroll chains to the ancestor. (The other half
    /// of `auto` — the platform's rubber-band bounce — has nothing to do here,
    /// because there is no bounce to suppress.)
    Auto,
    /// The scroll stops at this box. `none` additionally suppresses the bounce,
    /// which is the only way it differs from `contain` on a platform that has
    /// one; here the two behave identically and honestly so.
    Contain,
    /// See [`OverscrollBehavior::Contain`].
    None,
}

impl OverscrollBehavior {
    /// Does a wheel that this box cannot use reach its ancestors?
    pub fn chains(self) -> bool {
        matches!(self, Self::Auto)
    }
}

/// CSS `text-transform`, applied to the string before it reaches the shaper.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

/// CSS `white-space`, at the level the renderer needs it.
///
/// The spec decomposes the property into `white-space-collapse` and
/// `text-wrap`; these six are the legacy keywords everyone writes, and each
/// answers the two questions the renderer asks: are runs of white space
/// COLLAPSED to one space, and are source NEWLINES preserved as line breaks.
///
/// | keyword        | collapses spaces | keeps newlines | wraps |
/// |----------------|------------------|----------------|-------|
/// | `normal`       | yes              | no             | yes   |
/// | `nowrap`       | yes              | no             | no    |
/// | `pre`          | no               | yes            | no    |
/// | `pre-wrap`     | no               | yes            | yes   |
/// | `pre-line`     | yes              | yes            | yes   |
/// | `break-spaces` | no               | yes            | yes   |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhiteSpaceMode {
    Normal,
    Nowrap,
    Pre,
    PreWrap,
    PreLine,
    BreakSpaces,
}

impl WhiteSpaceMode {
    /// Is a RUN of white space collapsed to a single space?
    pub fn collapses_spaces(self) -> bool {
        matches!(self, WhiteSpaceMode::Normal | WhiteSpaceMode::Nowrap | WhiteSpaceMode::PreLine)
    }

    /// Is a source newline a line BREAK, or just more white space?
    pub fn preserves_newlines(self) -> bool {
        !matches!(self, WhiteSpaceMode::Normal | WhiteSpaceMode::Nowrap)
    }
}

impl TextTransform {
    /// Applies the case change. Returns `None` when the text is already in the
    /// right shape, so the renderer can keep borrowing the original.
    pub fn apply(self, text: &str) -> Option<String> {
        match self {
            TextTransform::None => None,
            TextTransform::Uppercase => Some(text.to_uppercase()),
            TextTransform::Lowercase => Some(text.to_lowercase()),
            // CSS capitalises the first letter of each WORD and leaves the rest
            // of the word exactly as authored (it does not lowercase it).
            TextTransform::Capitalize => {
                let mut out = String::with_capacity(text.len());
                let mut at_word_start = true;
                for ch in text.chars() {
                    if at_word_start && ch.is_alphabetic() {
                        out.extend(ch.to_uppercase());
                        at_word_start = false;
                    } else {
                        out.push(ch);
                        if ch.is_whitespace() {
                            at_word_start = true;
                        }
                    }
                }
                Some(out)
            }
        }
    }
}

/// `-webkit-text-fill-color`'s value (see [`Extras::text_fill_color`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextFill {
    /// `currentcolor`, the initial value: the glyphs take `color`.
    CurrentColor,
    /// A colour that replaces `color` for the glyphs.
    Color(Hsla),
}

/// An SVG `fill` / `stroke` as the cascade computed it.
///
/// vn-tree serializes an inline `<svg>` into a document for resvg and writes
/// each element's computed paint on it in place of the markup's attribute
/// (`vn_tree::svg`), so a concrete colour from a rule, `var()` resolved, is
/// drawn as that colour. A document whose paints are all `currentColor` takes
/// gpui's monochrome mask, tinted by `color`; anything else is rasterised in
/// full colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SvgPaint {
    /// `none` — this half of the geometry is not painted.
    None,
    /// `currentColor` — painted in the cascade's `color`.
    Current,
    /// A concrete colour, its alpha included.
    Color(Hsla),
    /// A paint this engine does not carry: a paint server (`url(…)`),
    /// `context-fill` / `context-stroke`, or a colour the mapper cannot
    /// resolve. Drawn in `currentColor`, and vn-tree says so.
    Server,
}

impl SvgPaint {
    /// How this paint is spelled in the serialized SVG the renderer feeds
    /// resvg: `none`, `currentColor`, or `rgb(r,g,b)` with the alpha as
    /// `rgba` when it is not opaque (resvg folds a colour's alpha into the
    /// paint's opacity, as a browser does).
    pub fn as_attr(self) -> std::borrow::Cow<'static, str> {
        match self {
            SvgPaint::None => "none".into(),
            SvgPaint::Current | SvgPaint::Server => "currentColor".into(),
            SvgPaint::Color(c) => {
                let c = gpui::Rgba::from(c);
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let (r, g, b) = (q(c.r), q(c.g), q(c.b));
                let a = (c.a.clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
                if a >= 1.0 {
                    format!("rgb({r},{g},{b})").into()
                } else {
                    format!("rgba({r},{g},{b},{a})").into()
                }
            }
        }
    }
}

impl Extras {
    /// Later-wins merge, matching `Refineable::refine` for the refinements.
    pub(crate) fn refine(&mut self, other: &Extras) {
        if other.object_fit.is_some() {
            self.object_fit = other.object_fit;
        }
        if other.window_drag.is_some() {
            self.window_drag = other.window_drag;
        }
        if other.window_control.is_some() {
            self.window_control = other.window_control;
        }
        if other.fixed.is_some() {
            self.fixed = other.fixed;
        }
        if other.appearance_none.is_some() {
            self.appearance_none = other.appearance_none;
        }
        if other.accent_color.is_some() {
            self.accent_color = other.accent_color;
        }
        if other.z_index.is_some() {
            self.z_index = other.z_index;
        }
        if other.isolate.is_some() {
            self.isolate = other.isolate;
        }
        if other.positioned.is_some() {
            self.positioned = other.positioned;
        }
        if other.pointer_events_none.is_some() {
            self.pointer_events_none = other.pointer_events_none;
        }
        if other.visibility_hidden.is_some() {
            self.visibility_hidden = other.visibility_hidden;
        }
        if other.user_select.is_some() {
            self.user_select = other.user_select;
        }
        if other.inline_level.is_some() {
            self.inline_level = other.inline_level;
        }
        if other.fill.is_some() {
            self.fill = other.fill;
        }
        if other.stroke.is_some() {
            self.stroke = other.stroke;
        }
        if other.white_space.is_some() {
            self.white_space = other.white_space;
        }
        if other.text_transform.is_some() {
            self.text_transform = other.text_transform;
        }
        if other.content.is_some() {
            self.content = other.content.clone();
        }
        // `blur` is `Some` for every `filter` declaration, and the
        // `drop-shadow()` is the same declaration's: a later `filter` replaces
        // both (`filter: none` takes an earlier shadow away).
        if other.blur.is_some() {
            self.blur = other.blur;
            self.drop_shadow = other.drop_shadow;
            self.filter_url = other.filter_url.clone();
        }
        if other.background_clip_text.is_some() {
            self.background_clip_text = other.background_clip_text;
        }
        if other.text_fill_color.is_some() {
            self.text_fill_color = other.text_fill_color;
        }
        if let Some(theirs) = &other.svg_presentation {
            self.svg_presentation = Some(match self.svg_presentation.take() {
                None => theirs.clone(),
                Some(mine) => {
                    let mut merged = (*mine).clone();
                    for (name, value) in theirs.iter() {
                        set_presentation(&mut merged, name, value.clone());
                    }
                    Rc::new(merged)
                }
            });
        }
        if other.background_key_color.is_some() {
            self.background_key_color = other.background_key_color;
        }
        if other.background_images.is_some() {
            self.background_images = other.background_images.clone();
            self.background_urls = other.background_urls.clone();
        }
        if other.background_size.is_some() {
            self.background_size = other.background_size.clone();
        }
        if other.background_position.is_some() {
            self.background_position = other.background_position.clone();
        }
        if other.background_repeat.is_some() {
            self.background_repeat = other.background_repeat.clone();
        }
        if other.list_marker.is_some() {
            self.list_marker = other.list_marker;
        }
        if other.list_marker_inside.is_some() {
            self.list_marker_inside = other.list_marker_inside;
        }
        if other.web_font.is_some() {
            self.web_font = other.web_font.clone();
        }
        if other.underline_offset.is_some() {
            self.underline_offset = other.underline_offset;
        }
        if other.scrollbar_width.is_some() {
            self.scrollbar_width = other.scrollbar_width;
        }
        if other.scrollbar_color.is_some() {
            self.scrollbar_color = other.scrollbar_color;
        }
        if other.overscroll_x.is_some() {
            self.overscroll_x = other.overscroll_x;
        }
        if other.overscroll_y.is_some() {
            self.overscroll_y = other.overscroll_y;
        }
        if other.overflow_x.is_some() {
            self.overflow_x = other.overflow_x;
        }
        if other.overflow_y.is_some() {
            self.overflow_y = other.overflow_y;
        }
        if other.mask_images.is_some() {
            self.mask_images = other.mask_images;
        }
        if other.mask_clips.is_some() {
            self.mask_clips = other.mask_clips;
        }
        if other.mask_composite.is_some() {
            self.mask_composite = other.mask_composite;
        }
        self.current_color.0 = (self.current_color.0 & !other.current_color_overrides.0)
            | other.current_color.0;
        self.current_color_overrides.0 |= other.current_color_overrides.0;
        self.radius_inherit.refine(&other.radius_inherit);
    }

    /// The `mask` this element ends up with is the gradient-border idiom: two
    /// opaque layers, the top one clipped to the content box, composited with
    /// `exclude`. What it leaves visible is the ring of padding and border,
    /// which gpui paints with `Style::vn_mask_ring`. Any other mask is not
    /// applied (v1), and the mapper says so where it is declared.
    pub fn mask_ring(&self) -> bool {
        self.mask_images == Some(MaskImages::TwoOpaque)
            && self.mask_clips == Some(MaskClips::ContentThenBorder)
            && self.mask_composite == Some(MaskCompositing::Exclude)
    }

    /// Folds the `background-image` stack together with the three geometry
    /// properties into the layer list gpui paints, BOTTOM layer first.
    ///
    /// CSS matches each geometry list to the image list by index, repeating the
    /// shorter one — `background: url(a), url(b) / background-size: 4px` sizes
    /// both. A layer with no `background-size` is not tiled at all, which is
    /// the same thing as a tile the size of the box.
    pub fn resolve_background_layers(&self) -> Vec<gpui::Background> {
        let Some(images) = self.background_images.as_ref() else {
            return Vec::new();
        };
        let mut out: Vec<gpui::Background> =
            images.iter().map(|layer| self.tiled_gradient(layer)).collect();
        // CSS paints layer 0 on top; gpui paints the list in order.
        out.reverse();
        out
    }

    /// The whole background stack, BOTTOM layer first, when it holds at least
    /// one `url()` image — `None` when it is gradients only, which gpui paints
    /// on its own ([`Extras::resolve_background_layers`]).
    ///
    /// An image is not a gpui `Background`: vn-tree loads it through the same
    /// loader `<img>` uses and paints it itself, under the content. Once one
    /// layer has to be painted there, they all are, so a gradient written ABOVE
    /// an image (the darkened-banner idiom, `linear-gradient(#0008, #0008),
    /// url(banner.png)`) still paints above it.
    pub fn resolve_background_stack(&self) -> Option<Vec<PaintLayer>> {
        let urls = self.background_urls.as_ref().filter(|u| !u.is_empty())?;
        let mut stack: Vec<(u16, PaintLayer)> = Vec::new();
        for layer in self.background_images.iter().flat_map(|l| l.iter()) {
            stack.push((layer.index, PaintLayer::Gradient(self.tiled_gradient(layer))));
        }
        for u in urls.iter() {
            let i = u.index as usize;
            stack.push((
                u.index,
                PaintLayer::Image {
                    url: u.url.clone(),
                    size: cycled(self.background_size.as_ref(), i)
                        .unwrap_or(BackgroundSize::Explicit(None, None)),
                    position: cycled(self.background_position.as_ref(), i)
                        .unwrap_or((TileLength::Fraction(0.0), TileLength::Fraction(0.0))),
                    repeat: cycled(self.background_repeat.as_ref(), i).unwrap_or((true, true)),
                },
            ));
        }
        // CSS order (layer 0 on top), stable so a banded gradient keeps its
        // bands in order — then bottom first, as gpui paints.
        stack.sort_by_key(|(index, _)| *index);
        Some(stack.into_iter().rev().map(|(_, layer)| layer).collect())
    }

    /// One gradient layer with its `background-size` / `-position` /
    /// `-repeat` folded on. A layer with no explicit two-axis size is not
    /// tiled: a gradient has no intrinsic size, so `auto`, `cover` and
    /// `contain` all resolve to the box itself.
    fn tiled_gradient(&self, layer: &BackgroundLayer) -> gpui::Background {
        let index = layer.index as usize;
        let mut background = layer.background;
        let size = cycled(self.background_size.as_ref(), index);
        if let Some(BackgroundSize::Explicit(Some(w), Some(h))) = size {
            let position = cycled(self.background_position.as_ref(), index)
                .unwrap_or((TileLength::Fraction(0.0), TileLength::Fraction(0.0)));
            let repeat = cycled(self.background_repeat.as_ref(), index).unwrap_or((true, true));
            background = background.tiled(
                (w.value(), h.value()),
                (w.is_fraction(), h.is_fraction()),
                (position.0.value(), position.1.value()),
                (position.0.is_fraction(), position.1.is_fraction()),
                repeat,
            );
        }
        background
    }

    /// The declared integer `z-index`; `None` for `auto` and for no
    /// declaration, which are the same thing.
    pub fn z(&self) -> Option<i32> {
        self.z_index.flatten()
    }

    /// True when this node MAY be painted in gpui's deferred layer:
    /// `position: fixed`, or a non-zero `z-index` that was declared. A superset
    /// of what the renderer defers, because whether a `z-index` applies depends
    /// on the parent's display, which only the renderer sees
    /// (`vn_tree::render::layer_of`). vn-tree's cache uses it to refuse a
    /// boundary, where being conservative is safe. (`position: absolute`, the
    /// other deferred box, the cache refuses on its own.)
    pub fn may_defer(&self) -> bool {
        self.z().is_some_and(|z| z != 0) || self.fixed == Some(true)
    }
}

/// One declaration block, split into the normal/important refinements and the
/// transition lists each half declares (`None` = the half declared none, which
/// is what lets a later rule's `transition` override an earlier one without an
/// empty list being mistaken for `transition: none`).
pub(crate) struct Converted {
    pub normal: Option<Rc<StyleRefinement>>,
    pub important: Option<Rc<StyleRefinement>>,
    pub normal_transitions: Option<Rc<Vec<TransitionSpec>>>,
    pub important_transitions: Option<Rc<Vec<TransitionSpec>>>,
    pub normal_transition_declarations: Option<Rc<Collector>>,
    pub important_transition_declarations: Option<Rc<Collector>>,
    /// The folded `animation` list each half declares (`None` = none declared,
    /// which is what lets a later rule's `animation` override an earlier one
    /// without an empty list being mistaken for `animation: none`).
    pub normal_animations: Option<Rc<Vec<AnimationSpec>>>,
    pub important_animations: Option<Rc<Vec<AnimationSpec>>>,
    /// Longhand-only `animation-*` declarations — see [`AnimationPatch`].
    pub normal_animation_patch: Option<AnimationPatch>,
    pub important_animation_patch: Option<AnimationPatch>,
    pub normal_extras: Extras,
    pub important_extras: Extras,
    /// `--x` declarations, in source order (`None` = the half declared none).
    pub normal_custom: Option<Rc<Decls>>,
    pub important_custom: Option<Rc<Decls>>,
    /// Known properties whose value referenced `var()`.
    pub normal_dynamic: Option<Rc<Decls>>,
    pub important_dynamic: Option<Rc<Decls>>,
    /// The font-relative declarations, and every static one after the first
    /// of them, applied again per element ([`FontDecls`]).
    pub normal_font: Option<FontDecls>,
    pub important_font: Option<FontDecls>,
    /// The advisories this block's DECLARATIONS produced.
    ///
    /// Kept with the rule instead of being logged at sheet-compile time,
    /// because a compile-time advisory describes the STYLESHEET and what a
    /// developer needs to know is what is wrong with the PAGE. A generated
    /// Tailwind sheet ships a utility for every property in CSS, so compiling
    /// one reported `resize`, `table-layout` and `text-indent` — none of which
    /// any element on any screen was using. `Engine::compute_node` emits these
    /// the first time the rule actually matches an element, and
    /// `vn_tree::tree::warn_css` dedupes from there.
    ///
    /// SELECTOR-level advisories stay at compile time: a rule that was dropped
    /// for an unimplementable selector can never match, so it could never
    /// report itself.
    pub diagnostics: Option<Rc<Vec<String>>>,
}

/// A block half's font-relative declarations as parsed properties, from the
/// first one that read an em (or `ch`, `ex`, a `font-size` percentage…) to
/// the end of the half, static declarations only. The per-element fold
/// applies them over the refinement the block compiled to, under the element's
/// font size (see "em: the element's own font size" above). The static ones
/// after the first font-relative one ride along so that a later
/// `padding-left: 4px` still beats an earlier `padding: 1em`.
pub type FontDecls = Rc<Vec<Property<'static>>>;

/// One converted declaration half.
#[derive(Default)]
struct Half {
    refinement: Option<Rc<StyleRefinement>>,
    transitions: Option<Rc<Vec<TransitionSpec>>>,
    transition_declarations: Option<Rc<Collector>>,
    animations: Option<Rc<Vec<AnimationSpec>>>,
    animation_patch: Option<AnimationPatch>,
    extras: Extras,
    custom: Option<Rc<Decls>>,
    dynamic: Option<Rc<Decls>>,
    font: Option<FontDecls>,
}

/// Applies one static declaration, and keeps it for the per-element pass
/// once the half has had a font-relative one ([`FontDecls`]).
fn apply_static<'i>(
    p: &Property<'i>,
    r: &mut StyleRefinement,
    x: &mut Extras,
    font: &mut Option<Vec<Property<'static>>>,
    diagnostics: &mut Vec<String>,
) {
    use lightningcss::traits::IntoOwned as _;
    let ((), relative) = noting_font_units(|| apply(p, r, x, diagnostics));
    if relative || font.is_some() {
        font.get_or_insert_with(Vec::new).push(p.clone().into_owned());
    }
}

/// Which channel a declaration travels on.
enum Channel {
    /// `--x: …` — folds into the element's custom-property map instead of into
    /// a refinement. (`--shading` is the one exception; see [`effect`].)
    Custom(String, String),
    /// A known property whose value referenced `var()` or `env()`. lightningcss
    /// hands both back as `Property::Unparsed`; they are substituted and
    /// re-parsed per element, because a `var()` depends on where the element
    /// sits and an `env()` on the live window state.
    Dynamic(String, String),
    /// Everything else — mapped once, here.
    Static,
}

/// The raw value text of a declaration, recovered by printing it back out.
/// `Property`'s own `ToCss` writes `name: value`, and a property name can never
/// contain `:`, so the split is exact.
fn value_text(p: &Property) -> Option<String> {
    let css = p.to_css_string(false, PrinterOptions::default()).ok()?;
    let (_, value) = css.split_once(':')?;
    Some(value.trim().to_string())
}

/// Physical inset sides reset by `initial` / `unset` (top, right, bottom,
/// left). Insets do not inherit, so both keywords mean `auto`, including the
/// logical spellings under this engine's horizontal-tb/ltr model.
fn initial_inset_sides(
    name: &str,
    value: &lightningcss::properties::custom::TokenList<'_>,
) -> u8 {
    if !single_ident_is(value, "initial") && !single_ident_is(value, "unset") {
        return 0;
    }
    match name {
        "top" | "inset-block-start" => 0b0001,
        "right" | "inset-inline-end" => 0b0010,
        "bottom" | "inset-block-end" => 0b0100,
        "left" | "inset-inline-start" => 0b1000,
        "inset-block" => 0b0101,
        "inset-inline" => 0b1010,
        "inset" => 0b1111,
        _ => 0,
    }
}

/// A block's declarations as the author wrote them: `(property, value,
/// important)` in source order, normal declarations first (which is the order
/// the cascade folds them in).
///
/// Printed back out of the parsed form rather than sliced out of the source —
/// lightningcss keeps no source spans, and the printed form is the same text
/// [`value_text`] already trusts for the `var()` channel. Only the devtools
/// styles panel calls this, through `vn_css::set_capture_rule_text`.
pub(crate) fn decl_pairs(block: &DeclarationBlock) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for (props, important) in [(&block.declarations, false), (&block.important_declarations, true)] {
        for p in props.iter() {
            let Some(css) = p.to_css_string(false, PrinterOptions::default()).ok() else {
                continue;
            };
            match css.split_once(':') {
                Some((name, value)) => {
                    out.push((name.trim().to_string(), value.trim().to_string(), important))
                }
                // A property that prints without a colon cannot exist, but a
                // dropped declaration would be a lie in a provenance panel.
                None => out.push((css, String::new(), important)),
            }
        }
    }
    out
}

fn channel(p: &Property, diagnostics: &mut Vec<String>) -> Channel {
    use lightningcss::properties::custom::CustomPropertyName;
    match p {
        Property::Custom(c) => match &c.name {
            // `--shading` is a REAL mapped property that merely happens to be
            // spelled as a custom property (a GPU effect has no web equivalent,
            // so it needs a spelling a browser parses, stores and ignores). It
            // therefore keeps travelling on the static channel.
            CustomPropertyName::Custom(name) if name.0.as_ref() == effect::SHADING => Channel::Static,
            // `--window-control` is the same shape of thing: a real interpreted
            // property spelled as a custom one so a browser degrades it to
            // "does nothing" rather than dropping it (see [`WindowControl`]).
            // It must NOT fold into the element's `Vars` map — the renderer
            // reads it off `Extras`, and it is deliberately not inherited
            // (docs/WINDOWING.md D-14), which every real custom property is.
            CustomPropertyName::Custom(name) if name.0.as_ref() == WINDOW_CONTROL => Channel::Static,
            CustomPropertyName::Custom(name) => match value_text(p) {
                Some(v) => Channel::Custom(name.0.as_ref().to_string(), v),
                None => {
                    once(
                        diagnostics,
                        format!(
                            "`{}` could not be serialized back to text and was dropped (v1)",
                            name.0.as_ref()
                        ),
                    );
                    Channel::Static
                }
            },
            // An unknown/vendor-prefixed property (`-webkit-line-clamp`), or a
            // real CSS property lightningcss has no typed representation for
            // (`object-fit`, `content`, `outline-offset`). `apply` handles
            // them — but only once any `var()` in the value is resolved, and
            // that is exactly what the dynamic channel does. Tailwind's
            // `content-*` utilities are `content: var(--tw-content)`, so
            // without this the one property that decides whether a `::before`
            // box exists could never see its own value.
            CustomPropertyName::Unknown(name) if has_substitution(&c.value) => {
                match value_text(p) {
                    Some(v) => Channel::Dynamic(name.0.as_ref().to_string(), v),
                    None => Channel::Static,
                }
            }
            CustomPropertyName::Unknown(_) => Channel::Static,
        },
        // A `clip-path` lightningcss has no typed value for, and no `var()`
        // to resolve first: `apply` advises it.
        Property::Unparsed(u)
            if matches!(u.property_id, PropertyId::ClipPath(_)) && !has_substitution(&u.value) =>
        {
            Channel::Static
        }
        // CSS-wide inset resets have a known static value. Keeping literal
        // resets on the static channel also preserves their source order
        // against another inset declaration in the same rule.
        Property::Unparsed(u) if initial_inset_sides(u.property_id.name(), &u.value) != 0 => {
            Channel::Static
        }
        Property::Unparsed(u) => match value_text(p) {
            Some(v) => Channel::Dynamic(u.property_id.name().to_string(), v),
            None => Channel::Static,
        },
        _ => Channel::Static,
    }
}

/// Converts a declaration block into (normal, important) refinements plus the
/// transitions, custom properties and `var()`-composed declarations each half
/// declares.
pub(crate) fn convert_block(block: &DeclarationBlock) -> Converted {
    let mut diagnostics: Vec<String> = Vec::new();
    let diagnostics = &mut diagnostics;
    let mut convert = |props: &[Property]| -> Half {
        if props.is_empty() {
            return Half::default();
        }
        let mut r = StyleRefinement::default();
        let mut extras = Extras::default();
        let mut transitions = Collector::default();
        let mut animations = AnimationCollector::default();
        let mut custom: Decls = Vec::new();
        let mut dynamic: Decls = Vec::new();
        let mut font: Option<Vec<Property<'static>>> = None;
        let mut styled = false;
        for p in props {
            // `-moz-osx-font-smoothing`, `-webkit-touch-callout`: dropped
            // without a word, as Chromium drops them (`crate::prefix`).
            if crate::prefix::property_dropped(p) {
                continue;
            }
            match channel(p, diagnostics) {
                Channel::Custom(name, value) => custom.push((name, value)),
                Channel::Dynamic(name, value) => dynamic.push((name, value)),
                Channel::Static => {
                    if transitions.take(p, diagnostics) {
                        continue;
                    }
                    if animations.take(p, diagnostics) {
                        continue;
                    }
                    // Within one block the mapped declarations are folded now
                    // and the `var()`-composed ones per element afterwards, so
                    // a block that declares the SAME property both ways loses
                    // the source order between them. Rare enough to be worth
                    // saying rather than modelling.
                    let id = p.property_id();
                    if dynamic.iter().any(|(n, _)| *n == id.name()) {
                        once(
                            diagnostics,
                            format!(
                                "`{}` is declared twice in one rule, once through var() and once \
                                 literally — the literal one wins regardless of source order (v1)",
                                id.name()
                            ),
                        );
                    }
                    apply_static(p, &mut r, &mut extras, &mut font, diagnostics);
                    styled = true;
                }
            }
        }
        let (animation_list, animation_patch) = animations.finish();
        Half {
            refinement: styled.then(|| Rc::new(r)),
            transition_declarations: transitions.declarations(),
            transitions: transitions.finish().map(Rc::new),
            animations: animation_list.map(Rc::new),
            animation_patch,
            extras,
            custom: (!custom.is_empty()).then(|| Rc::new(custom)),
            dynamic: (!dynamic.is_empty()).then(|| Rc::new(dynamic)),
            font: font.map(Rc::new),
        }
    };
    let normal = convert(&block.declarations);
    let important = convert(&block.important_declarations);
    let diagnostics = std::mem::take(diagnostics);
    Converted {
        normal: normal.refinement,
        important: important.refinement,
        normal_transitions: normal.transitions,
        important_transitions: important.transitions,
        normal_transition_declarations: normal.transition_declarations,
        important_transition_declarations: important.transition_declarations,
        normal_animations: normal.animations,
        important_animations: important.animations,
        normal_animation_patch: normal.animation_patch,
        important_animation_patch: important.animation_patch,
        normal_extras: normal.extras,
        important_extras: important.extras,
        normal_custom: normal.custom,
        important_custom: important.custom,
        normal_dynamic: normal.dynamic,
        important_dynamic: important.dynamic,
        normal_font: normal.font,
        important_font: important.font,
        diagnostics: (!diagnostics.is_empty()).then(|| Rc::new(diagnostics)),
    }
}

// ---------------------------------------------------------------------------
// computed-value time: var() substitution + re-parse
// ---------------------------------------------------------------------------

/// Applies one `var()`-composed declaration against an element's resolved
/// custom-property map: substitute the text, re-parse it through lightningcss
/// with the ORIGINAL property id, then hand the typed property to the ordinary
/// mapper. Every property the mapper already supports therefore supports
/// `var()` with no per-property code.
///
/// Three failure modes, each with its own advisory and none of them silent:
/// * the substitution itself is invalid (missing variable with no fallback,
///   a reference cycle) — the declaration is dropped, exactly as CSS says;
/// * the substituted text does not parse as that property — also dropped, and
///   also what a browser does;
/// * the property parses but vn-css does not map it — the ordinary
///   "property not mapped" advisory, from `apply`.
pub(crate) fn apply_dynamic(
    name: &str,
    raw: &str,
    vars: &Vars,
    env: &crate::Env,
    r: &mut StyleRefinement,
    x: &mut Extras,
    diagnostics: &mut Vec<String>,
) {
    let value = match vars::substitute_with(raw, vars, env) {
        Ok(v) => v,
        // An unset `var()` with no fallback makes the declaration invalid at
        // computed-value time, and CSS says such a declaration is thrown away
        // — which is exactly what happens here. Dropping it is the CORRECT
        // behaviour rather than a gap, so it is silent.
        //
        // It is also common by design: reka sets `--reka-select-trigger-width`
        // from JS when a popover opens, so every rule reading it is invalid
        // until then, in a browser too. The other three errors are author
        // mistakes rather than a design, so those still speak.
        Err(vars::VarError::Unresolved(_)) => return,
        Err(e) => {
            once(
                diagnostics,
                format!(
                    "`{name}: {raw}` is invalid at computed-value time — {} (the declaration was \
                     dropped, as it would be in a browser)",
                    e.describe()
                ),
            );
            return;
        }
    };
    let property = match Property::parse_string(PropertyId::from(name), &value, ParserOptions::default())
    {
        Ok(p) => p,
        Err(_) => {
            once(
                diagnostics,
                format!("`{name}: {value}` did not parse after var() substitution (v1)"),
            );
            return;
        }
    };
    // A `transition` is folded into its own list at sheet-compile time and the
    // list is per rule, not per element — so a var()-composed one has nowhere
    // to land. Say so instead of applying it as an unmapped property.
    if matches!(
        property,
        Property::Transition(..)
            | Property::TransitionProperty(..)
            | Property::TransitionDuration(..)
            | Property::TransitionDelay(..)
            | Property::TransitionTimingFunction(..)
    ) {
        once(
            diagnostics,
            format!(
                "`{name}` composed with var() is not honoured (v1) — the transition list is folded \
                 once per rule, not per element; declare the transition literally"
            ),
        );
        return;
    }
    apply(&property, r, x, diagnostics);
}

/// Parses an inline `style` body ("color: red; padding: 4px") into a
/// refinement — the transport for Vue `:style` objects and `style=""` strings.
///
/// Transitions declared inline are DROPPED here (the refinement has nowhere to
/// carry them); use [`parse_inline_with_transitions`] to keep them.
pub fn parse_inline(body: &str, diagnostics: &mut Vec<String>) -> StyleRefinement {
    let (refinement, transitions, _) = parse_inline_full(body, diagnostics);
    if !transitions.is_empty() {
        diagnostics.push(
            "inline `transition` is parsed but dropped by parse_inline — the caller must use \
             parse_inline_with_transitions to honor it"
                .into(),
        );
    }
    refinement
}

/// Parses an inline `style` body into a refinement plus the transitions it
/// declares. An empty `Vec` means "declared none"; `transition: none` also
/// yields an empty `Vec` (it clears, and inline is the last cascade origin).
pub fn parse_inline_with_transitions(
    body: &str,
    diagnostics: &mut Vec<String>,
) -> (StyleRefinement, Vec<TransitionSpec>) {
    let (r, t, _) = parse_inline_full(body, diagnostics);
    (r, t)
}

/// [`parse_inline_with_transitions`] plus the [`Extras`] the body declared —
/// the form the retained tree uses, so an inline `style="object-fit: cover"`
/// wins over the sheet the way every other inline declaration does.
pub fn parse_inline_full(
    body: &str,
    diagnostics: &mut Vec<String>,
) -> (StyleRefinement, Vec<TransitionSpec>, Extras) {
    let inline = parse_inline_style(body, diagnostics);
    (
        inline.refinement.map(|r| (*r).clone()).unwrap_or_default(),
        inline.transitions.map(|t| (*t).clone()).unwrap_or_default(),
        inline.extras,
    )
}

/// Motion-only priority metadata. Aggregate InlineStyle fields remain usable
/// by legacy callers; parsed declarations additionally retain these channels.
#[derive(Clone, Debug, Default)]
pub struct InlineMotion {
    pub(crate) transitions: Option<Rc<Collector>>,
    pub(crate) animations: Option<AnimationPatch>,
}
impl InlineMotion {
    fn is_empty(&self) -> bool { self.transitions.is_none() && self.animations.is_none() }
}

/// Everything an inline `style` body contributes to the cascade.
///
/// The custom-property and `var()` halves cannot be folded here the way the
/// mapped declarations are: both depend on where the element sits in the tree,
/// so they ride along as text and are resolved by [`Engine::compute_node`]
/// (crate::Engine) exactly like a rule's are.
/// One priority half retained when inline declarations depend on custom values.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InlinePriority {
    pub refinement: Option<Rc<StyleRefinement>>,
    pub extras: Extras,
    pub custom: Option<Rc<Decls>>,
    pub dynamic: Option<Rc<Decls>>,
    pub font: Option<FontDecls>,
}

#[derive(Clone, Debug, Default)]
pub struct InlineStyle {
    /// Optional parsed normal/important motion declarations, folded at their
    /// own priority without changing general refinement handling.
    pub normal_motion: Option<InlineMotion>,
    pub important_motion: Option<InlineMotion>,
    pub refinement: Option<Rc<StyleRefinement>>,
    /// `None` = the body declared no `transition` at all (which is what lets a
    /// matched rule's list survive); `Some([])` is `transition: none`.
    pub transitions: Option<Rc<Vec<TransitionSpec>>>,
    /// Same rule for `animation`. This is not decoration: reka-ui's `Presence`
    /// writes `style.animationName = "none"` to CANCEL a running exit
    /// animation, and `style.animationFillMode = "forwards"` to pin the last
    /// frame while it unmounts. Dropping either leaves an element animating
    /// after the component decided it should stop.
    pub animations: Option<Rc<Vec<AnimationSpec>>>,
    /// Longhand-only `animation-*` — see [`AnimationPatch`]. reka-ui writes
    /// `style.animationFillMode = "forwards"` on its own, and folding that as a
    /// LIST would delete the very animation it means to freeze.
    pub animation_patch: Option<AnimationPatch>,
    pub extras: Extras,
    /// `--x` declarations. This is the channel `element.style.setProperty("--x")`
    /// lands on — reka-ui writes its `--reka-popper-*` sizing vars that way.
    pub custom: Decls,
    /// Known properties whose value referenced `var()`.
    pub dynamic: Decls,
    /// The font-relative declarations, applied again per element under its
    /// font size ([`FontDecls`]). An inline `padding: .5em` is the element's
    /// own em, which the parse cannot know.
    pub font: Option<FontDecls>,
    /// Normal and important halves for registered-variable dependency replay.
    #[doc(hidden)]
    pub priority: Option<Rc<[InlinePriority; 2]>>,
}

impl InlineStyle {
    pub fn is_empty(&self) -> bool {
        self.refinement.is_none()
            && self.transitions.is_none()
            && self.animations.is_none()
            && self.animation_patch.is_none()
            && self.extras == Extras::default()
            && self.custom.is_empty()
            && self.dynamic.is_empty()
            && self.font.is_none()
    }
}

/// Does this engine take `name: value` as one declaration it applies?
///
/// It is what the array form of Vue's `:style` needs. runtime-dom sets each
/// value of `display: ["-webkit-box", "flex"]` in turn and the browser keeps
/// the last one its parser accepts; here the last one THIS engine accepts is
/// kept, which is the same choice wherever both accept the same values. A
/// value is accepted when it parses as one declaration of a typed value the
/// mapper folds without a diagnostic. An untyped value (lightningcss's
/// `Unparsed`) is accepted only when it composes `var()`/`env()`, which a
/// browser also takes at parse time, or is a CSS-wide keyword. A custom
/// property takes any value.
pub fn accepts_declaration(name: &str, value: &str) -> bool {
    let name = name.trim();
    if name.starts_with("--") {
        return !value.contains(';');
    }
    let body = format!("{name}: {value}");
    let Ok(block) = DeclarationBlock::parse_string(&body, ParserOptions::default()) else {
        return false;
    };
    if block.len() != 1 {
        return false;
    }
    let p = block.declarations.iter().chain(block.important_declarations.iter()).next();
    if p.is_some_and(crate::prefix::property_dropped) {
        return false;
    }
    if let Some(Property::Unparsed(u)) = p {
        return has_substitution(&u.value)
            || matches!(
                single_ident_raw(&u.value).map(str::to_ascii_lowercase).as_deref(),
                Some("inherit" | "initial" | "unset" | "revert" | "revert-layer")
            );
    }
    let mut diagnostics = Vec::new();
    let style = parse_inline_style(&body, &mut diagnostics);
    diagnostics.is_empty() && !style.is_empty()
}

/// Parses an inline `style` body into everything it contributes.
pub fn parse_inline_style(body: &str, diagnostics: &mut Vec<String>) -> InlineStyle {
    let mut r = StyleRefinement::default();
    let mut extras = Extras::default();
    let mut normal_transitions = Collector::default();
    let mut important_transitions = Collector::default();
    let mut normal_animations = AnimationCollector::default();
    let mut important_animations = AnimationCollector::default();
    let mut custom: Decls = Vec::new();
    let mut dynamic: Decls = Vec::new();
    let mut font: Option<Vec<Property<'static>>> = None;
    let mut styled = false;
    let mut priority = None;
    // CSS's error handling, as for the stylesheet (`Engine::compile`): a
    // malformed declaration (`0: d`, a stray `}`) is dropped and the rest of
    // the body still applies. Without it one bad declaration cost the element
    // its whole inline style.
    let warnings = std::sync::Arc::new(std::sync::RwLock::new(Vec::new()));
    let options = ParserOptions {
        error_recovery: true,
        warnings: Some(warnings.clone()),
        ..ParserOptions::default()
    };
    let parsed = DeclarationBlock::parse_string(body, options);
    if let Ok(skipped) = warnings.read() {
        for e in skipped.iter() {
            diagnostics.push(format!("inline style: {e} in {body:?} — skipped, as a browser skips it"));
        }
    }
    match parsed {
        Ok(block) => {
            for (important, declarations) in [(false, &block.declarations), (true, &block.important_declarations)] {
            let transitions = if important { &mut important_transitions } else { &mut normal_transitions };
            let animations = if important { &mut important_animations } else { &mut normal_animations };
            for p in declarations {
                if crate::prefix::property_dropped(p) {
                    continue;
                }
                match channel(p, diagnostics) {
                    Channel::Custom(name, value) => custom.push((name, value)),
                    Channel::Dynamic(name, value) => dynamic.push((name, value)),
                    Channel::Static => {
                        if transitions.take(p, diagnostics) {
                            continue;
                        }
                        if animations.take(p, diagnostics) {
                            continue;
                        }
                        apply_static(p, &mut r, &mut extras, &mut font, diagnostics);
                        styled = true;
                    }
                }
            }
            }
            if !custom.is_empty() || !dynamic.is_empty() || !block.important_declarations.is_empty() {
                let converted = convert_block(&block);
                let half = |important| InlinePriority {
                    refinement: if important { converted.important.clone() } else { converted.normal.clone() },
                    extras: if important { converted.important_extras.clone() } else { converted.normal_extras.clone() },
                    custom: if important { converted.important_custom.clone() } else { converted.normal_custom.clone() },
                    dynamic: if important { converted.important_dynamic.clone() } else { converted.normal_dynamic.clone() },
                    font: if important { converted.important_font.clone() } else { converted.normal_font.clone() },
                };
                priority = Some(Rc::new([half(false), half(true)]));
            }
        }
        Err(e) => diagnostics.push(format!("inline style parse error: {e} in {body:?}")),
    }
    let normal_motion = InlineMotion { transitions: normal_transitions.declarations(), animations: normal_animations.clone().finish().1 };
    let important_motion = InlineMotion { transitions: important_transitions.declarations(), animations: important_animations.clone().finish().1 };
    normal_transitions.merge(&important_transitions);
    normal_animations.merge(&important_animations);
    let (animation_list, animation_patch) = normal_animations.finish();
    InlineStyle {
        normal_motion: (!normal_motion.is_empty()).then_some(normal_motion),
        important_motion: (!important_motion.is_empty()).then_some(important_motion),
        refinement: styled.then(|| Rc::new(r)),
        transitions: normal_transitions.finish().map(Rc::new),
        animations: animation_list.map(Rc::new),
        animation_patch,
        extras,
        custom,
        dynamic,
        font: font.map(Rc::new),
        priority,
    }
}

// ---------------------------------------------------------------------------
// value converters
// ---------------------------------------------------------------------------

fn color(c: &CssColor) -> Option<Hsla> {
    match c {
        CssColor::RGBA(c) => Some(
            gpui::Rgba {
                r: c.red as f32 / 255.0,
                g: c.green as f32 / 255.0,
                b: c.blue as f32 / 255.0,
                a: c.alpha as f32 / 255.0,
            }
            .into(),
        ),
        _ => None,
    }
}

thread_local! {
    /// See [`set_rem_basis`].
    static REM_BASIS: std::cell::Cell<f32> = const { std::cell::Cell::new(INITIAL_REM_BASIS) };
    /// See [`set_viewport_basis`]. `(width, height)` in CSS px.
    static VIEWPORT_BASIS: std::cell::Cell<(f32, f32)> =
        const { std::cell::Cell::new(INITIAL_VIEWPORT_BASIS) };
    /// Set the first time a sheet resolves a viewport unit. Nothing else has to
    /// recompile when the window resizes, and almost no sheet uses one.
    static VIEWPORT_UNITS_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Set while one declaration is mapped when a `calc()` in it had to be
    /// dropped. [`apply`] turns it into the advisory. See [`calc_dropped`].
    static CALC_DROPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// See [`set_font_source`].
    static FONT_SOURCE: std::cell::RefCell<Option<Box<dyn Fn() -> Vec<String>>>> =
        const { std::cell::RefCell::new(None) };
    /// The installed families, read from [`FONT_SOURCE`] the first time a
    /// `font-family` is mapped.
    static INSTALLED_FONTS: std::cell::RefCell<Option<Rc<std::collections::HashSet<String>>>> =
        const { std::cell::RefCell::new(None) };
    /// See [`take_font_misses`].
    static FONT_MISSES: std::cell::RefCell<Vec<(String, Option<String>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// See [`set_web_font_families`]; lowercase.
    static WEB_FONTS: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// The web font families that have a face still waiting to load (the host's
/// `@font-face` rules, which load lazily). A `font-family` naming one of them
/// before the family it is drawn with records it in [`Extras::web_font`].
/// Takes effect on the next compile, which the host runs when a face lands.
pub fn set_web_font_families(families: impl IntoIterator<Item = String>) {
    let set = families.into_iter().map(|f| f.to_lowercase()).collect();
    WEB_FONTS.with(|w| *w.borrow_mut() = set);
}

/// [`Extras::web_font`] for a `font-family` list: its waiting web fonts, in
/// order, up to the first family installed (or a generic). A waiting family
/// that is installed already (a face of it is) ends the list too.
fn waiting_web_font(families: &[CssFontFamily]) -> Option<Rc<[String]>> {
    use lightningcss::traits::ToCss;
    WEB_FONTS.with(|w| {
        let w = w.borrow();
        if w.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for f in families {
            let CssFontFamily::FamilyName(name) = f else { break };
            let Ok(css) = name.to_css_string(Default::default()) else { continue };
            let clean = css.trim_matches('"');
            if w.contains(&clean.to_lowercase()) {
                out.push(clean.to_string());
            }
            if font_installed(clean) != Some(false) {
                break;
            }
        }
        Some(Rc::from(out))
    })
}

/// Where the installed font families come from — the host's text system
/// (`gpui::TextSystem::all_font_names`, real families only since zed v1.26).
///
/// Read lazily, once, on the first `font-family` the mapper sees: it is a
/// platform enumeration over every installed family, and a sheet with no
/// `font-family` never pays for it. Without a source (unit tests, tools with no
/// text system) every named family counts as installed, which is the old
/// "first named family wins" behaviour.
pub fn set_font_source(source: impl Fn() -> Vec<String> + 'static) {
    FONT_SOURCE.with(|s| *s.borrow_mut() = Some(Box::new(source)));
    INSTALLED_FONTS.with(|c| *c.borrow_mut() = None);
}

/// `Some(installed?)`, or `None` when no source was set.
fn font_installed(name: &str) -> Option<bool> {
    let set = INSTALLED_FONTS.with(|c| c.borrow().clone()).or_else(|| {
        let names = FONT_SOURCE.with(|s| s.borrow().as_ref().map(|f| f()))?;
        let set = Rc::new(names.into_iter().collect::<std::collections::HashSet<_>>());
        INSTALLED_FONTS.with(|c| *c.borrow_mut() = Some(set.clone()));
        Some(set)
    })?;
    Some(set.contains(name))
}

/// `(asked for, used instead)` for every `font-family` whose FIRST entry names
/// a family that is not installed — drained by the renderer's one-shot
/// advisory. `used` is `None` when nothing in the list was available and
/// gpui's own default takes over.
pub fn take_font_misses() -> Vec<(String, Option<String>)> {
    FONT_MISSES.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

fn note_font_miss(asked: String, used: Option<String>) {
    FONT_MISSES.with(|m| {
        let mut m = m.borrow_mut();
        // A recompile maps the same declarations again.
        if !m.iter().any(|(a, _)| *a == asked) {
            m.push((asked, used));
        }
    });
}

/// The family a generic stands for — Chrome's defaults on Windows first, then
/// their usual stand-ins elsewhere. `system-ui` is gpui's own system UI font.
/// The `ui-*` four are Safari-only (Chrome skips them, so do we), and `emoji` /
/// `math` / `fangsong` are never what a list means as its PRIMARY font; the
/// CSS-wide keywords lightningcss parses as generics are not families at all.
fn generic_family(g: &lightningcss::properties::font::GenericFontFamily) -> Option<String> {
    use lightningcss::properties::font::GenericFontFamily as G;
    let candidates: &[&str] = match g {
        G::SystemUI => return Some(".SystemUIFont".into()),
        G::SansSerif => &["Arial", "Helvetica", "Liberation Sans"],
        G::Serif => &["Times New Roman", "Times", "Liberation Serif"],
        G::Monospace => &["Consolas", "Menlo", "DejaVu Sans Mono", "Liberation Mono", "Courier New"],
        G::Cursive => &["Comic Sans MS"],
        G::Fantasy => &["Impact"],
        _ => return None,
    };
    // A generic always resolves in a browser, so it does here too.
    Some(
        candidates
            .iter()
            .find(|c| font_installed(c) != Some(false))
            .map_or(".SystemUIFont", |c| *c)
            .into(),
    )
}

/// CSS Fonts 4 §5's "first available font": the first entry of the list that
/// is installed (or a generic, which always is). Only that first available
/// family is mapped here; per-glyph fallback through the REST of the list is
/// [`composed_font_stack`]'s, when web families are composed, and otherwise
/// DirectWrite's system fallback covers missing glyphs.
///
/// When nothing in the list is available the first named family is kept, so
/// gpui's own default applies exactly as before.
fn first_available_family(families: &[CssFontFamily]) -> Option<String> {
    use lightningcss::traits::ToCss;
    let mut first_named = None;
    let mut asked_first = None;
    for (i, f) in families.iter().enumerate() {
        let found = match f {
            CssFontFamily::FamilyName(name) => {
                let Ok(css) = name.to_css_string(Default::default()) else { continue };
                let clean = css.trim_matches('"').to_string();
                if font_installed(&clean) == Some(false) {
                    if i == 0 {
                        asked_first = Some(clean.clone());
                    }
                    first_named.get_or_insert(clean);
                    None
                } else {
                    Some(clean)
                }
            }
            CssFontFamily::Generic(g) => generic_family(g),
        };
        if let Some(used) = found {
            if let Some(asked) = asked_first {
                note_font_miss(asked, Some(used.clone()));
            }
            return Some(used);
        }
    }
    if let Some(asked) = asked_first {
        note_font_miss(asked, None);
    }
    first_named
}

thread_local! {
    /// See [`set_composed_font_families`]: `(lowercase family, covers Latin)`.
    static COMPOSED_FONTS: std::cell::RefCell<Vec<(String, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
    /// See [`take_unreached_fonts`].
    static UNREACHED_FONTS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The web font families the host draws character by character, from the
/// faces whose `unicode-range` covers each (gpui's `FamilyComposer`), with
/// whether each covers Latin text. A family that does not (a Private Use Area
/// icon font) is never installed as far as [`set_font_source`] goes, so it is
/// never the first available font. When any is set, every `font-family` hands
/// gpui the rest of its list as the font's `fallbacks`
/// ([`composed_font_stack`]). Takes effect on the next compile.
pub fn set_composed_font_families(families: impl IntoIterator<Item = (String, bool)>) {
    let list = families.into_iter().map(|(f, latin)| (f.to_lowercase(), latin)).collect();
    COMPOSED_FONTS.with(|c| *c.borrow_mut() = list);
}

/// `(web family, family drawn instead)` for every `font-family` list in
/// which a composed web family comes after the installed family the text is
/// drawn with: the characters that family has no glyph for go to the system
/// fallback, not to the web family. Drained by the host's advisory.
pub fn take_unreached_fonts() -> Vec<(String, String)> {
    UNREACHED_FONTS.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// The family and `fallbacks` a `font-family` list draws with when web
/// families are composed ([`set_composed_font_families`]): the families that
/// are installed or composed, generics as the family they stand for, in list
/// order. The first is the font's family: a composed family that covers no
/// Latin text when it leads, else the first available one (as
/// [`first_available_family`]). The rest are its `fallbacks` when that family
/// is composed, so a character none of its faces covers reaches the next one
/// (CSS Fonts 4 §5.3). An installed family has no such fallbacks (DirectWrite
/// falls back to the system's), and empty ones are set so that a parent's are
/// not inherited. `None` when nothing is composed, or nothing in the list is
/// available.
fn composed_font_stack(families: &[CssFontFamily]) -> Option<(String, gpui::FontFallbacks)> {
    use lightningcss::traits::ToCss;
    let composed = COMPOSED_FONTS.with(|c| c.borrow().clone());
    if composed.is_empty() {
        return None;
    }
    let mut stack: Vec<(String, Option<bool>)> = Vec::new();
    for f in families {
        let (name, generic) = match f {
            CssFontFamily::FamilyName(name) => {
                let Ok(css) = name.to_css_string(Default::default()) else { continue };
                (css.trim_matches('"').to_string(), false)
            }
            CssFontFamily::Generic(g) => match generic_family(g) {
                Some(name) => (name, true),
                None => continue,
            },
        };
        let latin = composed.iter().find(|(c, _)| c.eq_ignore_ascii_case(&name)).map(|c| c.1);
        if latin.is_none() && !generic && font_installed(&name) == Some(false) {
            continue;
        }
        if !stack.iter().any(|(s, _)| s.eq_ignore_ascii_case(&name)) {
            stack.push((name, latin));
        }
    }
    let (first, latin) = stack.first()?.clone();
    if latin.is_none() {
        // An installed family draws the text; a composed one after it is
        // reached only by the characters it lacks, and DirectWrite's own
        // fallback takes those.
        for (name, _) in stack.iter().skip(1).filter(|s| s.1.is_some()) {
            UNREACHED_FONTS.with(|u| {
                let mut u = u.borrow_mut();
                if !u.iter().any(|(n, _)| n == name) {
                    u.push((name.clone(), first.clone()));
                }
            });
        }
        return Some((first, gpui::FontFallbacks::default()));
    }
    let rest = stack.into_iter().skip(1).map(|s| s.0).collect();
    Some((first, gpui::FontFallbacks::from_fonts(rest)))
}

/// A browser's initial root font size, and the basis every sheet compiles
/// against until an app says otherwise.
pub const INITIAL_REM_BASIS: f32 = 16.0;

/// The px value of `1rem`.
pub fn rem_basis() -> f32 {
    REM_BASIS.with(|c| c.get())
}

/// Sets the px value of `1rem`, returning whether it actually changed.
///
/// **A sheet already compiled against the old basis does not move**: `rem` is
/// flattened to px HERE, at parse time, because gpui's style fields are mostly
/// `Pixels` and threading a `Rems` through every one of them is a different
/// project. So the caller's contract is that a `true` return means "recompile
/// the stylesheet" — `vn_tree::tree::restyle_all` is the only caller and does
/// exactly that.
///
/// The basis is the ROOT element's computed `font-size`, which is what CSS says
/// `rem` means. It is not a curiosity: `@argon/theme` sets
/// `documentElement.style.fontSize = "14px"`, so every Tailwind spacing,
/// width and type step on apps/meet-probe is 12.5 % smaller in a browser than
/// a 16px basis produces — the page laid out 14 % too large end to end and its
/// footer fell off the bottom of the window.
///
/// Values outside a sane range are refused rather than clamped: a bad basis
/// would silently rescale the whole app, and "the root has no font-size yet"
/// (0.0) is a normal state during boot.
pub fn set_rem_basis(px: f32) -> bool {
    if !(px.is_finite() && (1.0..=200.0).contains(&px)) {
        return false;
    }
    REM_BASIS.with(|c| {
        if (c.get() - px).abs() < f32::EPSILON {
            false
        } else {
            c.set(px);
            true
        }
    })
}

/// The viewport a sheet compiled before the host published a real one. Any
/// non-zero pair works — 0 would make every `vh` collapse to nothing on the
/// frames before the first `set_env`, which is visually indistinguishable from
/// the property being dropped.
pub const INITIAL_VIEWPORT_BASIS: (f32, f32) = (1280.0, 720.0);

/// The px value of `100vw` / `100vh`.
pub fn viewport_basis() -> (f32, f32) {
    VIEWPORT_BASIS.with(|c| c.get())
}

/// Whether the sheet compiled so far resolved any viewport-relative unit.
///
/// This is what keeps [`set_viewport_basis`] free for the apps that do not use
/// one: a resize only has to touch the stylesheet when a length in it actually
/// depends on the viewport.
pub fn uses_viewport_units() -> bool {
    VIEWPORT_UNITS_SEEN.with(|c| c.get())
}

/// Forgets that a viewport unit was seen — called when a new sheet is compiled,
/// so the flag describes the CURRENT sheet rather than every sheet ever loaded.
pub fn reset_viewport_units_seen() {
    VIEWPORT_UNITS_SEEN.with(|c| c.set(false));
}

/// Runs `f` and says whether IT resolved a viewport unit, whatever the flag
/// said before; the flag ends up as before OR that. The compile uses it to
/// find the few declaration blocks a resize has to convert again
/// (`Engine::rebase_viewport`).
pub(crate) fn noting_viewport_units<R>(f: impl FnOnce() -> R) -> (R, bool) {
    let before = VIEWPORT_UNITS_SEEN.with(|c| c.replace(false));
    let out = f();
    let seen = VIEWPORT_UNITS_SEEN.with(|c| c.get());
    VIEWPORT_UNITS_SEEN.with(|c| c.set(before || seen));
    (out, seen)
}

/// Sets the px size of the viewport, returning whether it actually changed.
///
/// Viewport units are flattened to px at SHEET COMPILE time, so a `true` return
/// means "re-resolve them" (`Engine::rebase_viewport`, or a recompile when that
/// says no), but only if [`uses_viewport_units`] says the sheet cares.
///
/// The viewport here is vue-native's viewport, which is the ROOT element's box
/// (`window.innerWidth`/`innerHeight` report the same numbers — m12's inset
/// fix), not the OS window: a host that wraps the app in chrome must not make
/// `100vh` taller than the area the app actually has.
pub fn set_viewport_basis(width: f32, height: f32) -> bool {
    if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
        return false;
    }
    VIEWPORT_BASIS.with(|c| {
        let (w, h) = c.get();
        if (w - width).abs() < f32::EPSILON && (h - height).abs() < f32::EPSILON {
            false
        } else {
            c.set((width, height));
            true
        }
    })
}

/// `vw` / `vh` / `vmin` / `vmax` and the dynamic-viewport spellings, in px.
///
/// The small/large/dynamic families (`svh`, `lvh`, `dvh`, …) are all the same
/// number on a desktop window: they differ only where a browser's UI chrome
/// grows and shrinks, and there is none here. Resolving them rather than
/// dropping them is the whole point — `max-h-[85vh]` on a dialog is not a
/// decoration, it is the only thing keeping the box inside the window.
fn viewport_value_px(v: &LengthValue) -> Option<f32> {
    let (w, h) = viewport_basis();
    let px = match v {
        // `vi` / `vb` are the inline / block axis; in `horizontal-tb` — the only
        // writing mode there is here — those are the horizontal and vertical
        // ones.
        LengthValue::Vw(p)
        | LengthValue::Svw(p)
        | LengthValue::Lvw(p)
        | LengthValue::Dvw(p)
        | LengthValue::Vi(p)
        | LengthValue::Svi(p)
        | LengthValue::Lvi(p)
        | LengthValue::Dvi(p) => p / 100.0 * w,
        LengthValue::Vh(p)
        | LengthValue::Svh(p)
        | LengthValue::Lvh(p)
        | LengthValue::Dvh(p)
        | LengthValue::Vb(p)
        | LengthValue::Svb(p)
        | LengthValue::Lvb(p)
        | LengthValue::Dvb(p) => p / 100.0 * h,
        LengthValue::Vmin(p) | LengthValue::Svmin(p) | LengthValue::Lvmin(p)
        | LengthValue::Dvmin(p) => p / 100.0 * w.min(h),
        LengthValue::Vmax(p) | LengthValue::Svmax(p) | LengthValue::Lvmax(p)
        | LengthValue::Dvmax(p) => p / 100.0 * w.max(h),
        _ => return None,
    };
    VIEWPORT_UNITS_SEEN.with(|c| c.set(true));
    Some(px)
}

/// The same resolution as [`viewport_value_px`], reached from a raw token
/// (`Token::Dimension { unit, .. }`) rather than a parsed `LengthValue` — the
/// path a `var()`-composed declaration takes.
fn viewport_unit_px(unit: &str, value: f32) -> Option<f32> {
    let (w, h) = viewport_basis();
    let basis = match unit {
        "vw" | "svw" | "lvw" | "dvw" | "vi" | "svi" | "lvi" | "dvi" => w,
        "vh" | "svh" | "lvh" | "dvh" | "vb" | "svb" | "lvb" | "dvb" => h,
        "vmin" | "svmin" | "lvmin" | "dvmin" => w.min(h),
        "vmax" | "svmax" | "lvmax" | "dvmax" => w.max(h),
        _ => return None,
    };
    VIEWPORT_UNITS_SEEN.with(|c| c.set(true));
    Some(value / 100.0 * basis)
}

fn length_value_px(v: &LengthValue) -> Option<f32> {
    match v {
        LengthValue::Px(v) => Some(*v),
        LengthValue::Rem(v) => Some(v * rem_basis()),
        LengthValue::Em(v) => Some(v * em_basis()),
        LengthValue::Ch(v) => Some(v * character_basis(false)),
        LengthValue::Ex(v) => Some(v * character_basis(true)),
        LengthValue::Rch(v) | LengthValue::Rex(v) => Some(v * HALF_EM * rem_basis()),
        other => viewport_value_px(other),
    }
}

// CSS Values 4's fallback when the host cannot measure a font.
const HALF_EM: f32 = 0.5;

/// Font-relative lengths supplied by the same text system that paints glyphs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontRelativeMetrics {
    /// The advance of the shaped `0` glyph, in CSS pixels.
    pub zero_advance: f32,
    /// The face's x-height, in CSS pixels.
    pub x_height: f32,
}

thread_local! {
    static FONT_METRIC_SOURCE: std::cell::RefCell<Option<Box<dyn Fn(&gpui::Font, f32) -> Option<FontRelativeMetrics>>>> = const { std::cell::RefCell::new(None) };
    static FONT_METRIC_GENERATION: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static FONT_FACE: std::cell::RefCell<Option<gpui::Font>> = const { std::cell::RefCell::new(None) };
}

/// Installs the host's font metrics. Replacing the source invalidates cached
/// folds, so a style computed before host setup cannot retain fallback widths.
pub fn set_font_metric_source(source: impl Fn(&gpui::Font, f32) -> Option<FontRelativeMetrics> + 'static) {
    FONT_METRIC_SOURCE.with(|slot| *slot.borrow_mut() = Some(Box::new(source)));
    FONT_METRIC_GENERATION.with(|generation| generation.set(generation.get().wrapping_add(1)));
}

pub(crate) fn font_metric_generation() -> u32 {
    FONT_METRIC_GENERATION.with(|generation| generation.get())
}

pub(crate) fn with_font_face<R>(font: &gpui::Font, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<gpui::Font>);
    impl Drop for Restore {
        fn drop(&mut self) {
            FONT_FACE.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(FONT_FACE.with(|slot| slot.replace(Some(font.clone()))));
    f()
}

fn character_basis(x_height: bool) -> f32 {
    let size = em_basis();
    let font = FONT_FACE.with(|slot| slot.borrow().clone());
    font.and_then(|font| FONT_METRIC_SOURCE.with(|slot| slot.borrow().as_ref().and_then(|source| source(&font, size))))
        .map(|metrics| if x_height { metrics.x_height } else { metrics.zero_advance })
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(HALF_EM * size)
}

/// The px value of one `unit` of the font-relative lengths [`length_value_px`]
/// resolves, reached from a raw token: the path a `var()`-composed declaration
/// takes.
fn font_unit_px(unit: &str, value: f32) -> Option<f32> {
    match unit {
        "rem" => Some(value * rem_basis()),
        "em" => Some(value * em_basis()),
        "ch" => Some(value * character_basis(false)),
        "ex" => Some(value * character_basis(true)),
        "rch" | "rex" => Some(value * HALF_EM * rem_basis()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// em: the element's own font size
// ---------------------------------------------------------------------------
//
// `em` (and `ch`, `ex`, a `font-size` percentage, `smaller` / `larger`, a
// `line-height` percentage) depends on where the element sits, so a sheet
// cannot flatten it once. While a block compiles, such a length reads the rem
// basis and marks the declaration FONT-RELATIVE ([`noting_font_units`]). The
// block keeps that declaration, and every static one after it, as parsed
// properties (`Converted::normal_font`). `Engine::compute_node_at` applies
// them again per element, under [`with_font_basis`]: the element's own
// computed font size, or for `font-size` itself its parent's. That is CSS
// Values 4 §6.1.1. The refinement compiled against the rem basis stays in the
// rule as well, so compile-time inspection of a rule (state advisories, the
// `@keyframes` path) sees every property it declares. The per-element pass
// overwrites the em-derived fields, and re-applying the static declarations
// that follow keeps the block's source order.

thread_local! {
    /// What `em` resolves against right now. NaN outside a per-element fold:
    /// compiling a sheet or parsing an inline style, where it reads the rem
    /// basis.
    static FONT_BASIS: std::cell::Cell<f32> = const { std::cell::Cell::new(f32::NAN) };
    /// Set whenever a length read [`em_basis`]: the declaration being converted
    /// is font-relative.
    static FONT_UNITS_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The px value of `1em` for the declaration being converted.
fn em_basis() -> f32 {
    FONT_UNITS_SEEN.with(|c| c.set(true));
    let basis = FONT_BASIS.with(|c| c.get());
    if basis.is_nan() {
        rem_basis()
    } else {
        basis
    }
}

/// Runs `f` with `1em` = `px`.
pub(crate) fn with_font_basis<R>(px: f32, f: impl FnOnce() -> R) -> R {
    struct Restore(f32);
    impl Drop for Restore {
        fn drop(&mut self) {
            FONT_BASIS.with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(FONT_BASIS.with(|c| c.replace(px)));
    f()
}

/// Runs `f`, and says whether anything it converted read the font basis.
pub(crate) fn noting_font_units<R>(f: impl FnOnce() -> R) -> (R, bool) {
    let before = FONT_UNITS_SEEN.with(|c| c.replace(false));
    let out = f();
    let seen = FONT_UNITS_SEEN.with(|c| c.replace(before));
    FONT_UNITS_SEEN.with(|c| c.set(before || seen));
    (out, seen)
}

/// Whether `p` sets `font-size`, which resolves its em against the PARENT's
/// font size where every other property uses the element's own.
pub(crate) fn sets_font_size(p: &Property) -> bool {
    matches!(p, Property::FontSize(_))
}

/// Applies one font-relative declaration with `1em` = `basis`.
pub(crate) fn apply_at(
    p: &Property,
    basis: f32,
    r: &mut StyleRefinement,
    x: &mut Extras,
    diagnostics: &mut Vec<String>,
) {
    with_font_basis(basis, || apply(p, r, x, diagnostics));
}

/// A computed `font-size` in px. The mapper writes px; a `Rems` is read
/// against the current basis.
pub(crate) fn font_size_px(l: AbsoluteLength) -> f32 {
    match l {
        AbsoluteLength::Pixels(p) => f32::from(p),
        AbsoluteLength::Rems(r) => r.0 * rem_basis(),
    }
}

/// The px sizes Chromium gives the absolute `font-size` keywords at a 16px
/// `medium`.
fn absolute_font_size_px(k: &lightningcss::properties::font::AbsoluteFontSize) -> f32 {
    use lightningcss::properties::font::AbsoluteFontSize as A;
    match k {
        A::XXSmall => 9.0,
        A::XSmall => 10.0,
        A::Small => 13.0,
        A::Medium => 16.0,
        A::Large => 18.0,
        A::XLarge => 24.0,
        A::XXLarge => 32.0,
        A::XXXLarge => 48.0,
    }
}

/// `calc()` over ABSOLUTE lengths, evaluated to pixels.
///
/// This is the third item on the Tailwind-v4 list in
/// `packages/vue-native/tailwind.ts`, and it is what makes shadcn's own radius
/// scale work: `rounded-md` is `border-radius: calc(var(--radius) - 2px)`, so
/// without it three quarters of the corner radii in the library vanish the
/// moment the theme stops being pre-resolved.
///
/// Percentages are deliberately NOT evaluated: they need the containing block,
/// which does not exist until layout. A `calc()` that mixes one in returns
/// `None` and takes the ordinary "value is unresolved" path.
fn calc_px<V>(calc: &lightningcss::values::calc::Calc<V>, leaf: &dyn Fn(&V) -> Option<f32>) -> Option<f32> {
    use lightningcss::values::calc::{Calc, MathFunction};
    match calc {
        Calc::Value(v) => leaf(v),
        Calc::Number(n) => Some(*n),
        Calc::Sum(a, b) => Some(calc_px(a, leaf)? + calc_px(b, leaf)?),
        Calc::Product(n, v) => Some(n * calc_px(v, leaf)?),
        Calc::Function(f) => match &**f {
            MathFunction::Calc(c) => calc_px(c, leaf),
            MathFunction::Min(list) => list
                .iter()
                .map(|c| calc_px(c, leaf))
                .try_fold(f32::INFINITY, |acc, v| Some(acc.min(v?))),
            MathFunction::Max(list) => list
                .iter()
                .map(|c| calc_px(c, leaf))
                .try_fold(f32::NEG_INFINITY, |acc, v| Some(acc.max(v?))),
            MathFunction::Clamp(lo, v, hi) => {
                Some(calc_px(v, leaf)?.clamp(calc_px(lo, leaf)?, calc_px(hi, leaf)?))
            }
            MathFunction::Abs(v) => Some(calc_px(v, leaf)?.abs()),
            MathFunction::Sign(v) => Some(calc_px(v, leaf)?.signum()),
            // round()/rem()/mod()/hypot() have no user in any sheet we serve;
            // they take the "unresolved" path with the ordinary advisory.
            _ => None,
        },
    }
}

fn advise_list_marker(diagnostics: &mut Vec<String>) {
    warn_once(
        diagnostics,
        "this list marker is not generated (v1) — the PREDEFINED counter styles (`disc`, \
         `circle`, `square`, `decimal`, `decimal-leading-zero`, and the alpha and roman ones) do \
         generate a real marker box now. A `@counter-style` rule and a `list-style-image` have no \
         model here: the first needs a counter-style engine, the second a raster in the marker \
         box. Pick a predefined style, or render the marker as a real element (a `<span>`, or \
         `::before { content: \"…\" }`)",
    );
}

/// `letter-spacing` / `word-spacing`. `normal` is zero.
///
/// `em` resolves against the ROOT font size here, like every other relative
/// length in this mapper — the sheet is flattened to pixels at compile time and
/// the element's own font size is not known until the cascade runs. That is the
/// same approximation `length_value_px` already documents, and tracking is
/// almost always written in `em` against an unchanged root.
fn spacing_px(spacing: &lightningcss::properties::text::Spacing) -> Option<f32> {
    use lightningcss::properties::text::Spacing;
    match spacing {
        Spacing::Normal => Some(0.0),
        Spacing::Length(l) => css_length_px(l),
    }
}

fn css_length_px(l: &CssLength) -> Option<f32> {
    match l {
        CssLength::Value(v) => length_value_px(v),
        CssLength::Calc(c) => calc_px(c, &|v: &CssLength| css_length_px(v)),
    }
}

/// The absolute half of a `<length-percentage>` inside a `calc()`.
fn lp_leaf_px(lp: &LengthPercentage) -> Option<f32> {
    match lp {
        DimensionPercentage::Dimension(v) => length_value_px(v),
        // A percentage needs the containing block; a calc() that mixes one in
        // is left unresolved rather than guessed at.
        DimensionPercentage::Percentage(_) => None,
        DimensionPercentage::Calc(c) => calc_px(c, &lp_leaf_px),
    }
}

/// [`calc_px`] over a `<length-percentage>` for a property whose percentage is
/// NOT resolved at layout or paint here (a background position, a gradient
/// stop, a translation): a calc() that mixes one in is dropped, and
/// [`calc_dropped`] says so. A radius resolves at paint (`corner`).
fn lp_calc_px(c: &lightningcss::values::calc::Calc<LengthPercentage>) -> Option<f32> {
    let v = calc_px(c, &lp_leaf_px);
    if v.is_none() {
        calc_dropped();
    }
    v
}

/// Record that the declaration being mapped lost a `calc()` (see [`apply`]).
fn calc_dropped() {
    CALC_DROPPED.with(|c| c.set(true));
}

/// A `calc()` over `<length-percentage>`s folded to its LINEAR form,
/// `(fraction, px)` meaning `fraction × basis + px`, or `None` when it is not
/// linear in the basis.
///
/// That is every calc() a layout ever needs from a stylesheet in practice:
/// sums, differences and scalings of percentages and lengths (reka's slider
/// thumb `left: calc(38.89% + 13.33px)`, Tailwind's `w-[calc(100%-2rem)]`,
/// `calc(var(--chat-width, 600px) * 0.8)`). `min()` / `max()` / `clamp()`
/// with a percentage inside are piecewise in the basis, so they stay
/// unresolved, as do a bare number in a length position and a division by a
/// length; [`lp_definite`] reports those.
fn calc_linear(c: &lightningcss::values::calc::Calc<LengthPercentage>) -> Option<(f32, f32)> {
    use lightningcss::values::calc::{Calc, MathFunction};
    match c {
        Calc::Value(v) => match &**v {
            DimensionPercentage::Dimension(d) => Some((0.0, length_value_px(d)?)),
            DimensionPercentage::Percentage(p) => Some((p.0, 0.0)),
            DimensionPercentage::Calc(inner) => calc_linear(inner),
        },
        Calc::Number(_) => None,
        Calc::Sum(a, b) => {
            let (fa, pa) = calc_linear(a)?;
            let (fb, pb) = calc_linear(b)?;
            Some((fa + fb, pa + pb))
        }
        Calc::Product(n, v) => {
            let (f, p) = calc_linear(v)?;
            Some((n * f, n * p))
        }
        Calc::Function(f) => match &**f {
            MathFunction::Calc(inner) => calc_linear(inner),
            // Piecewise in the basis unless every term is absolute, in which
            // case it is a constant.
            _ => calc_px(c, &lp_leaf_px).map(|p| (0.0, p)),
        },
    }
}

fn lp_definite(lp: &LengthPercentage) -> Option<DefiniteLength> {
    match lp {
        DimensionPercentage::Dimension(v) => length_value_px(v).map(|p| px(p).into()),
        DimensionPercentage::Percentage(p) => Some(relative(p.0)),
        // A calc() of lengths alone is a constant; one that mixes in a
        // percentage is resolved by the layout, as a percentage is
        // (`DefiniteLength::Calc`, a vue-native fork variant).
        DimensionPercentage::Calc(c) => match calc_px(c, &lp_leaf_px) {
            Some(p) => Some(px(p).into()),
            None => match calc_linear(c) {
                Some((fraction, pixels)) if fraction == 0.0 => Some(px(pixels).into()),
                Some((fraction, pixels)) => {
                    Some(DefiniteLength::Calc { fraction, pixels: px(pixels) })
                }
                None => {
                    calc_dropped();
                    None
                }
            },
        },
    }
}

fn lpa_length(v: &LengthPercentageOrAuto) -> Option<Length> {
    match v {
        LengthPercentageOrAuto::Auto => Some(Length::Auto),
        LengthPercentageOrAuto::LengthPercentage(lp) => lp_definite(lp).map(Length::Definite),
    }
}

fn size_length(v: &CssSize) -> Option<Length> {
    match v {
        CssSize::Auto => Some(Length::Auto),
        CssSize::LengthPercentage(lp) => lp_definite(lp).map(Length::Definite),
        _ => None,
    }
}

fn max_size_length(v: &lightningcss::properties::size::MaxSize) -> Option<Length> {
    use lightningcss::properties::size::MaxSize;
    match v {
        MaxSize::None => Some(Length::Auto),
        MaxSize::LengthPercentage(lp) => lp_definite(lp).map(Length::Definite),
        _ => None,
    }
}

/// One `border-*-radius` corner, horizontal and vertical radius.
///
/// A circle of one length goes in gpui's `corner_radii`. A percentage, a
/// linear calc() with one, or an elliptical `a / b` corner goes in the fork's
/// `vn_corner_radii`, which gpui resolves against the box at paint (with
/// CSS's overlap rule, and as a circle of the smaller radius:
/// `gpui::vn_resolve_corner_radii`). Each writes both fields, so whichever
/// declaration comes later in the cascade wins the corner. A calc() that is
/// not linear leaves the corner alone and is advised ([`calc_dropped`]).
/// Answers whether the corner was written, which is what cancels an earlier
/// `inherit` on it ([`RadiusInherit`]).
fn corner(
    length: &mut Option<AbsoluteLength>,
    relative: &mut Option<Option<gpui::VnCornerRadius>>,
    radius: &lightningcss::values::size::Size2D<LengthPercentage>,
) -> bool {
    let (Some(x), Some(y)) = (lp_definite(&radius.0), lp_definite(&radius.1)) else {
        return false;
    };
    match (x, y) {
        (DefiniteLength::Absolute(a), DefiniteLength::Absolute(b)) if a == b => {
            *length = Some(a);
            *relative = Some(None);
        }
        (x, y) => {
            *length = Some(px(0.).into());
            *relative = Some(Some(gpui::VnCornerRadius { x, y }));
        }
    }
    true
}

fn lp_absolute(lp: &LengthPercentage) -> Option<AbsoluteLength> {
    match lp {
        DimensionPercentage::Dimension(v) => length_value_px(v).map(|p| px(p).into()),
        DimensionPercentage::Calc(c) => lp_calc_px(c).map(|p| px(p).into()),
        DimensionPercentage::Percentage(_) => None,
    }
}

fn gap_definite(v: &lightningcss::properties::align::GapValue) -> Option<DefiniteLength> {
    use lightningcss::properties::align::GapValue;
    match v {
        GapValue::LengthPercentage(lp) => lp_definite(lp),
        GapValue::Normal => None,
    }
}

/// The cross-axis alignment keywords, shared by `align-items` / `align-self`
/// (gpui aliases `AlignSelf = AlignItems`).
fn self_position(v: &SelfPosition) -> gpui::AlignItems {
    use gpui::AlignItems as G;
    match v {
        SelfPosition::Center => G::Center,
        SelfPosition::Start | SelfPosition::FlexStart | SelfPosition::SelfStart => G::FlexStart,
        SelfPosition::End | SelfPosition::FlexEnd | SelfPosition::SelfEnd => G::FlexEnd,
    }
}

/// `repeat(N, minmax(<min>, 1fr))` — the ONLY track list gpui's `GridTemplate`
/// can express, and (not by accident) exactly what Tailwind's `grid-cols-N` /
/// `grid-rows-N` emit. Anything else returns `None` and gets an advisory.
fn grid_template(v: &lightningcss::properties::grid::TrackSizing) -> Option<gpui::GridTemplate> {
    use lightningcss::properties::grid::{
        RepeatCount, TrackBreadth, TrackListItem, TrackSize, TrackSizing,
    };
    let TrackSizing::TrackList(list) = v else {
        return None; // `none`
    };
    let [TrackListItem::TrackRepeat(repeat)] = list.items.as_slice() else {
        return None;
    };
    let RepeatCount::Number(count) = repeat.count else {
        return None; // auto-fill / auto-fit
    };
    if count < 0 {
        return None;
    }
    let [TrackSize::MinMax { min, max }] = repeat.track_sizes.as_slice() else {
        return None;
    };
    // The max half must be the `1fr` that makes the tracks share the space.
    if !matches!(max, TrackBreadth::Flex(f) if *f == 1.0) {
        return None;
    }
    let min_size = match min {
        TrackBreadth::Length(DimensionPercentage::Dimension(l)) if length_value_px(l) == Some(0.0) => {
            gpui::GridTemplateMinSize::Zero
        }
        TrackBreadth::MinContent => gpui::GridTemplateMinSize::MinContent,
        TrackBreadth::MaxContent => gpui::GridTemplateMinSize::MaxContent,
        _ => return None,
    };
    Some(gpui::GridTemplate {
        repeat: count as u16,
        min_size,
    })
}

/// One track of a `grid-template-columns` / `grid-template-rows` list, in the
/// `minmax()` normal form gpui stores. `None` = a breadth this engine cannot
/// express, which is the caller's cue to advise and drop the whole list (a
/// half-understood track list lays out worse than none at all).
fn track_breadth(b: &lightningcss::properties::grid::TrackBreadth) -> Option<gpui::GridTrackSize> {
    use lightningcss::properties::grid::TrackBreadth as B;
    use gpui::GridTrackSize as G;
    Some(match b {
        B::Auto => G::Auto,
        B::MinContent => G::MinContent,
        B::MaxContent => G::MaxContent,
        B::Flex(f) => G::Fr(*f),
        B::Length(DimensionPercentage::Dimension(l)) => G::Px(length_value_px(l)?),
        // taffy's percentages are fractions, CSS's are 0..100.
        B::Length(DimensionPercentage::Percentage(p)) => G::Percent(p.0),
        // A calc() of lengths is a constant; one mixing in a percentage has
        // no box to resolve against here.
        B::Length(DimensionPercentage::Calc(c)) => G::Px(lp_calc_px(c)?),
    })
}

/// One `<track-size>` — a bare breadth, or `minmax()`. CSS says a bare breadth
/// `X` is `minmax(X, X)` and a bare `<flex>` is `minmax(auto, <flex>)`, which is
/// what makes `1fr` shrink to content when the container has none to spare.
fn track_size(s: &lightningcss::properties::grid::TrackSize) -> Option<gpui::GridTrack> {
    use lightningcss::properties::grid::{TrackBreadth as B, TrackSize as S};
    match s {
        S::TrackBreadth(B::Flex(f)) => Some(gpui::GridTrack::fr(*f)),
        S::TrackBreadth(b) => Some(gpui::GridTrack::fixed(track_breadth(b)?)),
        S::MinMax { min, max } => Some(gpui::GridTrack {
            min: track_breadth(min)?,
            max: track_breadth(max)?,
        }),
        // `fit-content(<len>)`: taffy has the primitive, gpui's GridTrackSize
        // does not carry it. Approximating it with `auto` would silently change
        // the layout, so the whole list is refused instead.
        S::FitContent(_) => None,
    }
}

/// "No `repeat(auto-fill | auto-fit, …)` in this list" — written whenever a
/// list is, because a refinement cannot UNSET a field: without it an earlier
/// rule's run would outlive a later rule's plain list. An empty run is what
/// gpui ignores.
const NO_AUTO_REPEAT: gpui::GridAutoRepeat = gpui::GridAutoRepeat { fit: false, start: 0, len: 0 };

/// A full `grid-template-*` track list, with integer `repeat()` expanded, and
/// its one `repeat(auto-fill | auto-fit, …)` run if it has one.
///
/// The auto run is not expanded here: its count depends on the container's
/// resolved size. gpui hands it to taffy as a repetition (`GridAutoRepeat`),
/// and taffy counts it at layout time — Inventory, Emoji & Stickers and the
/// Connections grids are all `repeat(auto-fill, minmax(<len>, 1fr))` (m26
/// client sweep #15). CSS allows one auto repetition per list, and only with
/// FIXED-size tracks everywhere in the list (css-grid §7.2.3.2); anything else
/// is invalid CSS and returns `None`.
fn grid_tracks(
    v: &lightningcss::properties::grid::TrackSizing,
) -> Option<(Vec<gpui::GridTrack>, gpui::GridAutoRepeat)> {
    use lightningcss::properties::grid::{RepeatCount, TrackListItem, TrackSizing};
    let TrackSizing::TrackList(list) = v else {
        return None; // `none`
    };
    let mut out: Vec<gpui::GridTrack> = Vec::new();
    let mut auto: Option<gpui::GridAutoRepeat> = None;
    for item in &list.items {
        match item {
            TrackListItem::TrackSize(s) => out.push(track_size(s)?),
            TrackListItem::TrackRepeat(rep) => {
                let one: Vec<gpui::GridTrack> =
                    rep.track_sizes.iter().map(track_size).collect::<Option<_>>()?;
                let fit = match rep.count {
                    RepeatCount::Number(count) => {
                        if count < 0 || count > 1024 {
                            return None;
                        }
                        for _ in 0..count {
                            out.extend(one.iter().copied());
                        }
                        continue;
                    }
                    RepeatCount::AutoFill => false,
                    RepeatCount::AutoFit => true,
                };
                if auto.is_some() || one.is_empty() || out.len() + one.len() > u16::MAX as usize {
                    return None;
                }
                auto = Some(gpui::GridAutoRepeat {
                    fit,
                    start: out.len() as u16,
                    len: one.len() as u16,
                });
                out.extend(one);
            }
        }
    }
    if out.is_empty() {
        return None;
    }
    if auto.is_some() && !out.iter().all(fixed_track) {
        return None;
    }
    Some((out, auto.unwrap_or(NO_AUTO_REPEAT)))
}

/// A `<fixed-size>` track (css-grid §7.2.3.1): one end a length or percentage,
/// and not a flexible minimum — the only kind a list with an auto repetition
/// may hold, since the repetition count is worked out from them.
fn fixed_track(t: &gpui::GridTrack) -> bool {
    use gpui::GridTrackSize as G;
    let fixed = |s: G| matches!(s, G::Px(_) | G::Percent(_));
    fixed(t.min) || (fixed(t.max) && !matches!(t.min, G::Fr(_)))
}

/// `grid-row-start` &c. `None` = a NAMED line or area, which needs the named
/// grid taffy does not model here.
fn grid_line(l: &lightningcss::properties::grid::GridLine<'_>) -> Option<gpui::GridPlacement> {
    use lightningcss::properties::grid::GridLine as L;
    match l {
        L::Auto => Some(gpui::GridPlacement::Auto),
        L::Line { index, name: None } => Some(gpui::GridPlacement::Line(*index as i16)),
        L::Span { index, name: None } if *index > 0 => {
            Some(gpui::GridPlacement::Span(*index as u16))
        }
        _ => None,
    }
}

/// `outline` and its longhands accumulate into ONE [`gpui::Outline`], so a
/// sheet that writes `outline: 2px solid transparent; outline-offset: 2px`
/// (which is exactly what Tailwind's `outline-none` is) lands as a single ring.
///
/// The seed is a ZERO-width transparent ring, so a longhand on its own paints
/// nothing — which is what a browser does with `outline-style`'s initial `none`.
fn outline_mut(r: &mut StyleRefinement) -> &mut gpui::Outline {
    r.vn_outline.get_or_insert(gpui::Outline {
        width: px(0.0),
        offset: px(0.0),
        color: Hsla::default(),
        style: gpui::BorderStyle::Solid,
    })
}

/// A list marker, as `list-style-type` names it.
///
/// The ordered kinds carry a counter that `vn_tree::render` resolves from the
/// item's position among its element siblings — which is what `list-style-type`
/// means, and the same number `counter(list-item)` would produce.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMarker {
    /// `list-style: none` — an explicit "no marker", which is a different thing
    /// from "this element declared nothing".
    None,
    Disc,
    Circle,
    Square,
    Decimal,
    DecimalLeadingZero,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
}

impl ListMarker {
    /// The marker string for the `n`-th item (1-based), or `None` for no marker.
    pub fn render(self, n: usize) -> Option<String> {
        fn alpha(mut n: usize, upper: bool) -> String {
            let base = if upper { b'A' } else { b'a' };
            let mut out = Vec::new();
            while n > 0 {
                n -= 1;
                out.push(base + (n % 26) as u8);
                n /= 26;
            }
            out.reverse();
            String::from_utf8(out).unwrap_or_default()
        }
        fn roman(mut n: usize, upper: bool) -> String {
            const TABLE: [(usize, &str); 13] = [
                (1000, "m"),
                (900, "cm"),
                (500, "d"),
                (400, "cd"),
                (100, "c"),
                (90, "xc"),
                (50, "l"),
                (40, "xl"),
                (10, "x"),
                (9, "ix"),
                (5, "v"),
                (4, "iv"),
                (1, "i"),
            ];
            let mut out = String::new();
            for (value, glyph) in TABLE {
                while n >= value {
                    out.push_str(glyph);
                    n -= value;
                }
            }
            if upper { out.to_uppercase() } else { out }
        }
        match self {
            ListMarker::None => None,
            ListMarker::Disc => Some("\u{2022}".into()),
            ListMarker::Circle => Some("\u{25e6}".into()),
            ListMarker::Square => Some("\u{25aa}".into()),
            ListMarker::Decimal => Some(format!("{n}.")),
            ListMarker::DecimalLeadingZero => Some(format!("{n:02}.")),
            ListMarker::LowerAlpha => Some(format!("{}.", alpha(n, false))),
            ListMarker::UpperAlpha => Some(format!("{}.", alpha(n, true))),
            ListMarker::LowerRoman => Some(format!("{}.", roman(n, false))),
            ListMarker::UpperRoman => Some(format!("{}.", roman(n, true))),
        }
    }
}

/// `list-style-type` → the marker it names, or `None` for a counter style with
/// no model here (a `@counter-style` rule, or a literal string).
fn list_marker(style: &lightningcss::properties::list::ListStyleType<'_>) -> Option<ListMarker> {
    use lightningcss::properties::list::{
        CounterStyle as C, ListStyleType as T, PredefinedCounterStyle as P,
    };
    match style {
        T::None => Some(ListMarker::None),
        T::CounterStyle(C::Predefined(p)) => Some(match p {
            P::Disc => ListMarker::Disc,
            P::Circle => ListMarker::Circle,
            P::Square => ListMarker::Square,
            P::Decimal => ListMarker::Decimal,
            P::DecimalLeadingZero => ListMarker::DecimalLeadingZero,
            P::LowerAlpha | P::LowerLatin => ListMarker::LowerAlpha,
            P::UpperAlpha | P::UpperLatin => ListMarker::UpperAlpha,
            P::LowerRoman => ListMarker::LowerRoman,
            P::UpperRoman => ListMarker::UpperRoman,
            _ => return None,
        }),
        _ => None,
    }
}

/// The properties an element wrote `currentColor` into, as a bitmask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CurrentColorTargets(pub u16);

impl CurrentColorTargets {
    pub const BORDER: u16 = 1 << 0;
    pub const OUTLINE: u16 = 1 << 1;
    pub const BACKGROUND: u16 = 1 << 2;
    pub const FILL: u16 = 1 << 3;
    pub const STROKE: u16 = 1 << 4;
    pub const DECORATION: u16 = 1 << 5;
    pub const BORDER_TOP: u16 = 1 << 6;
    pub const BORDER_RIGHT: u16 = 1 << 7;
    pub const BORDER_BOTTOM: u16 = 1 << 8;
    pub const BORDER_LEFT: u16 = 1 << 9;

    pub fn has(self, bit: u16) -> bool {
        self.0 & bit != 0
    }
}

/// CSS matches a background geometry list to the image list by index and
/// repeats the shorter one.
fn cycled<T: Copy>(list: Option<&Rc<Vec<T>>>, i: usize) -> Option<T> {
    let list = list.filter(|l| !l.is_empty())?;
    Some(list[i % list.len()])
}

/// A `background-image: url()` layer: the URL as written, next to its CSS layer
/// index (see [`BackgroundLayer`]). vn-css does not resolve it: vn-tree does,
/// through the loader `<img src>` uses, so the two accept the same URLs.
#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundUrl {
    /// Position in the `background-image` list, layer 0 being the top one.
    pub index: u16,
    pub url: Rc<str>,
}

/// One layer's `background-size`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BackgroundSize {
    /// `<length-percentage> | auto` per axis, `None` being `auto`.
    Explicit(Option<TileLength>, Option<TileLength>),
    Cover,
    Contain,
}

/// One layer of [`Extras::resolve_background_stack`].
#[derive(Clone, Debug, PartialEq)]
pub enum PaintLayer {
    /// A gradient, tiled already.
    Gradient(gpui::Background),
    /// A `url()` image and the geometry that places it.
    Image {
        url: Rc<str>,
        size: BackgroundSize,
        position: (TileLength, TileLength),
        /// `(x, y)`; `space` and `round` count as `repeat`.
        repeat: (bool, bool),
    },
}

/// One `background-image` layer, kept next to the CSS layer index it came from
/// because `background-size` and friends are matched to the image list BY
/// index — and one CSS layer can become several gpui backgrounds when its stop
/// list is too long for one (see [`band_stops`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackgroundLayer {
    /// Position in the `background-image` list, layer 0 being the top one.
    pub index: u16,
    pub background: gpui::Background,
}

/// A `background-size` or `background-position` component: CSS lets either be
/// a length or a percentage, and a percentage of WHAT differs between the two
/// (see [`gpui::Background::tiled`]), so the distinction is carried rather than
/// resolved here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TileLength {
    Pixels(f32),
    Fraction(f32),
}

impl TileLength {
    pub fn value(self) -> f32 {
        match self {
            TileLength::Pixels(v) | TileLength::Fraction(v) => v,
        }
    }

    pub fn is_fraction(self) -> bool {
        matches!(self, TileLength::Fraction(_))
    }

    fn from_length_percentage(v: &LengthPercentage) -> Option<Self> {
        match v {
            DimensionPercentage::Percentage(p) => Some(TileLength::Fraction(p.0)),
            DimensionPercentage::Dimension(d) => length_value_px(d).map(TileLength::Pixels),
            DimensionPercentage::Calc(c) => lp_calc_px(c).map(TileLength::Pixels),
        }
    }
}

/// One colour stop, reduced to what the GPU carries.
#[derive(Clone, Copy, Debug)]
struct Stop {
    color: Hsla,
    /// `None` until [`resolve_stop_positions`] fills it in.
    position: Option<StopPosition>,
}

/// Where a stop sits on the gradient line. CSS lets an author write either, and
/// neither can be converted into the other here: the line's length is a
/// function of the box, which only the shader knows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum StopPosition {
    /// A fraction of the gradient line.
    Fraction(f32),
    /// An absolute length along it.
    Pixels(f32),
}

/// Fills in the stops CSS left unpositioned: the first sits at the start of the
/// line and the last at the end, and a run in between is spread evenly across
/// the gap its neighbours leave. Positions are then made non-decreasing, which
/// is what turns a pair written out of order into a hard stop rather than a
/// ramp that runs backwards.
///
/// A run whose two anchors are written in DIFFERENT units cannot be
/// interpolated — that is the same missing quantity as above — so those stops
/// fall back to an even spread of the whole line by index.
fn resolve_stop_positions(stops: &mut [Stop]) {
    let n = stops.len();
    if n == 0 {
        return;
    }
    if stops[0].position.is_none() {
        stops[0].position = Some(StopPosition::Fraction(0.0));
    }
    if stops[n - 1].position.is_none() {
        stops[n - 1].position = Some(StopPosition::Fraction(1.0));
    }

    let mut anchor = 0;
    for i in 1..n {
        if stops[i].position.is_none() {
            continue;
        }
        if i > anchor + 1 {
            let gap = (i - anchor) as f32;
            let ends = match (stops[anchor].position, stops[i].position) {
                (Some(StopPosition::Fraction(a)), Some(StopPosition::Fraction(b))) => {
                    Some((a, b, true))
                }
                (Some(StopPosition::Pixels(a)), Some(StopPosition::Pixels(b))) => {
                    Some((a, b, false))
                }
                _ => None,
            };
            for (step, slot) in (anchor + 1..i).enumerate() {
                let t = (step + 1) as f32 / gap;
                stops[slot].position = Some(match ends {
                    Some((a, b, true)) => StopPosition::Fraction(a + (b - a) * t),
                    Some((a, b, false)) => StopPosition::Pixels(a + (b - a) * t),
                    None => StopPosition::Fraction(slot as f32 / (n - 1).max(1) as f32),
                });
            }
        }
        anchor = i;
    }

    // Non-decreasing, per unit: comparing a fraction against a length would be
    // comparing two different quantities, so a change of unit resets the floor.
    let mut floor: Option<StopPosition> = None;
    for stop in stops.iter_mut() {
        match (&mut stop.position, floor) {
            (Some(StopPosition::Fraction(p)), Some(StopPosition::Fraction(f))) => *p = p.max(f),
            (Some(StopPosition::Pixels(p)), Some(StopPosition::Pixels(f))) => *p = p.max(f),
            _ => {}
        }
        floor = stop.position;
    }
}

/// CSS's colour transition hint — a bare position between two stops that moves
/// the 50 % mix onto it. The ramp gpui evaluates is piecewise linear, so the
/// hint becomes the extra stop it describes: the midpoint colour, AT the hint.
/// The curve then passes through the right colour at the right place and
/// differs only in the easing either side of it.
fn hint_stop(before: Hsla, after: Hsla, position: StopPosition) -> Stop {
    Stop {
        color: mix_hsla(before, after),
        position: Some(position),
    }
}

/// The midpoint of two colours, hue averaged the short way round so
/// amber→yellow→amber does not come back cyan.
fn mix_hsla(a: Hsla, b: Hsla) -> Hsla {
    let mut hue_delta = b.h - a.h;
    if hue_delta > 0.5 {
        hue_delta -= 1.0;
    } else if hue_delta < -0.5 {
        hue_delta += 1.0;
    }
    Hsla {
        h: (a.h + hue_delta * 0.5).rem_euclid(1.0),
        s: (a.s + b.s) * 0.5,
        l: (a.l + b.l) * 0.5,
        a: (a.a + b.a) * 0.5,
    }
}

/// A `<length-percentage>` stop position: a percentage is a fraction of the
/// gradient line, anything absolute is a length along it.
fn stop_position_lp(v: &LengthPercentage) -> Option<StopPosition> {
    match v {
        DimensionPercentage::Percentage(p) => Some(StopPosition::Fraction(p.0)),
        DimensionPercentage::Dimension(d) => length_value_px(d).map(StopPosition::Pixels),
        DimensionPercentage::Calc(c) => lp_calc_px(c).map(StopPosition::Pixels),
    }
}

/// The same for a conic gradient, whose stops are angles. One turn IS the
/// gradient line, so both spellings reduce to a fraction of it.
fn stop_position_angle(
    v: &DimensionPercentage<lightningcss::values::angle::Angle>,
) -> Option<StopPosition> {
    match v {
        DimensionPercentage::Percentage(p) => Some(StopPosition::Fraction(p.0)),
        DimensionPercentage::Dimension(a) => {
            Some(StopPosition::Fraction(a.to_degrees() / 360.0))
        }
        DimensionPercentage::Calc(_) => {
            calc_dropped();
            None
        }
    }
}

/// Turns a gradient's item list — stops interleaved with transition hints —
/// into a flat, fully positioned stop list.
///
/// `None` means a colour did not resolve to a concrete value, which the caller
/// reports as an unresolved colour rather than painting a guess.
fn collect_stops<P: Clone>(
    items: &[lightningcss::values::gradient::GradientItem<P>],
    position_of: &dyn Fn(&P) -> Option<StopPosition>,
) -> Option<Vec<Stop>> {
    use lightningcss::values::gradient::GradientItem;

    let mut stops: Vec<Stop> = Vec::with_capacity(items.len());
    let mut pending_hints: Vec<StopPosition> = Vec::new();
    for item in items {
        match item {
            GradientItem::ColorStop(s) => {
                let color = color(&s.color)?;
                // A hint only means something between two stops, so it is
                // resolved once the stop after it arrives.
                for hint in pending_hints.drain(..) {
                    if let Some(before) = stops.last().map(|s: &Stop| s.color) {
                        stops.push(hint_stop(before, color, hint));
                    }
                }
                stops.push(Stop {
                    color,
                    position: s.position.as_ref().and_then(|p| position_of(p)),
                });
            }
            GradientItem::Hint(h) => {
                if let Some(p) = position_of(h) {
                    pending_hints.push(p);
                }
            }
        }
    }
    if stops.is_empty() {
        return None;
    }
    resolve_stop_positions(&mut stops);
    Some(stops)
}

/// A radial or conic gradient's centre, as a fraction of the box or an absolute
/// offset into it — CSS's `at <position>`.
fn gradient_center(position: &lightningcss::values::position::Position) -> ((f32, f32), bool) {
    use lightningcss::values::position::{
        HorizontalPositionKeyword as H, PositionComponent as P, VerticalPositionKeyword as V,
    };

    fn component<S: Copy>(c: &P<S>, from_start: impl Fn(S) -> bool) -> (f32, bool) {
        let absolute = |v: &LengthPercentage| match v {
            DimensionPercentage::Dimension(d) => length_value_px(d),
            DimensionPercentage::Calc(c) => lp_calc_px(c),
            DimensionPercentage::Percentage(_) => None,
        };
        match c {
            P::Center => (0.5, false),
            P::Length(DimensionPercentage::Percentage(p)) => (p.0, false),
            P::Length(v) => absolute(v).map(|px| (px, true)).unwrap_or((0.5, false)),
            P::Side { side, offset } => {
                let near = from_start(*side);
                match offset {
                    None => (if near { 0.0 } else { 1.0 }, false),
                    Some(DimensionPercentage::Percentage(p)) => {
                        (if near { p.0 } else { 1.0 - p.0 }, false)
                    }
                    // A length measured from the FAR side would need the box.
                    // `right 10px` is not a shape stylesheets write; the near
                    // side is, and it is exact.
                    Some(v) => match absolute(v) {
                        Some(px) if near => (px, true),
                        _ => (if near { 0.0 } else { 1.0 }, false),
                    },
                }
            }
        }
    }

    let (x, x_px) = component(&position.x, |s| matches!(s, H::Left));
    let (y, y_px) = component(&position.y, |s| matches!(s, V::Top));
    // One flag covers both axes, so a mixed pair keeps the fraction and drops
    // the length — `at 10px center` is vanishingly rare and `at 50% 50%` is
    // what every stylesheet actually writes.
    match (x_px, y_px) {
        (true, true) => ((x, y), true),
        (true, false) => ((0.0, y), false),
        (false, true) => ((x, 0.0), false),
        (false, false) => ((x, y), false),
    }
}

/// Packs resolved stops onto a background, flagging the ones written as
/// absolute lengths so the shader divides them by the gradient line.
fn with_resolved_stops(background: gpui::Background, stops: &[Stop]) -> gpui::Background {
    let packed: Vec<gpui::LinearColorStop> = stops
        .iter()
        .map(|s| {
            let value = match s.position {
                Some(StopPosition::Pixels(v)) | Some(StopPosition::Fraction(v)) => v,
                None => 0.0,
            };
            gpui::linear_color_stop(s.color, value)
        })
        .collect();
    let mut background = background.with_stops(&packed);
    for (i, stop) in stops.iter().enumerate().take(gpui::GRADIENT_STOPS) {
        if matches!(stop.position, Some(StopPosition::Pixels(_))) {
            background = background.stop_in_pixels(i);
        }
    }
    background
}

/// Splits a stop list too long for one [`gpui::Background`] into consecutive
/// BANDS, each its own background layer, painted over one another.
///
/// A band carries its own slice of the ramp plus a transparent guard stop at
/// each shared edge, so every pixel is opaque in exactly one band and
/// transparent in the rest. Compositing transparent over a band is the identity,
/// and the guards are hard stops (two positions the same), so there is no seam
/// and no double blend — the result is the same ramp, exactly, without a cap on
/// how many stops it may have.
fn band_stops(stops: &[Stop]) -> Vec<Vec<Stop>> {
    const REAL_PER_BAND: usize = gpui::GRADIENT_STOPS - 2;
    if stops.len() <= gpui::GRADIENT_STOPS {
        return vec![stops.to_vec()];
    }
    let transparent = |at: Option<StopPosition>| Stop {
        color: Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        },
        position: at,
    };
    let mut bands = Vec::new();
    let mut start = 0;
    while start + 1 < stops.len() {
        let end = (start + REAL_PER_BAND - 1).min(stops.len() - 1);
        let mut band: Vec<Stop> = Vec::with_capacity(gpui::GRADIENT_STOPS);
        if start > 0 {
            band.push(transparent(stops[start].position));
        }
        band.extend_from_slice(&stops[start..=end]);
        if end + 1 < stops.len() {
            band.push(transparent(stops[end].position));
        }
        bands.push(band);
        start = end;
    }
    bands
}

/// A `mask-image` list, as [`MaskImages`] tells layers apart.
fn mask_images<'a>(
    images: impl Iterator<Item = &'a lightningcss::values::image::Image<'a>>,
) -> MaskImages {
    use lightningcss::values::image::Image;
    let images: Vec<_> = images.collect();
    if images.iter().all(|i| matches!(i, Image::None)) {
        MaskImages::None
    } else if images.len() == 2 && images.iter().all(|i| opaque_mask_layer(i)) {
        MaskImages::TwoOpaque
    } else {
        MaskImages::Other
    }
}

/// Does this mask layer cover its whole box? A gradient whose every stop is an
/// opaque colour does, at any size and angle: the ends extend.
fn opaque_mask_layer(image: &lightningcss::values::image::Image<'_>) -> bool {
    use lightningcss::values::gradient::{Gradient, GradientItem};
    use lightningcss::values::image::Image;
    fn opaque<D>(items: &[GradientItem<D>]) -> bool {
        items.iter().all(|item| match item {
            GradientItem::ColorStop(stop) => color(&stop.color).is_some_and(|c| c.a >= 1.0),
            GradientItem::Hint(_) => true,
        })
    }
    let Image::Gradient(gradient) = image else {
        return false;
    };
    match gradient.as_ref() {
        Gradient::Linear(g) | Gradient::RepeatingLinear(g) => opaque(&g.items),
        Gradient::Radial(g) | Gradient::RepeatingRadial(g) => opaque(&g.items),
        Gradient::Conic(g) | Gradient::RepeatingConic(g) => opaque(&g.items),
        Gradient::WebKitGradient(_) => false,
    }
}

/// A `mask-clip` list, as [`MaskClips`] tells it apart.
fn mask_clips<'a>(
    clips: impl Iterator<Item = &'a lightningcss::properties::masking::MaskClip>,
) -> MaskClips {
    use lightningcss::properties::masking::{GeometryBox as B, MaskClip as C};
    let clips: Vec<_> = clips.collect();
    match clips.as_slice() {
        [C::GeometryBox(B::ContentBox), C::GeometryBox(B::BorderBox)] => MaskClips::ContentThenBorder,
        _ => MaskClips::Other,
    }
}

fn mask_compositing(c: lightningcss::properties::masking::MaskComposite) -> MaskCompositing {
    use lightningcss::properties::masking::MaskComposite as C;
    match c {
        C::Add => MaskCompositing::Add,
        C::Exclude => MaskCompositing::Exclude,
        C::Subtract | C::Intersect => MaskCompositing::Other,
    }
}

/// One `-webkit-mask-composite` keyword. The top layer is the source and the
/// layers below are the destination, so `destination-out` (below minus this
/// one) leaves the same ring as `xor` over the gradient-border pair.
fn webkit_mask_compositing(keyword: &str) -> MaskCompositing {
    match keyword.to_ascii_lowercase().as_str() {
        "source-over" => MaskCompositing::Add,
        "xor" | "destination-out" => MaskCompositing::Exclude,
        _ => MaskCompositing::Other,
    }
}

/// Says once that a mask is not applied, for the declaration just recorded in
/// `x` (one rule's declarations, not the element's cascade). The
/// gradient-border idiom says nothing: its parts are each supported.
fn advise_mask(x: &Extras, diagnostics: &mut Vec<String>) {
    let unsupported = x.mask_images == Some(MaskImages::Other)
        || x.mask_composite == Some(MaskCompositing::Other)
        || (x.mask_images == Some(MaskImages::TwoOpaque) && x.mask_clips == Some(MaskClips::Other));
    if unsupported {
        warn_once(
            diagnostics,
            "`mask` is not applied (v1), and the element paints unmasked. gpui has no \
             per-element alpha mask. The one mask vue-native cuts is the gradient-border ring: \
             two opaque layers, the top one clipped to `content-box`, with `mask-composite: \
             exclude` (or `-webkit-mask-composite: xor`). That paints the background in the \
             padding and border only",
        );
    }
}

/// Maps a `background-image` list onto [`Extras::background_images`], keeping
/// each gpui background next to the CSS layer index that placed it.
fn collect_background_images<'a>(
    images: impl Iterator<Item = &'a lightningcss::values::image::Image<'a>>,
    x: &mut Extras,
    diagnostics: &mut Vec<String>,
) {
    let mut layers: Vec<BackgroundLayer> = Vec::new();
    let mut urls: Vec<BackgroundUrl> = Vec::new();
    for (index, image) in images.enumerate() {
        if let lightningcss::values::image::Image::Url(u) = image {
            // `url("")` is an invalid image, which CSS draws as no layer.
            if !u.url.trim().is_empty() {
                urls.push(BackgroundUrl {
                    index: index.min(u16::MAX as usize) as u16,
                    url: Rc::from(u.url.as_ref()),
                });
            }
            continue;
        }
        let Some((backgrounds, key)) = gradient_layer(image, diagnostics) else {
            continue;
        };
        // The KEY colour describes the whole element, and the top layer is what
        // a reader sees, so the first one to resolve wins.
        if x.background_key_color.is_none() || index == 0 {
            x.background_key_color = Some(key);
        }
        for background in backgrounds {
            layers.push(BackgroundLayer {
                index: index.min(u16::MAX as usize) as u16,
                background,
            });
        }
    }
    x.background_images = Some(Rc::new(layers));
    x.background_urls = Some(Rc::new(urls));
}

/// `background-size`. A gradient has no intrinsic size, so for one `auto`,
/// `cover` and `contain` all resolve to the box — which is what an untiled
/// background already is ([`Extras::resolve_background_layers`]). A `url()`
/// image has one, and vn-tree works them out against it.
fn background_size(size: &lightningcss::properties::background::BackgroundSize) -> BackgroundSize {
    use lightningcss::properties::background::BackgroundSize as BS;
    match size {
        BS::Cover => BackgroundSize::Cover,
        BS::Contain => BackgroundSize::Contain,
        BS::Explicit { width, height } => {
            let axis = |v: &LengthPercentageOrAuto| match v {
                LengthPercentageOrAuto::Auto => None,
                LengthPercentageOrAuto::LengthPercentage(lp) => {
                    TileLength::from_length_percentage(lp)
                }
            };
            BackgroundSize::Explicit(axis(width), axis(height))
        }
    }
}

fn background_repeat(
    repeat: &lightningcss::properties::background::BackgroundRepeat,
) -> (bool, bool) {
    use lightningcss::properties::background::BackgroundRepeatKeyword as K;
    // `space` and `round` adjust the gap or the tile size so a whole number of
    // tiles fits. Both still repeat, and both differ from `repeat` only by a
    // sub-tile amount; treating them as `repeat` is the closer of the two
    // available answers.
    let on = |k: &K| !matches!(k, K::NoRepeat);
    (on(&repeat.x), on(&repeat.y))
}

fn background_position(
    position: &lightningcss::properties::background::BackgroundPosition,
    diagnostics: &mut Vec<String>,
) -> (TileLength, TileLength) {
    (
        position_component(&position.x, true, diagnostics),
        position_component(&position.y, false, diagnostics),
    )
}

/// One axis of a `background-position`.
///
/// A percentage aligns the tile inside the space it leaves — `50%` centres it —
/// so it stays a fraction all the way to the shader, which is the only place
/// that knows the box and the tile.
fn position_component<S: Copy>(
    component: &lightningcss::values::position::PositionComponent<S>,
    horizontal: bool,
    diagnostics: &mut Vec<String>,
) -> TileLength
where
    S: PositionSide,
{
    use lightningcss::values::position::PositionComponent as P;
    let absolute = |v: &LengthPercentage| match v {
        DimensionPercentage::Dimension(d) => length_value_px(d),
        DimensionPercentage::Calc(c) => lp_calc_px(c),
        DimensionPercentage::Percentage(_) => None,
    };
    match component {
        P::Center => TileLength::Fraction(0.5),
        P::Length(DimensionPercentage::Percentage(p)) => TileLength::Fraction(p.0),
        P::Length(v) => absolute(v)
            .map(TileLength::Pixels)
            .unwrap_or(TileLength::Fraction(0.0)),
        P::Side { side, offset } => {
            let near = side.is_near_edge();
            match offset {
                None => TileLength::Fraction(if near { 0.0 } else { 1.0 }),
                Some(DimensionPercentage::Percentage(p)) => {
                    TileLength::Fraction(if near { p.0 } else { 1.0 - p.0 })
                }
                Some(v) if near => absolute(v)
                    .map(TileLength::Pixels)
                    .unwrap_or(TileLength::Fraction(0.0)),
                // `right 10px` is "10px in from the far edge", which is one
                // fraction MINUS one length — two numbers where the tile
                // carries one. The near-edge spellings and every percentage
                // form are exact; this is the one that is not.
                Some(_) => {
                    warn_once(
                        diagnostics,
                        &format!(
                            "`background-position: {} <length>` is placed flush against that edge \
                             (v1) — an offset from the FAR edge is a fraction of the box minus a \
                             length, and the tile carries one number per axis. Write it from the \
                             near edge (`left`/`top`), or as a percentage, both of which are exact",
                            if horizontal { "right" } else { "bottom" }
                        ),
                    );
                    TileLength::Fraction(1.0)
                }
            }
        }
    }
}

/// Lets [`position_component`] ask a horizontal or vertical side keyword the
/// one question it has: is this the edge offsets are measured FROM?
pub(crate) trait PositionSide {
    fn is_near_edge(self) -> bool;
}

impl PositionSide for lightningcss::values::position::HorizontalPositionKeyword {
    fn is_near_edge(self) -> bool {
        matches!(self, Self::Left)
    }
}

impl PositionSide for lightningcss::values::position::VerticalPositionKeyword {
    fn is_near_edge(self) -> bool {
        matches!(self, Self::Top)
    }
}

/// One CSS background image layer → the gpui backgrounds that paint it, plus
/// the single colour that stands in for the layer when only one is available
/// ([`Extras::background_key_color`]).
///
/// The whole gradient grammar renders: `linear-`, `radial-` and
/// `conic-gradient()` and their three `repeating-` forms, up to
/// [`gpui::GRADIENT_STOPS`] colour stops inline and any number through
/// [`band_stops`], stop positions written as percentages OR absolute lengths,
/// transition hints, corner keywords aimed at the real corner, and `at
/// <position>` / `circle` / `ellipse` / the four extent keywords.
///
/// Tiling (`background-size` / `-position` / `-repeat`) is NOT applied here:
/// those are separate properties that can arrive from separate rules, so the
/// caller folds them on afterwards.
fn gradient_layer(
    img: &lightningcss::values::image::Image<'_>,
    diagnostics: &mut Vec<String>,
) -> Option<(Vec<gpui::Background>, Hsla)> {
    use lightningcss::values::gradient::{Circle, Ellipse, EndingShape, Gradient, LineDirection, ShapeExtent};
    use lightningcss::values::image::Image;
    use lightningcss::values::position::{HorizontalPositionKeyword as H, VerticalPositionKeyword as V};

    let gradient = match img {
        Image::Gradient(g) => g.as_ref(),
        // Carried as a `BackgroundUrl` by `collect_background_images`.
        Image::Url(_) => return None,
        Image::ImageSet(_) => {
            warn_once(
                diagnostics,
                "background-image: image-set() is not painted (v1) — write one source as a plain \
                 `url()`, which is; there is no resolution negotiation here",
            );
            return None;
        }
        Image::None => return None,
    };

    // A `repeating-*` gradient restarts its ramp every period instead of
    // clamping at the ends; everything else about it is identical, which is why
    // the two forms share every arm below.
    let (background, stops) = match gradient {
        Gradient::Linear(l) | Gradient::RepeatingLinear(l) => {
            let stops = collect_stops(&l.items, &stop_position_lp)?;
            let angle = match &l.direction {
                LineDirection::Angle(a) => Some(a.to_degrees()),
                LineDirection::Vertical(V::Top) => Some(0.0),
                LineDirection::Vertical(V::Bottom) => Some(180.0),
                LineDirection::Horizontal(H::Right) => Some(90.0),
                LineDirection::Horizontal(H::Left) => Some(270.0),
                // CSS aims the line at the actual corner, so the angle depends
                // on the box's aspect ratio and is resolved in the shader.
                LineDirection::Corner { .. } => None,
            };
            let background = match angle {
                Some(angle) => gpui::linear_gradient_stops(angle, &[]),
                None => {
                    let LineDirection::Corner { horizontal, vertical } = &l.direction else {
                        unreachable!("only a corner direction leaves the angle unresolved")
                    };
                    gpui::linear_gradient_stops(0.0, &[]).to_corner(
                        match (horizontal, vertical) {
                            (H::Right, V::Top) => 0,
                            (H::Right, V::Bottom) => 1,
                            (H::Left, V::Bottom) => 2,
                            (H::Left, V::Top) => 3,
                        },
                    )
                }
            };
            (background, stops)
        }
        Gradient::Radial(r) | Gradient::RepeatingRadial(r) => {
            let stops = collect_stops(&r.items, &stop_position_lp)?;
            let (center, center_px) = gradient_center(&r.position);
            let extent = |e: &ShapeExtent| match e {
                ShapeExtent::ClosestSide => 1u8,
                ShapeExtent::FarthestSide => 2,
                ShapeExtent::ClosestCorner => 3,
                ShapeExtent::FarthestCorner => 4,
            };
            let background = gpui::radial_gradient(&[]).at(center, center_px);
            let background = match &r.shape {
                EndingShape::Circle(Circle::Extent(e)) => background.with_extent(extent(e), false),
                EndingShape::Circle(Circle::Radius(len)) => match css_length_px(len) {
                    Some(radius) => background.with_radii((radius, radius), true),
                    None => background.with_extent(4, false),
                },
                EndingShape::Ellipse(Ellipse::Extent(e)) => background.with_extent(extent(e), true),
                EndingShape::Ellipse(Ellipse::Size { x, y }) => {
                    let axis = |v: &LengthPercentage| match v {
                        DimensionPercentage::Percentage(p) => Some((p.0, false)),
                        DimensionPercentage::Dimension(d) => length_value_px(d).map(|v| (v, true)),
                        DimensionPercentage::Calc(c) => {
                            lp_calc_px(c).map(|v| (v, true))
                        }
                    };
                    match (axis(x), axis(y)) {
                        // Both axes have to agree on the unit, because one flag
                        // covers the pair. A mixed pair keeps the fractions.
                        (Some((rx, true)), Some((ry, true))) => {
                            background.with_radii((rx, ry), true)
                        }
                        (Some((rx, false)), Some((ry, false))) => {
                            background.with_radii((rx, ry), false)
                        }
                        _ => background.with_extent(4, true),
                    }
                }
            };
            (background, stops)
        }
        Gradient::Conic(c) | Gradient::RepeatingConic(c) => {
            let stops = collect_stops(&c.items, &stop_position_angle)?;
            let (center, center_px) = gradient_center(&c.position);
            (
                gpui::conic_gradient(c.angle.to_degrees(), &[]).at(center, center_px),
                stops,
            )
        }
        Gradient::WebKitGradient(_) => {
            warn_once(
                diagnostics,
                "the legacy `-webkit-gradient()` is not painted (v1) — write the standard \
                 `linear-gradient()`, which does",
            );
            return None;
        }
    };

    let repeating = matches!(
        gradient,
        Gradient::RepeatingLinear(_) | Gradient::RepeatingRadial(_) | Gradient::RepeatingConic(_)
    );
    let background = background.repeating(repeating);

    // A single stop is a flat fill, which is what CSS paints too.
    let key = if stops.len() == 1 {
        stops[0].color
    } else {
        mix_hsla(stops[0].color, stops[stops.len() - 1].color)
    };
    let layers = band_stops(&stops)
        .into_iter()
        .map(|band| with_resolved_stops(background, &band))
        .collect();
    Some((layers, key))
}

/// Advisories are ONE-SHOT: `diagnostics` accumulates for a whole stylesheet
/// compile, and a generated sheet hits the same gap over and over (a broad
/// Tailwind sweep produced 123 `--tw-*` declarations and 63 `var()`-composed
/// values). Printing each would bury every other message, so a given advisory
/// is recorded once and the rest are folded into it.
pub(crate) fn once(diagnostics: &mut Vec<String>, msg: String) {
    if !diagnostics.iter().any(|d| *d == msg) {
        diagnostics.push(msg);
    }
}

fn unsupported(diagnostics: &mut Vec<String>, what: &str) {
    once(diagnostics, format!("unsupported value for {what} (v1)"));
}

fn warn_once(diagnostics: &mut Vec<String>, msg: &str) {
    once(diagnostics, msg.to_string());
}

/// [`color`] only understands a resolved RGBA. `currentColor`, `color-mix()`,
/// `light-dark()` and friends land here — and USED to fall through silently,
/// which meant Tailwind's `bg-current` / `text-current` did nothing at all with
/// no way to find out.
/// Substitutes an element's computed `color` into the properties that were
/// written as `currentColor`.
///
/// Runs after the cascade rather than during it: `currentColor` is a *computed
/// value*, defined as this element's `color`, which is not known until the
/// whole element has been folded and its parent's colour is in hand.
pub(crate) fn apply_current_color(
    r: &mut StyleRefinement,
    targets: CurrentColorTargets,
    color: Hsla,
) {
    let sides = [r.vn_border_colors.top, r.vn_border_colors.right,
        r.vn_border_colors.bottom, r.vn_border_colors.left];
    if sides.iter().all(|side| *side == Some(None))
        || (targets.has(CurrentColorTargets::BORDER) && r.border_color.is_none()
            && sides.iter().all(Option::is_none))
    {
        r.border_color = Some(color);
    }
    // A currentColor placeholder belongs to this refinement, not every
    // state that contributed to Extras. A hover literal must not cancel a
    // base placeholder, nor may hover currentColor repaint a base literal.
    for slot in [&mut r.vn_border_colors.top, &mut r.vn_border_colors.right,
        &mut r.vn_border_colors.bottom, &mut r.vn_border_colors.left]
    {
        if *slot == Some(None) { *slot = Some(Some(color)); }
    }
    if targets.has(CurrentColorTargets::OUTLINE) {
        if let Some(outline) = r.vn_outline.as_mut() {
            outline.color = color;
        }
    }
    if targets.has(CurrentColorTargets::BACKGROUND) {
        r.background = Some(gpui::Fill::Color(color.into()));
    }
}

pub(crate) fn border_needs_current_color(r: &StyleRefinement) -> bool {
    [r.vn_border_colors.top, r.vn_border_colors.right,
        r.vn_border_colors.bottom, r.vn_border_colors.left].contains(&Some(None))
}

fn border_side_color(
    slot: &mut Option<Option<Hsla>>, bit: u16, value: &CssColor,
    extras: &mut Extras, diagnostics: &mut Vec<String>,
) {
    let resolved = match value {
        CssColor::CurrentColor => None,
        value => match color(value) {
            Some(color) => Some(color),
            None => { unresolved_color(diagnostics, "border-color"); return; }
        },
    };
    *slot = Some(resolved);
    extras.current_color_overrides.0 |= bit | CurrentColorTargets::BORDER;
    extras.current_color.0 &= !(bit | CurrentColorTargets::BORDER);
    if matches!(value, CssColor::CurrentColor) { extras.current_color.0 |= bit; }
}

fn all_border_colors(r: &mut StyleRefinement, x: &mut Extras, colors: [&CssColor; 4], diagnostics: &mut Vec<String>) {
    border_side_color(&mut r.vn_border_colors.top, CurrentColorTargets::BORDER_TOP, colors[0], x, diagnostics);
    border_side_color(&mut r.vn_border_colors.right, CurrentColorTargets::BORDER_RIGHT, colors[1], x, diagnostics);
    border_side_color(&mut r.vn_border_colors.bottom, CurrentColorTargets::BORDER_BOTTOM, colors[2], x, diagnostics);
    border_side_color(&mut r.vn_border_colors.left, CurrentColorTargets::BORDER_LEFT, colors[3], x, diagnostics);
    r.border_color = color(colors[0]);
    if colors.iter().all(|value| matches!(value, CssColor::CurrentColor)) {
        x.current_color.0 |= CurrentColorTargets::BORDER;
    }
}

fn unresolved_color(diagnostics: &mut Vec<String>, what: &str) {
    once(
        diagnostics,
        format!(
            "`{what}` value does not resolve to a concrete color (v1) — `currentColor`, \
             `color-mix()` and system colors are not computed"
        ),
    );
}


// `--shading`, the one custom property that IS mapped. Declared here rather
// than in lib.rs so the whole effect surface — the module, its interception in
// the `Custom(_)` branch below, and nothing else — lands in one place.
#[path = "effect.rs"]
pub(crate) mod effect;

// ---------------------------------------------------------------------------
// the mapper
// ---------------------------------------------------------------------------

/// `object-fit: <keyword>`. lightningcss has no typed `object-fit`, so the
/// declaration arrives as an unknown property carrying a raw token list — the
/// same shape `--shading` uses, minus the function call.
fn object_fit(value: &lightningcss::properties::custom::TokenList, diagnostics: &mut Vec<String>) -> Option<ObjectFit> {
    match single_ident(value, "object-fit", diagnostics) {
        Some("fill") => Some(ObjectFit::Fill),
        Some("contain") => Some(ObjectFit::Contain),
        Some("cover") => Some(ObjectFit::Cover),
        Some("scale-down") => Some(ObjectFit::ScaleDown),
        Some("none") => Some(ObjectFit::None),
        other => {
            once(
                diagnostics,
                format!(
                    "unsupported value for object-fit: {} — expected fill | contain | cover | \
                     scale-down | none",
                    other.unwrap_or("<not a keyword>")
                ),
            );
            None
        }
    }
}

/// The custom-property name that carries a caption-button role.
pub(crate) const WINDOW_CONTROL: &str = "--window-control";

/// The single keyword a token list carries, or `None` when it carries zero or
/// more than one (a browser drops a multi-keyword declaration on any of these
/// properties).
fn single_ident<'a>(
    value: &'a lightningcss::properties::custom::TokenList<'_>,
    what: &str,
    diagnostics: &mut Vec<String>,
) -> Option<&'a str> {
    use lightningcss::properties::custom::{Token, TokenOrValue};
    let mut idents = value.0.iter().filter_map(|t| match t {
        TokenOrValue::Token(Token::Ident(s)) => Some(s.as_ref()),
        _ => None,
    });
    let first = idents.next();
    if idents.next().is_some() {
        once(
            diagnostics,
            format!("`{what}` takes a single keyword — the declaration was dropped"),
        );
        return None;
    }
    first
}

/// `clip-path` as gpui's rounded content mask carries it (`gpui::VnClipPath`),
/// or `None` (advised) for a shape the mask cannot be.
///
/// `inset()` and `circle()` are exact, and so is a `<geometry-box>` alone,
/// rounded by the element's `border-radius`. `ellipse()` and `polygon()` are
/// exact too, as an isolated layer composited through the shape (gpui
/// `VnClipGeometry`); `path()` arrives unparsed ([`clip_path_path`]).
/// `url()` is not applied.
fn clip_path(v: &CssClipPath<'_>, diagnostics: &mut Vec<String>) -> Option<gpui::VnClipPath> {
    use gpui::{VnClipBox, VnClipPath, VnShapeRadius};
    use lightningcss::properties::masking::GeometryBox as G;
    use lightningcss::values::shape::{BasicShape, ShapeRadius};
    // CSS Masking 1 §5.1: an element with a CSS box takes `fill-box` as its
    // content box, and `stroke-box` and `view-box` as its border box.
    let reference = |b: &G| match b {
        G::MarginBox => VnClipBox::Margin,
        G::BorderBox | G::StrokeBox | G::ViewBox => VnClipBox::Border,
        G::PaddingBox => VnClipBox::Padding,
        G::ContentBox | G::FillBox => VnClipBox::Content,
    };
    let radius = |r: &ShapeRadius| match r {
        ShapeRadius::LengthPercentage(lp) => lp_definite(lp).map(VnShapeRadius::Length),
        ShapeRadius::ClosestSide => Some(VnShapeRadius::ClosestSide),
        ShapeRadius::FarthestSide => Some(VnShapeRadius::FarthestSide),
    };
    let unsupported = |diagnostics: &mut Vec<String>, what: &str| {
        clip_path_unsupported(diagnostics, what);
        None
    };
    match v {
        CssClipPath::None => Some(VnClipPath::None),
        CssClipPath::Box(b) => Some(VnClipPath::Box(reference(b))),
        CssClipPath::Url(_) => unsupported(diagnostics, "url()"),
        CssClipPath::Shape { shape, reference_box } => {
            let reference = reference(reference_box);
            match &**shape {
                BasicShape::Inset(inset) => {
                    let r = &inset.rect;
                    let insets = [lp_definite(&r.0)?, lp_definite(&r.1)?, lp_definite(&r.2)?, lp_definite(&r.3)?];
                    let corner = |c: &lightningcss::values::size::Size2D<LengthPercentage>| {
                        Some(gpui::VnCornerRadius { x: lp_definite(&c.0)?, y: lp_definite(&c.1)? })
                    };
                    let b = &inset.radius;
                    let radii =
                        [corner(&b.top_left)?, corner(&b.top_right)?, corner(&b.bottom_right)?, corner(&b.bottom_left)?];
                    Some(VnClipPath::Inset { reference, insets, radii })
                }
                BasicShape::Circle(c) => {
                    let r = radius(&c.radius)?;
                    let center = shape_center(&c.position)?;
                    Some(VnClipPath::Ellipse { reference, center, radius: [r, r], circle: true })
                }
                BasicShape::Ellipse(e) => {
                    let (rx, ry) = (radius(&e.radius_x)?, radius(&e.radius_y)?);
                    let center = shape_center(&e.position)?;
                    // Equal radii are the rounded mask; others the outline.
                    Some(VnClipPath::Ellipse { reference, center, radius: [rx, ry], circle: false })
                }
                BasicShape::Polygon(polygon) => match clip_path_polygon(polygon) {
                    Some(points) => Some(VnClipPath::Polygon {
                        reference,
                        fill_rule: fill_rule(polygon.fill_rule),
                        points: points.into(),
                    }),
                    None => unsupported(diagnostics, "polygon() with a vertex that is no length or percentage"),
                },
            }
        }
    }
}

/// The advisory for a `clip-path` that is no rounded rectangle, `what` being
/// its function (`polygon()`).
fn clip_path_unsupported(diagnostics: &mut Vec<String>, what: &str) {
    warn_once(
        diagnostics,
        &format!(
            "`clip-path: {what}` is not applied (v1) and the element paints unclipped — \
             `inset()`, `circle()`, `ellipse()`, `polygon()`, `path()` and a box \
             (`padding-box`…) are honoured"
        ),
    );
}

fn fill_rule(rule: lightningcss::values::shape::FillRule) -> gpui::VnFillRule {
    match rule {
        lightningcss::values::shape::FillRule::Nonzero => gpui::VnFillRule::NonZero,
        lightningcss::values::shape::FillRule::Evenodd => gpui::VnFillRule::EvenOdd,
    }
}

/// `polygon()`'s vertices as x and y lengths, a percentage of the reference
/// box's width and height. lightningcss keeps a vertex's two values private,
/// so each vertex is printed back to CSS (`10% calc(100% - 4px)`) and read as
/// two `<length-percentage>`s, split at the space outside any parentheses.
fn clip_path_polygon(polygon: &lightningcss::values::shape::Polygon) -> Option<Vec<[DefiniteLength; 2]>> {
    use lightningcss::traits::{Parse, ToCss};
    polygon
        .points
        .iter()
        .map(|point| {
            let text = point.to_css_string(lightningcss::stylesheet::PrinterOptions::default()).ok()?;
            let mut depth = 0i32;
            let split = text.char_indices().find_map(|(i, c)| {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ' ' if depth == 0 => return Some(i),
                    _ => {}
                }
                None
            })?;
            let x = LengthPercentage::parse_string(text[..split].trim()).ok()?;
            let y = LengthPercentage::parse_string(text[split..].trim()).ok()?;
            Some([lp_definite(&x)?, lp_definite(&y)?])
        })
        .collect()
}

/// `clip-path: path([<fill-rule>,]? <string>) <geometry-box>?`, which
/// lightningcss leaves unparsed. `None` when the value is no `path()`;
/// otherwise the clip, or why the declaration is invalid. The string is SVG
/// path data ([`crate::path_data`]) in pixels from the reference box's corner.
fn clip_path_path(
    tokens: &lightningcss::properties::custom::TokenList<'_>,
) -> Option<Result<gpui::VnClipPath, String>> {
    use gpui::{VnClipBox, VnFillRule};
    use lightningcss::properties::custom::{Token, TokenOrValue};
    let mut path = None;
    let mut reference = None;
    for t in &tokens.0 {
        match t {
            TokenOrValue::Function(f) if f.name.0.as_ref().eq_ignore_ascii_case("path") && path.is_none() => {
                path = Some(&f.arguments);
            }
            TokenOrValue::Token(Token::Ident(id)) if path.is_some() && reference.is_none() => {
                reference = Some(match id.as_ref().to_ascii_lowercase().as_str() {
                    "margin-box" => VnClipBox::Margin,
                    "border-box" | "stroke-box" | "view-box" => VnClipBox::Border,
                    "padding-box" => VnClipBox::Padding,
                    "content-box" | "fill-box" => VnClipBox::Content,
                    other => return Some(Err(format!("`{other}` is no <geometry-box>"))),
                });
            }
            TokenOrValue::Token(Token::WhiteSpace(_)) => {}
            // Another function, or a box before the shape: not ours.
            _ if path.is_none() => return None,
            _ => return Some(Err("unexpected text after `path()`".into())),
        }
    }
    let arguments = path?;
    let mut fill_rule = None;
    let mut data = None;
    let mut comma = false;
    for t in &arguments.0 {
        match t {
            TokenOrValue::Token(Token::WhiteSpace(_)) => {}
            TokenOrValue::Token(Token::Ident(id)) if fill_rule.is_none() && data.is_none() => {
                fill_rule = Some(match id.as_ref().to_ascii_lowercase().as_str() {
                    "nonzero" => VnFillRule::NonZero,
                    "evenodd" => VnFillRule::EvenOdd,
                    other => return Some(Err(format!("`{other}` is no <fill-rule>"))),
                });
            }
            TokenOrValue::Token(Token::Comma) if fill_rule.is_some() && !comma => comma = true,
            TokenOrValue::Token(Token::String(d)) if data.is_none() && (fill_rule.is_none() || comma) => {
                data = Some(d.as_ref().to_string());
            }
            _ => return Some(Err("`path()` takes an optional fill rule, a comma and a string".into())),
        }
    }
    let Some(data) = data else {
        return Some(Err("`path()` has no path string".into()));
    };
    Some(crate::path_data::parse(&data).map(|verbs| gpui::VnClipPath::Path {
        reference: reference.unwrap_or_default(),
        fill_rule: fill_rule.unwrap_or_default(),
        verbs: verbs.into(),
    }))
}

/// A `circle()` or `ellipse()` centre (`at <position>`), from the reference
/// box's top left corner. An offset from the far side, `right 10px`, is
/// `calc(100% - 10px)`.
fn shape_center(position: &lightningcss::values::position::Position) -> Option<[DefiniteLength; 2]> {
    use lightningcss::values::position::PositionComponent as P;
    fn component<S: PositionSide + Copy>(c: &P<S>) -> Option<DefiniteLength> {
        match c {
            P::Center => Some(relative(0.5)),
            P::Length(lp) => lp_definite(lp),
            P::Side { side, offset } => {
                let near = side.is_near_edge();
                let Some(offset) = offset else { return Some(relative(if near { 0.0 } else { 1.0 })) };
                let offset = lp_definite(offset)?;
                if near {
                    return Some(offset);
                }
                match offset {
                    DefiniteLength::Fraction(f) => Some(relative(1.0 - f)),
                    DefiniteLength::Absolute(AbsoluteLength::Pixels(p)) => {
                        Some(DefiniteLength::Calc { fraction: 1.0, pixels: -p })
                    }
                    DefiniteLength::Calc { fraction, pixels } => {
                        Some(DefiniteLength::Calc { fraction: 1.0 - fraction, pixels: -pixels })
                    }
                    DefiniteLength::Absolute(AbsoluteLength::Rems(_)) => None,
                }
            }
        }
    }
    Some([component(&position.x)?, component(&position.y)?])
}

/// `true` when a `clip: rect(…)` names a zero-area rectangle — the
/// screen-reader-only idiom, which clips the box away entirely.
///
/// Both spellings count: `rect(0, 0, 0, 0)` and `rect(0 0 0 0)`. `auto` is not
/// a rect and is not empty.
fn clip_rect_is_empty(value: &lightningcss::properties::custom::TokenList<'_>) -> bool {
    use lightningcss::properties::custom::{Token, TokenOrValue};

    fn scan(
        tokens: &[lightningcss::properties::custom::TokenOrValue<'_>],
        saw_rect: &mut bool,
        numbers: &mut Vec<f32>,
    ) {
        for t in tokens {
            match t {
                // lightningcss keeps a function's arguments NESTED rather than
                // flat, so `rect(0, 0, 0, 0)` is one `Function` whose arguments
                // are the four numbers.
                TokenOrValue::Function(f) => {
                    if f.name.0.as_ref() == "rect" {
                        *saw_rect = true;
                    }
                    scan(&f.arguments.0, saw_rect, numbers);
                }
                TokenOrValue::Token(Token::Function(name)) if name.as_ref() == "rect" => {
                    *saw_rect = true
                }
                TokenOrValue::Token(Token::Number { value, .. }) => numbers.push(*value),
                TokenOrValue::Length(l) => numbers.push(length_value_px(l).unwrap_or(f32::NAN)),
                TokenOrValue::Token(Token::Dimension { value, .. }) => numbers.push(*value),
                _ => {}
            }
        }
    }

    let mut saw_rect = false;
    let mut numbers: Vec<f32> = Vec::new();
    scan(&value.0, &mut saw_rect, &mut numbers);
    // `rect(top, right, bottom, left)`: empty when the two horizontal edges
    // coincide or the two vertical ones do.
    saw_rect
        && numbers.len() == 4
        && ((numbers[0] - numbers[2]).abs() < f32::EPSILON
            || (numbers[3] - numbers[1]).abs() < f32::EPSILON)
}

/// A value that is a single CSS-wide keyword. Those are handled by the cascade
/// (or, for `inherit`, by not writing the field at all — see `channel`), so a
/// property-specific parser should skip rather than reject them.
fn is_css_wide_keyword(value: &lightningcss::properties::custom::TokenList<'_>) -> bool {
    single_ident_raw(value).is_some_and(|v| {
        matches!(
            v.to_ascii_lowercase().as_str(),
            "inherit" | "initial" | "unset" | "revert" | "revert-layer"
        )
    })
}

/// A single keyword token equal to `expected`.
fn single_ident_is(value: &lightningcss::properties::custom::TokenList<'_>, expected: &str) -> bool {
    single_ident_raw(value).is_some_and(|v| v.eq_ignore_ascii_case(expected))
}

/// `font-feature-settings` / `font-variant-numeric` / `font-variant-ligatures`
/// / `font-kerning` → gpui's `FontFeatures`, the tag/value list DirectWrite
/// installs as an `IDWriteTypography` over the run.
///
/// The keyword properties are defined in terms of OpenType features, so they
/// map one-to-one rather than approximately.
fn font_features(
    value: &lightningcss::properties::custom::TokenList<'_>,
    property: &str,
) -> Option<gpui::FontFeatures> {
    use lightningcss::properties::custom::{Token, TokenOrValue};
    if is_css_wide_keyword(value) {
        return Some(gpui::FontFeatures::default());
    }
    let mut pairs: Vec<(String, u32)> = Vec::new();
    if property == "font-variant-numeric" {
        for t in value.0.iter() {
            // Tailwind composes this out of five custom properties that each
            // default to a single space, so most of the token list is
            // whitespace with one keyword in it.
            let TokenOrValue::Token(Token::Ident(name)) = t else { continue };
            let tag = match name.as_ref() {
                "normal" => continue,
                "lining-nums" => "lnum",
                "oldstyle-nums" => "onum",
                "proportional-nums" => "pnum",
                "tabular-nums" => "tnum",
                "diagonal-fractions" => "frac",
                "stacked-fractions" => "afrc",
                "ordinal" => "ordn",
                "slashed-zero" => "zero",
                _ => return None,
            };
            pairs.push((tag.to_string(), 1));
        }
        return Some(gpui::FontFeatures(std::sync::Arc::new(pairs)));
    }
    // `font-kerning` is the `kern` feature. `auto` / `normal` write nothing:
    // DirectWrite kerns by default now (zed v1.26), as a browser does.
    if property == "font-kerning" {
        return match single_ident_raw(value)?.to_ascii_lowercase().as_str() {
            "auto" | "normal" => Some(gpui::FontFeatures::default()),
            "none" => Some(gpui::FontFeatures(std::sync::Arc::new(vec![("kern".into(), 0)]))),
            _ => None,
        };
    }
    // `font-variant-ligatures`: CSS Fonts 4 §6.4 defines each keyword as
    // OpenType features, so this is one to one as well.
    if property == "font-variant-ligatures" {
        for t in value.0.iter() {
            let TokenOrValue::Token(Token::Ident(name)) = t else { continue };
            let set: &[(&str, u32)] = match name.as_ref() {
                "normal" => continue,
                "none" => &[("liga", 0), ("clig", 0), ("dlig", 0), ("hlig", 0), ("calt", 0)],
                "common-ligatures" => &[("liga", 1), ("clig", 1)],
                "no-common-ligatures" => &[("liga", 0), ("clig", 0)],
                "discretionary-ligatures" => &[("dlig", 1)],
                "no-discretionary-ligatures" => &[("dlig", 0)],
                "historical-ligatures" => &[("hlig", 1)],
                "no-historical-ligatures" => &[("hlig", 0)],
                "contextual" => &[("calt", 1)],
                "no-contextual" => &[("calt", 0)],
                _ => return None,
            };
            pairs.extend(set.iter().map(|(tag, v)| (tag.to_string(), *v)));
        }
        return Some(gpui::FontFeatures(std::sync::Arc::new(pairs)));
    }

    // `font-feature-settings: "liga" 0, "ss01"` — a quoted tag, optionally
    // followed by a number or `on`/`off`, repeated. A tag with nothing after it
    // is `1`.
    let mut pending: Option<String> = None;
    for t in value.0.iter() {
        match t {
            TokenOrValue::Token(Token::WhiteSpace(_)) | TokenOrValue::Token(Token::Comma) => {}
            TokenOrValue::Token(Token::String(tag)) => {
                if let Some(previous) = pending.take() {
                    pairs.push((previous, 1));
                }
                if tag.len() != 4 {
                    return None;
                }
                pending = Some(tag.to_string());
            }
            TokenOrValue::Token(Token::Number { value, .. }) => {
                pairs.push((pending.take()?, *value as u32));
            }
            TokenOrValue::Token(Token::Ident(word)) => match word.as_ref() {
                "on" => pairs.push((pending.take()?, 1)),
                "off" => pairs.push((pending.take()?, 0)),
                "normal" => {}
                _ => return None,
            },
            _ => return None,
        }
    }
    if let Some(previous) = pending {
        pairs.push((previous, 1));
    }
    Some(gpui::FontFeatures(std::sync::Arc::new(pairs)))
}

fn single_length_px(value: &lightningcss::properties::custom::TokenList<'_>) -> Option<f32> {
    use lightningcss::properties::custom::{Token, TokenOrValue};
    let mut found = None;
    for t in value.0.iter() {
        match t {
            TokenOrValue::Token(Token::WhiteSpace(_)) => {}
            TokenOrValue::Length(l) => {
                if found.is_some() {
                    return None;
                }
                found = length_value_px(l);
            }
            TokenOrValue::Token(Token::Dimension { value, unit, .. }) => {
                if found.is_some() {
                    return None;
                }
                found = match unit.as_ref() {
                    "px" => Some(*value),
                    other => match font_unit_px(other, *value).or_else(|| viewport_unit_px(other, *value)) {
                        Some(px) => Some(px),
                        None => return None,
                    },
                };
            }
            TokenOrValue::Token(Token::Number { value, .. }) if *value == 0.0 => {
                if found.is_some() {
                    return None;
                }
                found = Some(0.0);
            }
            _ => return None,
        }
    }
    found
}

/// Does this token list reference `var()` or `env()` anywhere?
fn has_substitution(value: &lightningcss::properties::custom::TokenList<'_>) -> bool {
    use lightningcss::properties::custom::{TokenOrValue, UnresolvedColor};
    value.0.iter().any(|token| match token {
        TokenOrValue::Var(_) | TokenOrValue::Env(_) => true,
        TokenOrValue::Function(function) => has_substitution(&function.arguments),
        TokenOrValue::UnresolvedColor(UnresolvedColor::RGB { alpha, .. }
            | UnresolvedColor::HSL { alpha, .. }) => has_substitution(alpha),
        TokenOrValue::UnresolvedColor(UnresolvedColor::LightDark { light, dark }) =>
            has_substitution(light) || has_substitution(dark),
        _ => false,
    })
}

/// What a `content` declaration resolved to.
enum ContentValue {
    /// A literal string (possibly several, concatenated — CSS allows that).
    Text(Rc<str>),
    /// `none` / `normal` — no generated box.
    None,
    /// Something real that has no model here; the text is for the advisory.
    Unsupported(String),
}

/// `content: "…"` — the only form that generates a box here.
fn content_string(value: &lightningcss::properties::custom::TokenList<'_>) -> ContentValue {
    use lightningcss::properties::custom::{Token, TokenOrValue};
    let mut out = String::new();
    let mut saw_string = false;
    for t in value.0.iter() {
        match t {
            TokenOrValue::Token(Token::WhiteSpace(_)) => {}
            TokenOrValue::Token(Token::String(sv)) => {
                out.push_str(sv.as_ref());
                saw_string = true;
            }
            TokenOrValue::Token(Token::Ident(id)) if matches!(id.as_ref(), "none" | "normal") => {
                return ContentValue::None;
            }
            TokenOrValue::Token(Token::Function(f)) => {
                return ContentValue::Unsupported(format!("{}()", f.as_ref()));
            }
            TokenOrValue::Url(_) => return ContentValue::Unsupported("url()".into()),
            TokenOrValue::Token(Token::Ident(id)) => {
                return ContentValue::Unsupported(id.as_ref().to_string());
            }
            TokenOrValue::Var(_) => return ContentValue::Unsupported("var()".into()),
            _ => return ContentValue::Unsupported("that value".into()),
        }
    }
    if saw_string {
        ContentValue::Text(Rc::from(out.as_str()))
    } else {
        ContentValue::None
    }
}

/// The single keyword a token list carries, with no advisory when there is
/// none — the quiet half of [`single_ident`].
fn single_ident_raw<'a>(value: &'a lightningcss::properties::custom::TokenList<'_>) -> Option<&'a str> {
    use lightningcss::properties::custom::{Token, TokenOrValue};
    let mut found = None;
    for t in value.0.iter() {
        match t {
            TokenOrValue::Token(Token::WhiteSpace(_)) => {}
            TokenOrValue::Token(Token::Ident(id)) => {
                if found.is_some() {
                    return None;
                }
                found = Some(id.as_ref());
            }
            _ => return None,
        }
    }
    found
}

/// The single bare `<number>` a token list carries.
fn single_number(value: &lightningcss::properties::custom::TokenList<'_>) -> Option<f32> {
    use lightningcss::properties::custom::{Token, TokenOrValue};
    let mut found = None;
    for t in value.0.iter() {
        match t {
            TokenOrValue::Token(Token::WhiteSpace(_)) => {}
            TokenOrValue::Token(Token::Number { value, .. }) => {
                if found.is_some() {
                    return None;
                }
                found = Some(*value);
            }
            _ => return None,
        }
    }
    found
}

/// `window-drag: move | none` and its two legacy spellings
/// (`app-region: drag | no-drag`, `-webkit-app-region: …`).
///
/// All three property names are accepted with no advisory (D-13); both value
/// vocabularies are accepted on all three names, because Chromium does the same
/// and an author who mixes them is not making a mistake worth a warning. An
/// unrecognised VALUE does advise (A-11).
fn window_drag(
    property: &str,
    value: &lightningcss::properties::custom::TokenList,
    diagnostics: &mut Vec<String>,
) -> Option<WindowDrag> {
    match single_ident(value, property, diagnostics) {
        Some("move") | Some("drag") => Some(WindowDrag::Move),
        Some("none") | Some("no-drag") => Some(WindowDrag::None),
        other => {
            once(
                diagnostics,
                format!(
                    "unsupported value for `{property}`: {} — expected `move` | `none` \
                     (or the legacy `drag` | `no-drag`)",
                    other.unwrap_or("<not a keyword>")
                ),
            );
            None
        }
    }
}

/// `scrollbar-width: auto | thin | none`.
fn scrollbar_width(
    value: &lightningcss::properties::custom::TokenList,
    diagnostics: &mut Vec<String>,
) -> Option<ScrollbarWidth> {
    match single_ident(value, "scrollbar-width", diagnostics) {
        Some("auto") => Some(ScrollbarWidth::Auto),
        Some("thin") => Some(ScrollbarWidth::Thin),
        Some("none") => Some(ScrollbarWidth::None),
        other => {
            once(
                diagnostics,
                format!(
                    "unsupported value for `scrollbar-width`: {} — expected `auto` | `thin`                      | `none`",
                    other.unwrap_or("<not a keyword>")
                ),
            );
            None
        }
    }
}

/// A CSS colour written as a bare identifier (`red`, `transparent`, a named
/// colour, or a CSS-wide keyword that is not one). Unknown properties arrive as
/// a raw token list, so the typed `CssColor` lightningcss produces for `#f00`
/// or `rgb(…)` is simply not there for `red` — re-parse it as one.
fn named_color(ident: &str) -> Option<Hsla> {
    match Property::parse_string(PropertyId::from("color"), ident, ParserOptions::default()) {
        Ok(Property::Color(c)) => color(&c),
        _ => None,
    }
}

/// `scrollbar-color: <thumb> <track>`. Both colours are required by the
/// grammar; `auto` (the initial value) means "the platform's", which is what
/// declaring nothing already gets.
fn scrollbar_color(
    value: &lightningcss::properties::custom::TokenList,
    diagnostics: &mut Vec<String>,
) -> Option<(Hsla, Hsla)> {
    use lightningcss::properties::custom::{CustomProperty, CustomPropertyName, Token, TokenOrValue};
    let tokens: Vec<_> = value.0.iter().filter(|token| !token.is_whitespace()).collect();
    if matches!(tokens.as_slice(), [TokenOrValue::Token(Token::Ident(id))] if id.as_ref().eq_ignore_ascii_case("auto")) {
        return None;
    }
    // Unknown-property tokens can retain hsl()/rgb() functions after var()
    // substitution. Parse the pair through the typed colour grammar rather
    // than mistaking such a function for another list component.
    let property = Property::Custom(CustomProperty {
        name: CustomPropertyName::Unknown(lightningcss::values::ident::Ident("scrollbar-color".into())),
        value: value.clone(),
    });
    if tokens.len() == 2 {
        if let Some(raw) = value_text(&property) {
            if let Ok(Property::BorderColor(pair)) = Property::parse_string(PropertyId::from("border-color"), &raw, ParserOptions::default()) {
                if let (Some(thumb), Some(track)) = (color(&pair.top), color(&pair.right)) {
                    return Some((thumb, track));
                }
            }
        }
    }
    once(diagnostics, "`scrollbar-color` requires auto or two valid colours (thumb then track)".to_string());
    None
}

/// `overscroll-behavior: auto | contain | none`, one axis at a time.
fn overscroll(
    property: &str,
    value: &lightningcss::properties::custom::TokenList,
    diagnostics: &mut Vec<String>,
) -> Option<OverscrollBehavior> {
    match single_ident(value, property, diagnostics) {
        Some("auto") => Some(OverscrollBehavior::Auto),
        Some("contain") => Some(OverscrollBehavior::Contain),
        Some("none") => Some(OverscrollBehavior::None),
        other => {
            once(
                diagnostics,
                format!(
                    "unsupported value for `{property}`: {} — expected `auto` | `contain`                      | `none`",
                    other.unwrap_or("<not a keyword>")
                ),
            );
            None
        }
    }
}

/// `--window-control: minimize | maximize | close | none`.
fn window_control(
    value: &lightningcss::properties::custom::TokenList,
    diagnostics: &mut Vec<String>,
) -> Option<WindowControl> {
    match single_ident(value, WINDOW_CONTROL, diagnostics) {
        Some("minimize") => Some(WindowControl::Minimize),
        Some("maximize") => Some(WindowControl::Maximize),
        Some("close") => Some(WindowControl::Close),
        Some("none") => Some(WindowControl::None),
        other => {
            once(
                diagnostics,
                format!(
                    "unsupported value for `{WINDOW_CONTROL}`: {} — expected \
                     `minimize` | `maximize` | `close` | `none`",
                    other.unwrap_or("<not a keyword>")
                ),
            );
            None
        }
    }
}

/// Maps one declaration — and says so when a `calc()` in it was dropped.
///
/// The sizing properties (width/height and their min/max, inset, margin,
/// padding, gap, flex-basis, line-height) take any LINEAR calc(), percentage
/// or not (`DefiniteLength::Calc`). Everything else resolves its percentages
/// before layout here, so a calc() mixing one in has nothing to resolve
/// against; it used to vanish without a word (reka's slider thumbs sat at 0
/// with no advisory, m26 client sweep #12).
fn apply(p: &Property, r: &mut StyleRefinement, x: &mut Extras, diagnostics: &mut Vec<String>) {
    CALC_DROPPED.with(|c| c.set(false));
    apply_property(p, r, x, diagnostics);
    if CALC_DROPPED.with(|c| c.replace(false)) {
        once(
            diagnostics,
            format!(
                "`{}`: this calc() is dropped (v1) — a calc() that mixes a percentage with a \
                 length is resolved at layout only by the sizing properties (width/height and \
                 their min/max, inset, margin, padding, gap, flex-basis) and at paint by \
                 border-radius, and only in its linear form (sums and scalings; min()/max()/clamp() over a percentage are \
                 not). Here the declaration is ignored, as if it were invalid. Write the \
                 length or the percentage alone, or move the offset into a sizing property",
                p.property_id().name()
            ),
        );
    }
}

fn apply_property(
    p: &Property,
    r: &mut StyleRefinement,
    x: &mut Extras,
    diagnostics: &mut Vec<String>,
) {
    use Property::*;
    match p {
        Display(d) => {
            use lightningcss::properties::display::{Display as D, DisplayOutside};
            // Reset on every `display` declaration, so a later `display: flex`
            // un-does an earlier `display: inline-flex` rather than leaving the
            // flag behind (`refine` is later-wins per FIELD).
            x.inline_level = Some(matches!(
                d,
                D::Pair(p) if matches!(p.outside, DisplayOutside::Inline)
            ));
            r.display = Some(match d {
                D::Keyword(DisplayKeyword::None) => gpui::Display::None,
                D::Pair(pair) => {
                    // `inline-flex` / `inline-grid` / `inline-block` DO map:
                    // taffy models the INNER display type, and that is what
                    // decides how the children lay out. What it has no notion
                    // of is the OUTER one — every box here is block-level, so
                    // an inline-flex box still takes a line of its own instead
                    // of sitting in a line box beside its siblings, and it
                    // fills the available width instead of shrinking to fit.
                    //
                    // …and MOST inline-level boxes do not diverge at all, which
                    // is why there is no advisory here any more. CSS blockifies
                    // an inline-level box that is a flex or grid item, or that
                    // is out of flow (CSS Display 3 §2.7), so `inline-flex` on
                    // a flex child computes to `flex` in a browser too — and
                    // shadcn's Badge and Button, which sit in flex rows, are
                    // exactly that. Where the parent is a block container,
                    // `vn_tree::render::apply_inline_row` recovers the rest by
                    // turning a run of inline content into a wrapping flex row.
                    // Only the renderer knows which of those applies, so the
                    // advisory lives there (`advise_inline_level`) — the same
                    // split `transform`'s negative-z advisory has.
                    match pair.inside {
                        DisplayInside::Flex(_) => gpui::Display::Flex,
                        DisplayInside::Grid => gpui::Display::Grid,
                        _ => gpui::Display::Block,
                    }
                }
                _ => gpui::Display::Block,
            });
        }
        FlexDirection(d, _) => {
            r.flex_direction = Some(match d {
                CssFlexDirection::Row => gpui::FlexDirection::Row,
                CssFlexDirection::RowReverse => gpui::FlexDirection::RowReverse,
                CssFlexDirection::Column => gpui::FlexDirection::Column,
                CssFlexDirection::ColumnReverse => gpui::FlexDirection::ColumnReverse,
            });
        }
        FlexGrow(v, _) => r.flex_grow = Some(*v),
        FlexShrink(v, _) => r.flex_shrink = Some(*v),
        FlexBasis(v, _) => r.flex_basis = lpa_length(v),
        // `flex: 1 1 0%` (Tailwind `flex-1`), `flex: 0 1 auto` (`flex-initial`), …
        Flex(v, _) => {
            r.flex_grow = Some(v.grow);
            r.flex_shrink = Some(v.shrink);
            r.flex_basis = lpa_length(&v.basis);
        }
        FlexWrap(v, _) => {
            use lightningcss::properties::flex::FlexWrap as W;
            r.flex_wrap = Some(match v {
                W::NoWrap => gpui::FlexWrap::NoWrap,
                W::Wrap => gpui::FlexWrap::Wrap,
                W::WrapReverse => gpui::FlexWrap::WrapReverse,
            });
        }
        AlignItems(v, _) => {
            use gpui::AlignItems as G;
            let mapped = match v {
                lightningcss::properties::align::AlignItems::Stretch => Some(G::Stretch),
                lightningcss::properties::align::AlignItems::SelfPosition { value, .. } => {
                    Some(self_position(value))
                }
                lightningcss::properties::align::AlignItems::BaselinePosition(_) => Some(G::Baseline),
                _ => None,
            };
            match mapped {
                Some(m) => r.align_items = Some(m),
                None => unsupported(diagnostics, "align-items"),
            }
        }
        AlignSelf(v, _) => {
            use lightningcss::properties::align::AlignSelf as A;
            let mapped = match v {
                // `self-auto`: inherit the parent's align-items. gpui spells
                // that as "unset", and a refinement cannot express "clear" —
                // so leave the field alone rather than pin it to Stretch.
                A::Auto => None,
                A::Stretch => Some(gpui::AlignItems::Stretch),
                A::BaselinePosition(_) => Some(gpui::AlignItems::Baseline),
                A::SelfPosition { value, .. } => Some(self_position(value)),
                A::Normal => None,
            };
            match mapped {
                Some(m) => r.align_self = Some(m),
                // `auto`/`normal` mean "defer to the parent", which is already
                // what an unset refinement does — but say so, because a rule
                // that MEANT to undo an earlier `self-center` will not.
                None => unsupported(
                    diagnostics,
                    "align-self: auto/normal — cannot undo an earlier align-self",
                ),
            }
        }
        AlignContent(v, _) => {
            use gpui::AlignContent as G;
            use lightningcss::properties::align::AlignContent as A;
            let mapped = match v {
                A::ContentDistribution(d) => {
                    use lightningcss::properties::align::ContentDistribution as CD;
                    Some(match d {
                        CD::SpaceBetween => G::SpaceBetween,
                        CD::SpaceAround => G::SpaceAround,
                        CD::SpaceEvenly => G::SpaceEvenly,
                        CD::Stretch => G::Stretch,
                    })
                }
                A::ContentPosition { value, .. } => Some(match value {
                    ContentPosition::Center => G::Center,
                    ContentPosition::Start | ContentPosition::FlexStart => G::FlexStart,
                    ContentPosition::End | ContentPosition::FlexEnd => G::FlexEnd,
                }),
                _ => None,
            };
            match mapped {
                Some(m) => r.align_content = Some(m),
                None => unsupported(diagnostics, "align-content"),
            }
        }
        JustifyContent(v, _) => {
            use gpui::JustifyContent as G;
            let mapped = match v {
                lightningcss::properties::align::JustifyContent::ContentDistribution(d) => {
                    use lightningcss::properties::align::ContentDistribution as CD;
                    Some(match d {
                        CD::SpaceBetween => G::SpaceBetween,
                        CD::SpaceAround => G::SpaceAround,
                        CD::SpaceEvenly => G::SpaceEvenly,
                        CD::Stretch => G::Stretch,
                    })
                }
                lightningcss::properties::align::JustifyContent::ContentPosition { value, .. } => {
                    match value {
                        ContentPosition::Center => Some(G::Center),
                        ContentPosition::Start | ContentPosition::FlexStart => Some(G::FlexStart),
                        ContentPosition::End | ContentPosition::FlexEnd => Some(G::FlexEnd),
                    }
                }
                _ => None,
            };
            match mapped {
                Some(m) => r.justify_content = Some(m),
                None => unsupported(diagnostics, "justify-content"),
            }
        }
        Gap(gap) => {
            if let Some(v) = gap_definite(&gap.row) {
                r.gap.height = Some(v);
            }
            if let Some(v) = gap_definite(&gap.column) {
                r.gap.width = Some(v);
            }
        }
        // Tailwind `gap-y-*` / `gap-x-*` emit the longhands, never the shorthand.
        RowGap(v) => r.gap.height = gap_definite(v),
        ColumnGap(v) => r.gap.width = gap_definite(v),
        Width(v) => r.size.width = size_length(v),
        Height(v) => r.size.height = size_length(v),
        MinWidth(v) => r.min_size.width = size_length(v),
        MinHeight(v) => r.min_size.height = size_length(v),
        MaxWidth(v) => r.max_size.width = max_size_length(v),
        MaxHeight(v) => r.max_size.height = max_size_length(v),
        Padding(v) => {
            r.padding.top = lp_definite_of_lpa(&v.top);
            r.padding.right = lp_definite_of_lpa(&v.right);
            r.padding.bottom = lp_definite_of_lpa(&v.bottom);
            r.padding.left = lp_definite_of_lpa(&v.left);
        }
        PaddingTop(v) => r.padding.top = lp_definite_of_lpa(v),
        PaddingRight(v) => r.padding.right = lp_definite_of_lpa(v),
        PaddingBottom(v) => r.padding.bottom = lp_definite_of_lpa(v),
        PaddingLeft(v) => r.padding.left = lp_definite_of_lpa(v),
        Margin(v) => {
            r.margin.top = lpa_length(&v.top);
            r.margin.right = lpa_length(&v.right);
            r.margin.bottom = lpa_length(&v.bottom);
            r.margin.left = lpa_length(&v.left);
        }
        MarginTop(v) => r.margin.top = lpa_length(v),
        MarginRight(v) => r.margin.right = lpa_length(v),
        MarginBottom(v) => r.margin.bottom = lpa_length(v),
        MarginLeft(v) => r.margin.left = lpa_length(v),
        // The logical spellings resolve against `horizontal-tb` + `ltr`, the
        // only writing mode and direction we lay out in: inline-start is the
        // left side, block-start the top. They write the same slots as the
        // physical ones, so whichever is declared later wins, as in CSS.
        PaddingInline(v) => {
            r.padding.left = lp_definite_of_lpa(&v.inline_start);
            r.padding.right = lp_definite_of_lpa(&v.inline_end);
        }
        PaddingBlock(v) => {
            r.padding.top = lp_definite_of_lpa(&v.block_start);
            r.padding.bottom = lp_definite_of_lpa(&v.block_end);
        }
        PaddingInlineStart(v) => r.padding.left = lp_definite_of_lpa(v),
        PaddingInlineEnd(v) => r.padding.right = lp_definite_of_lpa(v),
        PaddingBlockStart(v) => r.padding.top = lp_definite_of_lpa(v),
        PaddingBlockEnd(v) => r.padding.bottom = lp_definite_of_lpa(v),
        MarginInline(v) => {
            r.margin.left = lpa_length(&v.inline_start);
            r.margin.right = lpa_length(&v.inline_end);
        }
        MarginBlock(v) => {
            r.margin.top = lpa_length(&v.block_start);
            r.margin.bottom = lpa_length(&v.block_end);
        }
        MarginInlineStart(v) => r.margin.left = lpa_length(v),
        MarginInlineEnd(v) => r.margin.right = lpa_length(v),
        MarginBlockStart(v) => r.margin.top = lpa_length(v),
        MarginBlockEnd(v) => r.margin.bottom = lpa_length(v),
        Position(v) => {
            use lightningcss::properties::position::Position as P;
            x.fixed = Some(matches!(v, P::Fixed));
            x.sticky = Some(matches!(v, P::Sticky(_)));
            x.positioned = Some(!matches!(v, P::Static));
            r.position = Some(match v {
                P::Absolute => gpui::Position::Absolute,
                // `fixed` is an out-of-flow box against the VIEWPORT. taffy has
                // exactly one out-of-flow mode and it resolves against the
                // element's parent, so that is what `fixed` becomes — plus a
                // deferred paint layer (see `Extras::deferred_priority`) so the
                // box lands above its siblings the way a fixed one does.
                //
                // The two agree exactly when the fixed element's parent IS the
                // window — which is the case that matters, because a portalled
                // overlay (`<Teleport to="body">`) has the app root as its
                // parent. They diverge when a `fixed` element is left inside a
                // smaller ancestor: it is then positioned against that ancestor
                // rather than the viewport.
                // The advisory for both of these lives in the RENDERER
                // (`vn_tree::render::advise_out_of_flow`), because whether
                // either diverges at all is a question about the element's
                // ANCESTORS, which the mapper cannot see: a `fixed` element
                // portalled to the app root IS laid out against the viewport,
                // and a `sticky` element with no scrolling ancestor behaves
                // exactly like `relative` in a browser too.
                P::Fixed => gpui::Position::Absolute,
                P::Static | P::Relative => gpui::Position::Relative,
                _ => gpui::Position::Relative,
            });
        }
        // gpui has no per-element z. The renderer turns the value into a place
        // in the paint order, its stacking context's path included
        // (`vn_tree::render::Stacking`), so the value is recorded as written:
        // `auto` is NOT `0`, because only an integer makes a context.
        ZIndex(v) => {
            use lightningcss::properties::position::ZIndex as Z;
            match v {
                Z::Integer(n) => x.z_index = Some(Some(*n)),
                Z::Auto => x.z_index = Some(None),
            }
        }
        // `transform` is a PAINT-level property now (gpui's
        // `Style::vn_transform`), so it goes into the refinement rather than
        // into `Extras` — which is what gives it `:hover`, transitions and
        // `@keyframes` through the ordinary machinery.
        Transform(list, _) => apply_transform(list, r, diagnostics),
        // `transform-origin` — the point rotate()/scale() turn about. Written
        // onto the same spec `transform` writes, seeded with CSS's initial
        // `50% 50%` so declaring the origin ALONE (which changes nothing, as in
        // a browser) does not accidentally spell `transform: none`.
        TransformOrigin(pos, _) => {
            use lightningcss::values::position::{
                HorizontalPositionKeyword as H, PositionComponent as P, VerticalPositionKeyword as V,
            };
            fn axis<S: Copy>(
                c: &P<S>,
                keyword: impl Fn(&S) -> Option<f32>,
            ) -> Option<TransformLength> {
                match c {
                    P::Center => Some(TransformLength::Fraction(0.5)),
                    P::Length(DimensionPercentage::Dimension(l)) => {
                        length_value_px(l).map(TransformLength::Pixels)
                    }
                    P::Length(DimensionPercentage::Percentage(p)) => {
                        Some(TransformLength::Fraction(p.0))
                    }
                    P::Length(DimensionPercentage::Calc(_)) => {
                        calc_dropped();
                        None
                    }
                    P::Side { side, offset: None } => keyword(side).map(TransformLength::Fraction),
                    // `right 10px` — a keyword plus an offset, which needs the
                    // box to resolve into one number.
                    P::Side { .. } => None,
                }
            }
            let x = axis(&pos.x, |s: &H| {
                Some(match s {
                    H::Left => 0.0,
                    H::Right => 1.0,
                })
            });
            let y = axis(&pos.y, |s: &V| {
                Some(match s {
                    V::Top => 0.0,
                    V::Bottom => 1.0,
                })
            });
            match (x, y) {
                (Some(x), Some(y)) => {
                    let spec = r.vn_transform.get_or_insert(TransformSpec::IDENTITY);
                    spec.origin = (x, y);
                }
                _ => unsupported(
                    diagnostics,
                    "transform-origin with a calc() or keyword-plus-offset component — lengths, \
                     percentages and the bare `left/right/top/bottom/center` keywords are modelled",
                ),
            }
        }
        // ---- text -----------------------------------------------------------
        // The legacy `-webkit-box` machinery a `line-clamp` utility drags
        // along. `-webkit-line-clamp` is honoured on its own (below, in the
        // unknown-property bucket where lightningcss leaves it), so these are
        // genuinely inert rather than a missing feature.
        BoxOrient(..) | BoxAlign(..) | BoxPack(..) => {}
        // `text-decoration-skip-ink` decides whether an underline BREAKS around
        // descenders. gpui draws one unbroken rule, and the property has no
        // other effect — so `none` (draw it straight through) is exactly what
        // it already does, and only `auto` diverges.
        TextDecorationSkipInk(v, _) => {
            use lightningcss::properties::text::TextDecorationSkipInk as Skip;
            if !matches!(v, Skip::None) {
                warn_once(
                    diagnostics,
                    "`text-decoration-skip-ink: auto` is not applied (v1) — gpui draws one                      unbroken underline and does not lift it around descenders. `none` (draw it                      straight through) is what it already does",
                );
            }
        }
        // A decoration's colour, style and thickness live ON the decoration in
        // gpui, not beside it — so these only land when the same cascade has
        // already produced an underline or a strikethrough (which
        // `text-decoration-line`, or the `text-decoration` shorthand, does).
        TextDecorationColor(c, _) => match color(c) {
            Some(v) => {
                let mut hit = false;
                if let Some(u) = r.text.underline.as_mut() {
                    u.color = Some(v);
                    hit = true;
                }
                if let Some(s) = r.text.strikethrough.as_mut() {
                    s.color = Some(v);
                    hit = true;
                }
                // `hit` false means no decoration LINE was declared on this
                // element, so there is nothing to colour — and a browser paints
                // nothing here either, because the property is only ever read
                // through a line. No divergence, nothing to report.
                let _ = hit;
            }
            None => unresolved_color(diagnostics, "text-decoration-color"),
        },
        TextDecorationStyle(s, _) => {
            use lightningcss::properties::text::TextDecorationStyle as S;
            if let Some(u) = r.text.underline.as_mut() {
                u.wavy = matches!(s, S::Wavy);
            }
            if !matches!(s, S::Solid | S::Wavy) {
                unsupported(
                    diagnostics,
                    "text-decoration-style — gpui draws a solid or a wavy line, so `double`, \
                     `dotted` and `dashed` fall back to solid",
                );
            }
        }
        TextDecorationThickness(t) => {
            use lightningcss::properties::text::TextDecorationThickness as T;
            if let T::LengthPercentage(DimensionPercentage::Dimension(l)) = t {
                if let (Some(w), Some(u)) = (length_value_px(l), r.text.underline.as_mut()) {
                    u.thickness = px(w);
                }
            }
        }
        // `text-decoration` shorthand: the line half is the part that renders,
        // and it must run BEFORE colour/style so they have something to land on.
        TextDecoration(d, _) => {
            use lightningcss::properties::text::TextDecorationLine as TDL;
            if d.line.contains(TDL::Underline) {
                r.text.underline = Some(gpui::UnderlineStyle::default());
            }
            if d.line.contains(TDL::LineThrough) {
                r.text.strikethrough = Some(gpui::StrikethroughStyle::default());
            }
            if let Some(c) = color(&d.color) {
                if let Some(u) = r.text.underline.as_mut() {
                    u.color = Some(c);
                }
                if let Some(s) = r.text.strikethrough.as_mut() {
                    s.color = Some(c);
                }
            }
            if d.line.is_empty() {
                unsupported(
                    diagnostics,
                    "text-decoration: none — cannot undo an earlier underline",
                );
            }
        }
        // SVG paint. Read by the `<svg>` serializer for the element and every
        // element inside it (see `SvgPaint`); like `object-fit` on a
        // non-replaced element it is parsed and folded for every node,
        // because that is what a browser computes.
        Fill(p) => x.fill = Some(svg_paint(p, "fill", diagnostics)),
        Stroke(p) => x.stroke = Some(svg_paint(p, "stroke", diagnostics)),
        // The rest of SVG's presentation properties travel as text to the
        // `<svg>` serializer (`Extras::svg_presentation`): resvg parses them
        // itself, so there is nothing to model here.
        FillOpacity(_) | FillRule(_) | StrokeOpacity(_) | StrokeWidth(_) | StrokeLinecap(_)
        | StrokeLinejoin(_) | StrokeMiterlimit(_) | StrokeDasharray(_) | StrokeDashoffset(_)
        | ShapeRendering(_) => {
            if let (Some(name), Some(value)) = (presentation_name(p), value_text(p)) {
                let value = presentation_value(name, value);
                let list = x.svg_presentation.get_or_insert_with(Default::default);
                set_presentation(Rc::make_mut(list), name, value.into());
            }
        }
        // taffy is border-box and only border-box, so `border-box` is not a
        // divergence — it is the model. `content-box` is one, and says so.
        BoxSizing(v, _) => {
            use lightningcss::properties::size::BoxSizing as BS;
            if matches!(v, BS::ContentBox) {
                warn_once(
                    diagnostics,
                    "`box-sizing: content-box` is not supported (v1) — taffy sizes every box as \
                     `border-box`, so padding and border stay INSIDE the declared width/height",
                );
            }
        }
        Top(v) => r.inset.top = lpa_length(v),
        Right(v) => r.inset.right = lpa_length(v),
        Bottom(v) => r.inset.bottom = lpa_length(v),
        Left(v) => r.inset.left = lpa_length(v),
        Inset(v) => {
            r.inset.top = lpa_length(&v.top);
            r.inset.right = lpa_length(&v.right);
            r.inset.bottom = lpa_length(&v.bottom);
            r.inset.left = lpa_length(&v.left);
        }
        // Logical insets: `horizontal-tb` + `ltr`, as the margins above.
        InsetInline(v) => {
            r.inset.left = lpa_length(&v.inline_start);
            r.inset.right = lpa_length(&v.inline_end);
        }
        InsetBlock(v) => {
            r.inset.top = lpa_length(&v.block_start);
            r.inset.bottom = lpa_length(&v.block_end);
        }
        InsetInlineStart(v) => r.inset.left = lpa_length(v),
        InsetInlineEnd(v) => r.inset.right = lpa_length(v),
        InsetBlockStart(v) => r.inset.top = lpa_length(v),
        InsetBlockEnd(v) => r.inset.bottom = lpa_length(v),
        Overflow(v) => {
            r.overflow.x = Some(overflow(v.x));
            r.overflow.y = Some(overflow(v.y));
            x.overflow_x = Some(v.x);
            x.overflow_y = Some(v.y);
        }
        // The longhands are what Tailwind's `overflow-x-*` / `overflow-y-*`
        // emit; the shorthand only covers `overflow-*`. One axis each: the
        // other axis's computed value depends on this one, which only the
        // whole cascade knows ([`compute_overflow`]).
        OverflowX(v) => {
            r.overflow.x = Some(overflow(*v));
            x.overflow_x = Some(*v);
        }
        OverflowY(v) => {
            r.overflow.y = Some(overflow(*v));
            x.overflow_y = Some(*v);
        }
        Visibility(v) => {
            use lightningcss::properties::display::Visibility as V;
            x.visibility_hidden = Some(!matches!(v, V::Visible));
            r.visibility = Some(match v {
                V::Visible => gpui::Visibility::Visible,
                V::Hidden => gpui::Visibility::Hidden,
                // CSS defines `collapse` AS `hidden` for everything that is
                // not a table row/column or a flex item, and there are no
                // tables here — so on every box vue-native lays out, the two
                // values genuinely mean the same thing.
                V::Collapse => gpui::Visibility::Hidden,
            });
        }
        AspectRatio(v) => match &v.ratio {
            Some(ratio) if !v.auto && ratio.1 != 0.0 => r.aspect_ratio = Some(ratio.0 / ratio.1),
            // `aspect-auto` / `auto <ratio>`: a refinement cannot say "clear",
            // so an earlier ratio survives.
            _ => unsupported(diagnostics, "aspect-ratio: auto — cannot undo an earlier ratio"),
        },
        Opacity(v) => r.opacity = Some(v.0),
        // `color: currentColor` IS `color: inherit` — the property resolves
        // against itself — and inheriting is what not writing the field does.
        Color(CssColor::CurrentColor) => {}
        Color(c) => match color(c) {
            Some(c) => r.text.color = Some(c),
            None => unresolved_color(diagnostics, "color"),
        },
        BackgroundColor(CssColor::CurrentColor) => {
            x.current_color.0 |= CurrentColorTargets::BACKGROUND;
        }
        BackgroundColor(c) => match color(c) {
            Some(c) => {
                r.background = Some(gpui::Fill::Color(c.into()));
                x.background_key_color = Some(c);
            }
            None => unresolved_color(diagnostics, "background-color"),
        },
        Background(layers) => {
            // Per CSS the colour lives on the LAST layer and paints UNDER every
            // image layer, which is exactly the relationship between gpui's
            // `background` and its `background_layers`.
            if let Some(last) = layers.last() {
                if let Some(c) = color(&last.color) {
                    r.background = Some(gpui::Fill::Color(c.into()));
                    x.background_key_color = Some(c);
                }
            }
            collect_background_images(
                layers.iter().map(|l| &l.image),
                x,
                diagnostics,
            );
            x.background_size = Some(Rc::new(
                layers.iter().map(|l| background_size(&l.size)).collect(),
            ));
            x.background_position = Some(Rc::new(
                layers
                    .iter()
                    .map(|l| background_position(&l.position, diagnostics))
                    .collect(),
            ));
            x.background_repeat = Some(Rc::new(
                layers.iter().map(|l| background_repeat(&l.repeat)).collect(),
            ));
        }
        BackgroundImage(images) => {
            collect_background_images(images.iter(), x, diagnostics);
        }
        BackgroundSize(sizes) => {
            x.background_size = Some(Rc::new(sizes.iter().map(background_size).collect()));
        }
        BackgroundPosition(positions) => {
            x.background_position = Some(Rc::new(
                positions
                    .iter()
                    .map(|p| background_position(p, diagnostics))
                    .collect(),
            ));
        }
        BackgroundPositionX(list) => {
            let previous = x.background_position.clone();
            x.background_position = Some(Rc::new(
                list.iter()
                    .enumerate()
                    .map(|(i, h)| {
                        let y = previous
                            .as_ref()
                            .filter(|p| !p.is_empty())
                            .map(|p| p[i % p.len()].1)
                            .unwrap_or(TileLength::Fraction(0.0));
                        (position_component(h, true, diagnostics), y)
                    })
                    .collect(),
            ));
        }
        BackgroundPositionY(list) => {
            let previous = x.background_position.clone();
            x.background_position = Some(Rc::new(
                list.iter()
                    .enumerate()
                    .map(|(i, v)| {
                        let h = previous
                            .as_ref()
                            .filter(|p| !p.is_empty())
                            .map(|p| p[i % p.len()].0)
                            .unwrap_or(TileLength::Fraction(0.0));
                        (h, position_component(v, false, diagnostics))
                    })
                    .collect(),
            ));
        }
        BackgroundRepeat(list) => {
            x.background_repeat = Some(Rc::new(list.iter().map(background_repeat).collect()));
        }
        // `background-attachment` and `background-origin` still have no effect,
        // and for a reason that is not about the gradient work: there is no
        // scrolling viewport a `fixed` background could be pinned to, and the
        // background always fills the padding box.
        BackgroundAttachment(_) | BackgroundOrigin(_) => {}
        BackgroundClip(v, _) => {
            use lightningcss::properties::background::BackgroundClip as BC;
            if matches!(v.first(), Some(BC::Text)) {
                x.background_clip_text = Some(true);
            } else {
                warn_once(
                    diagnostics,
                    "`background-clip` is ignored (v1) — the background always fills the padding \
                     box (`padding-box`, the value a rounded box wants anyway)",
                );
            }
        }
        // CSS `mask`. The paint path cuts one shape, the ring of the
        // gradient-border idiom (`Extras::mask_ring`). Each property records
        // its part, because they cascade apart. Whatever is not that idiom is
        // advised where it is declared.
        Mask(layers, _) => {
            x.mask_images = Some(mask_images(layers.iter().map(|l| &l.image)));
            x.mask_clips = Some(mask_clips(layers.iter().map(|l| &l.clip)));
            x.mask_composite =
                Some(layers.first().map_or(MaskCompositing::Add, |l| mask_compositing(l.composite)));
            advise_mask(x, diagnostics);
        }
        MaskImage(images, _) => {
            x.mask_images = Some(mask_images(images.iter()));
            advise_mask(x, diagnostics);
        }
        MaskClip(clips, _) => x.mask_clips = Some(mask_clips(clips.iter())),
        MaskComposite(list) => {
            x.mask_composite = Some(list.first().map_or(MaskCompositing::Add, |c| mask_compositing(*c)));
            advise_mask(x, diagnostics);
        }
        // Typed, in case a lightningcss release parses it. This one arrives as
        // a custom property (see the `-webkit-mask-composite` arm there).
        WebKitMaskComposite(list) => {
            use lightningcss::traits::ToCss;
            let first = list.first().and_then(|c| c.to_css_string(Default::default()).ok());
            x.mask_composite =
                Some(webkit_mask_compositing(first.as_deref().unwrap_or("source-over")));
            advise_mask(x, diagnostics);
        }
        BorderColor(v) => all_border_colors(r, x, [&v.top, &v.right, &v.bottom, &v.left], diagnostics),
        Border(b) => {
            if let Some(w) = border_width_px(&b.width) {
                let w: AbsoluteLength = px(w).into();
                r.border_widths.top = Some(w);
                r.border_widths.right = Some(w);
                r.border_widths.bottom = Some(w);
                r.border_widths.left = Some(w);
            }
            all_border_colors(r, x, [&b.color; 4], diagnostics);
            set_border_style(r, b.style, diagnostics);
        }
        BorderWidth(v) => {
            r.border_widths.top = border_width_px(&v.top).map(|w| px(w).into());
            r.border_widths.right = border_width_px(&v.right).map(|w| px(w).into());
            r.border_widths.bottom = border_width_px(&v.bottom).map(|w| px(w).into());
            r.border_widths.left = border_width_px(&v.left).map(|w| px(w).into());
        }
        BorderTopColor(c) => border_side_color(&mut r.vn_border_colors.top, CurrentColorTargets::BORDER_TOP, c, x, diagnostics),
        BorderRightColor(c) => border_side_color(&mut r.vn_border_colors.right, CurrentColorTargets::BORDER_RIGHT, c, x, diagnostics),
        BorderBottomColor(c) => border_side_color(&mut r.vn_border_colors.bottom, CurrentColorTargets::BORDER_BOTTOM, c, x, diagnostics),
        BorderLeftColor(c) => border_side_color(&mut r.vn_border_colors.left, CurrentColorTargets::BORDER_LEFT, c, x, diagnostics),
        BorderTop(b) => {
            r.border_widths.top = border_width_px(&b.width).map(|w| px(w).into());
            border_side_color(&mut r.vn_border_colors.top, CurrentColorTargets::BORDER_TOP, &b.color, x, diagnostics);
            set_border_style(r, b.style, diagnostics);
        }
        BorderRight(b) => {
            r.border_widths.right = border_width_px(&b.width).map(|w| px(w).into());
            border_side_color(&mut r.vn_border_colors.right, CurrentColorTargets::BORDER_RIGHT, &b.color, x, diagnostics);
            set_border_style(r, b.style, diagnostics);
        }
        BorderBottom(b) => {
            r.border_widths.bottom = border_width_px(&b.width).map(|w| px(w).into());
            border_side_color(&mut r.vn_border_colors.bottom, CurrentColorTargets::BORDER_BOTTOM, &b.color, x, diagnostics);
            set_border_style(r, b.style, diagnostics);
        }
        BorderLeft(b) => {
            r.border_widths.left = border_width_px(&b.width).map(|w| px(w).into());
            border_side_color(&mut r.vn_border_colors.left, CurrentColorTargets::BORDER_LEFT, &b.color, x, diagnostics);
            set_border_style(r, b.style, diagnostics);
        }
        BorderTopWidth(v) => r.border_widths.top = border_width_px(v).map(|w| px(w).into()),
        BorderRightWidth(v) => r.border_widths.right = border_width_px(v).map(|w| px(w).into()),
        BorderBottomWidth(v) => r.border_widths.bottom = border_width_px(v).map(|w| px(w).into()),
        BorderLeftWidth(v) => r.border_widths.left = border_width_px(v).map(|w| px(w).into()),
        // gpui has ONE border style for the whole box — take the top side, the
        // same compromise `border-color` already makes.
        BorderStyle(v) => set_border_style(r, v.top, diagnostics),
        BorderTopStyle(v) | BorderRightStyle(v) | BorderBottomStyle(v) | BorderLeftStyle(v) => {
            set_border_style(r, *v, diagnostics);
        }
        BorderRadius(v, _) => {
            let mut wrote = 0;
            if corner(&mut r.corner_radii.top_left, &mut r.vn_corner_radii.top_left, &v.top_left) {
                wrote |= RadiusInherit::TOP_LEFT;
            }
            if corner(&mut r.corner_radii.top_right, &mut r.vn_corner_radii.top_right, &v.top_right)
            {
                wrote |= RadiusInherit::TOP_RIGHT;
            }
            if corner(
                &mut r.corner_radii.bottom_right,
                &mut r.vn_corner_radii.bottom_right,
                &v.bottom_right,
            ) {
                wrote |= RadiusInherit::BOTTOM_RIGHT;
            }
            if corner(
                &mut r.corner_radii.bottom_left,
                &mut r.vn_corner_radii.bottom_left,
                &v.bottom_left,
            ) {
                wrote |= RadiusInherit::BOTTOM_LEFT;
            }
            x.radius_inherit.write(wrote, false);
        }
        // `rounded-t` / `rounded-tl-lg` never use the shorthand.
        BorderTopLeftRadius(v, _) => {
            if corner(&mut r.corner_radii.top_left, &mut r.vn_corner_radii.top_left, v) {
                x.radius_inherit.write(RadiusInherit::TOP_LEFT, false);
            }
        }
        BorderTopRightRadius(v, _) => {
            if corner(&mut r.corner_radii.top_right, &mut r.vn_corner_radii.top_right, v) {
                x.radius_inherit.write(RadiusInherit::TOP_RIGHT, false);
            }
        }
        BorderBottomRightRadius(v, _) => {
            if corner(&mut r.corner_radii.bottom_right, &mut r.vn_corner_radii.bottom_right, v) {
                x.radius_inherit.write(RadiusInherit::BOTTOM_RIGHT, false);
            }
        }
        BorderBottomLeftRadius(v, _) => {
            if corner(&mut r.corner_radii.bottom_left, &mut r.vn_corner_radii.bottom_left, v) {
                x.radius_inherit.write(RadiusInherit::BOTTOM_LEFT, false);
            }
        }
        // `grid-cols-N` / `grid-rows-N` -> `repeat(N, minmax(0, 1fr))`, which is
        // exactly (and only) what gpui's GridTemplate models.
        // Two shapes, cheapest first: `repeat(N, minmax(0, 1fr))` is what every
        // Tailwind `grid-cols-N` emits and gpui carries it as a single
        // `GridTemplate`, so it stays on that path. Anything else — `1fr auto`,
        // `200px 1fr`, `repeat(2, auto) 1fr` — becomes an explicit track list.
        GridTemplateColumns(v) => match grid_template(v) {
            Some(t) => r.grid_cols = Some(t),
            None => match grid_tracks(v) {
                Some((tracks, auto)) => {
                    r.vn_grid_cols = Some(tracks);
                    r.vn_grid_cols_auto = Some(auto);
                }
                None => unsupported(
                    diagnostics,
                    "grid-template-columns — `fit-content()`, named lines, a `calc()` breadth \
                     with a percentage in it, and a `repeat(auto-fill|auto-fit, …)` next to a \
                     track that is not fixed-size (invalid CSS) are not modelled; every track \
                     list of lengths, percentages, `auto`, `min-content`, `max-content`, \
                     `<flex>`, `minmax()` and `repeat()` (counted or auto) is",
                ),
            },
        },
        GridTemplateRows(v) => match grid_template(v) {
            Some(t) => r.grid_rows = Some(t),
            None => match grid_tracks(v) {
                Some((tracks, auto)) => {
                    r.vn_grid_rows = Some(tracks);
                    r.vn_grid_rows_auto = Some(auto);
                }
                None => unsupported(
                    diagnostics,
                    "grid-template-rows — `fit-content()`, named lines, a `calc()` breadth with \
                     a percentage in it, and a `repeat(auto-fill|auto-fit, …)` next to a track \
                     that is not fixed-size (invalid CSS) are not modelled; every track list of \
                     lengths, percentages, `auto`, `min-content`, `max-content`, `<flex>`, \
                     `minmax()` and `repeat()` (counted or auto) is",
                ),
            },
        },
        // Placement. gpui carries a `GridLocation` (a start..end pair per axis)
        // and taffy resolves it; only NAMED lines and areas are out of reach.
        GridColumnStart(l) | GridColumnEnd(l) | GridRowStart(l) | GridRowEnd(l) => {
            let Some(placement) = grid_line(l) else {
                unsupported(
                    diagnostics,
                    "grid-row/grid-column with a NAMED line or area — the named grid is not \
                     modelled; use line numbers or `span N`",
                );
                return;
            };
            let loc = r.grid_location.get_or_insert_default();
            match p {
                GridColumnStart(_) => loc.column.start = placement,
                GridColumnEnd(_) => loc.column.end = placement,
                GridRowStart(_) => loc.row.start = placement,
                _ => loc.row.end = placement,
            }
        }
        GridColumn(v) => {
            match (grid_line(&v.start), grid_line(&v.end)) {
                (Some(s), Some(e)) => {
                    let loc = r.grid_location.get_or_insert_default();
                    loc.column = s..e;
                }
                _ => unsupported(
                    diagnostics,
                    "grid-column with a NAMED line or area — use line numbers or `span N`",
                ),
            }
        }
        GridRow(v) => {
            match (grid_line(&v.start), grid_line(&v.end)) {
                (Some(s), Some(e)) => {
                    let loc = r.grid_location.get_or_insert_default();
                    loc.row = s..e;
                }
                _ => unsupported(
                    diagnostics,
                    "grid-row with a NAMED line or area — use line numbers or `span N`",
                ),
            }
        }
        GridArea(v) => {
            match (
                grid_line(&v.row_start),
                grid_line(&v.row_end),
                grid_line(&v.column_start),
                grid_line(&v.column_end),
            ) {
                (Some(rs), Some(re), Some(cs), Some(ce)) => {
                    let loc = r.grid_location.get_or_insert_default();
                    loc.row = rs..re;
                    loc.column = cs..ce;
                }
                _ => unsupported(
                    diagnostics,
                    "grid-area naming a template AREA — `grid-template-areas` is not modelled; \
                     the four-line form (`grid-area: 1 / 1 / 3 / 2`) is",
                ),
            }
        }
        GridAutoFlow(v) => {
            use lightningcss::properties::grid::GridAutoFlow as F;
            let dense = v.contains(F::Dense);
            r.vn_grid_auto_flow = Some(if v.contains(F::Column) {
                if dense {
                    gpui::GridAutoFlow::ColumnDense
                } else {
                    gpui::GridAutoFlow::Column
                }
            } else if dense {
                gpui::GridAutoFlow::RowDense
            } else {
                gpui::GridAutoFlow::Row
            });
        }
        GridTemplateAreas(_) | GridTemplate(_) | Grid(_) => unsupported(
            diagnostics,
            "grid-template-areas (and the `grid` / `grid-template` shorthands that carry it) — \
             the named grid is not modelled. Declare `grid-template-columns`/`-rows` and place \
             items with `grid-column`/`grid-row` line numbers",
        ),
        GridAutoColumns(_) | GridAutoRows(_) => unsupported(
            diagnostics,
            "grid-auto-columns / grid-auto-rows — implicit tracks size as `auto`; give the \
             explicit track list on `grid-template-columns`/`-rows` instead",
        ),
        // taffy honours the inline axis too; gpui had only ever exposed the
        // block one (`align-items`).
        JustifyItems(v) => {
            use lightningcss::properties::align::JustifyItems as J;
            match v {
                J::Normal => r.vn_justify_items = Some(gpui::AlignItems::Stretch),
                J::Stretch => r.vn_justify_items = Some(gpui::AlignItems::Stretch),
                J::BaselinePosition(_) => r.vn_justify_items = Some(gpui::AlignItems::Baseline),
                J::SelfPosition { value, .. } => {
                    r.vn_justify_items = Some(self_position(value));
                }
                J::Legacy(_) => unsupported(diagnostics, "justify-items: legacy"),
                // `justify-items: left | right` are the physical spellings of
                // start/end in a left-to-right writing mode, which is the only
                // one taffy lays out.
                J::Left { .. } => r.vn_justify_items = Some(gpui::AlignItems::FlexStart),
                J::Right { .. } => r.vn_justify_items = Some(gpui::AlignItems::FlexEnd),
            }
        }
        JustifySelf(v) => {
            use lightningcss::properties::align::JustifySelf as J;
            match v {
                J::Auto | J::Normal | J::Stretch => {
                    r.vn_justify_self = Some(gpui::AlignSelf::Stretch)
                }
                J::BaselinePosition(_) => r.vn_justify_self = Some(gpui::AlignSelf::Baseline),
                J::SelfPosition { value, .. } => r.vn_justify_self = Some(self_position(value)),
                J::Left { .. } => r.vn_justify_self = Some(gpui::AlignSelf::FlexStart),
                J::Right { .. } => r.vn_justify_self = Some(gpui::AlignSelf::FlexEnd),
            }
        }
        PlaceItems(v) => {
            // `place-items: <align> <justify>`, both axes at once.
            use lightningcss::properties::align::{AlignItems as A, JustifyItems as J};
            match &v.align {
                A::Stretch | A::Normal => r.align_items = Some(gpui::AlignItems::Stretch),
                A::BaselinePosition(_) => r.align_items = Some(gpui::AlignItems::Baseline),
                A::SelfPosition { value, .. } => r.align_items = Some(self_position(value)),
            }
            match &v.justify {
                J::Normal | J::Stretch => r.vn_justify_items = Some(gpui::AlignItems::Stretch),
                J::BaselinePosition(_) => r.vn_justify_items = Some(gpui::AlignItems::Baseline),
                J::SelfPosition { value, .. } => r.vn_justify_items = Some(self_position(value)),
                J::Legacy(_) => unsupported(diagnostics, "place-items: legacy justify"),
                J::Left { .. } => r.vn_justify_items = Some(gpui::AlignItems::FlexStart),
                J::Right { .. } => r.vn_justify_items = Some(gpui::AlignItems::FlexEnd),
            }
        }
        // ---- outline: a ring outside the border box, taking no layout space --
        Outline(o) => {
            use lightningcss::properties::outline::OutlineStyle as OS;
            let out = outline_mut(r);
            match &o.style {
                OS::Auto => out.style = gpui::BorderStyle::Solid,
                OS::LineStyle(LineStyle::None) | OS::LineStyle(LineStyle::Hidden) => {
                    out.width = px(0.0)
                }
                OS::LineStyle(ls) => {
                    out.style = if matches!(ls, LineStyle::Dashed | LineStyle::Dotted) {
                        gpui::BorderStyle::Dashed
                    } else {
                        gpui::BorderStyle::Solid
                    }
                }
            }
            // `outline: none` is the whole declaration: there is no width to
            // read and no colour to resolve (the shorthand's is the initial
            // `currentColor`, which would advise for nothing).
            if !matches!(&o.style, OS::LineStyle(LineStyle::None | LineStyle::Hidden)) {
                if let Some(w) = border_width_px(&o.width) {
                    r.vn_outline.as_mut().unwrap().width = px(w);
                }
                match &o.color {
                    // `outline-color`'s initial value IS `currentColor`, so the
                    // shorthand almost always carries it. Resolved after the
                    // cascade, like every other `currentColor`.
                    CssColor::CurrentColor => x.current_color.0 |= CurrentColorTargets::OUTLINE,
                    c => match color(c) {
                        Some(c) => r.vn_outline.as_mut().unwrap().color = c,
                        None => unresolved_color(diagnostics, "outline-color"),
                    },
                }
            }
        }
        OutlineWidth(w) => {
            if let Some(v) = border_width_px(w) {
                outline_mut(r).width = px(v);
            }
        }
        OutlineColor(CssColor::CurrentColor) => {
            outline_mut(r);
            x.current_color.0 |= CurrentColorTargets::OUTLINE;
        }
        OutlineColor(c) => match color(c) {
            Some(v) => outline_mut(r).color = v,
            None => unresolved_color(diagnostics, "outline-color"),
        },
        OutlineStyle(s) => {
            use lightningcss::properties::outline::OutlineStyle as OS;
            let out = outline_mut(r);
            match s {
                OS::LineStyle(LineStyle::None) | OS::LineStyle(LineStyle::Hidden) => {
                    out.width = px(0.0)
                }
                other => {
                    out.style = if matches!(
                        other,
                        OS::LineStyle(LineStyle::Dashed) | OS::LineStyle(LineStyle::Dotted)
                    ) {
                        gpui::BorderStyle::Dashed
                    } else {
                        gpui::BorderStyle::Solid
                    };
                    // CSS's initial `outline-width` is `medium` (3px); a style
                    // on its own has to become visible.
                    if out.width == px(0.0) {
                        out.width = px(3.0);
                    }
                }
            }
        }
        BoxShadow(shadows, _) => {
            let mut out = Vec::new();
            for s in shadows.iter() {
                let (Some(c), Some(x), Some(y)) =
                    (color(&s.color), css_length_px(&s.x_offset), css_length_px(&s.y_offset))
                else {
                    // One unresolvable shadow drops ITSELF, not the list — but
                    // it does not drop in silence, because a missing ring in a
                    // stack of three is invisible to read back.
                    unresolved_color(diagnostics, "box-shadow");
                    continue;
                };
                out.push(gpui::BoxShadow {
                    color: c,
                    offset: gpui::point(px(x), px(y)),
                    blur_radius: px(css_length_px(&s.blur).unwrap_or(0.0)),
                    spread_radius: px(css_length_px(&s.spread).unwrap_or(0.0)),
                    inset: s.inset,
                });
            }
            r.box_shadow = Some(out);
        }
        FontFamily(families) => {
            x.web_font = waiting_web_font(families);
            if let Some(family) = first_available_family(families) {
                r.text.font_family = Some(gpui::SharedString::from(family));
            }
            if let Some((family, fallbacks)) = composed_font_stack(families) {
                r.text.font_family = Some(gpui::SharedString::from(family));
                r.text.font_fallbacks = Some(fallbacks);
            }
        }
        FontSize(v) => {
            // Its em, its percentage and `smaller` / `larger` are the PARENT's
            // font size, which the per-element fold makes the basis for this
            // one property (`Engine::compute_node_at`).
            use lightningcss::properties::font::{FontSize as FS, RelativeFontSize as R};
            let size = match v {
                FS::Length(DimensionPercentage::Percentage(p)) => Some(p.0 * em_basis()),
                FS::Length(DimensionPercentage::Calc(c)) => match calc_px(c, &lp_leaf_px) {
                    Some(p) => Some(p),
                    None => calc_linear(c).map(|(fraction, pixels)| fraction * em_basis() + pixels),
                },
                FS::Length(lp) => lp_absolute(lp).map(font_size_px),
                FS::Absolute(k) => Some(absolute_font_size_px(k)),
                // CSS Fonts 4 leaves the ratio to the UA and suggests 1.2.
                FS::Relative(R::Smaller) => Some(em_basis() / 1.2),
                FS::Relative(R::Larger) => Some(em_basis() * 1.2),
            };
            if let Some(size) = size {
                r.text.font_size = Some(px(size.max(0.0)).into());
            }
        }
        FontWeight(w) => {
            let n = match w {
                CssFontWeight::Absolute(AbsoluteFontWeight::Weight(n)) => Some(*n),
                CssFontWeight::Absolute(AbsoluteFontWeight::Normal) => Some(400.0),
                CssFontWeight::Absolute(AbsoluteFontWeight::Bold) => Some(700.0),
                _ => None,
            };
            match n {
                Some(n) => r.text.font_weight = Some(gpui::FontWeight(n)),
                // `bolder` / `lighter` step relative to the INHERITED weight,
                // through the table in CSS Fonts 4 § 2.2.1. vn-css folds no
                // inherited text property (gpui's `TextStyleRefinement`
                // cascades at paint time), so the step is taken from the
                // initial `normal` — which is the row every real use lands on:
                // `b, strong { font-weight: bolder }` in a UA sheet, over text
                // that is 400.
                None => {
                    r.text.font_weight = Some(match w {
                        CssFontWeight::Bolder => gpui::FontWeight(700.0),
                        _ => gpui::FontWeight(100.0),
                    })
                }
            }
        }
        LineHeight(v) => {
            r.text.line_height = match v {
                // A number inherits as the number: gpui's fraction is taken of
                // each element's own font size at paint.
                CssLineHeight::Number(n) => Some(relative(*n)),
                // A percentage computes to a length, as `1.5em` does, and the
                // children inherit that length.
                CssLineHeight::Length(DimensionPercentage::Percentage(p)) => Some(px(p.0 * em_basis()).into()),
                CssLineHeight::Length(lp) => lp_definite(lp),
                CssLineHeight::Normal => Some(relative(1.2)),
            };
        }
        FontStyle(v) => {
            use lightningcss::properties::font::FontStyle as FS;
            r.text.font_style = Some(match v {
                FS::Normal => gpui::FontStyle::Normal,
                FS::Italic => gpui::FontStyle::Italic,
                FS::Oblique(_) => {
                    unsupported(diagnostics, "font-style: oblique <angle> (angle ignored)");
                    gpui::FontStyle::Oblique
                }
            });
        }
        TextAlign(v) => {
            use gpui::TextAlign as G;
            let mapped = match v {
                CssTextAlign::Left | CssTextAlign::Start => Some(G::Left),
                CssTextAlign::Center => Some(G::Center),
                CssTextAlign::Right | CssTextAlign::End => Some(G::Right),
                _ => None,
            };
            match mapped {
                Some(m) => r.text.text_align = Some(m),
                None => unsupported(diagnostics, "text-align"),
            }
        }
        WhiteSpace(v) => {
            use lightningcss::properties::text::WhiteSpace as WS;
            // TWO halves, and they go to two different places. gpui's
            // `WhiteSpace` decides whether a line may WRAP; the COLLAPSING —
            // which of the source's spaces, tabs and newlines survive — is
            // applied to the string by the renderer (`vn_tree`'s
            // `processed_text`), for the same reason `text-transform` is: gpui
            // shapes whatever string it is handed.
            //
            // Until m21 only the wrap half existed and every non-`normal`
            // keyword produced an advisory saying whitespace was still
            // collapsed. It was not — INTERIOR runs were passed through
            // untouched, so a text node holding a `\n` rendered as two lines
            // where a browser renders one. That is what garbled every
            // multi-line `console.log` in the devtools console
            // (research/reports/m21/devtools-v2.md).
            let mode = match v {
                WS::Normal => WhiteSpaceMode::Normal,
                WS::NoWrap => WhiteSpaceMode::Nowrap,
                WS::Pre => WhiteSpaceMode::Pre,
                WS::PreWrap => WhiteSpaceMode::PreWrap,
                WS::PreLine => WhiteSpaceMode::PreLine,
                WS::BreakSpaces => WhiteSpaceMode::BreakSpaces,
            };
            x.white_space = Some(mode);
            r.text.white_space = Some(match mode {
                // The keywords that do not wrap. `break-spaces` wraps, and
                // differs from `pre-wrap` only in how it breaks INSIDE a run
                // of spaces — a distinction gpui's line breaker does not
                // expose, so the two are the same box here.
                WhiteSpaceMode::Nowrap | WhiteSpaceMode::Pre => gpui::WhiteSpace::Nowrap,
                _ => gpui::WhiteSpace::Normal,
            });
        }
        TextOverflow(v, _) => {
            use lightningcss::properties::overflow::TextOverflow as TO;
            match v {
                TO::Ellipsis => r.text.text_overflow = Some(gpui::TextOverflow::Truncate("…".into())),
                // A refinement cannot express "clear", so `text-clip` after an
                // `text-ellipsis` is a no-op rather than a reset.
                TO::Clip => unsupported(
                    diagnostics,
                    "text-overflow: clip — cannot undo an earlier ellipsis",
                ),
            }
        }
        TextDecorationLine(v, _) => {
            use lightningcss::properties::text::TextDecorationLine as TDL;
            if v.contains(TDL::Underline) {
                r.text.underline = Some(gpui::UnderlineStyle::default());
            }
            if v.contains(TDL::LineThrough) {
                r.text.strikethrough = Some(gpui::StrikethroughStyle::default());
            }
            if v.intersects(TDL::Overline | TDL::Blink) {
                unsupported(diagnostics, "text-decoration-line: overline/blink");
            }
            if v.is_empty() {
                // `text-decoration-line: none` has to UNDO an earlier
                // declaration — `.no-underline` exists precisely to cancel a
                // `.underline` — and a `StyleRefinement` cannot say "clear this
                // field": its `Option` means "declared or not", never "declared
                // as nothing". So `none` is expressed as a decoration that
                // paints nothing, zero thickness AND transparent, because
                // either one alone would be at the mercy of gpui's stroke
                // snapping (`Window::snap_stroke` rounds a hairline UP to one
                // device pixel).
                r.text.underline = Some(gpui::UnderlineStyle {
                    thickness: px(0.0),
                    color: Some(gpui::transparent_black()),
                    wavy: false,
                    offset: px(0.0),
                });
                r.text.strikethrough = Some(gpui::StrikethroughStyle {
                    thickness: px(0.0),
                    color: Some(gpui::transparent_black()),
                });
            }
        }
        Cursor(c) => {
            use lightningcss::properties::ui::CursorKeyword as K;
            let mapped = match c.keyword {
                K::Pointer => Some(gpui::CursorStyle::PointingHand),
                K::Default | K::Auto => Some(gpui::CursorStyle::Arrow),
                K::Text => Some(gpui::CursorStyle::IBeam),
                _ => None,
            };
            if let Some(m) = mapped {
                r.mouse_cursor = Some(m);
            }
        }
        Filter(list, _) => apply_filter(list, x, diagnostics),
        // `backdrop-filter` — `blur()` is the one filter function the renderer
        // implements (gpui `BackdropBlurRect`, D3D11 Dual-Kawase pyramid).
        // Everything else in the list is advised rather than silently dropped.
        BackdropFilter(list, _) => apply_backdrop_filter(list, r, diagnostics),
        Custom(c) => match &c.name {
            // `--shading` is the ONE custom property vue-native interprets: a
            // GPU effect has no web equivalent, so it needs a spelling a
            // browser parses, stores and ignores rather than one a browser
            // DROPS. It is a real mapped property that merely happens to be
            // spelled as a custom property, so `channel()` keeps it on the
            // static channel and it lands here. Everything about it — grammar,
            // schemas, advisories, the inheritance divergence — lives in
            // `crate::effect`.
            lightningcss::properties::custom::CustomPropertyName::Custom(name)
                if name.0.as_ref() == effect::SHADING =>
            {
                if let Some(spec) = effect::parse(&c.value, diagnostics) {
                    r.vn_effect = Some(spec);
                }
            }
            // `--window-control` — the second interpreted custom property; see
            // [`WindowControl`] for why the caption-button role is spelled as
            // one rather than as a `window-drag` value. `channel()` keeps it on
            // the static channel with `--shading`, which is how it reaches here
            // instead of folding into the element's `Vars` map.
            lightningcss::properties::custom::CustomPropertyName::Custom(name)
                if name.0.as_ref() == WINDOW_CONTROL =>
            {
                if let Some(c) = window_control(&c.value, diagnostics) {
                    x.window_control = Some(c);
                }
            }
            // Every other `--x` travels the custom-property channel (see
            // `channel`) and never reaches the mapper.
            lightningcss::properties::custom::CustomPropertyName::Custom(_) => {}
            // Not a custom property at all — an unknown/vendor-prefixed one
            // (`-webkit-line-clamp`), which is a real "we do not map this".
            //
            // `object-fit` is a REAL CSS property that lightningcss simply has
            // no typed representation for, so it lands in this bucket too; it
            // is intercepted here and parsed by hand.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "object-fit" =>
            {
                if let Some(fit) = object_fit(&c.value, diagnostics) {
                    x.object_fit = Some(fit);
                }
            }
            // `window-drag` is a REAL CSS property (CSS Basic User Interface 4,
            // CSSWG 2026-01-21) that lightningcss has no typed representation
            // for yet, so it lands in the same bucket `object-fit` does.
            // `app-region` / `-webkit-app-region` are Chromium's permanent
            // aliases and are accepted with NO advisory — Chromium states it is
            // not deprecating them, so warning would be pure noise (D-13).
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if matches!(
                    name.0.as_ref(),
                    "window-drag" | "app-region" | "-webkit-app-region"
                ) =>
            {
                if let Some(d) = window_drag(name.0.as_ref(), &c.value, diagnostics) {
                    x.window_drag = Some(d);
                }
            }
            // `pointer-events` is a real CSS property that lightningcss has no
            // typed representation for either. `none` is expressed by giving
            // the node no hitbox at all: no listeners, no hover/active rungs
            // (see `vn-tree`'s renderer). It therefore stops REACTING; a click
            // still does not fall through to whatever is painted underneath,
            // because gpui resolves hits per hitbox rather than per pixel.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "pointer-events" =>
            {
                match single_ident(&c.value, "pointer-events", diagnostics) {
                    Some("none") => x.pointer_events_none = Some(true),
                    Some("auto") => x.pointer_events_none = Some(false),
                    other => once(
                        diagnostics,
                        format!(
                            "unsupported value for pointer-events: {} — only `none` and `auto` \
                             are modelled (SVG-only values have nothing to apply to)",
                            other.unwrap_or("<not a keyword>")
                        ),
                    ),
                }
            }
            // `outline-offset` and `-webkit-line-clamp` are REAL properties
            // that lightningcss has no typed representation for; both are
            // honoured, so they are parsed by hand out of the token list the
            // same way `object-fit` is.
            // `content` — the property that decides whether a `::before` /
            // `::after` box exists at all. Only the literal-string forms
            // generate one here; counters and `attr()` need machinery
            // (a counter tree, a live attribute subscription) that does not
            // exist, and `none`/`normal` mean "no box", which is the default.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "content" =>
            {
                match content_string(&c.value) {
                    ContentValue::Text(t) => x.content = Some(t),
                    ContentValue::None => x.content = None,
                    ContentValue::Unsupported(what) => once(
                        diagnostics,
                        format!(
                            "`content: {what}` does not generate a box (v1) — only a literal \
                             string does. Counters, `attr()`, `url()` and the `open-quote` \
                             family have no model here; render the value as a real element"
                        ),
                    ),
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "outline-offset" =>
            {
                match single_length_px(&c.value) {
                    Some(v) => outline_mut(r).offset = px(v),
                    None => unsupported(diagnostics, "outline-offset (a <length> is expected)"),
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "-webkit-line-clamp" || name.0.as_ref() == "line-clamp" =>
            {
                match single_number(&c.value) {
                    Some(n) if n >= 1.0 => r.text.line_clamp = Some(n as usize),
                    // `line-clamp: none` — a refinement cannot spell "clear".
                    _ => unsupported(
                        diagnostics,
                        "line-clamp: none — cannot undo an earlier clamp",
                    ),
                }
            }
            // `-webkit-text-fill-color` (see `Extras::text_fill_color`). A
            // named colour arrives as a bare ident, as in `scrollbar-color`.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref().eq_ignore_ascii_case("-webkit-text-fill-color") =>
            {
                use lightningcss::properties::custom::{Token, TokenOrValue};
                let mut tokens = c
                    .value
                    .0
                    .iter()
                    .filter(|t| !matches!(t, TokenOrValue::Token(Token::WhiteSpace(_))));
                let fill = match (tokens.next(), tokens.next()) {
                    (Some(TokenOrValue::Color(cc)), None) => color(cc).map(TextFill::Color),
                    (Some(TokenOrValue::Token(Token::Ident(id))), None)
                        if id.as_ref().eq_ignore_ascii_case("currentcolor") =>
                    {
                        Some(TextFill::CurrentColor)
                    }
                    (Some(TokenOrValue::Token(Token::Ident(id))), None) => {
                        named_color(id.as_ref()).map(TextFill::Color)
                    }
                    _ => None,
                };
                match fill {
                    Some(f) => x.text_fill_color = Some(f),
                    None if is_css_wide_keyword(&c.value) => {}
                    None => unsupported(diagnostics, "-webkit-text-fill-color (a <color> is expected)"),
                }
            }
            // `text-underline-offset` moves the line down from where the
            // font's metrics put it, which is one number on the decoration the
            // paint already reads.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "text-underline-offset" =>
            {
                if single_ident_is(&c.value, "auto") || is_css_wide_keyword(&c.value) {
                    // `auto` IS the font's own position.
                } else if let Some(offset) = single_length_px(&c.value) {
                    // The decoration is carried ON the run, so the offset lands
                    // on whatever underline the cascade has already produced,
                    // and on the one a later `text-decoration-line` produces.
                    x.underline_offset = Some(offset);
                    if let Some(u) = r.text.underline.as_mut() {
                        u.offset = px(offset);
                    }
                } else {
                    unsupported(diagnostics, "text-underline-offset (a <length> is expected)");
                }
            }
            // `text-decoration-skip-ink` decides whether an underline BREAKS
            // around descenders. gpui draws one unbroken rule, and the property
            // has no other effect.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "text-decoration-skip-ink" =>
            {
                if !single_ident_is(&c.value, "none") && !is_css_wide_keyword(&c.value) {
                    warn_once(
                        diagnostics,
                        "`text-decoration-skip-ink: auto` is not applied (v1) — gpui draws one \
                         unbroken underline and does not lift it around descenders. `none` (draw \
                         it straight through) is what it already does",
                    );
                }
            }
            // OpenType FEATURES reach the shaper for real: gpui's `Font`
            // carries a `FontFeatures` tag/value list and DirectWrite applies
            // it as an `IDWriteTypography` over the run.
            //
            // All four properties share that ONE field. Within a rule they
            // merge (a later tag wins); across rules and down the tree gpui's
            // refinement replaces the list whole, so a child that sets
            // `font-variant-numeric` does not keep its parent's
            // `font-kerning: none` the way CSS's independent properties would.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if matches!(
                    name.0.as_ref(),
                    "font-feature-settings"
                        | "font-variant-numeric"
                        | "font-variant-ligatures"
                        | "font-kerning"
                ) =>
            {
                match font_features(&c.value, name.0.as_ref()) {
                    // `normal` and the CSS-wide keywords write nothing: the
                    // first is the initial value (no features) and the rest
                    // mean "take what you would have taken anyway".
                    Some(features) if features.0.is_empty() => {}
                    Some(features) => {
                        let mut pairs = r
                            .text
                            .font_features
                            .take()
                            .map(|f| (*f.0).clone())
                            .unwrap_or_default();
                        for (tag, v) in features.0.iter() {
                            match pairs.iter_mut().find(|(t, _)| t == tag) {
                                Some(slot) => slot.1 = *v,
                                None => pairs.push((tag.clone(), *v)),
                            }
                        }
                        r.text.font_features = Some(gpui::FontFeatures(std::sync::Arc::new(pairs)));
                    }
                    None => warn_once(
                        diagnostics,
                        "an OpenType FEATURE value was not understood (v1) — \
                         `font-feature-settings` takes `\"tag\" <number|on|off>` pairs, \
                         `font-variant-numeric` / `font-variant-ligatures` their keyword sets and \
                         `font-kerning` `auto | normal | none`; all reach the shaper when they \
                         parse",
                    ),
                }
            }
            // VARIATION axes are a different thing and genuinely absent: gpui's
            // `Font` has no axis list, and its DirectWrite path selects a face
            // through the weight/width/slope model rather than
            // `IDWriteFontFace5::GetFontAxisValues`. `font-optical-sizing` is
            // the exception — its `auto` IS applied now, by
            // `SetAutomaticFontAxes(OPTICAL_SIZE)` in the fork, which is what a
            // browser does by default; only an explicit `none` diverges.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "font-optical-sizing" =>
            {
                if !single_ident_is(&c.value, "auto") && !is_css_wide_keyword(&c.value) {
                    warn_once(
                        diagnostics,
                        "`font-optical-sizing: none` is not applied (v1) — a variable font's \
                         `opsz` axis is driven from the font size here, which is what `auto` (the \
                         initial value, and what every browser does) asks for. There is no way to \
                         switch it off",
                    );
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "font-variation-settings" =>
            {
                // `normal` IS "no axis values", which is exactly what a face
                // selected through the weight/width/slope model has.
                if single_ident_is(&c.value, "normal") || is_css_wide_keyword(&c.value) {
                    return;
                }
                warn_once(
                    diagnostics,
                    "`font-variation-settings` is not applied (v1) — a face is selected through \
                     the weight/width/slope model, not by axis value, so an arbitrary axis \
                     (`wdth`, `slnt`, a custom one) cannot be instantiated. `font-weight` and \
                     `font-style` do reach the axis the WWS model covers, and `opsz` is driven \
                     from the font size",
                )
            }
            // `-webkit-font-smoothing`, `-moz-osx-font-smoothing`,
            // `-webkit-tap-highlight-color`, `-webkit-overflow-scrolling` and
            // `-ms-overflow-style` never get here: `crate::prefix` drops them
            // (desktop Chromium on Windows draws none of them either).
            //
            // There is no touch scroller here, so every value of these
            // produces the same behaviour and none of them has anything to
            // diverge from.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if matches!(name.0.as_ref(), "touch-action" | "scroll-behavior") => {}
            // `scrollbar-width` / `scrollbar-color` are real CSS properties
            // lightningcss has no typed representation for. They mattered to
            // nobody while we drew no scrollbars; now they decide whether the
            // overlay bar appears at all, and what colour it is.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "scrollbar-width" =>
            {
                if let Some(w) = scrollbar_width(&c.value, diagnostics) {
                    x.scrollbar_width = Some(w);
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "scrollbar-color" =>
            {
                if let Some(pair) = scrollbar_color(&c.value, diagnostics) {
                    x.scrollbar_color = Some(pair);
                }
            }
            // `overscroll-behavior` — whether a wheel this box cannot use
            // reaches its ancestors. The shorthand sets both axes; the
            // longhands one each.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if matches!(
                    name.0.as_ref(),
                    "overscroll-behavior"
                        | "overscroll-behavior-x"
                        | "overscroll-behavior-y"
                        | "overscroll-behavior-inline"
                        | "overscroll-behavior-block"
                ) =>
            {
                let property = name.0.as_ref();
                if let Some(b) = overscroll(property, &c.value, diagnostics) {
                    // The logical spellings resolve against `horizontal-tb`,
                    // the only writing mode we lay out in.
                    match property {
                        "overscroll-behavior-x" | "overscroll-behavior-inline" => {
                            x.overscroll_x = Some(b)
                        }
                        "overscroll-behavior-y" | "overscroll-behavior-block" => {
                            x.overscroll_y = Some(b)
                        }
                        _ => {
                            x.overscroll_x = Some(b);
                            x.overscroll_y = Some(b);
                        }
                    }
                }
            }
            // `isolation` — a real property lightningcss has no typed
            // representation for. `isolate` makes the box a stacking context;
            // that is all it does here, as there is no `mix-blend-mode` for it
            // to contain.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "isolation" =>
            {
                if single_ident_is(&c.value, "isolate") {
                    x.isolate = Some(true);
                } else if single_ident_is(&c.value, "auto") {
                    x.isolate = Some(false);
                } else if !is_css_wide_keyword(&c.value) {
                    unsupported(diagnostics, "isolation (expected `auto` or `isolate`)");
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if matches!(
                    name.0.as_ref(),
                    "border-collapse" | "border-spacing" | "caption-side" | "table-layout"
                        | "empty-cells"
                ) =>
            {
                warn_once(
                    diagnostics,
                    "the table properties are not applied (v1) — `<table>` and friends lay out as \
                     plain blocks; build tables out of flex rows or a grid track list, which both \
                     render",
                )
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "clip" =>
            {
                // The deprecated `clip` survives in exactly one idiom: the
                // screen-reader-only box, `clip: rect(0, 0, 0, 0)`. That rect
                // is EMPTY, and an element clipped to an empty rect paints
                // nothing — which is `visibility: hidden` with the layout box
                // intact, and is expressible. A non-degenerate rect would need
                // a per-element content mask in the paint path.
                if clip_rect_is_empty(&c.value) {
                    r.visibility = Some(gpui::Visibility::Hidden);
                } else {
                    warn_once(
                        diagnostics,
                        "a non-empty legacy `clip: rect()` is not applied (v1) — it is deprecated \
                         in CSS too, and clipping a box to an arbitrary sub-rectangle needs a \
                         content mask the paint path does not carry. `clip: rect(0,0,0,0)` (the \
                         visually-hidden idiom) IS honoured, and `overflow: hidden` on a sized \
                         box clips for real",
                    );
                }
            }
            // lightningcss has a `-webkit-mask-composite` property, but its name
            // lookup strips the prefix first, finds the unprefixed
            // `mask-composite` (which takes no prefix), and gives up. So the
            // declaration arrives here as raw tokens. The first keyword is the
            // top layer's, the only one composited onto anything.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref().eq_ignore_ascii_case("-webkit-mask-composite") =>
            {
                use lightningcss::properties::custom::{Token, TokenOrValue};
                let first = c.value.0.iter().find_map(|t| match t {
                    TokenOrValue::Token(Token::Ident(id)) => Some(id.as_ref()),
                    _ => None,
                });
                x.mask_composite = Some(webkit_mask_compositing(first.unwrap_or("source-over")));
                advise_mask(x, diagnostics);
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name) => {
                once(
                    diagnostics,
                    format!("property not mapped (v1): {}", name.0.as_ref()),
                );
            }
        },
        // ---- named, not mapped ----------------------------------------------
        // Everything below reaches the renderer as a real declaration and does
        // nothing. The generic "property not mapped (v1)" is true but useless;
        // each of these says WHY and, where there is one, names the thing to
        // write instead. That is the whole difference between a diagnostic and
        // an advisory (docs/COMPAT.md).
        // `normal` is zero extra spacing, which is the initial value and also
        // what not writing the field means — so it needs no explicit clear.
        LetterSpacing(v) => {
            if let Some(px) = spacing_px(v) {
                r.text.letter_spacing = Some(gpui::px(px));
            }
        }
        WordSpacing(v) => {
            if let Some(px) = spacing_px(v) {
                r.text.word_spacing = Some(gpui::px(px));
            }
        }
        VerticalAlign(v) => {
            use lightningcss::properties::font::VerticalAlign as VA;
            use lightningcss::properties::font::VerticalAlignKeyword as VK;
            // `baseline` is the initial value AND what a flex row already does
            // with `align-items: baseline`, so it is not a divergence. The other
            // keywords align a box INSIDE a line box, which taffy has no concept
            // of.
            // …and `vertical-align` only applies to an INLINE-LEVEL box or a
            // table cell, so on a block-level one it does nothing in a browser
            // either. Tailwind's preflight relies on exactly that pairing:
            // `img, svg, video, canvas, … { display: block; vertical-align:
            // middle }` sets both in one rule so the `middle` is inert unless
            // an author puts the `display` back.
            let block_level = matches!(
                r.display,
                Some(gpui::Display::Block | gpui::Display::Flex | gpui::Display::Grid)
            ) && x.inline_level != Some(true);
            if !matches!(v, VA::Keyword(VK::Baseline)) && !block_level {
                warn_once(
                    diagnostics,
                    "`vertical-align` is not applied (v1) — it aligns boxes inside a LINE box, \
                     and taffy has no inline formatting context. `align-items` / `align-self` on \
                     the flex parent is the equivalent that works",
                );
            }
        }
        TextIndent(v) => {
            // `text-indent: 0` is the initial value; indenting by nothing is
            // exactly what an unimplemented first-line indent does.
            if lp_definite(&v.value).is_none_or(|l| l != gpui::px(0.0).into()) {
                warn_once(
                    diagnostics,
                    "`text-indent` is not applied (v1) — there is no first-line concept in the                      shaper. `padding-left` indents the whole block",
                );
            }
        }
        // `list-style: none` — the reset every UI framework writes — asks for no
        // marker, which is what a tree with no marker boxes already produces.
        // `list-style-position` has nothing to position while that holds.
        ListStyle(v) => {
            use lightningcss::properties::list::ListStylePosition as P;
            match list_marker(&v.list_style_type) {
                Some(marker) => x.list_marker = Some(marker),
                None => advise_list_marker(diagnostics),
            }
            x.list_marker_inside = Some(matches!(v.position, P::Inside));
        }
        ListStyleType(v) => match list_marker(v) {
            Some(marker) => x.list_marker = Some(marker),
            None => advise_list_marker(diagnostics),
        },
        // A marker IMAGE would be a raster in the marker box, which is the one
        // shape of marker there is no box to put.
        ListStyleImage(v) => {
            if !matches!(v, lightningcss::values::image::Image::None) {
                advise_list_marker(diagnostics);
            }
        }
        ListStylePosition(v) => {
            use lightningcss::properties::list::ListStylePosition as P;
            x.list_marker_inside = Some(matches!(v, P::Inside));
        }
        Resize(v) => {
            use lightningcss::properties::ui::Resize as R;
            // `resize: none` is the initial value and already true of every box
            // here; the others ask for a UA drag affordance that gpui has no
            // equivalent of.
            if !matches!(v, R::None) {
                warn_once(
                    diagnostics,
                    "`resize` is not applied (v1) — there is no user-resize affordance on a gpui                      box. A drag handle has to be a real element with a pointer handler",
                );
            }
        }
        UserSelect(v, _) => {
            use lightningcss::properties::ui::UserSelect as U;
            use crate::mapper::UserSelect as S;
            // Page text is selectable (vn_tree::selection), so each value is
            // applied as written — except `contain`, which keeps no selection
            // inside the element here and is said once.
            x.user_select = Some(match v {
                U::Auto => S::Auto,
                U::None => S::None,
                U::Text => S::Text,
                U::All => S::All,
                U::Contain => {
                    warn_once(
                        diagnostics,
                        "`user-select: contain` selects like `text` here: a selection that starts \
                         inside the element is not kept inside it",
                    );
                    S::Text
                }
            });
        }
        // `appearance` switches a native OS control skin on or off. The one
        // element here that has one is `<select>`: its arrow, and the room
        // Chromium keeps for it, are drawn unless the value is `none`. Every
        // other value is the skin Chromium draws for a select too (`menulist`,
        // `menulist-button`, `auto`…), and on any other element nothing
        // changes, so there is nothing to advise.
        AccentColor(c) => {
            use lightningcss::properties::ui::ColorOrAuto;
            x.accent_color = Some(match c {
                ColorOrAuto::Auto => None,
                ColorOrAuto::Color(c) => color(c),
            });
        }
        Appearance(a, _) => x.appearance_none = Some(matches!(a, lightningcss::properties::ui::Appearance::None)),
        // `tab-size` sizes a preserved tab. Whitespace is always collapsed here
        // (`white-space` says so, once, where the divergence actually is), so a
        // tab is a single space at every tab-size and this property could not
        // change the result if it were implemented.
        TabSize(..) => {}
        // `text-size-adjust` exists to stop MOBILE BROWSERS inflating text.
        // Nothing here inflates it, under any value.
        TextSizeAdjust(..) => {}
        // `clip-path` cuts the element's whole painting and its hit test to a
        // shape (CSS Masking 1 §5), which gpui does with a rounded content
        // mask around the box (`gpui::Style::vn_clip_path`): `inset()`,
        // `circle()` and a lone `<geometry-box>` exactly, `ellipse()` as a
        // capsule. The screen-reader-only idiom, `inset(50%)`, encloses
        // nothing and so paints nothing, while the layout box and the CSS
        // `visibility` stay. `polygon()`, `path()` and `url()` are advised
        // in [`clip_path`].
        ClipPath(v, _) => {
            if let Some(clip) = clip_path(v, diagnostics) {
                r.vn_clip_path = Some(clip);
            }
        }
        // The line breaker's three CSS Text 3 properties, inherited through
        // gpui's text style like `white-space` (the fork's `TextWrap`).
        // `word-wrap` is the legacy alias for `overflow-wrap` — CSS Text 3
        // defines the two as the same property, and lightningcss parses them
        // into two ids.
        WordWrap(v) | OverflowWrap(v) => {
            use lightningcss::properties::text::OverflowWrap as OW;
            r.text.overflow_wrap = Some(match v {
                OW::Normal => gpui::OverflowWrap::Normal,
                OW::BreakWord => gpui::OverflowWrap::BreakWord,
                OW::Anywhere => gpui::OverflowWrap::Anywhere,
            });
        }
        WordBreak(v) => {
            use lightningcss::properties::text::WordBreak as WB;
            r.text.word_break = Some(match v {
                WB::Normal => gpui::WordBreak::Normal,
                WB::BreakAll => gpui::WordBreak::BreakAll,
                WB::KeepAll => gpui::WordBreak::KeepAll,
                WB::BreakWord => gpui::WordBreak::BreakWord,
            });
        }
        // `manual` (the initial value) breaks after a soft hyphen and shows a
        // hyphen there; `none` does not break there. `auto` hyphenates words
        // from a dictionary, which there is none of: it is `manual`.
        Hyphens(v, _) => {
            use lightningcss::properties::text::Hyphens as H;
            r.text.hyphens = Some(match v {
                H::None => gpui::Hyphens::None,
                H::Manual => gpui::Hyphens::Manual,
                H::Auto => {
                    warn_once(
                        diagnostics,
                        "`hyphens: auto` hyphenates only at soft hyphens (`&shy;`), as `manual` \
                         does (v1) — there is no hyphenation dictionary",
                    );
                    gpui::Hyphens::Manual
                }
            });
        }
        // `scroll-margin` / `scroll-padding` only shift where `scrollIntoView`
        // and scroll snapping land. Neither exists here — there is no
        // `scrollIntoView` in the DOM shim and no snap implementation — so the
        // properties have nothing to be wrong about; the absence of the two
        // features is where that divergence lives.
        ScrollMarginTop(_) | ScrollMarginBottom(_) | ScrollMarginLeft(_) | ScrollMarginRight(_)
        | ScrollMargin(_) | ScrollMarginBlock(_) | ScrollMarginInline(_)
        | ScrollMarginBlockStart(_) | ScrollMarginBlockEnd(_) | ScrollMarginInlineStart(_)
        | ScrollMarginInlineEnd(_) | ScrollPaddingTop(_) | ScrollPaddingBottom(_)
        | ScrollPaddingLeft(_) | ScrollPaddingRight(_) | ScrollPadding(_) => {}
        TextTransform(t) => {
            use crate::mapper::TextTransform as TT;
            use lightningcss::properties::text::TextTransformCase as C;
            x.text_transform = Some(match t.case {
                C::None => TT::None,
                C::Uppercase => TT::Uppercase,
                C::Lowercase => TT::Lowercase,
                C::Capitalize => TT::Capitalize,
            });
        }
        // A KNOWN property whose value did not parse into a typed value. These
        // are `var()`-composed and travel the dynamic channel, so reaching the
        // mapper means the value was STILL unparsed after substitution — an
        // unbalanced `var(` or a token stream that is not valid for the
        // property. Kept as a defensive advisory rather than silence.
        Unparsed(u) => {
            // Vaul's drawer extension uses `bottom: initial` when its top
            // edge is pinned. It is a valid reset, not an unresolved var().
            // Write Auto explicitly so it cancels an earlier inset.
            let reset = initial_inset_sides(u.property_id.name(), &u.value);
            if reset != 0 {
                if reset & 0b0001 != 0 {
                    r.inset.top = Some(Length::Auto);
                }
                if reset & 0b0010 != 0 {
                    r.inset.right = Some(Length::Auto);
                }
                if reset & 0b0100 != 0 {
                    r.inset.bottom = Some(Length::Auto);
                }
                if reset & 0b1000 != 0 {
                    r.inset.left = Some(Length::Auto);
                }
                return;
            }
            // …with one exception that is not a failure: lightningcss has no
            // typed value for a list property's `none`, so `box-shadow: none`
            // (Tailwind's `shadow-none`, the commonest shadow declaration
            // there is) always lands here. An EMPTY list is exactly what it
            // means, and writing it is what lets a later rule cancel an
            // earlier one's shadow.
            if matches!(u.property_id, PropertyId::BoxShadow(_))
                && matches!(single_ident_raw(&u.value), Some("none"))
            {
                r.box_shadow = Some(Vec::new());
                return;
            }
            // …and one that is not a failure either: `inherit` is a CSS-wide
            // keyword, and lightningcss only gives it a typed value for
            // `PropertyId::All` — every other `x: inherit` parses as Unparsed
            // and lands here, `var()` or no `var()`.
            //
            // In vue-native it is also a NO-OP, which is the part worth stating.
            // vn-css folds no inherited property: the inherited ones are all
            // text properties, and gpui's `TextStyleRefinement` already cascades
            // down the element tree at paint time. So "take the parent's value"
            // is what NOT writing the property does — the declaration is
            // honoured exactly by ignoring it.
            //
            // Tailwind's preflight alone ships nine of these
            // (`button, input, … { font-family: inherit; font-weight: inherit;
            // letter-spacing: inherit; color: inherit; … }`), so without this
            // every app reports five or six "still unresolved after var()"
            // advisories that name a property nothing is wrong with — noise
            // that buries the real ones.
            //
            // Except for `border-radius`, which does not inherit: there
            // `inherit` asks for the parent's radius, and dropping it left the
            // card layers that round themselves with it (`.card-bg { overflow:
            // hidden; border-radius: inherit }`) square. It is recorded here and
            // filled in after the cascade ([`RadiusInherit`]).
            // `path()`, and the CSS Shapes 2 functions (`rect()`, `xywh()`,
            // `shape()`): Chromium applies them, and lightningcss has no typed
            // value for them. `path()` is parsed here ([`clip_path_path`]);
            // the others are advised.
            if matches!(u.property_id, PropertyId::ClipPath(_)) && !has_substitution(&u.value) {
                use lightningcss::properties::custom::{Token, TokenOrValue};
                match clip_path_path(&u.value) {
                    Some(Ok(clip)) => {
                        r.vn_clip_path = Some(clip);
                        return;
                    }
                    Some(Err(why)) => {
                        once(
                            diagnostics,
                            format!("`clip-path: path()` was dropped as invalid, as Chromium drops it: {why}"),
                        );
                        return;
                    }
                    None => {}
                }
                let what = u.value.0.iter().find_map(|t| match t {
                    TokenOrValue::Function(f) => Some(format!("{}()", f.name.0.as_ref())),
                    TokenOrValue::Token(Token::Function(name)) => Some(format!("{}()", name.as_ref())),
                    _ => None,
                });
                clip_path_unsupported(diagnostics, what.as_deref().unwrap_or("this value"));
                return;
            }
            if matches!(single_ident_raw(&u.value), Some("inherit")) {
                let corners = match u.property_id {
                    PropertyId::BorderRadius(_) => RadiusInherit::ALL,
                    PropertyId::BorderTopLeftRadius(_) => RadiusInherit::TOP_LEFT,
                    PropertyId::BorderTopRightRadius(_) => RadiusInherit::TOP_RIGHT,
                    PropertyId::BorderBottomRightRadius(_) => RadiusInherit::BOTTOM_RIGHT,
                    PropertyId::BorderBottomLeftRadius(_) => RadiusInherit::BOTTOM_LEFT,
                    _ => 0,
                };
                x.radius_inherit.write(corners, true);
                return;
            }
            once(
                diagnostics,
                format!(
                    "`{}` value is still unresolved after var()/env() substitution (v1)",
                    u.property_id.name()
                ),
            );
        }
        other => {
            once(
                diagnostics,
                format!("property not mapped (v1): {}", prop_name(other)),
            );
        }
    }
}

/// The attribute name of an SVG presentation property `Extras::
/// svg_presentation` carries (the same as the CSS property's).
fn presentation_name(p: &Property) -> Option<&'static str> {
    use Property::*;
    Some(match p {
        FillOpacity(_) => "fill-opacity",
        FillRule(_) => "fill-rule",
        StrokeOpacity(_) => "stroke-opacity",
        StrokeWidth(_) => "stroke-width",
        StrokeLinecap(_) => "stroke-linecap",
        StrokeLinejoin(_) => "stroke-linejoin",
        StrokeMiterlimit(_) => "stroke-miterlimit",
        StrokeDasharray(_) => "stroke-dasharray",
        StrokeDashoffset(_) => "stroke-dashoffset",
        ShapeRendering(_) => "shape-rendering",
        _ => return None,
    })
}

/// The CSS text of a presentation property as an SVG ATTRIBUTE spells it.
///
/// CSS keywords are case-insensitive and lightningcss prints them lowercase;
/// an attribute's are not, and resvg ignores `shape-rendering="crispedges"`
/// (the line is anti-aliased again). Every other carried keyword is lowercase
/// in SVG too.
fn presentation_value(name: &str, value: String) -> String {
    if name != "shape-rendering" {
        return value;
    }
    match value.to_ascii_lowercase().as_str() {
        "optimizespeed" => "optimizeSpeed".into(),
        "crispedges" => "crispEdges".into(),
        "geometricprecision" => "geometricPrecision".into(),
        _ => value,
    }
}

/// Later-wins, per name.
fn set_presentation(list: &mut Vec<(&'static str, Rc<str>)>, name: &'static str, value: Rc<str>) {
    match list.iter_mut().find(|(n, _)| *n == name) {
        Some(slot) => slot.1 = value,
        None => list.push((name, value)),
    }
}

/// One SVG paint declaration. See [`SvgPaint`].
fn svg_paint(
    p: &lightningcss::properties::svg::SVGPaint<'_>,
    what: &str,
    diagnostics: &mut Vec<String>,
) -> SvgPaint {
    use lightningcss::properties::svg::SVGPaint as P;
    match p {
        P::None => SvgPaint::None,
        P::Color(c) if is_current_color(c) => SvgPaint::Current,
        P::Color(c) => match color(c) {
            Some(c) => SvgPaint::Color(c),
            None => {
                unresolved_color(diagnostics, what);
                SvgPaint::Server
            }
        },
        // `context-fill`/`context-stroke` only mean anything inside a `<use>`
        // of a marker; treated as "painted", which is what they degrade to.
        P::Url { .. } | P::ContextFill | P::ContextStroke => SvgPaint::Server,
    }
}

fn is_current_color(c: &lightningcss::values::color::CssColor) -> bool {
    matches!(c, lightningcss::values::color::CssColor::CurrentColor)
}

/// `transform` — translate as a layout offset, rotate/scale onto the SVG
/// sprite matrix.
///
/// gpui applies transformation matrices to sprites (SVG paths), never to laid
/// out element trees, so there is no way to rotate, scale or skew a box. What
/// there IS a way to do is MOVE one, and translation is what the reused
/// component libraries actually need: shadcn centres its dialog with
/// `translate(-50%, -50%)`, slides its switch thumb with `translateX(1.25rem)`
/// and drives its progress bar with `translateX(-38%)`.
///
/// The move is applied as a layout offset by the renderer, so — unlike a real
/// transform — it participates in layout. For an absolutely positioned box
/// (every centring case) the two are identical; for an in-flow one the
/// following siblings shift too, which is what the advisory says.
///
/// Tailwind always emits the full identity chain
/// (`translate(…) rotate(0) skewX(0) skewY(0) scaleX(1) scaleY(1)`), so
/// identity components are recognised and pass silently — only a REAL rotate/
/// scale/skew is advised about.
fn apply_transform(
    list: &lightningcss::properties::transform::TransformList,
    r: &mut StyleRefinement,
    diagnostics: &mut Vec<String>,
) {
    use lightningcss::properties::transform::Transform as T;
    use lightningcss::values::percentage::NumberOrPercentage;

    fn len(v: &LengthPercentage) -> Option<TransformLength> {
        match v {
            DimensionPercentage::Dimension(l) => length_value_px(l).map(TransformLength::Pixels),
            DimensionPercentage::Percentage(p) => Some(TransformLength::Fraction(p.0)),
            DimensionPercentage::Calc(_) => {
                calc_dropped();
                None
            }
        }
    }
    fn is_one(v: &NumberOrPercentage) -> bool {
        match v {
            NumberOrPercentage::Number(n) => *n == 1.0,
            NumberOrPercentage::Percentage(p) => p.0 == 1.0,
        }
    }
    fn factor(v: &NumberOrPercentage) -> f32 {
        match v {
            NumberOrPercentage::Number(n) => *n,
            NumberOrPercentage::Percentage(p) => p.0,
        }
    }

    let zero = TransformLength::ZERO;
    let (mut tx, mut ty) = (zero, zero);
    let mut rotate = 0.0f32;
    let mut scale = (1.0f32, 1.0f32);
    let mut dropped: Vec<&str> = Vec::new();
    for t in &list.0 {
        match t {
            T::Translate(a, b) => match (len(a), len(b)) {
                (Some(a), Some(b)) => (tx, ty) = (a, b),
                _ => dropped.push("translate() with a calc() length"),
            },
            T::TranslateX(a) => match len(a) {
                Some(a) => tx = a,
                None => dropped.push("translateX() with a calc() length"),
            },
            T::TranslateY(b) => match len(b) {
                Some(b) => ty = b,
                None => dropped.push("translateY() with a calc() length"),
            },
            // `translate3d(x, y, 0)` is what @floating-ui writes for EVERY
            // positioned element (it opts into GPU compositing); the Z axis is
            // always 0 there, and a non-zero one has nothing to mean without a
            // perspective transform.
            T::Translate3d(a, b, z) => match (len(a), len(b)) {
                (Some(a), Some(b)) => {
                    (tx, ty) = (a, b);
                    if !matches!(css_length_px(z), Some(0.0)) {
                        dropped.push("the Z axis of a 3D translate");
                    }
                }
                _ => dropped.push("translate3d() with a calc() length"),
            },
            // Identity components (what Tailwind's transform stack is made of
            // when only a translate utility is applied) are genuinely nothing.
            T::Rotate(a) | T::RotateZ(a) | T::RotateX(a) | T::RotateY(a) if a.to_degrees() == 0.0 => {}
            T::SkewX(a) | T::SkewY(a) if a.to_degrees() == 0.0 => {}
            T::Skew(a, b) if a.to_degrees() == 0.0 && b.to_degrees() == 0.0 => {}
            T::ScaleX(s) | T::ScaleY(s) | T::ScaleZ(s) if is_one(s) => {}
            T::Scale(a, b) if is_one(a) && is_one(b) => {}
            // Z-axis rotation is the one rotate a 2D matrix can carry, and with
            // paint-level transforms it now reaches every element rather than
            // only an `<svg>` leaf. `rotateX`/`rotateY` need a 3D projection
            // and are dropped.
            T::Rotate(a) | T::RotateZ(a) => rotate += a.to_degrees(),
            T::RotateX(_) | T::RotateY(_) | T::Rotate3d(..) => dropped.push("rotate3d()/rotateX()/rotateY()"),
            T::Scale(a, b) => {
                scale = (scale.0 * factor(a), scale.1 * factor(b));
            }
            T::ScaleX(s) => scale.0 *= factor(s),
            T::ScaleY(s) => scale.1 *= factor(s),
            // `scale3d(s, s, s)` is what `tailwindcss-animate` writes for EVERY
            // `zoom-in-*` / `zoom-out-*` (it opts into GPU compositing the same
            // way `translate3d` does), so the X and Y factors are the whole
            // point — the same decomposition `translate3d` already got.
            //
            // The Z factor is dropped SILENTLY, and that is not a divergence:
            // a Z scale is only ever visible through a perspective projection,
            // `perspective()` / `perspective-property` are not supported (and
            // say so on their own line), and without one the rendering is
            // pixel-identical with Z ignored. Advising here would put a line in
            // every log that uses a zoom utility and tell the reader nothing.
            T::Scale3d(a, b, _z) => {
                scale = (scale.0 * factor(a), scale.1 * factor(b));
            }
            T::ScaleZ(_) => {}
            T::Skew(..) | T::SkewX(_) | T::SkewY(_) => dropped.push("skew()"),
            // `matrix(a, b, c, d, e, f)` DECOMPOSES: the CSS
            // translate/rotate/scale triple is recoverable from the 2x2 plus
            // the offset, and it is what @vueuse/motion and GSAP write when
            // they compose a transform themselves. Only the SKEW component has
            // nowhere to go, so only that is dropped.
            T::Matrix(m) => {
                let scale_x = (m.a * m.a + m.b * m.b).sqrt();
                let det = m.a * m.d - m.b * m.c;
                let scale_y = if scale_x == 0.0 { 0.0 } else { det / scale_x };
                let shear = if scale_x == 0.0 {
                    0.0
                } else {
                    (m.a * m.c + m.b * m.d) / (scale_x * scale_x)
                };
                if shear.abs() > 1e-4 {
                    dropped.push("the skew component of a matrix()");
                }
                tx = TransformLength::Pixels(m.e);
                ty = TransformLength::Pixels(m.f);
                rotate += m.b.atan2(m.a).to_degrees();
                scale = (scale.0 * scale_x, scale.1 * scale_y);
            }
            T::Matrix3d(_) => dropped.push("matrix3d()"),
            T::Perspective(_) => dropped.push("perspective()"),
            T::TranslateZ(_) => dropped.push("the Z axis of a 3D translate"),
        }
    }
    // Written UNCONDITIONALLY, including when it is the identity: `transform`
    // is a shorthand for the whole list, so a later declaration whose list
    // reduces to nothing has to cancel an earlier one. That is what
    // `Option<TransformSpec>` buys over a bare `TransformSpec` -- `None` is
    // "never declared", `Some(IDENTITY)` is `transform: none`.
    //
    // `transform-origin` is a SEPARATE property and must survive: a rule that
    // declares both in either order has to end up with both.
    let origin = r
        .vn_transform
        .map(|t| t.origin)
        .unwrap_or(TransformSpec::CENTRE_ORIGIN);
    r.vn_transform = Some(TransformSpec {
        translate: (tx, ty),
        rotate: rotate.to_radians(),
        scale,
        origin,
    });
    if !dropped.is_empty() {
        dropped.sort_unstable();
        dropped.dedup();
        once(
            diagnostics,
            format!(
                "`transform` supports translate(), rotate(), scale(), a decomposed matrix() and \
                 `transform-origin` — dropped {}. What is supported is applied at PAINT time, to \
                 this element and its whole subtree, and changes no layout, which is what a \
                 browser does",
                dropped.join(", ")
            ),
        );
    }
}

/// `backdrop-filter` → `StyleRefinement::backdrop_blur`.
///
/// The renderer implements exactly one filter function, `blur()`. The CSS
/// grammar allows a whole list, so the compat rule is: apply the blur if there
/// is one, advise about everything that did not survive, and never fail
/// silently.
///
/// `backdrop-filter: none` maps to a 0px radius rather than "undeclared" — a
/// refinement has no way to say "explicitly nothing", and 0px is what actually
/// cancels a blur inherited from an earlier rule in the cascade.
fn apply_backdrop_filter(
    list: &lightningcss::properties::effects::FilterList,
    r: &mut StyleRefinement,
    diagnostics: &mut Vec<String>,
) {
    use lightningcss::properties::effects::{Filter, FilterList};

    let filters = match list {
        FilterList::None => {
            r.backdrop_blur = Some(px(0.0));
            return;
        }
        FilterList::Filters(filters) => filters,
    };

    let mut blur_radius = None;
    let mut others = Vec::new();
    for filter in filters.iter() {
        match filter {
            Filter::Blur(length) => match css_length_px(length) {
                Some(radius) if blur_radius.is_none() => blur_radius = Some(radius.max(0.0)),
                Some(_) => others.push("a second blur()"),
                // `blur(calc(...))`, `blur(1vw)`, … — the length converter is
                // the same one every other property uses.
                None => others.push("blur() with a non-absolute length"),
            },
            Filter::Brightness(_) => others.push("brightness()"),
            Filter::Contrast(_) => others.push("contrast()"),
            Filter::Grayscale(_) => others.push("grayscale()"),
            Filter::HueRotate(_) => others.push("hue-rotate()"),
            Filter::Invert(_) => others.push("invert()"),
            Filter::Opacity(_) => others.push("opacity()"),
            Filter::Saturate(_) => others.push("saturate()"),
            Filter::Sepia(_) => others.push("sepia()"),
            Filter::DropShadow(_) => others.push("drop-shadow()"),
            Filter::Url(_) => others.push("url() SVG filters"),
        }
    }

    if let Some(radius) = blur_radius {
        r.backdrop_blur = Some(px(radius));
    }
    if !others.is_empty() {
        once(
            diagnostics,
            format!(
                "backdrop-filter: only blur() is implemented (v1) — dropped {}",
                others.join(", ")
            ),
        );
    }
}

/// Filter declarations share one cascade slot: a later declaration replaces
/// the blur, shadow and referenced color matrix together. Blur and shadow
/// have renderer-side shape restrictions; SVG matrices use an isolated layer.
///
/// `filter: none` records `Some(0.0)` rather than `None`, for the same reason
/// `backdrop-filter: none` maps to a 0px radius: "declared, and it is nothing"
/// has to beat an earlier rule in the cascade, which `None` ("undeclared")
/// would not.
fn apply_filter(
    list: &lightningcss::properties::effects::FilterList,
    x: &mut Extras,
    diagnostics: &mut Vec<String>,
) {
    use lightningcss::properties::effects::{Filter, FilterList};

    let filters = match list {
        FilterList::None => {
            x.blur = Some(0.0);
            x.drop_shadow = None;
            x.filter_url = None;
            return;
        }
        FilterList::Filters(filters) => filters,
    };

    let mut blur_radius = None;
    let mut drop_shadow = None;
    let mut filter_url = None;
    let mut others = Vec::new();
    for filter in filters.iter() {
        match filter {
            Filter::Blur(length) => match css_length_px(length) {
                Some(radius) if blur_radius.is_none() => blur_radius = Some(radius.max(0.0)),
                Some(_) => others.push("a second blur()"),
                None => others.push("blur() with a non-absolute length"),
            },
            Filter::Brightness(_) => others.push("brightness()"),
            Filter::Contrast(_) => others.push("contrast()"),
            Filter::Grayscale(_) => others.push("grayscale()"),
            Filter::HueRotate(_) => others.push("hue-rotate()"),
            Filter::Invert(_) => others.push("invert()"),
            Filter::Opacity(_) => others.push("opacity()"),
            Filter::Saturate(_) => others.push("saturate()"),
            Filter::Sepia(_) => others.push("sepia()"),
            Filter::DropShadow(shadow) => match drop_shadow_of(shadow) {
                Some(s) if drop_shadow.is_none() => drop_shadow = Some(s),
                Some(_) => others.push("a second drop-shadow()"),
                None => others.push("drop-shadow() with a non-absolute length"),
            },
            Filter::Url(url) if filters.len() == 1 && url.url.starts_with('#') => {
                filter_url = Some(Rc::from(url.url.as_ref()));
            }
            Filter::Url(_) => others.push("external or chained url() SVG filters"),
        }
    }

    x.blur = Some(blur_radius.unwrap_or(0.0));
    x.drop_shadow = drop_shadow;
    x.filter_url = filter_url;

    if !others.is_empty() {
        once(
            diagnostics,
            format!(
                "`filter`: supported functions are blur(), drop-shadow(), and a single \
                 same-document SVG feColorMatrix URL. Blur and drop-shadow require the \
                 painted shape described by the render-side advisory — dropped {}",
                others.join(", ")
            ),
        );
    }
}

/// One `drop-shadow()` argument list, in px. `None` = a length this engine
/// cannot flatten (`em` against an unknown font size, `calc()` with a
/// percentage), which is the same door every other length goes through.
fn drop_shadow_of(shadow: &lightningcss::properties::effects::DropShadow) -> Option<DropShadow> {
    Some(DropShadow {
        dx: css_length_px(&shadow.x_offset)?,
        dy: css_length_px(&shadow.y_offset)?,
        blur: css_length_px(&shadow.blur)?.max(0.0),
        // `None` here is `currentColor` — the CSS default for the function's
        // colour argument, and the one value `color()` cannot flatten.
        color: color(&shadow.color),
    })
}

/// gpui models exactly two border styles. `none`/`hidden` are NOT a third: per
/// CSS they compute `border-width` to 0, which is what we do here — that is
/// also what makes Tailwind's `border-none` render as no border.
fn set_border_style(r: &mut StyleRefinement, s: LineStyle, diagnostics: &mut Vec<String>) {
    if matches!(s, LineStyle::None | LineStyle::Hidden) {
        let zero: AbsoluteLength = px(0.0).into();
        r.border_widths.top = Some(zero);
        r.border_widths.right = Some(zero);
        r.border_widths.bottom = Some(zero);
        r.border_widths.left = Some(zero);
        return;
    }
    r.border_style = Some(match s {
        LineStyle::Dashed => gpui::BorderStyle::Dashed,
        LineStyle::Solid => gpui::BorderStyle::Solid,
        _ => {
            unsupported(
                diagnostics,
                "border-style (dotted/double/groove/ridge/inset/outset render as solid)",
            );
            gpui::BorderStyle::Solid
        }
    });
}

fn lp_definite_of_lpa(v: &LengthPercentageOrAuto) -> Option<DefiniteLength> {
    match v {
        LengthPercentageOrAuto::LengthPercentage(lp) => lp_definite(lp),
        LengthPercentageOrAuto::Auto => None,
    }
}

fn overflow(k: OverflowKeyword) -> Overflow {
    match k {
        OverflowKeyword::Hidden | OverflowKeyword::Clip => Overflow::Hidden,
        OverflowKeyword::Scroll | OverflowKeyword::Auto => Overflow::Scroll,
        OverflowKeyword::Visible => Overflow::Visible,
    }
}

/// The keyword an axis computes to so far: the refinement's `gpui::Overflow`,
/// qualified by the declared keyword where the refinement folded two into one.
/// The refinement decides, the keyword only refines, so a keyword a `:hover`
/// rule folded into the node's one [`Extras`] cannot turn the base style's
/// `hidden` into a `scroll`.
fn overflow_keyword(axis: Option<Overflow>, declared: Option<OverflowKeyword>) -> OverflowKeyword {
    use OverflowKeyword as K;
    match axis {
        None | Some(Overflow::Visible) => K::Visible,
        Some(Overflow::Clip) => K::Clip,
        Some(Overflow::Hidden) if declared == Some(K::Clip) => K::Clip,
        Some(Overflow::Hidden) => K::Hidden,
        Some(Overflow::Scroll) if declared == Some(K::Auto) => K::Auto,
        Some(Overflow::Scroll) => K::Scroll,
    }
}

/// `overflow-x` / `overflow-y` computed values (css-overflow-3 §3.1): "as
/// specified, except with `visible`/`clip` computing to `auto`/`hidden`
/// (respectively) if one of `overflow-x` or `overflow-y` is neither `visible`
/// nor `clip`". So `overflow-y: auto` alone makes the box a scroll container
/// on BOTH axes, as it is in a browser: content too wide for it scrolls
/// sideways instead of spilling out, and, as a flex item, its automatic
/// minimum width is 0 rather than its content's min-content width.
///
/// Run on the folded base style, after the whole cascade: one rule may
/// declare one axis and another rule the other. A `:hover`/`:focus` rung's own
/// overflow is not paired (v1), since the rung is a refinement over the base
/// that gpui applies at paint time.
pub(crate) fn compute_overflow(r: &mut StyleRefinement, x: &mut Extras) {
    use OverflowKeyword as K;
    let kx = overflow_keyword(r.overflow.x, x.overflow_x);
    let ky = overflow_keyword(r.overflow.y, x.overflow_y);
    let plain = |k: K| matches!(k, K::Visible | K::Clip);
    if plain(kx) == plain(ky) {
        return;
    }
    let computed = |k: K| match k {
        K::Visible => K::Auto,
        K::Clip => K::Hidden,
        k => k,
    };
    if plain(kx) {
        r.overflow.x = Some(overflow(computed(kx)));
        x.overflow_x = Some(computed(kx));
    } else {
        r.overflow.y = Some(overflow(computed(ky)));
        x.overflow_y = Some(computed(ky));
    }
}

impl Extras {
    /// `getComputedStyle`'s `overflow-x` and `overflow-y` for a node whose
    /// folded base style is `base`: `auto` and `clip` where they were declared
    /// (or computed, [`compute_overflow`]), not the `scroll` and `hidden`
    /// gpui folds them into.
    pub fn overflow_names(&self, base: &StyleRefinement) -> [&'static str; 2] {
        let name = |k: OverflowKeyword| match k {
            OverflowKeyword::Visible => "visible",
            OverflowKeyword::Hidden => "hidden",
            OverflowKeyword::Clip => "clip",
            OverflowKeyword::Scroll => "scroll",
            OverflowKeyword::Auto => "auto",
        };
        [
            name(overflow_keyword(base.overflow.x, self.overflow_x)),
            name(overflow_keyword(base.overflow.y, self.overflow_y)),
        ]
    }
}

fn border_width_px(w: &lightningcss::properties::border::BorderSideWidth) -> Option<f32> {
    use lightningcss::properties::border::BorderSideWidth as W;
    match w {
        W::Length(l) => css_length_px(l),
        W::Thin => Some(1.0),
        W::Medium => Some(3.0),
        W::Thick => Some(5.0),
    }
}

fn prop_name(p: &Property) -> String {
    let d = format!("{p:?}");
    d.split(['(', ' ', '{']).next().unwrap_or("?").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parses a declaration list the way a rule body would be parsed.
    fn map(body: &str) -> (StyleRefinement, Vec<String>) {
        let mut diagnostics = Vec::new();
        let r = parse_inline(body, &mut diagnostics);
        (r, diagnostics)
    }

    /// [`map`], plus the [`Extras`] the declarations produced.
    fn map_full(body: &str) -> (StyleRefinement, Extras, Vec<String>) {
        let mut diagnostics = Vec::new();
        let (refinement, _, extras) = parse_inline_full(body, &mut diagnostics);
        (refinement, extras, diagnostics)
    }

    fn clean(body: &str) -> StyleRefinement {
        let (r, d) = map(body);
        assert!(d.is_empty(), "{body:?} produced {d:?}");
        r
    }

    // ---- overflow longhands (Tailwind `overflow-x-*` / `overflow-y-*`) ----

    #[test]
    fn overflow_longhands_set_one_axis_each() {
        let r = clean("overflow-x: hidden");
        assert_eq!(r.overflow.x, Some(Overflow::Hidden));
        assert_eq!(r.overflow.y, None, "overflow-x must not touch the y axis");

        let r = clean("overflow-y: scroll");
        assert_eq!(r.overflow.y, Some(Overflow::Scroll));
        assert_eq!(r.overflow.x, None);

        // `auto` is Scroll (what makes the box scrollable), `visible` is neither.
        assert_eq!(clean("overflow-y: auto").overflow.y, Some(Overflow::Scroll));
        assert_eq!(
            clean("overflow-x: visible").overflow.x,
            Some(Overflow::Visible)
        );
    }

    #[test]
    fn a_longhand_after_the_shorthand_wins_on_its_axis_only() {
        let r = clean("overflow: hidden; overflow-y: scroll");
        assert_eq!(r.overflow.x, Some(Overflow::Hidden));
        assert_eq!(r.overflow.y, Some(Overflow::Scroll));
    }

    /// [`map_full`] and then the post-cascade pairing, which is what an
    /// element whose rules declared `body` ends up with.
    fn computed_overflow(body: &str) -> (StyleRefinement, [&'static str; 2]) {
        let (mut r, mut x, d) = map_full(body);
        assert!(d.is_empty(), "{body:?} produced {d:?}");
        compute_overflow(&mut r, &mut x);
        let names = x.overflow_names(&r);
        (r, names)
    }

    #[test]
    fn visible_beside_a_scrolling_axis_computes_to_auto() {
        // css-overflow-3 §3.1. Tailwind's `overflow-y-auto` alone is a scroll
        // container on both axes in a browser.
        let (r, names) = computed_overflow("overflow-y: auto");
        assert_eq!(names, ["auto", "auto"]);
        assert_eq!(r.overflow.x, Some(Overflow::Scroll), "x scrolls too, it is not just clipped");
        assert_eq!(r.overflow.y, Some(Overflow::Scroll));

        let (r, names) = computed_overflow("overflow-x: hidden");
        assert_eq!(names, ["hidden", "auto"]);
        assert_eq!(r.overflow.y, Some(Overflow::Scroll));

        assert_eq!(computed_overflow("overflow-x: scroll").1, ["scroll", "auto"]);
        assert_eq!(computed_overflow("overflow: auto").1, ["auto", "auto"]);
        assert_eq!(computed_overflow("overflow: hidden scroll").1, ["hidden", "scroll"]);
    }

    #[test]
    fn clip_beside_a_scrolling_axis_computes_to_hidden_and_beside_visible_stays() {
        assert_eq!(computed_overflow("overflow-x: clip; overflow-y: auto").1, ["hidden", "auto"]);
        // `clip` and `visible` are both "plain": nothing to pair, so neither
        // axis becomes a scroll container.
        let (r, names) = computed_overflow("overflow-x: clip");
        assert_eq!(names, ["clip", "visible"]);
        assert_eq!(r.overflow.y, None);
        assert_eq!(computed_overflow("overflow: clip").1, ["clip", "clip"]);
        assert_eq!(computed_overflow("color: red").1, ["visible", "visible"]);
    }

    #[test]
    fn the_refinement_decides_and_the_keyword_only_qualifies_it() {
        // A `:hover { overflow-y: auto }` folds its keyword into the node's one
        // `Extras` while its refinement goes to the hover rung: the base style
        // still has no overflow, and stays visible on both axes.
        let (mut base, mut x, _) = map_full("color: red");
        x.refine(&map_full("overflow-y: auto").1);
        compute_overflow(&mut base, &mut x);
        assert_eq!(x.overflow_names(&base), ["visible", "visible"]);
        assert_eq!(base.overflow.x, None);
    }

    // ---- gap longhands (Tailwind `gap-x-*` / `gap-y-*`) ------------------

    #[test]
    fn gap_longhands_map_to_the_matching_axis() {
        // CSS row-gap is the VERTICAL gap -> gpui's gap.height.
        let r = clean("row-gap: 4px; column-gap: 8px");
        assert_eq!(r.gap.height, Some(px(4.0).into()));
        assert_eq!(r.gap.width, Some(px(8.0).into()));
    }

    // ---- border width / style / radius longhands -------------------------

    #[test]
    fn border_width_shorthand_and_longhands() {
        let r = clean("border-width: 2px");
        let two: AbsoluteLength = px(2.0).into();
        assert_eq!(r.border_widths.top, Some(two));
        assert_eq!(r.border_widths.left, Some(two));

        let r = clean("border-top-width: 4px");
        assert_eq!(r.border_widths.top, Some(px(4.0).into()));
        assert_eq!(r.border_widths.bottom, None);
    }

    #[test]
    fn border_style_none_computes_the_widths_to_zero() {
        // Exactly what a browser does — and what makes `border-none` work.
        let r = clean("border: 3px solid red; border-style: none");
        let zero: AbsoluteLength = px(0.0).into();
        assert_eq!(r.border_widths.top, Some(zero));
        assert_eq!(r.border_widths.right, Some(zero));
        assert_eq!(r.border_widths.bottom, Some(zero));
        assert_eq!(r.border_widths.left, Some(zero));
    }

    #[test]
    fn border_style_dashed_and_an_advisory_for_the_rest() {
        assert_eq!(
            clean("border-style: dashed").border_style,
            Some(gpui::BorderStyle::Dashed)
        );
        let (r, d) = map("border-style: dotted");
        assert_eq!(r.border_style, Some(gpui::BorderStyle::Solid));
        assert!(d.iter().any(|m| m.contains("border-style")), "{d:?}");
    }

    #[test]
    fn corner_radius_longhands() {
        let r = clean("border-top-left-radius: 8px; border-bottom-right-radius: 2px");
        assert_eq!(r.corner_radii.top_left, Some(px(8.0).into()));
        assert_eq!(r.corner_radii.bottom_right, Some(px(2.0).into()));
        assert_eq!(r.corner_radii.top_right, None);
        // A length clears a percentage an earlier rule may have set.
        assert_eq!(r.vn_corner_radii.top_left, Some(None));
        assert_eq!(r.vn_corner_radii.top_right, None);
    }

    /// A percentage, a linear calc() and an elliptical corner go in the fork's
    /// `vn_corner_radii`, resolved against the box at paint.
    #[test]
    fn relative_and_elliptical_radii_resolve_at_paint() {
        let half = gpui::VnCornerRadius { x: relative(0.5), y: relative(0.5) };
        let r = clean("border-radius: 50%");
        for (length, relative) in [
            (r.corner_radii.top_left, r.vn_corner_radii.top_left),
            (r.corner_radii.top_right, r.vn_corner_radii.top_right),
            (r.corner_radii.bottom_right, r.vn_corner_radii.bottom_right),
            (r.corner_radii.bottom_left, r.vn_corner_radii.bottom_left),
        ] {
            assert_eq!(length, Some(px(0.).into()));
            assert_eq!(relative, Some(Some(half)));
        }

        let r = clean("border-top-left-radius: calc(50% - 2px)");
        let calc = DefiniteLength::Calc { fraction: 0.5, pixels: px(-2.) };
        assert_eq!(r.vn_corner_radii.top_left, Some(Some(gpui::VnCornerRadius { x: calc, y: calc })));

        let r = clean("border-radius: 10px / 20px");
        assert_eq!(
            r.vn_corner_radii.bottom_left,
            Some(Some(gpui::VnCornerRadius { x: px(10.).into(), y: px(20.).into() }))
        );

        // A circle of one length stays on gpui's own field.
        let r = clean("border-radius: 6px / 6px");
        assert_eq!(r.corner_radii.top_right, Some(px(6.).into()));
        assert_eq!(r.vn_corner_radii.top_right, Some(None));
    }

    // ---- flex shorthand / basis / wrap -----------------------------------

    #[test]
    fn flex_shorthand_fills_all_three_fields() {
        // Tailwind `flex-1`.
        let r = clean("flex: 1 1 0%");
        assert_eq!(r.flex_grow, Some(1.0));
        assert_eq!(r.flex_shrink, Some(1.0));
        assert_eq!(r.flex_basis, Some(Length::Definite(relative(0.0))));

        // Tailwind `flex-none`.
        let r = clean("flex: none");
        assert_eq!(r.flex_grow, Some(0.0));
        assert_eq!(r.flex_shrink, Some(0.0));
        assert_eq!(r.flex_basis, Some(Length::Auto));
    }

    #[test]
    fn flex_wrap_maps_all_three_keywords() {
        assert_eq!(clean("flex-wrap: wrap").flex_wrap, Some(gpui::FlexWrap::Wrap));
        assert_eq!(
            clean("flex-wrap: wrap-reverse").flex_wrap,
            Some(gpui::FlexWrap::WrapReverse)
        );
        assert_eq!(
            clean("flex-wrap: nowrap").flex_wrap,
            Some(gpui::FlexWrap::NoWrap)
        );
    }

    #[test]
    fn align_self_and_align_content() {
        assert_eq!(
            clean("align-self: center").align_self,
            Some(gpui::AlignItems::Center)
        );
        assert_eq!(
            clean("align-content: space-between").align_content,
            Some(gpui::AlignContent::SpaceBetween)
        );
        // `self-auto` cannot be expressed as a refinement — it must say so.
        let (r, d) = map("align-self: auto");
        assert_eq!(r.align_self, None);
        assert!(d.iter().any(|m| m.contains("align-self")), "{d:?}");
    }

    // ---- grid templates (Tailwind `grid-cols-N`) -------------------------

    #[test]
    fn grid_cols_maps_tailwinds_repeat_minmax() {
        let r = clean("grid-template-columns: repeat(3, minmax(0, 1fr))");
        assert_eq!(
            r.grid_cols,
            Some(gpui::GridTemplate {
                repeat: 3,
                min_size: gpui::GridTemplateMinSize::Zero,
            })
        );
        let r = clean("grid-template-rows: repeat(2, minmax(0, 1fr))");
        assert_eq!(r.grid_rows.map(|t| t.repeat), Some(2));
    }

    /// An ARBITRARY track list — the shape `grid-cols-[1fr_auto]` compiles to,
    /// and the one gpui's `repeat(N, …)` shorthand cannot express.
    #[test]
    fn an_arbitrary_track_list_becomes_an_explicit_track_list() {
        use gpui::{GridTrack, GridTrackSize as G};
        let r = clean("grid-template-columns: 200px 1fr");
        // The cheap `repeat` field stays untouched — the explicit list wins.
        assert_eq!(r.grid_cols, None);
        assert_eq!(
            r.vn_grid_cols,
            Some(vec![GridTrack::fixed(G::Px(200.0)), GridTrack::fr(1.0)])
        );
        // `1fr auto`, the meet app's `grid-cols-[1fr_auto]`.
        assert_eq!(
            clean("grid-template-columns: 1fr auto").vn_grid_cols,
            Some(vec![GridTrack::fr(1.0), GridTrack::fixed(G::Auto)])
        );
        // `minmax()` survives as itself, and an integer `repeat()` expands.
        assert_eq!(
            clean("grid-template-rows: repeat(2, minmax(min-content, 1fr)) 40px").vn_grid_rows,
            Some(vec![
                GridTrack { min: G::MinContent, max: G::Fr(1.0) },
                GridTrack { min: G::MinContent, max: G::Fr(1.0) },
                GridTrack::fixed(G::Px(40.0)),
            ])
        );
        // A percentage is a FRACTION on the taffy side.
        assert_eq!(
            clean("grid-template-columns: 25% 75%").vn_grid_cols,
            Some(vec![
                GridTrack::fixed(G::Percent(0.25)),
                GridTrack::fixed(G::Percent(0.75)),
            ])
        );
    }

    /// `repeat(auto-fill | auto-fit, …)` reaches taffy as a repetition that
    /// taffy counts against the container at layout time (m26 client sweep
    /// #15: Inventory and the emoji grid collapsed to one column).
    #[test]
    fn an_auto_repeat_is_counted_by_the_layout() {
        use gpui::{GridAutoRepeat, GridTrack, GridTrackSize as G};
        let r = clean("grid-template-columns: repeat(auto-fill, minmax(120px, 1fr))");
        assert_eq!(r.grid_cols, None);
        assert_eq!(r.vn_grid_cols, Some(vec![GridTrack { min: G::Px(120.0), max: G::Fr(1.0) }]));
        assert_eq!(r.vn_grid_cols_auto, Some(GridAutoRepeat { fit: false, start: 0, len: 1 }));

        // Fixed tracks around it; `auto-fit`; a two-track repetition.
        let r = clean("grid-template-rows: 40px repeat(auto-fit, 10px 20%) 1rem");
        assert_eq!(
            r.vn_grid_rows,
            Some(vec![
                GridTrack::fixed(G::Px(40.0)),
                GridTrack::fixed(G::Px(10.0)),
                GridTrack::fixed(G::Percent(0.2)),
                GridTrack::fixed(G::Px(16.0)),
            ])
        );
        assert_eq!(r.vn_grid_rows_auto, Some(GridAutoRepeat { fit: true, start: 1, len: 2 }));

        // A plain list says "no auto run" out loud, so an earlier rule's run
        // cannot survive it in the cascade.
        assert_eq!(
            clean("grid-template-columns: 1fr auto").vn_grid_cols_auto,
            Some(GridAutoRepeat { fit: false, start: 0, len: 0 })
        );
    }

    /// The track lists that genuinely have no model still advise rather than
    /// laying out wrongly: `fit-content()` has no `GridTrackSize`, and an auto
    /// repetition beside a flexible track is invalid CSS.
    #[test]
    fn an_unmodelable_track_list_is_an_advisory_not_a_wrong_grid() {
        for body in [
            "grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)) 1fr",
            "grid-template-columns: repeat(auto-fill, auto)",
            "grid-template-columns: fit-content(200px) 1fr",
        ] {
            let (r, d) = map(body);
            assert_eq!(r.grid_cols, None, "{body}");
            assert_eq!(r.vn_grid_cols, None, "{body}");
            assert!(d.iter().any(|m| m.contains("grid-template-columns")), "{body}: {d:?}");
        }
    }

    /// Placement: line numbers and `span N` reach taffy; a NAMED line does not.
    #[test]
    fn grid_placement_maps_lines_and_spans() {
        use gpui::GridPlacement as P;
        let r = clean("grid-column: 2 / 4");
        let loc = r.grid_location.clone().expect("a grid location");
        assert_eq!(loc.column, P::Line(2)..P::Line(4));
        let r = clean("grid-row: span 2");
        let loc = r.grid_location.clone().expect("a grid location");
        assert_eq!(loc.row.start, P::Span(2));
        let r = clean("grid-column-start: 3");
        assert_eq!(r.grid_location.unwrap().column.start, P::Line(3));
        let (_, d) = map("grid-column: sidebar / main");
        assert!(d.iter().any(|m| m.contains("NAMED line")), "{d:?}");
    }

    #[test]
    fn grid_auto_flow_maps_all_four_combinations() {
        for (body, want) in [
            ("grid-auto-flow: row", gpui::GridAutoFlow::Row),
            ("grid-auto-flow: column", gpui::GridAutoFlow::Column),
            ("grid-auto-flow: row dense", gpui::GridAutoFlow::RowDense),
            ("grid-auto-flow: column dense", gpui::GridAutoFlow::ColumnDense),
        ] {
            assert_eq!(clean(body).vn_grid_auto_flow, Some(want), "{body}");
        }
    }

    // ---- typography / visibility / aspect-ratio --------------------------

    #[test]
    fn typography_longhands() {
        assert_eq!(
            clean("font-style: italic").text.font_style,
            Some(gpui::FontStyle::Italic)
        );
        assert_eq!(
            clean("white-space: nowrap").text.white_space,
            Some(gpui::WhiteSpace::Nowrap)
        );
        assert!(clean("text-decoration-line: underline")
            .text
            .underline
            .is_some());
        assert!(clean("text-decoration-line: line-through")
            .text
            .strikethrough
            .is_some());
        assert!(matches!(
            clean("text-overflow: ellipsis").text.text_overflow,
            Some(gpui::TextOverflow::Truncate(_))
        ));
    }

    #[test]
    fn visibility_and_aspect_ratio() {
        assert_eq!(
            clean("visibility: hidden").visibility,
            Some(gpui::Visibility::Hidden)
        );
        assert_eq!(clean("aspect-ratio: 16 / 9").aspect_ratio, Some(16.0 / 9.0));
    }

    /// The DECLARED `visibility` rides beside the paint switch, which the
    /// screen-reader-only idioms also turn off: their box paints nothing, but
    /// its CSS `visibility` stays `visible`, and `el.focus()` asks that one.
    #[test]
    fn declared_visibility_is_not_the_paint_switch() {
        assert_eq!(extras("visibility: hidden").0.visibility_hidden, Some(true));
        assert_eq!(extras("visibility: collapse").0.visibility_hidden, Some(true));
        assert_eq!(extras("visibility: visible").0.visibility_hidden, Some(false));
        assert_eq!(extras("color: red").0.visibility_hidden, None);
        // `clip-path: inset(50%)`, the other idiom, is a real clip
        // (`clip_path_is_a_rounded_mask_and_the_other_shapes_advise`).
        for sr_only in ["clip: rect(0, 0, 0, 0)"] {
            let mut d = Vec::new();
            let inline = parse_inline_style(sr_only, &mut d);
            assert_eq!(inline.refinement.and_then(|r| r.visibility), Some(gpui::Visibility::Hidden), "{sr_only}");
            assert_eq!(inline.extras.visibility_hidden, None, "{sr_only}");
        }
        let mut d = Vec::new();
        let inline = parse_inline_style("clip-path: inset(50%)", &mut d);
        assert_eq!(inline.extras.visibility_hidden, None);
    }

    // ---- diagnostics are ONE-SHOT ----------------------------------------

    /// Custom properties are STORED, not advised about: they leave the mapper
    /// on their own channel and land in the element's [`Vars`] map.
    #[test]
    fn custom_properties_travel_their_own_channel() {
        let mut d = Vec::new();
        let inline = parse_inline_style(
            "--tw-shadow: 0 0 #0000; --tw-ring-color: red; --whatever: 3; color: blue",
            &mut d,
        );
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            inline.custom,
            vec![
                ("--tw-shadow".to_string(), "0 0 #0000".to_string()),
                ("--tw-ring-color".to_string(), "red".to_string()),
                ("--whatever".to_string(), "3".to_string()),
            ]
        );
        // …and the mapped declaration in the same body is unaffected.
        assert!(inline.refinement.is_some_and(|r| r.text.color.is_some()));
    }

    /// A `var()`-composed value is deferred, not dropped: it rides the dynamic
    /// channel as text and is substituted per element.
    #[test]
    fn a_var_composed_value_is_deferred_to_computed_value_time() {
        let mut d = Vec::new();
        let inline = parse_inline_style("box-shadow: var(--tw-shadow)", &mut d);
        assert!(d.is_empty(), "{d:?}");
        assert!(inline.refinement.is_none());
        assert_eq!(
            inline.dynamic,
            vec![("box-shadow".to_string(), "var(--tw-shadow)".to_string())]
        );
    }

    /// …and once the variable is known, the ordinary mapper arm runs.
    #[test]
    fn apply_dynamic_substitutes_then_maps() {
        let mut d = Vec::new();
        let vars = crate::vars::resolve(
            &Rc::new(Vars::default()),
            &[&("--tw-shadow".to_string(), "0 1px 2px 0 rgb(0 0 0 / 0.5)".to_string())],
            &crate::Env::default(),
            &mut d,
        );
        let mut r = StyleRefinement::default();
        let mut x = Extras::default();
        apply_dynamic("box-shadow", "var(--tw-shadow)", &vars, &crate::Env::default(), &mut r, &mut x, &mut d);
        assert!(d.is_empty(), "{d:?}");
        let shadows = r.box_shadow.expect("box-shadow mapped");
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].blur_radius, px(2.0));
    }

    /// An unresolvable reference drops the declaration — the CSS "invalid at
    /// computed-value time" rule — and does so SILENTLY, because that is
    /// exactly what a browser does with it. Tailwind ships the shape by design
    /// (`--tw-shadow-colored: … var(--tw-shadow-color)`, set only by a
    /// `shadow-<color>` utility), so an advisory here fires on every element
    /// that is behaving correctly.
    #[test]
    fn an_unresolvable_var_drops_the_declaration_silently() {
        let mut d = Vec::new();
        let mut r = StyleRefinement::default();
        let mut x = Extras::default();
        apply_dynamic("box-shadow", "var(--nope)", &Vars::default(), &crate::Env::default(), &mut r, &mut x, &mut d);
        assert_eq!(r.box_shadow, None);
        assert!(d.is_empty(), "{d:?}");
    }

    /// The OTHER three failures are author mistakes rather than a design, so
    /// they still speak.
    #[test]
    fn a_var_cycle_is_still_an_advisory() {
        let mut d = Vec::new();
        let mut r = StyleRefinement::default();
        let mut x = Extras::default();
        apply_dynamic(
            "width",
            "env(nonesuch)",
            &Vars::default(),
            &crate::Env::default(),
            &mut r,
            &mut x,
            &mut d,
        );
        assert!(
            d.iter().any(|m| m.contains("invalid at computed-value time")),
            "{d:?}"
        );
    }

    #[test]
    fn a_repeated_unmapped_property_is_reported_once() {
        let (_, d) = map("text-indent: 1px; text-indent: 2px; text-indent: 3px");
        assert_eq!(d.len(), 1, "{d:?}");
    }

    /// `currentColor` is the element's computed `color`, which is only known
    /// after the cascade — so the mapper RECORDS the target and
    /// `Engine::compute_node` substitutes. Nothing is advised, because nothing
    /// is missing.
    #[test]
    fn currentcolor_is_recorded_for_substitution_after_the_cascade() {
        let (r, x, d) = map_full("background-color: currentColor");
        assert_eq!(r.background, None, "not resolvable yet");
        assert!(x.current_color.has(CurrentColorTargets::BACKGROUND));
        assert!(d.is_empty(), "{d:?}");

        let (_, x, d) = map_full("border-color: currentColor");
        assert!(x.current_color.has(CurrentColorTargets::BORDER));
        assert!(d.is_empty(), "{d:?}");

        // `color: currentColor` resolves against itself, which IS `inherit`,
        // which is what writing nothing already means.
        let (r, x, d) = map_full("color: currentColor");
        assert_eq!(r.text.color, None);
        assert_eq!(x.current_color, CurrentColorTargets::default());
        assert!(d.is_empty(), "{d:?}");
    }

    // ---- calc() over absolute lengths ------------------------------------

    #[test]
    fn calc_sums_and_products_resolve_to_pixels() {
        // shadcn's own radius scale, once `--radius` resolves: `rounded-md` is
        // `calc(var(--radius) - 2px)`.
        assert_eq!(
            clean("border-radius: calc(0.5rem - 2px)").corner_radii.top_left,
            Some(px(6.0).into())
        );
        assert_eq!(clean("padding-top: calc(1rem + 4px)").padding.top, Some(px(20.0).into()));
        assert_eq!(clean("width: calc(2 * 8px)").size.width, Some(Length::Definite(px(16.0).into())));
        assert_eq!(
            clean("border-top-width: calc(4px + 1px)").border_widths.top,
            Some(px(5.0).into())
        );
        assert_eq!(
            clean("width: min(100px, 40px)").size.width,
            Some(Length::Definite(px(40.0).into()))
        );
        assert_eq!(
            clean("width: clamp(10px, 4px, 100px)").size.width,
            Some(Length::Definite(px(10.0).into()))
        );
    }

    /// A percentage inside a `calc()` needs the containing block, which does not
    /// exist before layout — so the sizing properties hand the LINEAR form to
    /// the layout (`DefiniteLength::Calc`), which resolves it as it resolves a
    /// percentage (m26 client sweep #12: reka's slider thumbs sat at 0).
    #[test]
    fn a_percentage_inside_calc_is_resolved_by_the_layout() {
        let calc = |fraction: f32, pixels: f32| {
            Some(Length::Definite(DefiniteLength::Calc { fraction, pixels: px(pixels) }))
        };
        assert_eq!(clean("left: calc(38.89% + 13.33px)").inset.left, calc(0.3889, 13.33));
        assert_eq!(clean("width: calc(100% - 8px)").size.width, calc(1.0, -8.0));
        // rem folds into the length half; products scale both halves.
        assert_eq!(clean("max-width: calc((100% - 1rem) * 0.5)").max_size.width, calc(0.5, -8.0));
        assert_eq!(clean("margin-left: calc(-50% + 2px)").margin.left, calc(-0.5, 2.0));
        assert_eq!(
            clean("padding-top: calc(10% - 1px)").padding.top,
            Some(DefiniteLength::Calc { fraction: 0.1, pixels: px(-1.0) })
        );
        // The percentages cancel: a plain length, not a calc.
        assert_eq!(
            clean("width: calc(50% - 50% + 3px)").size.width,
            Some(Length::Definite(px(3.0).into()))
        );
    }

    /// What the layout cannot take is dropped — and SAID, which it was not.
    #[test]
    fn a_calc_that_cannot_be_laid_out_is_advised() {
        let advised = |body: &str, property: &str| {
            let (_, d) = map(body);
            assert!(
                d.iter().any(|m| m.starts_with(&format!("`{property}`: this calc() is dropped"))),
                "{body:?} produced {d:?}"
            );
        };
        // Piecewise in the basis.
        advised("width: min(100%, 10px)", "width");
        advised("left: clamp(0px, 50%, 100px)", "left");
        // A property whose percentage is resolved before layout here.
        advised("background-position: calc(100% - 4px) 0", "background-position");
        // A radius resolves at paint, but only a linear calc().
        advised("border-radius: max(50%, 10px)", "border-radius");
        let (r, _) = map("width: min(100%, 10px)");
        assert_eq!(r.size.width, None);
    }

    // ---- positioning (compat roadmap G3) ---------------------------------

    fn extras(body: &str) -> (Extras, Vec<String>) {
        let mut d = Vec::new();
        let inline = parse_inline_style(body, &mut d);
        (inline.extras, d)
    }

    /// The computed `transform`, which lives on the REFINEMENT now — it is a
    /// paint property with `:hover`, transitions and `@keyframes`, not an
    /// `Extras` side channel (m13 item 2).
    fn transform(body: &str) -> (Option<TransformSpec>, Vec<String>) {
        let mut d = Vec::new();
        let inline = parse_inline_style(body, &mut d);
        (inline.refinement.and_then(|r| r.vn_transform), d)
    }

    #[test]
    fn absolute_and_inset_map_straight_through() {
        let r = clean("position: absolute; inset: 0; left: 50%; top: 8px");
        assert_eq!(r.position, Some(gpui::Position::Absolute));
        assert_eq!(r.inset.right, Some(Length::Definite(px(0.0).into())));
        assert_eq!(r.inset.left, Some(Length::Definite(relative(0.5))));
        assert_eq!(r.inset.top, Some(Length::Definite(px(8.0).into())));
    }

    #[test]
    fn css_wide_inset_resets_clear_offsets_and_keep_literal_source_order() {
        for keyword in ["initial", "unset"] {
            let r = clean(&format!(
                "inset: 9px; bottom: {keyword}; inset-inline-start: {keyword}"
            ));
            assert_eq!(r.inset.top, Some(px(9.0).into()));
            assert_eq!(r.inset.right, Some(px(9.0).into()));
            assert_eq!(r.inset.bottom, Some(Length::Auto));
            assert_eq!(r.inset.left, Some(Length::Auto));
            let r = clean(&format!("inset: {keyword}; bottom: 8px; inset-inline: 2px"));
            assert_eq!(r.inset.top, Some(Length::Auto));
            assert_eq!(r.inset.bottom, Some(px(8.0).into()));
            assert_eq!(r.inset.left, Some(px(2.0).into()));
            assert_eq!(r.inset.right, Some(px(2.0).into()));
        }
    }

    /// `appearance` records whether a `<select>` keeps its native skin, in the
    /// two spellings Chromium reads, and says nothing.
    #[test]
    fn appearance_none_takes_the_select_skin_away() {
        for (body, none) in [
            ("appearance: none", Some(true)),
            ("-webkit-appearance: none", Some(true)),
            // Gecko's prefix, which Chromium does not read either.
            ("-moz-appearance: none", None),
            ("appearance: auto", Some(false)),
            ("appearance: menulist-button", Some(false)),
            ("color: red", None),
        ] {
            let (x, d) = extras(body);
            assert_eq!(x.appearance_none, none, "{body}");
            assert!(d.is_empty(), "{body}: {d:?}");
        }
        let (mut x, _) = extras("appearance: none");
        x.refine(&extras("appearance: auto").0);
        assert_eq!(x.appearance_none, Some(false), "a later rule wins");
    }

    /// `fixed` records the flag and takes the deferred layer, and says NOTHING
    /// here: whether it diverges is a question about the element's parent
    /// (`vn_tree::render::advise_out_of_flow`), and the answer is "no" for
    /// every portalled overlay, which is what the idiom is.
    #[test]
    fn fixed_is_absolute_plus_a_deferred_layer_and_no_advisory() {
        let (x, d) = extras("position: fixed");
        assert_eq!(x.fixed, Some(true));
        assert!(x.may_defer());
        assert!(d.is_empty(), "{d:?}");
        // …and a later `relative` really undoes it.
        let (x, _) = extras("position: fixed; position: relative");
        assert_eq!(x.fixed, Some(false));
        assert!(!x.may_defer());
    }

    /// Same for `sticky`: the flag is recorded, the advisory belongs to the
    /// renderer, which is the only side that can see whether anything above
    /// the element scrolls.
    #[test]
    fn sticky_is_relative_and_records_the_flag() {
        let (x, d) = extras("position: sticky");
        assert_eq!(x.sticky, Some(true));
        assert!(d.is_empty(), "{d:?}");
        let (x, _) = extras("position: sticky; position: absolute");
        assert_eq!(x.sticky, Some(false));
    }

    /// m26 6d: `positioned` says whether a non-`static` position was DECLARED,
    /// which gpui's `position` cannot (static and relative are both its
    /// `Relative`). `z-index` applies only to a positioned box or a flex or grid
    /// item, so the renderer needs exactly this bit.
    #[test]
    fn positioned_records_a_declared_non_static_position() {
        for decl in ["position: relative", "position: absolute", "position: fixed", "position: sticky"] {
            assert_eq!(extras(decl).0.positioned, Some(true), "{decl}");
        }
        assert_eq!(extras("position: static").0.positioned, Some(false));
        assert_eq!(extras("z-index: 10").0.positioned, None, "no declaration");
        // Later wins, across declarations and across a refine.
        assert_eq!(extras("position: absolute; position: static").0.positioned, Some(false));
        let mut folded = extras("position: relative").0;
        folded.refine(&extras("position: static").0);
        assert_eq!(folded.positioned, Some(false));
        folded.refine(&extras("color: red").0);
        assert_eq!(folded.positioned, Some(false), "a block that says nothing keeps it");
    }

    #[test]
    fn a_non_zero_z_index_may_be_deferred() {
        let (x, d) = extras("z-index: 50");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(x.z(), Some(50));
        assert!(x.may_defer());

        // A negative one is a deferred draw too: it paints before its
        // context's in-flow content, wherever its box is.
        let (x, d) = extras("z-index: -1");
        assert_eq!(x.z(), Some(-1));
        assert!(x.may_defer());
        assert!(d.is_empty(), "{d:?}");

        // `0` makes a stacking context and paints in flow.
        assert!(!extras("z-index: 0").0.may_defer());
    }

    /// `auto` and `0` are different values: only an integer makes a stacking
    /// context. A later `auto` still undoes an earlier integer.
    #[test]
    fn z_index_auto_is_not_zero() {
        assert_eq!(extras("z-index: auto").0.z_index, Some(None));
        assert_eq!(extras("z-index: 0").0.z_index, Some(Some(0)));
        assert_eq!(extras("color: red").0.z_index, None);
        let mut folded = extras("z-index: 10").0;
        folded.refine(&extras("z-index: auto").0);
        assert_eq!(folded.z(), None);
        folded.refine(&extras("color: red").0);
        assert_eq!(folded.z_index, Some(None), "a block that says nothing keeps it");
    }

    #[test]
    fn isolation_maps_both_keywords() {
        let (x, d) = extras("isolation: isolate");
        assert_eq!(x.isolate, Some(true));
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(extras("isolation: auto").0.isolate, Some(false));
        let mut folded = extras("isolation: isolate").0;
        folded.refine(&extras("isolation: auto").0);
        assert_eq!(folded.isolate, Some(false));
    }

    /// `user-select` reaches the selection code (vn_tree::selection) as
    /// declared: the prefixed spelling is the same property, a declared
    /// `auto` overrides an earlier `none` in the fold, and only `contain`
    /// says anything.
    #[test]
    fn user_select_is_carried_to_the_selection() {
        assert_eq!(extras("user-select: none").0.user_select, Some(UserSelect::None));
        assert_eq!(extras("-webkit-user-select: none").0.user_select, Some(UserSelect::None));
        assert_eq!(extras("user-select: text").0.user_select, Some(UserSelect::Text));
        assert_eq!(extras("user-select: all").0.user_select, Some(UserSelect::All));
        assert_eq!(extras("user-select: auto").0.user_select, Some(UserSelect::Auto));
        assert_eq!(extras("color: red").0.user_select, None);
        let mut folded = extras("user-select: none").0;
        folded.refine(&extras("user-select: auto").0);
        assert_eq!(folded.user_select, Some(UserSelect::Auto));
        let (x, d) = extras("user-select: contain");
        assert_eq!(x.user_select, Some(UserSelect::Text));
        assert!(d.iter().any(|m| m.contains("user-select: contain")), "{d:?}");
        for quiet in ["user-select: none", "user-select: text", "user-select: auto", "user-select: all"] {
            assert!(extras(quiet).1.is_empty(), "{quiet}: {:?}", extras(quiet).1);
        }
    }

    #[test]
    fn pointer_events_maps_both_keywords() {
        assert_eq!(extras("pointer-events: none").0.pointer_events_none, Some(true));
        assert_eq!(extras("pointer-events: auto").0.pointer_events_none, Some(false));
        let (x, d) = extras("pointer-events: visiblePainted");
        assert_eq!(x.pointer_events_none, None);
        assert!(d.iter().any(|m| m.contains("pointer-events")), "{d:?}");
    }

    /// A percentage translate stays a percentage all the way to paint: it
    /// resolves against the element's OWN border box, which only the renderer
    /// knows. (It used to resolve against LAST frame's box, because translate
    /// was a margin offset and the mapper had no box at all.)
    #[test]
    fn translate_is_kept_in_its_authored_unit() {
        let (r, d) = transform("transform: translate(-50%, -50%)");
        assert!(d.is_empty(), "{d:?}");
        let t = r.expect("declared");
        assert_eq!(
            t.translate,
            (TransformLength::Fraction(-0.5), TransformLength::Fraction(-0.5))
        );
        assert_eq!(TransformLength::Fraction(-0.5).resolve(400.0), -200.0);

        let t = transform("transform: translateX(1.25rem)").0.expect("declared");
        assert_eq!(
            t.translate,
            (TransformLength::Pixels(20.0), TransformLength::Pixels(0.0))
        );
    }

    /// Tailwind's `-translate-x-1/2` emits the whole identity chain, and an
    /// identity component is genuinely nothing — no advisory, and a computed
    /// value that carries only the translate.
    #[test]
    fn tailwinds_identity_transform_chain_is_silent() {
        let (r, d) = transform(
            "transform: translate(-50%, -50%) rotate(0) skewX(0) skewY(0) scaleX(1) scaleY(1)",
        );
        assert!(d.is_empty(), "{d:?}");
        let t = r.expect("declared");
        assert_eq!(
            t.translate,
            (TransformLength::Fraction(-0.5), TransformLength::Fraction(-0.5))
        );
        assert_eq!(t.rotate, 0.0);
        assert_eq!(t.scale, (1.0, 1.0));
    }

    /// `transform` is a shorthand for the whole function list, so a
    /// declaration that reduces to the identity has to CANCEL an earlier one —
    /// which is why the computed value is `Some(IDENTITY)` and not `None`.
    #[test]
    fn transform_none_is_a_value_not_an_absence() {
        let (r, d) = transform("transform: rotate(0deg)");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(r, Some(TransformSpec::IDENTITY));
        assert!(r.unwrap().is_identity());
        // A declaration that was never written stays absent, which is what
        // lets an earlier rule's transform survive the cascade.
        assert_eq!(transform("color: red").0, None);
    }

    /// A real `rotate()`/`scale()` computes for EVERY element now — the
    /// "only an `<svg>` leaf can honour it" advisory is retired, because a
    /// laid out box carries a transformation matrix (m13 item 2).
    #[test]
    fn rotate_and_scale_reach_the_computed_value() {
        let (r, d) = transform("transform: translateY(4px) rotate(45deg) scale(1.5)");
        assert!(d.is_empty(), "{d:?}");
        let t = r.expect("declared");
        assert_eq!(
            t.translate,
            (TransformLength::Pixels(0.0), TransformLength::Pixels(4.0))
        );
        assert!((t.rotate - std::f32::consts::FRAC_PI_4).abs() < 1e-5, "{}", t.rotate);
        assert_eq!(t.scale, (1.5, 1.5));

        // `animate-spin`'s keyframe value, and Tailwind's `scale-x-*`.
        let spin = transform("transform: rotate(360deg)").0.expect("declared");
        assert!((spin.rotate - std::f32::consts::TAU).abs() < 1e-4, "{}", spin.rotate);
        assert_eq!(
            transform("transform: scaleX(-1)").0.expect("declared").scale,
            (-1.0, 1.0)
        );

        // A 3D rotation needs a projection nothing here has; still advised, and
        // the parts that DO work still land.
        let (r, d) = transform("transform: rotateY(30deg) skewX(10deg)");
        assert_eq!(r.expect("declared").rotate, 0.0);
        assert!(d.iter().any(|m| m.contains("skew()")), "{d:?}");
    }

    /// The transform-origin is the element's centre (CSS's initial value), so
    /// a pure rotation leaves the centre where it was and a pure scale grows
    // ---- gradients ------------------------------------------------------

    /// The whole gradient grammar renders. A background IMAGE layer is not the
    /// `background` fill: CSS paints the colour under the image stack, and
    /// gpui's `background` / `background_layers` split says the same thing, so
    /// the layers land in `Extras` for the renderer to fold.
    #[test]
    fn every_gradient_shape_becomes_a_background_layer() {
        let layers = |body: &str| {
            let (_, x, d) = map_full(body);
            assert!(d.is_empty(), "{body}: {d:?}");
            x.resolve_background_layers()
        };
        let one = |body: &str| {
            let list = layers(body);
            assert_eq!(list.len(), 1, "{body}");
            list[0]
        };

        // CSS and gpui share the angle convention (0deg = to top, clockwise),
        // so the keyword forms are the only conversion.
        let right = one("background-image: linear-gradient(to right, #ff0000, #0000ff)");
        let ninety = one("background: linear-gradient(90deg, #ff0000 0%, #0000ff 100%)");
        assert_eq!(format!("{right:?}"), format!("{ninety:?}"), "`to right` is 90deg");
        let down = one("background-image: linear-gradient(#fff, #000)");
        let explicit = one("background-image: linear-gradient(180deg, #fff, #000)");
        assert_eq!(format!("{down:?}"), format!("{explicit:?}"), "the default is `to bottom`");

        // Radial and conic are their own tags rather than a refusal.
        assert!(format!("{:?}", one("background-image: radial-gradient(#fff, #000)"))
            .starts_with("RadialGradient"));
        assert!(format!("{:?}", one("background-image: conic-gradient(#fff, #000)"))
            .starts_with("ConicGradient"));

        // More than two stops, and a `via-*` in the middle, all survive.
        let three = one("background-image: linear-gradient(#f00, #0f0, #00f)");
        assert_eq!(three.live_stops().len(), 3);

        // A `repeating-` form is the same ramp with the repeat flag.
        let repeating = one("background-image: repeating-linear-gradient(#f00 0px, #00f 10px)");
        assert_eq!(repeating.live_stops().len(), 2);
        assert_ne!(format!("{repeating:?}"), format!("{:?}", one(
            "background-image: linear-gradient(#f00 0px, #00f 10px)")));

        // Two layers stay two layers, and CSS's top-first order is reversed
        // into gpui's bottom-first paint order.
        let stack = layers(
            "background-image: linear-gradient(#f00, #f00), linear-gradient(#00f, #00f)",
        );
        assert_eq!(stack.len(), 2);
    }

    /// `background-size` / `-position` / `-repeat` arrive as SEPARATE
    /// properties — in Tailwind, as separate classes — so they are folded onto
    /// the layers after the cascade rather than while a declaration is mapped.
    /// This is the meet app's page grid, which is exactly that shape.
    #[test]
    fn background_size_and_position_tile_the_gradient() {
        let (_, x, d) = map_full(
            "background-image: linear-gradient(rgba(255,255,255,.02) 1px, transparent 1px),              linear-gradient(90deg, rgba(255,255,255,.02) 1px, transparent 1px);              background-size: 64px 64px",
        );
        assert!(d.is_empty(), "{d:?}");
        let layers = x.resolve_background_layers();
        assert_eq!(layers.len(), 2);
        for layer in &layers {
            assert!(layer.is_tiled(), "{layer:?}");
            // The hard stop is written as a LENGTH, which has no percentage
            // form; the shader divides it by the gradient line.
            assert_eq!(layer.live_stops()[0].percentage, 1.0);
        }
        // Bottom-first: the 90deg layer is written second in CSS and therefore
        // paints UNDER the first.
        assert!(format!("{:?}", layers[0]).contains("LinearGradient(90"));

        // Without a `background-size` there is no tile: the gradient covers the
        // box, which is what CSS says an `auto` size does for an image with no
        // intrinsic dimensions.
        let (_, x, _) = map_full("background-image: linear-gradient(#f00, #00f)");
        assert!(!x.resolve_background_layers()[0].is_tiled());
    }

    /// `background-image: url()` is carried to vn-tree with the geometry that
    /// places it (m26 client sweep #14: the emoji picker's sprites, a server
    /// banner) — and once an image is in the stack, the gradients are painted
    /// with it, in CSS order.
    #[test]
    fn a_url_background_is_carried_with_its_geometry() {
        let (r, x, d) = map_full(
            "background-image: url(atlas.webp); background-size: 6100% 600%; \
             background-position: 45% 20%; background-repeat: no-repeat",
        );
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(r.background, None);
        assert_eq!(
            x.resolve_background_stack(),
            Some(vec![PaintLayer::Image {
                url: Rc::from("atlas.webp"),
                size: BackgroundSize::Explicit(
                    Some(TileLength::Fraction(61.0)),
                    Some(TileLength::Fraction(6.0))
                ),
                position: (TileLength::Fraction(0.45), TileLength::Fraction(0.2)),
                repeat: (false, false),
            }])
        );

        // emojix's pixel form, and `cover` / `contain`.
        let (_, x, _) = map_full(
            "background-image: url(\"a.png\"); background-size: 1342px 268px; \
             background-position: -88px -44px",
        );
        let stack = x.resolve_background_stack().unwrap();
        let [PaintLayer::Image { size, position, repeat, .. }] = stack.as_slice() else {
            panic!("one image layer: {stack:?}");
        };
        assert_eq!(
            *size,
            BackgroundSize::Explicit(Some(TileLength::Pixels(1342.0)), Some(TileLength::Pixels(268.0)))
        );
        assert_eq!(*position, (TileLength::Pixels(-88.0), TileLength::Pixels(-44.0)));
        assert_eq!(*repeat, (true, true));
        let size_of = |body: &str| match map_full(body).1.resolve_background_stack().unwrap()[0] {
            PaintLayer::Image { size, .. } => size,
            ref other => panic!("{other:?}"),
        };
        assert_eq!(size_of("background: url(b.png) center / cover no-repeat"), BackgroundSize::Cover);
        assert_eq!(size_of("background-image: url(b.png); background-size: contain"), BackgroundSize::Contain);

        // The darkened-banner idiom: the gradient is written FIRST, so it
        // paints on top of the image — bottom-first, the image comes first.
        let (_, x, _) = map_full("background-image: linear-gradient(#0008, #0008), url(banner.png)");
        let stack = x.resolve_background_stack().unwrap();
        assert!(matches!(stack[0], PaintLayer::Image { .. }), "{stack:?}");
        assert!(matches!(stack[1], PaintLayer::Gradient(_)), "{stack:?}");

        // Gradients alone stay gpui's to paint.
        let (_, x, _) = map_full("background-image: linear-gradient(#f00, #00f)");
        assert_eq!(x.resolve_background_stack(), None);
        // `url("")` is an invalid image: no layer.
        assert_eq!(map_full("background-image: url(\"\")").1.resolve_background_stack(), None);
    }

    /// A later rule's gradient-only `background-image` replaces an earlier
    /// rule's `url()`: the two halves of one declaration cascade together.
    #[test]
    fn a_later_background_image_replaces_an_earlier_url() {
        let (_, mut x, _) = map_full("background-image: url(a.png)");
        let (_, later, _) = map_full("background-image: linear-gradient(#f00, #00f)");
        x.refine(&later);
        assert_eq!(x.resolve_background_stack(), None);
        assert_eq!(x.resolve_background_layers().len(), 1);
    }

    /// `image-set()` still has no painter, and says what does.
    #[test]
    fn an_image_set_names_the_alternative() {
        let (r, d) = map("background-image: image-set(\"a.png\" 1x, \"a-2x.png\" 2x)");
        assert_eq!(r.background, None);
        assert!(d.iter().any(|m| m.contains("image-set() is not painted")), "{d:?}");
    }


    // ---- m19: text spacing, decorations, features ------------------------

    /// `letter-spacing` and `word-spacing` reach the shaper now: gpui carries
    /// them on the `TextRun` and widens the already-shaped line, so there is no
    /// per-platform work and no advisory.
    #[test]
    fn letter_and_word_spacing_reach_the_text_style() {
        assert_eq!(clean("letter-spacing: 6.3px").text.letter_spacing, Some(px(6.3)));
        assert_eq!(clean("word-spacing: 2px").text.word_spacing, Some(px(2.0)));
        // `normal` is zero extra spacing, which is also the initial value.
        assert_eq!(clean("letter-spacing: normal").text.letter_spacing, Some(px(0.0)));
        // Compiled alone, with no element, `em` is the root's; the fold applies
        // it again under the element's own size (em_tests.rs).
        assert_eq!(
            clean("letter-spacing: 0.5em").text.letter_spacing,
            Some(px(0.5 * crate::rem_basis()))
        );
    }

    /// `text-decoration-line: none` has to UNDO an earlier `underline`, and a
    /// refinement cannot spell "clear" — so it declares a decoration that paints
    /// nothing instead. Zero thickness AND transparent, because gpui's stroke
    /// snapping would round a hairline up to a device pixel.
    #[test]
    fn no_underline_cancels_an_earlier_one() {
        let r = clean("text-decoration-line: underline; text-decoration-line: none");
        let u = r.text.underline.expect("declared, so it wins over the earlier one");
        assert_eq!(u.thickness, px(0.0));
        assert_eq!(u.color.map(|c| c.a), Some(0.0));
        let strike = r.text.strikethrough.expect("cleared too");
        assert_eq!(strike.thickness, px(0.0));
    }

    /// `text-underline-offset` moves the line down from where the font's own
    /// metrics put it. It and `text-decoration-line` are two properties — in
    /// Tailwind, `.underline` and `.underline-offset-4`, two classes — so the
    /// offset is also kept in `Extras` for the cascade to apply afterwards.
    #[test]
    fn underline_offset_lands_on_the_decoration() {
        let (r, x, d) = map_full("text-decoration-line: underline; text-underline-offset: 4px");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(r.text.underline.expect("declared").offset, px(4.0));
        assert_eq!(x.underline_offset, Some(4.0));

        // Declared the other way round, only the `Extras` half can carry it —
        // and that is the half `compute_node` applies.
        let (_, x, d) = map_full("text-underline-offset: 4px");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(x.underline_offset, Some(4.0));
    }

    /// `bolder` / `lighter` step relative to the inherited weight. vn-css folds
    /// no inherited text property, so the step is taken from the initial
    /// `normal` — the row every real use lands on (`b, strong { font-weight:
    /// bolder }` over 400 text).
    #[test]
    fn relative_font_weights_resolve_through_the_table() {
        assert_eq!(clean("font-weight: bolder").text.font_weight, Some(gpui::FontWeight(700.0)));
        assert_eq!(clean("font-weight: lighter").text.font_weight, Some(gpui::FontWeight(100.0)));
        assert_eq!(clean("font-weight: 500").text.font_weight, Some(gpui::FontWeight(500.0)));
    }

    /// OpenType FEATURES reach the shaper — gpui's `Font` carries a tag/value
    /// list DirectWrite installs over the run — while variation AXES do not,
    /// and only the second says so.
    #[test]
    fn font_features_reach_the_shaper_and_axes_do_not() {
        let features = clean("font-feature-settings: \"ss01\" 1, \"liga\" 0")
            .text
            .font_features
            .expect("declared");
        assert_eq!(&*features.0, &[("ss01".into(), 1u32), ("liga".into(), 0u32)]);

        // `font-variant-numeric`'s keywords ARE OpenType features, one to one.
        let numeric = clean("font-variant-numeric: tabular-nums")
            .text
            .font_features
            .expect("declared");
        assert_eq!(&*numeric.0, &[("tnum".into(), 1u32)]);

        let (_, d) = map("font-variation-settings: \"wdth\" 80");
        assert!(d.iter().any(|m| m.contains("font-variation-settings")), "{d:?}");
    }

    /// `font-kerning` and `font-variant-ligatures` are OpenType features too,
    /// and share the one `FontFeatures` field with the others — merged inside
    /// a rule, a later tag winning.
    #[test]
    fn kerning_and_ligature_keywords_become_features() {
        let features = |css: &str| {
            clean(css).text.font_features.map(|f| (*f.0).clone()).unwrap_or_default()
        };
        let pairs = |p: &[(&str, u32)]| -> Vec<(String, u32)> {
            p.iter().map(|(t, v)| (t.to_string(), *v)).collect()
        };
        assert_eq!(features("font-kerning: none"), pairs(&[("kern", 0)]));
        // The initial values write nothing — DirectWrite already kerns and
        // ligates by default, as a browser does.
        assert!(features("font-kerning: auto").is_empty());
        assert!(features("font-kerning: normal").is_empty());
        assert!(features("font-variant-ligatures: normal").is_empty());
        assert_eq!(
            features("font-variant-ligatures: none"),
            pairs(&[("liga", 0), ("clig", 0), ("dlig", 0), ("hlig", 0), ("calt", 0)])
        );
        assert_eq!(
            features("font-variant-ligatures: no-common-ligatures discretionary-ligatures"),
            pairs(&[("liga", 0), ("clig", 0), ("dlig", 1)])
        );
        assert_eq!(features("font-variant-ligatures: no-contextual"), pairs(&[("calt", 0)]));
        // One rule, three properties: merged, and the later `liga` wins.
        assert_eq!(
            features(
                "font-variant-numeric: tabular-nums; font-kerning: none; \
                 font-variant-ligatures: no-common-ligatures; font-feature-settings: \"liga\" 1"
            ),
            pairs(&[("tnum", 1), ("kern", 0), ("liga", 1), ("clig", 0)])
        );
        let (_, d) = map("font-kerning: sideways");
        assert!(d.iter().any(|m| m.contains("OpenType FEATURE")), "{d:?}");
    }

    /// `font-family` is CSS's "first available font": the first INSTALLED
    /// entry, with a generic standing for its platform default — and a missing
    /// FIRST choice recorded for the renderer's advisory.
    #[test]
    fn font_family_takes_the_first_installed_family() {
        let family = |css: &str| clean(css).text.font_family.map(|f| f.to_string());
        // No source (tools, tests): the first named family, as before.
        assert_eq!(family("font-family: Inter, sans-serif").as_deref(), Some("Inter"));

        set_font_source(|| vec!["Segoe UI".into(), "Consolas".into(), "Arial".into()]);
        assert_eq!(
            family("font-family: Inter, \"Segoe UI\", sans-serif").as_deref(),
            Some("Segoe UI")
        );
        // Tailwind's `font-mono`: the Apple faces are skipped, Consolas wins —
        // which is what Chrome on Windows does with the same list.
        assert_eq!(
            family(
                "font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \
                 \"Liberation Mono\", \"Courier New\", monospace"
            )
            .as_deref(),
            Some("Consolas")
        );
        // Tailwind's `font-sans`: `ui-sans-serif` is Safari-only and
        // `system-ui` is the platform UI font — NOT the emoji face the old
        // "first named family" rule landed on.
        assert_eq!(
            family(
                "font-family: ui-sans-serif, system-ui, sans-serif, \"Apple Color Emoji\", \
                 \"Segoe UI Emoji\""
            )
            .as_deref(),
            Some(".SystemUIFont")
        );
        assert_eq!(family("font-family: monospace").as_deref(), Some("Consolas"));
        assert_eq!(family("font-family: sans-serif").as_deref(), Some("Arial"));
        // No stand-in for `serif` is installed here: the UI font, not nothing.
        assert_eq!(family("font-family: serif").as_deref(), Some(".SystemUIFont"));
        // Nothing available: the first named family stays, and gpui's own
        // default applies as before.
        assert_eq!(family("font-family: Roboto, Helvetica").as_deref(), Some("Roboto"));
        // Only a missing FIRST entry is reported, once per family.
        let misses = take_font_misses();
        assert_eq!(
            misses,
            vec![
                ("Inter".to_string(), Some("Segoe UI".to_string())),
                ("Roboto".to_string(), None),
            ],
            "{misses:?}"
        );
        assert!(take_font_misses().is_empty());
    }

    /// Web families the host draws per character (`unicode-range` subsets)
    /// get the rest of their list as gpui `fallbacks`, so a character their
    /// faces do not cover reaches the next family; one that covers no Latin
    /// text leads only when it comes first.
    #[test]
    fn composed_web_families_hand_gpui_the_rest_of_the_list() {
        let font = |css: &str| {
            let t = clean(css).text;
            (
                t.font_family.map(|f| f.to_string()),
                t.font_fallbacks.map(|f| f.fallback_list().join("|")),
            )
        };
        set_font_source(|| vec!["Arial".into(), "Mix".into(), "Segoe UI".into()]);
        // Nothing composed: as before, no fallbacks at all.
        assert_eq!(font("font-family: Mix, Arial"), (Some("Mix".into()), None));

        set_composed_font_families([("Mix".to_string(), true), ("VN PUA".to_string(), false)]);
        let some = |f: &str, rest: &str| (Some(f.to_string()), Some(rest.to_string()));
        assert_eq!(font("font-family: Mix, \"Segoe UI\", sans-serif"), some("Mix", "Segoe UI|Arial"));
        // The Argon client's `"Alcarin Tengwar", Inter, …`; a family that is
        // neither installed nor composed is passed over.
        assert_eq!(font("font-family: \"VN PUA\", Nowhere, mix, sans-serif"), some("VN PUA", "mix|Arial"));
        assert_eq!(font("font-family: Nowhere, \"VN PUA\", Mix"), some("VN PUA", "Mix"));
        // An installed family first draws the text; the web families after
        // it are not reached, and say so.
        assert!(take_unreached_fonts().is_empty());
        assert_eq!(font("font-family: Arial, Mix, \"VN PUA\""), some("Arial", ""));
        assert_eq!(
            take_unreached_fonts(),
            vec![("Mix".to_string(), "Arial".to_string()), ("VN PUA".to_string(), "Arial".to_string())]
        );
        assert_eq!(font("font-family: Arial"), some("Arial", ""));
        assert!(take_unreached_fonts().is_empty());
        set_composed_font_families([]);
        assert_eq!(font("font-family: Mix"), (Some("Mix".into()), None));
    }

    /// The families with a web font face still to load are recorded for the
    /// host, which loads them for the text drawn with them; the text is drawn
    /// with the next family meanwhile.
    #[test]
    fn waiting_web_fonts_are_recorded_for_the_host() {
        let web = |css: &str| extras(css).0.web_font.map(|f| f.join("|"));
        set_font_source(|| vec!["Arial".into(), "VN Half".into()]);
        // No web font waiting: nothing recorded, nothing to clear.
        assert_eq!(web("font-family: \"VN Web\", Arial"), None);

        set_web_font_families(["VN Web".to_string(), "vn half".to_string(), "VN PUA".to_string()]);
        assert_eq!(web("font-family: \"VN Web\", Arial").as_deref(), Some("VN Web"));
        assert_eq!(clean("font-family: \"VN Web\", Arial").text.font_family.as_deref(), Some("Arial"));
        // A family not declared anywhere is passed over, as for drawing.
        assert_eq!(web("font-family: Nowhere, vn web, sans-serif").as_deref(), Some("vn web"));
        // Every waiting family before the one used: a face limited to some
        // characters leaves the rest of the text to the next one.
        assert_eq!(web("font-family: \"VN PUA\", \"VN Web\", serif").as_deref(), Some("VN PUA|VN Web"));
        // An installed family (or a generic) first: nothing to load, and the
        // empty list stops an ancestor's web fonts being inherited.
        assert_eq!(web("font-family: Arial, \"VN Web\"").as_deref(), Some(""));
        assert_eq!(web("font-family: sans-serif, \"VN Web\"").as_deref(), Some(""));
        // Installed with a face still waiting (another weight): still asked
        // for, and it ends the list.
        assert_eq!(web("font-family: \"VN Half\", \"VN Web\"").as_deref(), Some("VN Half"));
        // No `font-family` at all: inherited.
        assert_eq!(web("color: red"), None);
        set_web_font_families(Vec::new());
    }

    /// The screen-reader-only idiom is a box clipped to an EMPTY rectangle,
    /// which paints nothing — expressible as `visibility: hidden`, which keeps
    /// the layout box exactly as `clip` does.
    #[test]
    fn the_visually_hidden_clip_idiom_hides_the_box() {
        assert_eq!(
            clean("clip: rect(0, 0, 0, 0)").visibility,
            Some(gpui::Visibility::Hidden)
        );
        // The whitespace-separated spelling means the same thing.
        assert_eq!(
            clean("clip: rect(0 0 0 0)").visibility,
            Some(gpui::Visibility::Hidden)
        );
        // A rectangle with area is a real sub-rect clip, which is not.
        let (r, d) = map("clip: rect(0, 20px, 20px, 0)");
        assert_eq!(r.visibility, None);
        assert!(d.iter().any(|m| m.contains("legacy `clip")), "{d:?}");
    }


    /// `list-style-type` names a marker, and the predefined counter styles all
    /// produce one. The marker BOX is generated by `vn_tree::render` (it is a
    /// real child, so it lays out and inherits like one); the mapper's job is
    /// the value and the numbering.
    #[test]
    fn list_markers_map_to_their_counter_style() {
        use crate::ListMarker;
        let marker = |body: &str| map_full(body).1.list_marker;
        assert_eq!(marker("list-style-type: disc"), Some(ListMarker::Disc));
        assert_eq!(marker("list-style-type: circle"), Some(ListMarker::Circle));
        assert_eq!(marker("list-style-type: square"), Some(ListMarker::Square));
        assert_eq!(marker("list-style-type: decimal"), Some(ListMarker::Decimal));
        assert_eq!(marker("list-style-type: lower-roman"), Some(ListMarker::LowerRoman));
        // The shorthand carries the position with it.
        let (_, x, d) = map_full("list-style: square inside");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(x.list_marker, Some(ListMarker::Square));
        assert_eq!(x.list_marker_inside, Some(true));
        // `none` is an explicit "no marker", which is a different answer from
        // "this element declared nothing".
        assert_eq!(marker("list-style: none"), Some(ListMarker::None));
        assert_eq!(marker("color: red"), None);
    }

    /// The ordered styles are counters, and the numbering is the item's position
    /// among its element siblings — the same number `counter(list-item)` gives.
    #[test]
    fn ordered_markers_count_from_one() {
        use crate::ListMarker;
        assert_eq!(ListMarker::Disc.render(3).as_deref(), Some("\u{2022}"));
        assert_eq!(ListMarker::None.render(3), None);
        assert_eq!(ListMarker::Decimal.render(1).as_deref(), Some("1."));
        assert_eq!(ListMarker::DecimalLeadingZero.render(7).as_deref(), Some("07."));
        assert_eq!(ListMarker::LowerAlpha.render(1).as_deref(), Some("a."));
        assert_eq!(ListMarker::LowerAlpha.render(26).as_deref(), Some("z."));
        assert_eq!(ListMarker::LowerAlpha.render(27).as_deref(), Some("aa."));
        assert_eq!(ListMarker::UpperAlpha.render(28).as_deref(), Some("AB."));
        assert_eq!(ListMarker::LowerRoman.render(4).as_deref(), Some("iv."));
        assert_eq!(ListMarker::UpperRoman.render(1994).as_deref(), Some("MCMXCIV."));
    }

    // ---- outline --------------------------------------------------------

    /// `outline` and its longhands fold into ONE ring, which is what lets
    /// Tailwind's `outline-none` (`outline: 2px solid transparent` +
    /// `outline-offset: 2px`) and a real focus ring both land intact.
    #[test]
    fn outline_folds_the_shorthand_and_the_longhands_into_one_ring() {
        let r = clean("outline: 2px solid #ff0000; outline-offset: 3px");
        let o = r.vn_outline.expect("an outline");
        assert_eq!(o.width, px(2.0));
        assert_eq!(o.offset, px(3.0));
        assert_eq!(o.style, gpui::BorderStyle::Solid);
        assert!((o.color.a - 1.0).abs() < 1e-3, "{o:?}");

        // `outline: none` is a zero-width ring: nothing paints, and the value
        // still cancels an outline an earlier rule declared.
        let r = clean("outline: 2px solid #ff0000; outline: none");
        assert_eq!(r.vn_outline.unwrap().width, px(0.0));

        // Tailwind's `outline-none`.
        let r = clean("outline: 2px solid transparent; outline-offset: 2px");
        let o = r.vn_outline.unwrap();
        assert_eq!(o.width, px(2.0));
        assert_eq!(o.color.a, 0.0, "transparent paints nothing");

        // A longhand ALONE seeds a zero-width ring — CSS's initial
        // `outline-style: none` means a bare `outline-color` paints nothing.
        assert_eq!(clean("outline-color: #ff0000").vn_outline.unwrap().width, px(0.0));
        // …and a bare `outline-style` becomes visible at the initial `medium`.
        assert_eq!(clean("outline-style: solid").vn_outline.unwrap().width, px(3.0));
        assert_eq!(
            clean("outline-style: dashed").vn_outline.unwrap().style,
            gpui::BorderStyle::Dashed
        );
    }

    // ---- box-shadow -----------------------------------------------------

    /// Multiple shadows, spread, colour and `inset` all survive — which is
    /// what makes Tailwind's `--tw-ring-offset-shadow, --tw-ring-shadow`
    /// stack render as a focus ring.
    #[test]
    fn box_shadow_carries_every_shadow_with_spread_and_inset() {
        let r = clean(
            "box-shadow: 0 0 0 2px #ffffff, 0 0 0 4px #3b82f6, inset 0 1px 2px 0 rgb(0 0 0 / 0.4)",
        );
        let shadows = r.box_shadow.expect("shadows");
        assert_eq!(shadows.len(), 3);
        assert_eq!(shadows[0].spread_radius, px(2.0));
        assert_eq!(shadows[1].spread_radius, px(4.0));
        assert!(!shadows[0].inset && !shadows[1].inset);
        assert!(shadows[2].inset, "the third is an inset shadow");
        assert_eq!(shadows[2].blur_radius, px(2.0));
        assert_eq!(shadows[2].offset.y, px(1.0));
        // `box-shadow: none` — lightningcss parses it to an empty list, which
        // is written through as an empty `Vec` and therefore genuinely CLEARS
        // a shadow an earlier rule declared.
        // `box-shadow: none` arrives UNPARSED (lightningcss has no empty-list
        // value), and it has to CLEAR the list rather than advise — a
        // `shadow-none` utility overriding a `shadow-lg` one is the single most
        // common shadow declaration in a Tailwind sheet.
        let mut d = Vec::new();
        let mut r = StyleRefinement::default();
        r.box_shadow = Some(vec![gpui::BoxShadow {
            color: Hsla::default(),
            offset: gpui::point(px(0.0), px(1.0)),
            blur_radius: px(2.0),
            spread_radius: px(0.0),
            inset: false,
        }]);
        apply_dynamic("box-shadow", "none", &Vars::default(), &crate::Env::default(), &mut r, &mut Extras::default(), &mut d);
        assert_eq!(r.box_shadow, Some(Vec::new()), "{d:?}");
        assert!(d.is_empty(), "{d:?}");
    }

    // ---- content / generated boxes --------------------------------------

    /// `content` is the property that decides whether a `::before` box EXISTS,
    /// so it is parsed even though lightningcss has no typed representation.
    #[test]
    fn content_keeps_literal_strings_and_advises_the_rest() {
        let (x, d) = extras(r#"content: "->""#);
        assert_eq!(x.content.as_deref(), Some("->"));
        assert!(d.is_empty(), "{d:?}");
        // The decorative case: an empty string still generates a box.
        assert_eq!(extras(r#"content: """#).0.content.as_deref(), Some(""));
        // `none` / `normal` generate nothing, silently — that is the default.
        assert_eq!(extras("content: none").0.content, None);
        for body in ["content: counter(x)", "content: attr(data-x)", "content: url(a.png)"] {
            let (x, d) = extras(body);
            assert_eq!(x.content, None, "{body}");
            assert!(d.iter().any(|m| m.contains("does not generate a box")), "{body}: {d:?}");
        }
    }

    // ---- text-transform --------------------------------------------------

    #[test]
    fn text_transform_folds_onto_the_extras() {
        use crate::mapper::TextTransform as TT;
        assert_eq!(extras("text-transform: uppercase").0.text_transform, Some(TT::Uppercase));
        assert_eq!(extras("text-transform: lowercase").0.text_transform, Some(TT::Lowercase));
        assert_eq!(extras("text-transform: capitalize").0.text_transform, Some(TT::Capitalize));
        assert_eq!(extras("text-transform: none").0.text_transform, Some(TT::None));
        assert_eq!(TT::Uppercase.apply("Sign in").as_deref(), Some("SIGN IN"));
        assert_eq!(TT::Lowercase.apply("Sign In").as_deref(), Some("sign in"));
        // CSS capitalises the first letter of each word and leaves the rest.
        assert_eq!(TT::Capitalize.apply("sign iN now").as_deref(), Some("Sign IN Now"));
        assert_eq!(TT::None.apply("Sign in"), None);
    }

    // ---- filter: blur() / background-clip: text / the rem basis ----------

    #[test]
    fn referenced_color_filter_is_replaced_by_later_none_in_one_rule_or_cascade() {
        let (mut referenced, diagnostics) = extras("filter:url('#color-matrix')");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(referenced.filter_url.as_deref(), Some("#color-matrix"));
        referenced.refine(&extras("filter:none").0);
        assert!(referenced.filter_url.is_none());
        assert!(extras("filter:url('#color-matrix'); filter:none").0.filter_url.is_none());
        assert!(extras("filter:drop-shadow(1px 2px red); filter:none").0.drop_shadow.is_none());
    }

    #[test]
    fn a_lone_blur_is_recorded_and_the_other_filters_are_still_advised() {
        let (x, d) = extras("filter: blur(64px)");
        assert_eq!(x.blur, Some(64.0));
        assert!(d.is_empty(), "a lone blur() is implemented, not advised: {d:?}");

        // `filter: none` is a VALUE, not "undeclared" — it has to beat an
        // earlier rule's blur in the cascade.
        assert_eq!(extras("filter: none").0.blur, Some(0.0));

        // Tailwind emits the whole chain; every slot but `--tw-blur` substitutes
        // to nothing, so this is what actually reaches the mapper.
        assert_eq!(extras("filter: blur(8px)  ").0.blur, Some(8.0));

        let (x, d) = extras("filter: blur(4px) grayscale(1)");
        assert_eq!(x.blur, Some(4.0), "the blur still lands");
        assert!(
            d.iter().any(|m| m.contains("grayscale()")),
            "and the companion that cannot be drawn is named: {d:?}"
        );
    }

    #[test]
    fn nickname_six_color_and_thirteen_stop_travelling_gradients_keep_every_stop() {
        let palette = [0xff3355, 0xffaa00, 0x00ff44, 0x0088ff, 0x9933ff, 0xff55bb];
        for travelling in [false, true] {
            let colors: Vec<_> = if travelling { palette.iter().chain(palette.iter()).chain(palette[..1].iter()).copied().collect() }
                else { palette.to_vec() };
            let css = format!("background-image:linear-gradient(90deg,{});background-clip:text;color:transparent;background-size:200% 100%;background-position:-100% 0",
                colors.iter().map(|color| format!("#{color:06x}")).collect::<Vec<_>>().join(","));
            let (extras, diagnostics) = extras(&css);
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            let layers = extras.background_images.as_ref().expect("gradient brush stack");
            for (index, color) in colors.iter().enumerate() {
                let expected: Hsla = gpui::rgb(*color).into();
                let position = index as f32 / (colors.len() - 1) as f32;
                assert!(layers.iter().flat_map(|layer| layer.background.live_stops()).any(|stop|
                    stop.color == expected && (stop.percentage - position).abs() < 1e-5),
                    "stop {index}/{} must survive transparent-band packing", colors.len());
            }
            assert!(layers.iter().all(|layer| layer.background.live_stops().len() <= gpui::GRADIENT_STOPS));
            assert_eq!(extras.background_clip_text, Some(true));
            assert_eq!(extras.background_position.as_ref().unwrap()[0].0, TileLength::Fraction(-1.));
        }
    }

    #[test]
    fn background_clip_text_records_the_clip_and_background_metadata() {
        let (x, _) = extras(
            "background-image: linear-gradient(to right, #fbbf24, #eab308); \
             background-clip: text; color: transparent",
        );
        assert_eq!(x.background_clip_text, Some(true));
        // Representative metadata remains available, independently of the
        // actual multi-stop text brush. Both endpoints are saturated amber.
        let key = x.background_key_color.expect("a gradient contributes a key colour");
        assert!(key.s > 0.8 && key.l > 0.4 && key.l < 0.7, "{key:?}");
        assert!(key.h < 0.15, "an amber ramp must not average to a cold hue: {key:?}");

        // A solid background contributes itself.
        let (x, _) = extras("background-color: #ff0000; -webkit-background-clip: text");
        assert_eq!(x.background_clip_text, Some(true));
        assert_eq!(x.background_key_color.map(|c| c.h), Some(0.0));
    }

    #[test]
    fn rem_resolves_against_the_root_font_size() {
        assert_eq!(rem_basis(), INITIAL_REM_BASIS);
        assert_eq!(clean("width: 32rem").size.width, Some(Length::Definite(px(512.0).into())));

        assert!(set_rem_basis(14.0), "a new basis is a change");
        assert!(!set_rem_basis(14.0), "the same basis is not");
        assert_eq!(clean("width: 32rem").size.width, Some(Length::Definite(px(448.0).into())));

        // A basis that would silently rescale the whole app is refused, and
        // "the root has no font-size yet" is a normal state, not a change.
        assert!(!set_rem_basis(0.0));
        assert!(!set_rem_basis(f32::NAN));
        assert_eq!(rem_basis(), 14.0);

        set_rem_basis(INITIAL_REM_BASIS);
    }

    /// `ch` and `ex` are half an em (css-values-4's value for an unmeasured
    /// glyph). They used to be dropped, which made Argon's call timer digits
    /// (`width: 1ch`) 0 px wide. Compiled alone, with no element, the em is the
    /// root's; the fold applies them again under the element's own size
    /// (em_tests.rs).
    #[test]
    fn ch_and_ex_compile_to_half_an_em_of_the_root() {
        let w = |body: &str| clean(body).size.width;
        assert_eq!(w("width: 1ch"), Some(Length::Definite(px(8.0).into())));
        assert_eq!(w("width: 3ex"), Some(Length::Definite(px(24.0).into())));
        assert_eq!(w("width: 2rch"), Some(Length::Definite(px(16.0).into())));
        assert_eq!(w("width: 2rex"), Some(Length::Definite(px(16.0).into())));
        assert_eq!(w("width: calc(2ch + 1px)"), Some(Length::Definite(px(17.0).into())));

        assert!(set_rem_basis(14.0));
        assert_eq!(w("width: 1ch"), Some(Length::Definite(px(7.0).into())));
        set_rem_basis(INITIAL_REM_BASIS);
    }

    /// Tailwind's preflight gives `button`/`input`/`h1`-`h6` nine of these, and
    /// each used to print an advisory naming a property nothing is wrong with.
    #[test]
    fn the_css_wide_inherit_keyword_is_a_no_op_rather_than_an_advisory() {
        for body in [
            "letter-spacing: inherit",
            "font-weight: inherit",
            "color: inherit",
            "font-size: inherit",
            "line-height: inherit",
        ] {
            let (_, d) = map(body);
            assert!(d.is_empty(), "{body:?} should be silent, said {d:?}");
        }
    }

    /// `border-radius` does not inherit, so its `inherit` is not a no-op: it is
    /// recorded per corner, in cascade order, for the renderer to fill in.
    #[test]
    fn border_radius_inherit_is_recorded_per_corner() {
        // `inherit` has no typed value, so it travels the dynamic channel (the
        // one `var()` uses) and is mapped when the cascade applies it.
        let inherit = |name: &str| {
            let (mut r, mut x, mut d) = (StyleRefinement::default(), Extras::default(), Vec::new());
            apply_dynamic(name, "inherit", &Vars::default(), &crate::Env::default(), &mut r, &mut x, &mut d);
            assert!(d.is_empty(), "{name}: {d:?}");
            (r, x)
        };
        let (r, x) = inherit("border-radius");
        assert_eq!(x.radius_inherit, RadiusInherit { declared: 15, inherit: 15 });
        assert_eq!(r.corner_radii.top_left, None, "nothing to write until the parent is known");
        let (_, x) = inherit("border-bottom-left-radius");
        let bl = RadiusInherit::BOTTOM_LEFT;
        assert_eq!(x.radius_inherit, RadiusInherit { declared: bl, inherit: bl });
        // Not a radius: still the silent no-op it always was.
        assert_eq!(inherit("color").1.radius_inherit, RadiusInherit::default());

        // A length declares its corners as not inheriting…
        let (_, x, d) = map_full("border-radius: 4px; border-top-left-radius: 50%");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(x.radius_inherit, RadiusInherit { declared: 15, inherit: 0 });
        // …so across rules the later one wins each corner it declares.
        let mut folded = inherit("border-radius").1;
        folded.refine(&map_full("border-top-right-radius: 2px").1);
        assert_eq!(folded.radius_inherit.inherit, RadiusInherit::ALL & !RadiusInherit::TOP_RIGHT);
        folded.refine(&inherit("border-top-right-radius").1);
        assert_eq!(folded.radius_inherit.inherit, RadiusInherit::ALL);
    }

    // ---- line-clamp ------------------------------------------------------

    #[test]
    fn webkit_line_clamp_reaches_the_shaper() {
        assert_eq!(clean("-webkit-line-clamp: 3").text.line_clamp, Some(3));
        assert_eq!(clean("line-clamp: 2").text.line_clamp, Some(2));
    }

    // ---- the advisory sweep ---------------------------------------------

    /// Every property here USED to print the bare "property not mapped (v1)".
    /// The point of the change is that each now says why, and names what to
    /// write instead where there is something.
    ///
    /// The list is what is left after m19: an advisory has to name a REAL
    /// divergence, so a property whose every value produces the same box here
    /// as it does in a browser dropped off it — see the sweep below.
    #[test]
    fn the_unmapped_properties_say_why_rather_than_that() {
        for (body, needle) in [
            ("vertical-align: middle", "vertical-align"),
            ("text-indent: 2em", "text-indent"),
            ("list-style-image: url(dot.png)", "this list marker is not generated"),
            ("resize: both", "resize"),
            ("user-select: contain", "user-select"),
            // `blur()` alone is recorded rather than advised (see `apply_filter`),
            // so the advisory this sweep is about needs a filter that is not it.
            ("filter: grayscale(1)", "supported functions are blur(), drop-shadow()"),
            ("border-collapse: collapse", "table properties"),
            ("clip: rect(0,0,10px,10px)", "legacy `clip"),
            ("font-variation-settings: \"wdth\" 80", "font-variation-settings"),
        ] {
            let (_, d) = map(body);
            assert!(
                d.iter().any(|m| m.contains(needle)),
                "{body} should advise about {needle}: {d:?}",
            );
            assert!(
                !d.iter().any(|m| m.starts_with("property not mapped")),
                "{body} still hits the generic catch-all: {d:?}",
            );
        }
    }

    /// The other half of the same rule: a property that produces the SAME box
    /// under every value it can take says nothing at all.
    ///
    /// Each line here was an advisory before m19, and each was noise. A
    /// generated Tailwind sheet ships a utility for every property in CSS, so
    /// compiling one reported `resize`, `tab-size` and `table-layout` on apps
    /// where no element used any of them — and several of the messages named
    /// properties that could not have diverged under any value.
    #[test]
    fn a_property_with_no_divergence_says_nothing() {
        for body in [
            // Page text is selectable, and these are applied as written.
            "user-select: none",
            "user-select: text",
            "user-select: auto",
            "user-select: all",
            // No element carries a native control skin under any value.
            "appearance: none",
            "appearance: textfield",
            // Whitespace is always collapsed, so a tab is one space at every
            // tab-size; `white-space` is where that divergence lives.
            "tab-size: 4",
            // Nothing inflates text here, and nothing rubber-bands or
            // highlights a tap.
            "text-size-adjust: 100%",
            "touch-action: none",
            "-webkit-font-smoothing: antialiased",
            // `-webkit-line-clamp` is honoured on its own; the legacy flexbox
            // switches around it are inert in browsers too.
            "-webkit-box-orient: vertical",
            // The initial values.
            "vertical-align: baseline",
            "text-indent: 0",
            "resize: none",
            "overflow-wrap: normal",
            "word-break: normal",
            // Applied by the line breaker (gpui fork, `TextWrap`).
            "overflow-wrap: break-word",
            "overflow-wrap: anywhere",
            "word-wrap: break-word",
            "word-break: break-all",
            "word-break: keep-all",
            "word-break: break-word",
            "hyphens: manual",
            "hyphens: none",
            // `list-style: none` asks for no marker, which is what a tree with
            // no marker boxes already produces.
            // Every predefined counter style generates a real marker box now,
            // and `none` asks for the nothing a marker-less tree already has.
            "list-style: none",
            "list-style-type: none",
            "list-style-type: disc",
            "list-style: decimal inside",
            "list-style-position: inside",
            // Only ever read through a decoration LINE, which this has none of
            // — a browser paints nothing here either.
            "text-decoration-color: #f00",
            // `collapse` IS `hidden` outside tables, and there are no tables.
            "visibility: collapse",
            // These reach the shaper for real now.
            "letter-spacing: 0.05em",
            "word-spacing: 2px",
            "font-variant-numeric: tabular-nums",
            "font-feature-settings: \"ss01\" 1, \"liga\" 0",
            "font-feature-settings: normal",
            "font-kerning: none",
            "font-kerning: auto",
            "font-variant-ligatures: no-common-ligatures",
            "font-variant-ligatures: normal",
            "font-variation-settings: normal",
            // `auto` IS what the fork's DirectWrite path now does.
            "font-optical-sizing: auto",
            "text-underline-offset: 4px",
            "text-decoration-skip-ink: none",
            // The visually-hidden idiom clips the box away, which is
            // expressible as `visibility: hidden`.
            "clip: rect(0, 0, 0, 0)",
            // They only move where `scrollIntoView` and scroll snapping land,
            // and neither is modelled at all.
            "scroll-margin-top: 4rem",
            "scroll-padding: 1rem",
            // A gradient has no intrinsic size, so `cover` and `contain` both
            // resolve to the box — which is what an untiled background is.
            "background-size: cover",
            "background-image: linear-gradient(#f00, #00f); background-size: 64px 64px",
            "background-repeat: no-repeat",
            "background-position: center",
            // Every gradient shape renders now.
            "background-image: radial-gradient(circle at 30% 40%, #fff, #000)",
            "background-image: conic-gradient(from 45deg, #f00, #0f0, #00f)",
            "background-image: repeating-linear-gradient(45deg, #f00 0 10px, #00f 10px 20px)",
            "background-image: linear-gradient(to bottom right, #f00, #0f0, #00f, #ff0)",
        ] {
            let (_, d) = map(body);
            assert!(d.is_empty(), "{body:?} should be silent, said {d:?}");
        }
    }

    #[test]
    fn per_side_border_colours_are_independent_and_silent() {
        let (r, diagnostics) = map("border-top: 1px solid #ff0000; border-left-color: #00ff00");
        assert_eq!(r.border_widths.top, Some(px(1.0).into()));
        assert_eq!(r.border_widths.bottom, None);
        assert_eq!(r.vn_border_colors.top, Some(Some(gpui::rgb(0xff0000).into())));
        assert_eq!(r.vn_border_colors.left, Some(Some(gpui::rgb(0x00ff00).into())));
        assert_eq!(r.vn_border_colors.right, None);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn border_color_shorthand_retains_four_colours_and_resets_side_overrides() {
        let (r, diagnostics) = map("border-color: red green blue transparent");
        assert_eq!(r.vn_border_colors.top, Some(Some(gpui::rgb(0xff0000).into())));
        assert_eq!(r.vn_border_colors.right, Some(Some(gpui::rgb(0x008000).into())));
        assert_eq!(r.vn_border_colors.bottom, Some(Some(gpui::rgb(0x0000ff).into())));
        assert!(r.vn_border_colors.left.flatten().is_some_and(|c| c.is_transparent()));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let (r, diagnostics) = map("border-left-color: blue; border-color: red");
        assert_eq!(r.vn_border_colors.left, Some(Some(gpui::rgb(0xff0000).into())));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn scrollbar_color_accepts_functional_colours() {
        for body in ["scrollbar-color: hsl(240 5% 64% / 0.35) transparent",
            "scrollbar-color: rgba(128, 24, 255, 0.5) #0000",
            "scrollbar-color: hsl(0 0% 10%) transparent"] {
            let (_, extras, diagnostics) = map_full(body);
            assert!(extras.scrollbar_color.is_some(), "{body}: {diagnostics:?}");
            assert!(diagnostics.is_empty(), "{body}: {diagnostics:?}");
        }
    }

    // ---- transform-origin -----------------------------------------------

    /// `transform-origin` on its own changes nothing (as in CSS) but must not
    /// be lost when `transform` is declared beside it, in either order.
    #[test]
    fn transform_origin_survives_beside_the_transform() {
        let spec = transform("transform-origin: left top; transform: rotate(45deg)")
            .0
            .expect("declared");
        assert_eq!(
            spec.origin,
            (TransformLength::Fraction(0.0), TransformLength::Fraction(0.0))
        );
        // …and the other way round.
        let spec = transform("transform: rotate(45deg); transform-origin: 100% 0")
            .0
            .expect("declared");
        assert_eq!(
            spec.origin,
            (TransformLength::Fraction(1.0), TransformLength::Pixels(0.0))
        );
        // The initial value is the box centre.
        assert_eq!(
            transform("transform: scale(2)").0.unwrap().origin,
            TransformSpec::CENTRE_ORIGIN
        );
        // Declared ALONE it is still the identity — nothing to apply.
        let alone = transform("transform-origin: left top").0.expect("declared");
        assert!(alone.is_identity(), "an origin alone transforms nothing");
    }

    /// The origin is what rotate/scale turn about, so a corner origin pins
    /// that corner.
    #[test]
    fn a_corner_transform_origin_pins_that_corner() {
        use gpui::{Bounds, Point, Size};
        let bounds = Bounds {
            origin: Point { x: px(100.0), y: px(50.0) },
            size: Size { width: px(200.0), height: px(100.0) },
        };
        let spec = transform("transform-origin: left top; transform: scale(2)")
            .0
            .expect("declared");
        let m = spec.to_matrix(bounds, 1.0);
        let pinned = m.apply(bounds.origin);
        assert!((f32::from(pinned.x) - 100.0).abs() < 1e-3, "{pinned:?}");
        assert!((f32::from(pinned.y) - 50.0).abs() < 1e-3, "{pinned:?}");
        // …and doubles the far corner's distance from it.
        let far = m.apply(Point { x: px(300.0), y: px(150.0) });
        assert!((f32::from(far.x) - 500.0).abs() < 1e-3, "{far:?}");
        assert!((f32::from(far.y) - 250.0).abs() < 1e-3, "{far:?}");
    }

    /// `matrix(a,b,c,d,e,f)` decomposes into the translate/rotate/scale triple
    /// gpui carries — which is what an animation library that composes its own
    /// transform emits.
    #[test]
    fn a_matrix_decomposes_into_translate_rotate_scale() {
        // scale(2) then translate(10, 20): matrix(2, 0, 0, 2, 10, 20)
        let spec = transform("transform: matrix(2, 0, 0, 2, 10, 20)").0.expect("declared");
        assert_eq!(spec.translate, (TransformLength::Pixels(10.0), TransformLength::Pixels(20.0)));
        assert!((spec.scale.0 - 2.0).abs() < 1e-4, "{spec:?}");
        assert!((spec.scale.1 - 2.0).abs() < 1e-4, "{spec:?}");
        assert!(spec.rotate.abs() < 1e-4, "{spec:?}");
        // A quarter turn: matrix(0, 1, -1, 0, 0, 0)
        let spec = transform("transform: matrix(0, 1, -1, 0, 0, 0)").0.expect("declared");
        assert!(
            (spec.rotate - std::f32::consts::FRAC_PI_2).abs() < 1e-4,
            "{spec:?}"
        );
        assert!((spec.scale.0 - 1.0).abs() < 1e-4 && (spec.scale.1 - 1.0).abs() < 1e-4);
        // Only the SKEW half is still dropped, and it says so.
        let (_, d) = map("transform: matrix(1, 0, 0.5, 1, 0, 0)");
        assert!(d.iter().any(|m| m.contains("skew component")), "{d:?}");
    }

    /// about it.
    #[test]
    fn the_matrix_turns_about_the_box_centre() {
        use gpui::{Bounds, Point, Size};
        let bounds = Bounds {
            origin: Point { x: px(100.0), y: px(50.0) },
            size: Size { width: px(200.0), height: px(100.0) },
        };
        let quarter = transform("transform: rotate(90deg)").0.expect("declared");
        let m = quarter.to_matrix(bounds, 1.0);
        let centre = m.apply(bounds.center());
        assert!((f32::from(centre.x) - 200.0).abs() < 1e-3, "{centre:?}");
        assert!((f32::from(centre.y) - 100.0).abs() < 1e-3, "{centre:?}");
        // (-100, -50) from the centre becomes (+50, -100): a quarter turn
        // clockwise on screen, where y grows downward.
        let corner = m.apply(bounds.origin);
        assert!((f32::from(corner.x) - 250.0).abs() < 1e-3, "{corner:?}");
        assert!(f32::from(corner.y).abs() < 1e-3, "{corner:?}");

        // A percentage translate resolves against the element's own box.
        let half = transform("transform: translate(-50%, -50%)").0.expect("declared");
        let m = half.to_matrix(bounds, 1.0);
        let moved = m.apply(bounds.origin);
        assert!((f32::from(moved.x) - 0.0).abs() < 1e-3, "{moved:?}");
        assert!((f32::from(moved.y) - 0.0).abs() < 1e-3, "{moved:?}");
    }

    #[test]
    fn svg_paint_keeps_the_concrete_colour() {
        assert_eq!(extras("fill: none").0.fill, Some(SvgPaint::None));
        assert_eq!(extras("fill: currentColor").0.fill, Some(SvgPaint::Current));
        // A concrete colour is carried, alpha and all, and spelled for resvg.
        let red = extras("fill: #ff0000").0.fill;
        assert!(matches!(red, Some(SvgPaint::Color(_))), "{red:?}");
        assert_eq!(red.unwrap().as_attr(), "rgb(255,0,0)");
        let faint = extras("stroke: rgba(219, 232, 255, 0.17)").0.stroke.expect("declared");
        assert_eq!(faint.as_attr(), "rgba(219,232,255,0.169)");
        assert_eq!(extras("stroke: currentColor").0.stroke, Some(SvgPaint::Current));
        assert_eq!(SvgPaint::None.as_attr(), "none");
        // A paint server is drawn in currentColor (vn-tree advises).
        assert_eq!(extras("fill: url(#g)").0.fill, Some(SvgPaint::Server));
        assert_eq!(SvgPaint::Server.as_attr(), "currentColor");
        // Nothing declared stays None, which is what lets a presentation
        // attribute in the markup survive.
        assert_eq!(extras("color: red").0.fill, None);
    }

    #[test]
    fn svg_presentation_properties_travel_as_text_and_merge_per_name() {
        // Argon's broadcast connector.
        let (x, d) = extras(
            "fill: none; stroke: currentColor; stroke-opacity: 0.28; stroke-width: 1; \
             shape-rendering: crispEdges",
        );
        assert!(d.is_empty(), "{d:?}");
        let list = x.svg_presentation.clone().expect("carried");
        let get = |l: &[(&'static str, Rc<str>)], n: &str| l.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string());
        assert_eq!(get(&list, "stroke-opacity").as_deref(), Some(".28")); // lightningcss's minified text, a valid SVG <number>
        assert_eq!(get(&list, "stroke-width").as_deref(), Some("1px")); // a <length>, which an SVG attribute takes too
        assert_eq!(get(&list, "shape-rendering").as_deref(), Some("crispEdges"));
        // vue-flow's edges: the dash pair, which used to be "not mapped".
        let (dash, d) = extras("stroke-dasharray: 5 5; stroke-dashoffset: 10px");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(dash.svg_presentation.as_ref().map(|l| l.len()), Some(2));
        // A later rule replaces one name and keeps the rest.
        let mut merged = x.clone();
        merged.refine(&extras("stroke-width: 2").0);
        let list = merged.svg_presentation.expect("merged");
        assert_eq!(get(&list, "stroke-width").as_deref(), Some("2px"));
        assert_eq!(get(&list, "stroke-opacity").as_deref(), Some(".28"));
    }

    /// The logical box properties resolve against `horizontal-tb` + `ltr`
    /// and share their slots with the physical ones: the later one wins.
    #[test]
    fn logical_margin_padding_and_inset_are_their_physical_sides() {
        let r = clean(
            "margin-inline: auto; margin-block: 2px 4px; padding-inline: 6px 8px; \
             padding-block-start: 10px; padding-block-end: 12px; inset-inline: 0 -22px; \
             inset-block-start: 3px; inset-block-end: 5%",
        );
        assert_eq!(r.margin.left, Some(Length::Auto));
        assert_eq!(r.margin.right, Some(Length::Auto));
        assert_eq!(r.margin.top, Some(px(2.0).into()));
        assert_eq!(r.margin.bottom, Some(px(4.0).into()));
        assert_eq!(r.padding.left, Some(px(6.0).into()));
        assert_eq!(r.padding.right, Some(px(8.0).into()));
        assert_eq!(r.padding.top, Some(px(10.0).into()));
        assert_eq!(r.padding.bottom, Some(px(12.0).into()));
        assert_eq!(r.inset.left, Some(px(0.0).into()));
        assert_eq!(r.inset.right, Some(px(-22.0).into()));
        assert_eq!(r.inset.top, Some(px(3.0).into()));
        assert_eq!(r.inset.bottom, Some(gpui::relative(0.05).into()));

        let later = |body: &str| clean(body).margin.left;
        assert_eq!(later("margin-left: 3px; margin-inline-start: 5px"), Some(px(5.0).into()));
        assert_eq!(later("margin-inline-start: 5px; margin-left: 3px"), Some(px(3.0).into()));
        assert_eq!(clean("margin: 1px; margin-inline-end: 9px").margin.right, Some(px(9.0).into()));
    }

    // ---- the end-to-end Tailwind card ------------------------------------

    #[test]
    fn the_demo_cards_utilities_all_render() {
        // Byte-for-byte what Tailwind v3.4 emits for
        // `flex flex-col gap-2 p-4 rounded-xl bg-white/10 text-sm font-semibold`.
        let r = clean(
            "display: flex; flex-direction: column; gap: 0.5rem; padding: 1rem; \
             border-radius: 0.75rem; background-color: rgb(255 255 255 / 0.1); \
             font-size: 0.875rem; line-height: 1.25rem; font-weight: 600",
        );
        assert_eq!(r.display, Some(gpui::Display::Flex));
        assert_eq!(r.flex_direction, Some(gpui::FlexDirection::Column));
        assert_eq!(r.gap.width, Some(px(8.0).into()));
        assert_eq!(r.gap.height, Some(px(8.0).into()));
        assert_eq!(r.padding.top, Some(px(16.0).into()));
        assert_eq!(r.corner_radii.top_left, Some(px(12.0).into()));
        assert_eq!(r.text.font_size, Some(px(14.0).into()));
        assert_eq!(r.text.font_weight, Some(gpui::FontWeight(600.0)));
        assert!(matches!(r.background, Some(gpui::Fill::Color(_))));
    }

    // ---- the m22 CSS burn-down (research/reports/m22/engine-round.md §4) ----

    /// `drop-shadow()` is the second `filter` function this engine implements
    /// rather than drops: gpui's `BoxShadow` rasterises a Gaussian-blurred
    /// rounded rect with the element's own corner radii, which IS the blurred
    /// alpha of a box whose alpha is an opaque rounded rect (§4.4). The three
    /// lengths transfer 1:1 — CSS's third length and gpui's `blur_radius` are
    /// both sigmas.
    #[test]
    fn drop_shadow_parses_into_extras_and_says_nothing() {
        let (_, x, d) = map_full("filter: drop-shadow(2px 4px 6px rgb(0 0 0 / 0.5))");
        assert!(d.is_empty(), "the shape gpui can draw exactly must not advise: {d:?}");
        let s = x.drop_shadow.expect("drop-shadow() reached Extras");
        assert_eq!((s.dx, s.dy, s.blur), (2.0, 4.0, 6.0));
        assert!(s.color.is_some(), "an explicit colour is flattened here");

        // No colour argument is `currentColor`, which only the renderer can
        // resolve — so the mapper records `None` rather than guessing black.
        let (_, x, d) = map_full("filter: drop-shadow(1px 2px 3px)");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(x.drop_shadow.map(|s| s.color), Some(None));

        // A second one has nowhere to go, and that IS a divergence.
        let (_, x, d) =
            map_full("filter: drop-shadow(1px 1px 1px red) drop-shadow(2px 2px 2px blue)");
        assert!(x.drop_shadow.is_some(), "the first one is still honoured");
        assert!(d.iter().any(|m| m.contains("a second drop-shadow()")), "{d:?}");
    }

    /// The gradient-border idiom (the Argon client's Ultima price card, its
    /// profile frame and BorderTrace): a mask that leaves the padding and
    /// border ring. Its parts are recorded without a word, and only the whole
    /// cascade makes the ring. Any other mask is advised.
    #[test]
    fn the_gradient_border_mask_is_a_ring_and_others_are_advised() {
        let ring = "-webkit-mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0);
                    mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0);
                    -webkit-mask-composite: xor;
                    mask-composite: exclude";
        let (_, x, d) = map_full(ring);
        assert!(d.is_empty(), "{d:?}");
        assert!(x.mask_ring());
        for composite in ["-webkit-mask-composite: xor", "-webkit-mask-composite: destination-out"] {
            let (_, x, d) = map_full(&format!(
                "mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0); {composite}"
            ));
            assert!(d.is_empty() && x.mask_ring(), "{composite}: {d:?}");
        }
        // The shorthand alone composites with `add`: the union is the whole
        // border box, so the element paints as if unmasked, which is right.
        let (_, x, d) = map_full("mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0)");
        assert!(d.is_empty(), "{d:?}");
        assert!(!x.mask_ring());
        // The longhands spell it too.
        let (_, x, d) = map_full(
            "mask-image: linear-gradient(red, blue), linear-gradient(#fff 0 0);
             mask-clip: content-box, border-box; mask-composite: exclude",
        );
        assert!(d.is_empty() && x.mask_ring(), "{d:?}");
        // A shorthand AFTER the composite resets it to `add`.
        let (_, x, _) = map_full(&format!("{ring}; mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0)"));
        assert!(!x.mask_ring());

        for other in [
            "mask-image: linear-gradient(to right, black 90%, transparent)",
            "mask-image: url(icon.svg)",
            "mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0); mask-composite: intersect",
        ] {
            let (_, x, d) = map_full(other);
            assert!(!x.mask_ring(), "{other}");
            assert!(d.iter().any(|m| m.contains("`mask` is not applied")), "{other}: {d:?}");
        }
        let (_, x, d) = map_full("mask: none");
        assert!(d.is_empty() && !x.mask_ring(), "{d:?}");
    }

    /// `clip-path` reaches gpui as a rounded content mask (`gpui::VnClipPath`):
    /// `inset()`, `circle()` and a lone box exactly, `ellipse()` as a capsule
    /// with an advisory. `polygon()`, `path()` and `url()` are advised and
    /// set nothing. The screen-reader-only `inset(50%)` is a real clip to
    /// nothing: the paint switch and the CSS `visibility` are left alone.
    #[test]
    fn clip_path_is_a_rounded_mask_and_the_other_shapes_advise() {
        use gpui::{VnClipBox as B, VnClipPath as C, VnCornerRadius, VnShapeRadius as R};
        let at = |x: f32, y: f32| VnCornerRadius { x: px(x).into(), y: px(y).into() };
        let half = relative(0.5);
        for sr_only in ["clip-path: inset(50%)", "clip-path: inset(50% 50% 50% 50%)"] {
            let r = clean(sr_only);
            assert_eq!(r.visibility, None, "{sr_only}");
            assert_eq!(
                r.vn_clip_path,
                Some(C::Inset { reference: B::Border, insets: [half; 4], radii: [at(0., 0.); 4] }),
                "{sr_only}"
            );
        }
        // Two insets, the `round` radii (one elliptical corner) and a box.
        assert_eq!(
            clean("clip-path: inset(10px 20% round 8px 8px 8px 4px / 8px 8px 8px 6px) padding-box").vn_clip_path,
            Some(C::Inset {
                reference: B::Padding,
                insets: [px(10.).into(), relative(0.2), px(10.).into(), relative(0.2)],
                radii: [at(8., 8.), at(8., 8.), at(8., 8.), at(4., 6.)],
            })
        );
        assert_eq!(
            clean("clip-path: circle(40%)").vn_clip_path,
            Some(C::Ellipse {
                reference: B::Border,
                center: [half, half],
                radius: [R::Length(relative(0.4)); 2],
                circle: true,
            })
        );
        // The default radius is the closest side; an offset from the far side
        // is 100% less it.
        assert_eq!(
            clean("clip-path: circle(at right 10px top) content-box").vn_clip_path,
            Some(C::Ellipse {
                reference: B::Content,
                center: [DefiniteLength::Calc { fraction: 1.0, pixels: px(-10.) }, relative(0.)],
                radius: [R::ClosestSide; 2],
                circle: true,
            })
        );
        // An ellipse is exact now (an isolated layer through its outline),
        // so it says nothing.
        assert_eq!(
            clean("clip-path: ellipse(30% farthest-side at 25% 75%)").vn_clip_path,
            Some(C::Ellipse {
                reference: B::Border,
                center: [relative(0.25), relative(0.75)],
                radius: [R::Length(relative(0.3)), R::FarthestSide],
                circle: false,
            })
        );
        assert!(clean("clip-path: ellipse(20px 20px)").vn_clip_path.is_some());
        // A box alone; `fill-box` is the content box and `stroke-box` the
        // border box of an element with a CSS box.
        assert_eq!(clean("clip-path: margin-box").vn_clip_path, Some(C::Box(B::Margin)));
        assert_eq!(clean("clip-path: fill-box").vn_clip_path, Some(C::Box(B::Content)));
        assert_eq!(clean("clip-path: stroke-box").vn_clip_path, Some(C::Box(B::Border)));
        // What is still not applied: a reference and the CSS Shapes 2
        // functions.
        for other in ["url(#c)", "rect(0 10px 10px 0)", "xywh(0 0 10px 10px)", "shape(from 0 0, line to 10px 10px)"] {
            let (r, d) = map(&format!("clip-path: {other}"));
            assert_eq!(r.vn_clip_path, None, "{other}");
            assert!(d.iter().any(|m| m.contains("clip-path") && m.contains("not applied")), "{other}: {d:?}");
        }
        // `none` clears an earlier clip, and leaves the paint switch alone.
        let r = clean("clip-path: none");
        assert_eq!((r.vn_clip_path, r.visibility), (Some(C::None), None));
    }

    /// `polygon()` keeps its vertices as lengths and percentages of the
    /// reference box, and its fill rule; `path()`, which lightningcss leaves
    /// unparsed, is parsed here from an inline style as the overlay writes it
    /// every frame, with its fill rule and box. Broken path data drops the
    /// declaration, as Chromium does.
    #[test]
    fn clip_path_polygon_and_path_are_shapes() {
        use gpui::{VnClipBox as B, VnClipPath as C, VnFillRule as F, VnPathVerb::*};
        let polygon = |r: &StyleRefinement| match &r.vn_clip_path {
            Some(C::Polygon { reference, fill_rule, points }) => (*reference, *fill_rule, points.to_vec()),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            polygon(&clean("clip-path: polygon(50% 0, 100% 100%, 0 100%)")),
            (
                B::Border,
                F::NonZero,
                vec![[relative(0.5), px(0.).into()], [relative(1.), relative(1.)], [px(0.).into(), relative(1.)]]
            )
        );
        // The fill rule, a box, and `calc()` in a vertex (its space is no
        // separator between the two values).
        let (rb, rule, points) =
            polygon(&clean("clip-path: polygon(evenodd, calc(100% - 8px) 0, 8px 100%) content-box"));
        assert_eq!((rb, rule), (B::Content, F::EvenOdd));
        assert_eq!(points[0], [DefiniteLength::Calc { fraction: 1., pixels: px(-8.) }, px(0.).into()]);
        assert_eq!(points[1], [px(8.).into(), relative(1.)]);

        let path = |r: &StyleRefinement| match &r.vn_clip_path {
            Some(C::Path { reference, fill_rule, verbs }) => (*reference, *fill_rule, verbs.to_vec()),
            other => panic!("{other:?}"),
        };
        // The overlay's notch, as its inline style writes it.
        let notch = "clip-path: path('M0 0 C12.1506 0 22 9.8494 22 22 V42 Q22 64 44 64 H420 Q442 64 442 42 V22 C442 9.8494 451.8494 0 464 0 Z')";
        let (rb, rule, verbs) = path(&clean(notch));
        assert_eq!((rb, rule, verbs.len()), (B::Border, F::NonZero, 9));
        assert_eq!(verbs[3], QuadTo([22., 64.], [44., 64.]));
        // Through the public inline-style entry too, which vn-tree calls.
        let mut d = Vec::new();
        let inline = parse_inline_style(notch, &mut d);
        assert!(d.is_empty(), "{d:?}");
        assert!(format!("{inline:?}").contains("Path"), "{inline:?}");
        // Double quotes, a fill rule and a box.
        let (rb, rule, verbs) = path(&clean(r#"clip-path: path(evenodd, "M0 0 h10 v10 h-10 z M2 2 h6 v6 h-6 z") padding-box"#));
        assert_eq!((rb, rule, verbs.len()), (B::Padding, F::EvenOdd, 10));
        assert_eq!(verbs[5], MoveTo([2., 2.]));
        // An empty string is an empty path (it encloses nothing).
        assert_eq!(path(&clean("clip-path: path('')")).2, vec![]);
        // Broken data, a bad fill rule, or no string: dropped and advised.
        for bad in ["path('M0 0 L')", "path('L10 10')", "path(round, 'M0 0')", "path(evenodd)", "path('M0 0') blue"] {
            let (r, d) = map(&format!("clip-path: {bad}"));
            assert_eq!(r.vn_clip_path, None, "{bad}");
            assert!(d.iter().any(|m| m.contains("path()") && m.contains("invalid")), "{bad}: {d:?}");
        }
    }

    /// `overflow-wrap` (and its legacy alias `word-wrap`, which CSS Text 3
    /// defines as the same property), `word-break` and `hyphens` reach gpui's
    /// line breaker through the text style, and say nothing. `hyphens: auto`
    /// is `manual`, with an advisory: there is no hyphenation dictionary.
    #[test]
    fn the_line_breaking_properties_reach_the_text_style() {
        for (body, want) in [
            ("overflow-wrap: normal", gpui::OverflowWrap::Normal),
            ("overflow-wrap: break-word", gpui::OverflowWrap::BreakWord),
            ("overflow-wrap: anywhere", gpui::OverflowWrap::Anywhere),
            ("word-wrap: break-word", gpui::OverflowWrap::BreakWord),
        ] {
            assert_eq!(clean(body).text.overflow_wrap, Some(want), "{body}");
        }
        for (body, want) in [
            ("word-break: normal", gpui::WordBreak::Normal),
            ("word-break: break-all", gpui::WordBreak::BreakAll),
            ("word-break: keep-all", gpui::WordBreak::KeepAll),
            ("word-break: break-word", gpui::WordBreak::BreakWord),
        ] {
            assert_eq!(clean(body).text.word_break, Some(want), "{body}");
        }
        assert_eq!(clean("hyphens: none").text.hyphens, Some(gpui::Hyphens::None));
        assert_eq!(clean("-webkit-hyphens: manual").text.hyphens, Some(gpui::Hyphens::Manual));
        let (r, d) = map("hyphens: auto");
        assert_eq!(r.text.hyphens, Some(gpui::Hyphens::Manual));
        assert!(d.len() == 1 && d[0].contains("hyphenation dictionary"), "{d:?}");
    }

    /// `scrollbar-width` and `scrollbar-color` are applied now (m22 adoptions
    /// §2) — they are parsed into `Extras` and decide whether the overlay bar
    /// appears and what colour it is — so a *valid* value says nothing; only an
    /// unparseable one raises an advisory. `-ms-overflow-style` is IE-only and
    /// does not exist in the Chromium these apps are compared against either,
    /// so no value of it can diverge. reka-ui's runtime `<style>` writes all of
    /// these, and the engine round made that sheet reach the cascade; an
    /// advisory has to name a real divergence.
    #[test]
    fn the_scrollbar_properties_are_silent() {
        for decl in [
            "scrollbar-width: none",
            "scrollbar-width: thin",
            "scrollbar-color: red blue",
            "-ms-overflow-style: none",
            "-webkit-overflow-scrolling: touch",
        ] {
            let (_, d) = map(decl);
            assert!(d.is_empty(), "{decl:?} must say nothing: {d:?}");
        }
    }

}

#[cfg(test)]
mod scrollbar_tests {
    use super::*;

    fn extras_of(body: &str) -> Extras {
        let mut d = Vec::new();
        parse_inline_style(body, &mut d).extras
    }

    #[test]
    fn scrollbar_width_parses_from_an_inline_body() {
        assert_eq!(extras_of("scrollbar-width: none").scrollbar_width, Some(ScrollbarWidth::None));
        assert_eq!(extras_of("scrollbar-width: thin").scrollbar_width, Some(ScrollbarWidth::Thin));
        assert_eq!(extras_of("scrollbar-width: auto").scrollbar_width, Some(ScrollbarWidth::Auto));
        assert_eq!(extras_of("color: red").scrollbar_width, None);
    }

    #[test]
    fn scrollbar_color_takes_two_colours_thumb_first() {
        let x = extras_of("scrollbar-color: #ff0000 #0000ff");
        let (thumb, track) = x.scrollbar_color.expect("a pair");
        // Hue: red is 0, blue is 2/3 of the wheel.
        assert!(thumb.h < 0.02, "thumb is the first colour (red), got {thumb:?}");
        assert!((track.h - 0.6667).abs() < 0.02, "track is the second (blue), got {track:?}");
        assert_eq!(extras_of("scrollbar-color: auto").scrollbar_color, None);
        assert_eq!(extras_of("scrollbar-color: red").scrollbar_color, None, "one colour is not a pair");
        // NAMED colours arrive as bare idents, not as a typed `CssColor` — a
        // pair of them is as valid as a pair of hex triples, and used to be
        // dropped with an advisory that said the declaration had the wrong
        // number of colours in it.
        let named = extras_of("scrollbar-color: red blue");
        let (thumb, track) = named.scrollbar_color.expect("a pair of named colours");
        assert!(thumb.h < 0.02, "thumb is red, got {thumb:?}");
        assert!((track.h - 0.6667).abs() < 0.02, "track is blue, got {track:?}");
    }

    #[test]
    fn overscroll_behavior_sets_both_axes_or_one() {
        let both = extras_of("overscroll-behavior: contain");
        assert_eq!(both.overscroll_x, Some(OverscrollBehavior::Contain));
        assert_eq!(both.overscroll_y, Some(OverscrollBehavior::Contain));
        let y = extras_of("overscroll-behavior-y: none");
        assert_eq!(y.overscroll_x, None);
        assert_eq!(y.overscroll_y, Some(OverscrollBehavior::None));
        assert!(OverscrollBehavior::Auto.chains());
        assert!(!OverscrollBehavior::Contain.chains());
        assert!(!OverscrollBehavior::None.chains());
    }

    /// One malformed declaration in an inline body costs that declaration, as
    /// in a browser, not the element's whole inline style. The string `:style`
    /// bug fed the host `0: d; 1: i; …`, and nothing of the body applied.
    #[test]
    fn a_malformed_inline_declaration_is_skipped_and_the_rest_applies() {
        let mut d = Vec::new();
        let inline = parse_inline_style("0: d; display: flex; }; gap: 4px", &mut d);
        let r = inline.refinement.expect("the valid declarations still apply");
        assert_eq!(r.display, Some(gpui::Display::Flex));
        assert!(r.gap.width.is_some(), "gap after the bad declarations applies too");
        assert!(d.iter().any(|m| m.contains("skipped")), "{d:?}");
    }

    #[test]
    fn accepts_declaration_is_what_the_engine_applies() {
        assert!(accepts_declaration("display", "flex"));
        assert!(accepts_declaration("width", "10px"));
        assert!(accepts_declaration("width", "var(--w)"));
        assert!(accepts_declaration("color", "inherit"));
        assert!(accepts_declaration("--anything", "at all"));
        // Not a value of the property, so not a declaration a browser keeps.
        assert!(!accepts_declaration("width", "abc"));
        assert!(!accepts_declaration("width", "10qq"));
        assert!(!accepts_declaration("color", "notacolor"));
        assert!(!accepts_declaration("display", ""));
        // One declaration only: a `;` cannot smuggle a second one in.
        assert!(!accepts_declaration("display", "flex; color: red"));
        // A property the mapper does not fold is not applied, so not accepted.
        assert!(!accepts_declaration("-ms-flex-pack", "center"));
        // A prefix Chromium ignores is not accepted, a `var()` value included;
        // the `-webkit-` alias it honours is.
        assert!(!accepts_declaration("-moz-appearance", "none"));
        assert!(!accepts_declaration("-moz-transition", "var(--t)"));
        assert!(accepts_declaration("-webkit-user-select", "none"));
    }
}

/// Vendor prefixes (`crate::prefix`) and `-webkit-text-fill-color`.
#[cfg(test)]
mod prefix_tests {
    use super::*;

    fn extras(body: &str) -> (Extras, Vec<String>) {
        let mut d = Vec::new();
        let inline = parse_inline_style(body, &mut d);
        (inline.extras, d)
    }

    #[test]
    fn a_prefixed_declaration_the_engine_does_not_apply_is_dropped_without_a_word() {
        // What the Argon client, its libraries and Tailwind's preflight carry.
        for body in [
            "-moz-osx-font-smoothing: grayscale",
            "-webkit-font-smoothing: antialiased",
            "-webkit-touch-callout: none",
            "-webkit-tap-highlight-color: transparent",
            "-webkit-overflow-scrolling: touch",
            "-ms-overflow-style: none",
            "-moz-tab-size: 4",
            "-moz-user-select: none",
            "-moz-appearance: none",
            "-ms-flex: 1",
            "-o-transition: opacity 1s",
            "-webkit-text-decoration: underline dotted",
            "-moz-transition: var(--t)",
            "-WEBKIT-TOUCH-CALLOUT: none",
        ] {
            let mut d = Vec::new();
            let inline = parse_inline_style(body, &mut d);
            assert!(d.is_empty(), "{body:?} said {d:?}");
            assert!(inline.is_empty(), "{body:?} applied something");
        }
    }

    #[test]
    fn a_dropped_prefix_never_reaches_a_rule_and_an_honoured_one_does() {
        // Chromium ignores `-moz-user-select` and honours `-webkit-user-select`.
        assert_eq!(extras("-moz-user-select: none").0.user_select, None);
        assert_eq!(extras("-webkit-user-select: none").0.user_select, Some(UserSelect::None));
        // Through the stylesheet path too, which `@keyframes` share.
        fn block(body: &str) -> DeclarationBlock<'_> {
            DeclarationBlock::parse_string(body, ParserOptions::default()).expect("parses")
        }
        let c = convert_block(&block(
            "-moz-transition: opacity 1s; -ms-transform: scale(2); -webkit-touch-callout: none",
        ));
        assert!(c.normal.is_none() && c.normal_transitions.is_none());
        assert!(c.diagnostics.is_none(), "{:?}", c.diagnostics);
        let c = convert_block(&block("-webkit-transition: opacity 1s; -webkit-transform: scale(2)"));
        assert!(c.normal.is_some() && c.normal_transitions.is_some());
        assert!(c.diagnostics.is_none(), "{:?}", c.diagnostics);
        // A `-webkit-` property Chromium draws and this engine does not is
        // still heard about.
        let (_, d) = extras("-webkit-text-stroke: 1px red");
        assert!(d.iter().any(|m| m.contains("not mapped")), "{d:?}");
    }

    #[test]
    fn webkit_text_fill_color_is_a_glyph_colour() {
        let is_red = |f: Option<TextFill>| match f {
            Some(TextFill::Color(c)) => c.h.abs() < 1e-3 && (c.s - 1.0).abs() < 1e-3 && (c.l - 0.5).abs() < 1e-3,
            _ => false,
        };
        assert!(is_red(extras("-webkit-text-fill-color: #ff0000").0.text_fill_color));
        assert!(is_red(extras("-webkit-text-fill-color: red").0.text_fill_color));
        assert_eq!(
            extras("-webkit-text-fill-color: currentColor").0.text_fill_color,
            Some(TextFill::CurrentColor)
        );
        match extras("-webkit-text-fill-color: transparent").0.text_fill_color {
            Some(TextFill::Color(c)) => assert_eq!(c.a, 0.0),
            other => panic!("{other:?}"),
        }
        // The gradient-text idiom, every half of it applied and none advised
        // beyond what `background-clip: text` itself says.
        let (x, d) = extras(
            "background-image: linear-gradient(90deg, #f00, #00f); -webkit-background-clip: text; \
             background-clip: text; -webkit-text-fill-color: transparent",
        );
        assert_eq!(x.background_clip_text, Some(true));
        assert!(x.text_fill_color.is_some());
        assert!(!d.iter().any(|m| m.contains("not mapped")), "{d:?}");
        // Not a colour: said, not applied.
        let (x, d) = extras("-webkit-text-fill-color: 3px");
        assert_eq!(x.text_fill_color, None);
        assert!(!d.is_empty());
    }
}
