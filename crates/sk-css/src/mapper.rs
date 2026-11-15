//! `lightningcss::Property` -> `gpui::StyleRefinement`: this file decides what CSS renders. An
//! unsupported declaration produces a diagnostic, never silence.

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
use lightningcss::properties::font::{
    AbsoluteFontWeight, FontFamily as CssFontFamily, FontWeight as CssFontWeight,
    LineHeight as CssLineHeight,
};
use lightningcss::properties::masking::ClipPath as CssClipPath;
use lightningcss::properties::overflow::OverflowKeyword;
use lightningcss::properties::size::Size as CssSize;
use lightningcss::properties::text::TextAlign as CssTextAlign;
use lightningcss::properties::{Property, PropertyId};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions};
use lightningcss::values::color::CssColor;
use lightningcss::values::length::{
    Length as CssLength, LengthPercentage, LengthPercentageOrAuto, LengthValue,
};
use lightningcss::values::percentage::DimensionPercentage;

use crate::keyframes::{AnimationCollector, AnimationPatch, AnimationSpec};
use crate::transition::{Collector, TransitionSpec};
use crate::vars::{self, Vars};

/// `(name, raw value text)` pairs for what cannot be mapped at sheet-compile time: `--x`
/// declarations and known properties whose value used `var()`, both resolved per element.
pub type Decls = Vec<(String, String)>;

/// CSS `object-fit`. A local mirror of `gpui::ObjectFit` (which derives neither `Clone` nor
/// `PartialEq`); the initial value is `fill` while gpui's image default is `Contain`, so the
/// renderer must state `Fill` explicitly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectFit {
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
}

/// The CSSWG resolved on 2026-01-21 to standardise Electron's `-webkit-app-region` as `window-drag:
/// none | move`, and separately that the property is INHERITED.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowDrag {
    Move,
    None,
}

/// CSS `--window-control`: tells Windows 11 which element is the maximize button (snap layouts). A
/// custom property because a browser ignores those, while an unknown value of a known property
/// would drop and fall back to the inherited `move`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowControl {
    Minimize,
    Maximize,
    Close,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserSelect {
    /// Declared `auto`: kept apart from "not declared" so that it can override an earlier rule's
    /// `none` in the cascade.
    Auto,
    None,
    Text,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropShadow {
    pub dx: f32,
    pub dy: f32,
    /// The Gaussian's **sigma**, px. CSS's third length and gpui's `blur_radius` are both sigmas,
    /// so it transfers 1:1.
    pub blur: f32,
    pub color: Option<gpui::Hsla>,
}

/// Which corners' `border-*-radius` is `inherit`, in cascade order. `border-radius` does not
/// inherit, so `inherit` asks for the PARENT's computed radius, which the mapper cannot know.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RadiusInherit {
    pub declared: u8,
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

    pub(crate) fn refine(&mut self, later: &RadiusInherit) {
        self.inherit = (self.inherit & !later.declared) | (later.inherit & later.declared);
        self.declared |= later.declared;
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extras {
    pub object_fit: Option<ObjectFit>,
    /// `None` means undeclared, which lets the renderer inherit the parent's value; `Some(None)` is
    /// an explicit opt-out.
    pub window_drag: Option<WindowDrag>,
    pub window_control: Option<WindowControl>,
    pub fixed: Option<bool>,
    pub drop_shadow: Option<DropShadow>,
    /// Whether `sticky` diverges depends on whether an ancestor scrolls, which only the renderer
    /// knows (`advise_out_of_flow`).
    pub sticky: Option<bool>,
    pub z_index: Option<Option<i32>>,
    pub isolate: Option<bool>,
    /// The element declared a `position` other than `static`. gpui cannot say this: `static` and
    /// `relative` both map onto its default `Relative`.
    pub positioned: Option<bool>,
    /// The cascade folds only the element's own declaration; `restyle_with` rewrites it to the
    /// effective (inherited) value before the states are stored.
    pub pointer_events_none: Option<bool>,
    /// The declared `visibility`. gpui's own switch also goes off for the screen-reader-only `clip:
    /// rect(0 0 0 0)`, while CSS `visibility` stays `visible` (read by `el.focus()`).
    pub visibility_hidden: Option<bool>,
    /// `user-select` as declared; `auto` defers to the parent when a selection asks
    /// (`sk_tree::selection`), so it is not folded down.
    pub user_select: Option<UserSelect>,
    /// The `display` had an inline outer type. taffy lays the box out block-level, which ignores
    /// the parent's `text-align`; the renderer advises (`advise_inline_level`).
    pub inline_level: Option<bool>,
    pub appearance_none: Option<bool>,
    pub accent_color: Option<Option<Hsla>>,
    pub fill: Option<SvgPaint>,
    pub stroke: Option<SvgPaint>,
    pub svg_presentation: Option<Rc<Vec<(&'static str, Rc<str>)>>>,
    /// `text-transform`. gpui has no such text-style field: the shaper is handed a string, so the
    /// case change is applied to that string by the renderer.
    pub text_transform: Option<TextTransform>,
    /// The whitespace-processing half of `white-space`; gpui models only whether a line may wrap,
    /// so collapsing is applied to the string by the renderer, like `text_transform`.
    pub white_space: Option<WhiteSpaceMode>,
    pub content: Option<Rc<str>>,
    /// `content: none` / `normal` was declared: a later rule that says so takes back the string an
    /// earlier one gave, where leaving `content` unset says nothing.
    pub content_none: bool,
    pub background_clip_text: Option<bool>,
    /// `-webkit-text-fill-color` overrides foreground `color`, independently of the background
    /// clipped behind the text.
    pub text_fill_color: Option<TextFill>,
    pub background_key_color: Option<gpui::Hsla>,
    /// Kept apart from the refinement: size/position/repeat are separate properties, often from
    /// separate rules, folded onto the layers after the cascade.
    pub background_images: Option<Rc<Vec<BackgroundLayer>>>,
    pub background_urls: Option<Rc<Vec<BackgroundUrl>>>,
    pub background_size: Option<Rc<Vec<BackgroundSize>>>,
    pub background_position: Option<Rc<Vec<(TileLength, TileLength)>>>,
    pub background_repeat: Option<Rc<Vec<(bool, bool)>>>,
    pub list_marker: Option<ListMarker>,
    pub list_marker_inside: Option<bool>,
    /// The web fonts this element's `font-family` asks for that are not loaded yet: the families of
    /// the list, up to the one used, that [`set_web_font_families`] named, in order.
    pub web_font: Option<Rc<[String]>>,
    pub underline_offset: Option<f32>,
    pub decoration: DecorationPatch,
    pub outline: OutlinePatch,
    /// `currentColor` is the computed `color`, unknown while a declaration is mapped (no inherited
    /// text property is folded), so targets are recorded and substituted after the cascade.
    pub current_color: CurrentColorTargets,
    pub current_color_overrides: CurrentColorTargets,
    /// `filter: blur()` radius in px. Exact only for a solid, contentless leaf box (the "glow
    /// blob"), which the renderer repaints as its own shadow.
    pub blur: Option<f32>,
    pub filter_url: Option<Rc<str>>,
    pub scrollbar_width: Option<ScrollbarWidth>,
    pub scrollbar_color: Option<(gpui::Hsla, gpui::Hsla)>,
    pub overscroll_x: Option<OverscrollBehavior>,
    pub overscroll_y: Option<OverscrollBehavior>,
    /// `contain` includes paint containment: the box clips its content as `overflow: clip` does.
    pub contain_paint: Option<bool>,
    /// The axes `contain` size-contains ([`Extras::CONTAIN_WIDTH`], [`Extras::CONTAIN_HEIGHT`]).
    pub contain_size: Option<u8>,
    /// Only a qualifier of the refinement's axis, which stays the truth (see `compute_overflow`).
    pub overflow_x: Option<OverflowKeyword>,
    pub overflow_y: Option<OverflowKeyword>,
    /// `mask-image` (or the `mask` shorthand's images). The three `mask_*` fields are kept apart
    /// because they cascade apart, and [`Extras::mask_ring`] reads them together.
    pub mask_images: Option<MaskImages>,
    pub mask_clips: Option<MaskClips>,
    pub mask_composite: Option<MaskCompositing>,
    pub radius_inherit: RadiusInherit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskImages {
    None,
    TwoOpaque,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskClips {
    ContentThenBorder,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaskCompositing {
    Add,
    Exclude,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarWidth {
    /// The platform's own width — 14 px here, which is what Windows 11's overlay scrollbar occupies
    /// when it is not hovered.
    Auto,
    /// CSS gives no number; browsers pick something around half, and so do we (8 px).
    Thin,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverscrollBehavior {
    Auto,
    Contain,
    None,
}

impl OverscrollBehavior {
    pub fn chains(self) -> bool {
        matches!(self, Self::Auto)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextTransform {
    None,
    Uppercase,
    Lowercase,
    Capitalize,
}

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
    pub fn collapses_spaces(self) -> bool {
        matches!(
            self,
            WhiteSpaceMode::Normal | WhiteSpaceMode::Nowrap | WhiteSpaceMode::PreLine
        )
    }

    pub fn preserves_newlines(self) -> bool {
        !matches!(self, WhiteSpaceMode::Normal | WhiteSpaceMode::Nowrap)
    }
}

impl TextTransform {
    pub fn apply(self, text: &str) -> Option<String> {
        match self {
            TextTransform::None => None,
            TextTransform::Uppercase => Some(text.to_uppercase()),
            TextTransform::Lowercase => Some(text.to_lowercase()),
            // CSS capitalises the first letter of each WORD and leaves the rest of the word exactly
            // as authored (it does not lowercase it).
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextFill {
    CurrentColor,
    Color(Hsla),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SvgPaint {
    None,
    Current,
    Color(Hsla),
    Server,
}

impl SvgPaint {
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

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DecorationPatch {
    /// `Some(None)` is the `text-decoration` shorthand's reset to `currentColor`: it ends an
    /// earlier rule's colour.
    pub color: Option<Option<Hsla>>,
    pub thickness: Option<f32>,
    pub wavy: Option<bool>,
}

impl DecorationPatch {
    pub(crate) fn refine(&mut self, other: &Self) {
        if other.color.is_some() {
            self.color = other.color;
        }
        if other.thickness.is_some() {
            self.thickness = other.thickness;
        }
        if other.wavy.is_some() {
            self.wavy = other.wavy;
        }
    }

    /// Applied to the decoration the whole cascade produced: the colour lives on gpui's decoration,
    /// and `underline decoration-red-500` is separate rules in either order. A `none` decoration
    /// (zero thickness, transparent) is left alone so a thickness cannot revive it.
    pub(crate) fn apply(&self, text: &mut gpui::TextStyleRefinement) {
        let cancelled = |thickness: gpui::Pixels, color: Option<Hsla>| {
            thickness == px(0.0) && color == Some(gpui::transparent_black())
        };
        if let Some(underline) = text.underline.as_mut() {
            if !cancelled(underline.thickness, underline.color) {
                if let Some(Some(color)) = self.color {
                    underline.color = Some(color);
                }
                if let Some(thickness) = self.thickness {
                    underline.thickness = px(thickness);
                }
                if let Some(wavy) = self.wavy {
                    underline.wavy = wavy;
                }
            }
        }
        if let (Some(strikethrough), Some(Some(color))) = (text.strikethrough.as_mut(), self.color)
        {
            if !cancelled(strikethrough.thickness, strikethrough.color) {
                strikethrough.color = Some(color);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OutlinePatch {
    pub width: Option<f32>,
    pub style: Option<gpui::BorderStyle>,
    pub hidden: Option<bool>,
    pub color: Option<Option<Hsla>>,
    pub offset: Option<f32>,
}

impl OutlinePatch {
    pub(crate) fn refine(&mut self, other: &Self) {
        if other.width.is_some() {
            self.width = other.width;
        }
        if other.style.is_some() {
            self.style = other.style;
        }
        if other.hidden.is_some() {
            self.hidden = other.hidden;
        }
        if other.color.is_some() {
            self.color = other.color;
        }
        if other.offset.is_some() {
            self.offset = other.offset;
        }
    }

    pub(crate) fn merge_over(
        &self,
        earlier: Option<gpui::Outline>,
        slot: &mut Option<gpui::Outline>,
    ) {
        let Some(mut ring) = earlier else { return };
        if *self == Self::default() {
            return;
        }
        if self.hidden == Some(true) {
            ring.width = px(0.0);
        } else {
            if let Some(style) = self.style {
                ring.style = style;
                if ring.width == px(0.0) {
                    ring.width = px(3.0);
                }
            }
            if let Some(width) = self.width {
                ring.width = px(width);
            }
        }
        if let Some(Some(color)) = self.color {
            ring.color = color;
        }
        if let Some(offset) = self.offset {
            ring.offset = px(offset);
        }
        *slot = Some(ring);
    }

    /// A rung's ring is laid over the base whole, so what the rung left undeclared must come from
    /// the base ring, not the seed: `:focus { outline-offset: 4px }` keeps the width and colour of
    /// `outline: 2px solid`.
    pub(crate) fn inherit_from(&self, base: gpui::Outline, ring: &mut gpui::Outline) {
        if self.width.is_none() && self.hidden != Some(true) {
            let medium = self.style.is_some() && ring.width == px(3.0) && base.width > px(0.0);
            if ring.width == px(0.0) || medium {
                ring.width = base.width;
            }
        }
        if self.offset.is_none() && ring.offset == px(0.0) {
            ring.offset = base.offset;
        }
        if self.color.is_none() && ring.color == Hsla::default() {
            ring.color = base.color;
        }
        if self.style.is_none() && ring.style == gpui::BorderStyle::Solid {
            ring.style = base.style;
        }
    }
}

impl Extras {
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
        if other.sticky.is_some() {
            self.sticky = other.sticky;
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
            self.content_none = false;
        } else if other.content_none {
            self.content = None;
            self.content_none = true;
        }
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
        self.decoration.refine(&other.decoration);
        self.outline.refine(&other.outline);
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
        if other.contain_paint.is_some() {
            self.contain_paint = other.contain_paint;
        }
        if other.contain_size.is_some() {
            self.contain_size = other.contain_size;
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
        self.current_color.0 =
            (self.current_color.0 & !other.current_color_overrides.0) | other.current_color.0;
        self.current_color_overrides.0 |= other.current_color_overrides.0;
        self.radius_inherit.refine(&other.radius_inherit);
    }

    pub fn mask_ring(&self) -> bool {
        self.mask_images == Some(MaskImages::TwoOpaque)
            && self.mask_clips == Some(MaskClips::ContentThenBorder)
            && self.mask_composite == Some(MaskCompositing::Exclude)
    }

    pub fn resolve_background_layers(&self) -> Vec<gpui::Background> {
        let Some(images) = self.background_images.as_ref() else {
            return Vec::new();
        };
        let mut out: Vec<gpui::Background> = images
            .iter()
            .map(|layer| self.tiled_gradient(layer))
            .collect();
        out.reverse();
        out
    }

    pub fn resolve_background_stack(&self) -> Option<Vec<PaintLayer>> {
        let urls = self.background_urls.as_ref().filter(|u| !u.is_empty())?;
        let mut stack: Vec<(u16, PaintLayer)> = Vec::new();
        for layer in self.background_images.iter().flat_map(|l| l.iter()) {
            stack.push((
                layer.index,
                PaintLayer::Gradient(self.tiled_gradient(layer)),
            ));
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
        stack.sort_by_key(|(index, _)| *index);
        Some(stack.into_iter().rev().map(|(_, layer)| layer).collect())
    }

    /// A layer with no explicit two-axis size is not tiled: a gradient has no intrinsic size, so
    /// `auto`, `cover` and `contain` all resolve to the box itself.
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

    pub fn z(&self) -> Option<i32> {
        self.z_index.flatten()
    }

    /// A superset of what the renderer defers (whether `z-index` applies depends on the parent's
    /// display); safe for a cache that refuses boundaries conservatively.
    pub fn may_defer(&self) -> bool {
        self.z().is_some_and(|z| z != 0) || self.fixed == Some(true)
    }
}

/// One declaration block, split into normal/important halves. `None` for a list means "not
/// declared", so a later rule can override without an empty list reading as `transition: none`.
pub(crate) struct Converted {
    pub normal: Option<Rc<StyleRefinement>>,
    pub important: Option<Rc<StyleRefinement>>,
    pub normal_transitions: Option<Rc<Vec<TransitionSpec>>>,
    pub important_transitions: Option<Rc<Vec<TransitionSpec>>>,
    pub normal_transition_declarations: Option<Rc<Collector>>,
    pub important_transition_declarations: Option<Rc<Collector>>,
    pub normal_animations: Option<Rc<Vec<AnimationSpec>>>,
    pub important_animations: Option<Rc<Vec<AnimationSpec>>>,
    pub normal_animation_patch: Option<AnimationPatch>,
    pub important_animation_patch: Option<AnimationPatch>,
    pub normal_extras: Extras,
    pub important_extras: Extras,
    pub normal_custom: Option<Rc<Decls>>,
    pub important_custom: Option<Rc<Decls>>,
    pub normal_dynamic: Option<Rc<Decls>>,
    pub important_dynamic: Option<Rc<Decls>>,
    pub normal_font: Option<FontDecls>,
    pub important_font: Option<FontDecls>,
    /// Kept with the rule and logged when it matches: a compile-time advisory describes the
    /// stylesheet, not what is wrong with the page.
    pub diagnostics: Option<Rc<Vec<String>>>,
}

/// Font-relative declarations from the first one that read an em to the end of the half, plus the
/// static ones after it so a later `padding-left: 4px` still beats an earlier `padding: 1em`.
pub type FontDecls = Rc<Vec<Property<'static>>>;

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
        font.get_or_insert_with(Vec::new)
            .push(p.clone().into_owned());
    }
}

enum Channel {
    Custom(String, String),
    Dynamic(String, String),
    Static,
}

/// `Property`'s own `ToCss` writes `name: value`, and a property name can never contain `:`, so the
/// split is exact.
fn value_text(p: &Property) -> Option<String> {
    let css = p.to_css_string(false, PrinterOptions::default()).ok()?;
    let (_, value) = css.split_once(':')?;
    Some(value.trim().to_string())
}

/// Insets do not inherit, so both keywords mean `auto`, including the logical spellings under this
/// engine's horizontal-tb/ltr model.
fn initial_inset_sides(name: &str, value: &lightningcss::properties::custom::TokenList<'_>) -> u8 {
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

pub(crate) fn decl_pairs(block: &DeclarationBlock) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for (props, important) in [
        (&block.declarations, false),
        (&block.important_declarations, true),
    ] {
        for p in props.iter() {
            let Some(css) = p.to_css_string(false, PrinterOptions::default()).ok() else {
                continue;
            };
            match css.split_once(':') {
                Some((name, value)) => {
                    out.push((name.trim().to_string(), value.trim().to_string(), important))
                }
                // A property that prints without a colon cannot exist, but a dropped declaration
                // would be a lie in a provenance panel.
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
            CustomPropertyName::Custom(name) if name.0.as_ref() == effect::SHADING => {
                Channel::Static
            }
            CustomPropertyName::Custom(name) if name.0.as_ref() == WINDOW_CONTROL => {
                Channel::Static
            }
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
            CustomPropertyName::Unknown(name)
                if has_substitution(&c.value) || has_current_color_function(&c.value) =>
            {
                match value_text(p) {
                    Some(v) => Channel::Dynamic(name.0.as_ref().to_string(), v),
                    None => Channel::Static,
                }
            }
            CustomPropertyName::Unknown(_) => Channel::Static,
        },
        Property::Unparsed(u)
            if matches!(u.property_id, PropertyId::ClipPath(_)) && !has_substitution(&u.value) =>
        {
            Channel::Static
        }
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
            // `-moz-osx-font-smoothing`, `-webkit-touch-callout`: dropped without a word, as
            // Chromium drops them (`crate::prefix`).
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

thread_local! {
    /// While an element computes: what `currentColor` stands for inside a colour function — the
    /// parent's colour in `color` itself, the element's own everywhere else — and whether a
    /// declaration read it.
    static CURRENT_COLOR: std::cell::Cell<Option<(Hsla, Hsla)>> = const { std::cell::Cell::new(None) };
    static CURRENT_COLOR_READ: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The `currentColor` context of one element's cascade. `own` is a guess until the cascade has
/// computed `color`; [`CurrentColorScope::read`] says whether a second pass with the real one is
/// needed.
pub(crate) struct CurrentColorScope {
    outer: Option<(Hsla, Hsla)>,
    outer_read: bool,
}

impl CurrentColorScope {
    pub(crate) fn enter(parent: Hsla, own: Hsla) -> Self {
        Self {
            outer: CURRENT_COLOR.with(|c| c.replace(Some((parent, own)))),
            outer_read: CURRENT_COLOR_READ.with(|r| r.replace(false)),
        }
    }

    pub(crate) fn read(&self) -> bool {
        CURRENT_COLOR_READ.with(std::cell::Cell::get)
    }
}

impl Drop for CurrentColorScope {
    fn drop(&mut self) {
        CURRENT_COLOR.with(|c| c.set(self.outer));
        CURRENT_COLOR_READ.with(|r| r.set(self.outer_read));
    }
}

/// `value` with each `currentColor` keyword spelled as the colour it stands for, so that
/// `color-mix()` and relative colours over it compute; `None` outside an element's cascade or
/// when there is no such keyword.
fn current_color_substituted(name: &str, value: &str) -> Option<String> {
    const KEYWORD: &str = "currentcolor";
    let lower = value.to_ascii_lowercase();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'-' || b == b'_';
    let bytes = lower.as_bytes();
    let starts: Vec<usize> = lower
        .match_indices(KEYWORD)
        .map(|(at, _)| at)
        .filter(|&at| {
            let end = at + KEYWORD.len();
            (at == 0 || !ident(bytes[at - 1])) && (end == bytes.len() || !ident(bytes[end]))
        })
        .collect();
    if starts.is_empty() {
        return None;
    }
    let (parent, own) = CURRENT_COLOR.with(std::cell::Cell::get)?;
    CURRENT_COLOR_READ.with(|r| r.set(true));
    let c = gpui::Rgba::from(if name == "color" { parent } else { own });
    let literal = format!(
        "rgb({} {} {} / {})",
        c.r * 255.0,
        c.g * 255.0,
        c.b * 255.0,
        c.a
    );
    let mut out = String::with_capacity(value.len() + starts.len() * literal.len());
    let mut from = 0;
    for at in starts {
        out.push_str(&value[from..at]);
        out.push_str(&literal);
        from = at + KEYWORD.len();
    }
    out.push_str(&value[from..]);
    Some(out)
}

/// Three failures, each advised: invalid substitution (missing variable, cycle), a substituted text
/// that does not parse, or a property the mapper does not map. Invalid ones are dropped, as in CSS.
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
        // An unset `var()` with no fallback invalidates the declaration at computed-value time;
        // routine, so silent. The other errors are author mistakes and still speak.
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
    use lightningcss::traits::IntoOwned as _;
    let parse = |value: &str| {
        Property::parse_string(PropertyId::from(name), value, ParserOptions::default())
            .ok()
            .map(|p| p.into_owned())
    };
    let property = match parse(&value) {
        Some(unresolved @ (Property::Unparsed(_) | Property::Custom(_))) => {
            match current_color_substituted(name, &value) {
                Some(concrete) => parse(&concrete),
                None => Some(unresolved),
            }
        }
        parsed => parsed,
    };
    let Some(property) = property else {
        once(
            diagnostics,
            format!("`{name}: {value}` did not parse after var() substitution (v1)"),
        );
        return;
    };
    // A `transition` is folded into its own list at sheet-compile time and the list is per rule,
    // not per element — so a var()-composed one has nowhere to land.
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

pub fn parse_inline_with_transitions(
    body: &str,
    diagnostics: &mut Vec<String>,
) -> (StyleRefinement, Vec<TransitionSpec>) {
    let (r, t, _) = parse_inline_full(body, diagnostics);
    (r, t)
}

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

#[derive(Clone, Debug, Default)]
pub struct InlineMotion {
    pub(crate) transitions: Option<Rc<Collector>>,
    pub(crate) animations: Option<AnimationPatch>,
}
impl InlineMotion {
    fn is_empty(&self) -> bool {
        self.transitions.is_none() && self.animations.is_none()
    }
}

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
    pub normal_motion: Option<InlineMotion>,
    pub important_motion: Option<InlineMotion>,
    pub refinement: Option<Rc<StyleRefinement>>,
    /// `None` = the body declared no `transition` at all (which is what lets a matched rule's list
    /// survive); `Some([])` is `transition: none`.
    pub transitions: Option<Rc<Vec<TransitionSpec>>>,
    /// Inline `animation`: reka-ui's `Presence` writes `animationName = "none"` to cancel an exit
    /// animation, so it must be representable.
    pub animations: Option<Rc<Vec<AnimationSpec>>>,
    /// Longhand-only `animation-*`; folding `animationFillMode = "forwards"` as a list would delete
    /// the animation it means to freeze.
    pub animation_patch: Option<AnimationPatch>,
    pub extras: Extras,
    pub custom: Decls,
    pub dynamic: Decls,
    /// An inline `padding: .5em` is the element's own em, which the parse cannot know.
    pub font: Option<FontDecls>,
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
    let p = block
        .declarations
        .iter()
        .chain(block.important_declarations.iter())
        .next();
    if p.is_some_and(crate::prefix::property_dropped) {
        return false;
    }
    if let Some(Property::Unparsed(u)) = p {
        return has_substitution(&u.value)
            || matches!(
                single_ident_raw(&u.value)
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("inherit" | "initial" | "unset" | "revert" | "revert-layer")
            );
    }
    let mut diagnostics = Vec::new();
    let style = parse_inline_style(&body, &mut diagnostics);
    diagnostics.is_empty() && !style.is_empty()
}

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
    // Without it one bad declaration cost the element its whole inline style.
    let warnings = std::sync::Arc::new(std::sync::RwLock::new(Vec::new()));
    let options = ParserOptions {
        error_recovery: true,
        warnings: Some(warnings.clone()),
        ..ParserOptions::default()
    };
    let parsed = DeclarationBlock::parse_string(body, options);
    if let Ok(skipped) = warnings.read() {
        for e in skipped.iter() {
            diagnostics.push(format!(
                "inline style: {e} in {body:?} — skipped, as a browser skips it"
            ));
        }
    }
    match parsed {
        Ok(block) => {
            for (important, declarations) in [
                (false, &block.declarations),
                (true, &block.important_declarations),
            ] {
                let transitions = if important {
                    &mut important_transitions
                } else {
                    &mut normal_transitions
                };
                let animations = if important {
                    &mut important_animations
                } else {
                    &mut normal_animations
                };
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
            if !custom.is_empty() || !dynamic.is_empty() || !block.important_declarations.is_empty()
            {
                let converted = convert_block(&block);
                let half = |important| InlinePriority {
                    refinement: if important {
                        converted.important.clone()
                    } else {
                        converted.normal.clone()
                    },
                    extras: if important {
                        converted.important_extras.clone()
                    } else {
                        converted.normal_extras.clone()
                    },
                    custom: if important {
                        converted.important_custom.clone()
                    } else {
                        converted.normal_custom.clone()
                    },
                    dynamic: if important {
                        converted.important_dynamic.clone()
                    } else {
                        converted.normal_dynamic.clone()
                    },
                    font: if important {
                        converted.important_font.clone()
                    } else {
                        converted.normal_font.clone()
                    },
                };
                priority = Some(Rc::new([half(false), half(true)]));
            }
        }
        Err(e) => diagnostics.push(format!("inline style parse error: {e} in {body:?}")),
    }
    let normal_motion = InlineMotion {
        transitions: normal_transitions.declarations(),
        animations: normal_animations.clone().finish().1,
    };
    let important_motion = InlineMotion {
        transitions: important_transitions.declarations(),
        animations: important_animations.clone().finish().1,
    };
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

fn color(c: &CssColor) -> Option<Hsla> {
    let [r, g, b, a] = srgb(c)?;
    Some(gpui::Rgba { r, g, b, a }.into())
}

/// A concrete colour in sRGB channels. `lab()`, `oklch()`, `color()` and the rest are converted
/// and clipped per channel, which is what browsers paint on an sRGB display; `color-mix()` and
/// relative colours arrive here already computed.
pub(crate) fn srgb(c: &CssColor) -> Option<[f32; 4]> {
    use lightningcss::values::color::{ColorSpace, SRGB};
    match c {
        CssColor::RGBA(c) => Some([
            c.red as f32 / 255.0,
            c.green as f32 / 255.0,
            c.blue as f32 / 255.0,
            c.alpha_f32(),
        ]),
        CssColor::LAB(_) | CssColor::Predefined(_) | CssColor::Float(_) => {
            let c = SRGB::try_from(c).ok()?.resolve_missing();
            Some([c.r, c.g, c.b, c.alpha].map(|v| v.clamp(0.0, 1.0)))
        }
        CssColor::CurrentColor | CssColor::LightDark(..) | CssColor::System(_) => None,
    }
}

thread_local! {
    static REM_BASIS: std::cell::Cell<f32> = const { std::cell::Cell::new(INITIAL_REM_BASIS) };
    static VIEWPORT_BASIS: std::cell::Cell<(f32, f32)> =
        const { std::cell::Cell::new(INITIAL_VIEWPORT_BASIS) };
    /// Set the first time a sheet resolves a viewport unit. Nothing else has to recompile when the
    /// window resizes, and almost no sheet uses one.
    static VIEWPORT_UNITS_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static CALC_DROPPED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FONT_SOURCE: std::cell::RefCell<Option<Box<dyn Fn() -> Vec<String>>>> =
        const { std::cell::RefCell::new(None) };
    static INSTALLED_FONTS: std::cell::RefCell<Option<Rc<std::collections::HashSet<String>>>> =
        const { std::cell::RefCell::new(None) };
    static FONT_MISSES: std::cell::RefCell<Vec<(String, Option<String>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
    static WEB_FONTS: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

pub fn set_web_font_families(families: impl IntoIterator<Item = String>) {
    let set = families.into_iter().map(|f| f.to_lowercase()).collect();
    WEB_FONTS.with(|w| *w.borrow_mut() = set);
}

fn waiting_web_font(families: &[CssFontFamily]) -> Option<Rc<[String]>> {
    use lightningcss::traits::ToCss;
    WEB_FONTS.with(|w| {
        let w = w.borrow();
        if w.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for f in families {
            let CssFontFamily::FamilyName(name) = f else {
                break;
            };
            let Ok(css) = name.to_css_string(Default::default()) else {
                continue;
            };
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

/// Read lazily on the first `font-family`: enumerating installed families is costly. Without a
/// source every named family counts as installed.
pub fn set_font_source(source: impl Fn() -> Vec<String> + 'static) {
    FONT_SOURCE.with(|s| *s.borrow_mut() = Some(Box::new(source)));
    INSTALLED_FONTS.with(|c| *c.borrow_mut() = None);
}

fn font_installed(name: &str) -> Option<bool> {
    let set = INSTALLED_FONTS.with(|c| c.borrow().clone()).or_else(|| {
        let names = FONT_SOURCE.with(|s| s.borrow().as_ref().map(|f| f()))?;
        let set = Rc::new(names.into_iter().collect::<std::collections::HashSet<_>>());
        INSTALLED_FONTS.with(|c| *c.borrow_mut() = Some(set.clone()));
        Some(set)
    })?;
    Some(set.contains(name))
}

pub fn take_font_misses() -> Vec<(String, Option<String>)> {
    FONT_MISSES.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

fn note_font_miss(asked: String, used: Option<String>) {
    FONT_MISSES.with(|m| {
        let mut m = m.borrow_mut();
        if !m.iter().any(|(a, _)| *a == asked) {
            m.push((asked, used));
        }
    });
}

/// Maps a generic to a family. The `ui-*` four are Safari-only (Chrome skips them, so do we);
/// `emoji` / `math` / `fangsong` are never a list's primary font.
fn generic_family(g: &lightningcss::properties::font::GenericFontFamily) -> Option<String> {
    use lightningcss::properties::font::GenericFontFamily as G;
    let candidates: &[&str] = match g {
        G::SystemUI => return Some(".SystemUIFont".into()),
        G::SansSerif => &["Arial", "Helvetica", "Liberation Sans"],
        G::Serif => &["Times New Roman", "Times", "Liberation Serif"],
        G::Monospace => &[
            "Consolas",
            "Menlo",
            "DejaVu Sans Mono",
            "Liberation Mono",
            "Courier New",
        ],
        G::Cursive => &["Comic Sans MS"],
        G::Fantasy => &["Impact"],
        _ => return None,
    };
    Some(
        candidates
            .iter()
            .find(|c| font_installed(c) != Some(false))
            .map_or(".SystemUIFont", |c| *c)
            .into(),
    )
}

fn first_available_family(families: &[CssFontFamily]) -> Option<String> {
    use lightningcss::traits::ToCss;
    let mut first_named = None;
    let mut asked_first = None;
    for (i, f) in families.iter().enumerate() {
        let found = match f {
            CssFontFamily::FamilyName(name) => {
                let Ok(css) = name.to_css_string(Default::default()) else {
                    continue;
                };
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
    static COMPOSED_FONTS: std::cell::RefCell<Vec<(String, bool)>> = const { std::cell::RefCell::new(Vec::new()) };
    static UNREACHED_FONTS: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub fn set_composed_font_families(families: impl IntoIterator<Item = (String, bool)>) {
    let list = families
        .into_iter()
        .map(|(f, latin)| (f.to_lowercase(), latin))
        .collect();
    COMPOSED_FONTS.with(|c| *c.borrow_mut() = list);
}

pub fn take_unreached_fonts() -> Vec<(String, String)> {
    UNREACHED_FONTS.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

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
                let Ok(css) = name.to_css_string(Default::default()) else {
                    continue;
                };
                (css.trim_matches('"').to_string(), false)
            }
            CssFontFamily::Generic(g) => match generic_family(g) {
                Some(name) => (name, true),
                None => continue,
            },
        };
        let latin = composed
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(&name))
            .map(|c| c.1);
        if latin.is_none() && !generic && font_installed(&name) == Some(false) {
            continue;
        }
        if !stack.iter().any(|(s, _)| s.eq_ignore_ascii_case(&name)) {
            stack.push((name, latin));
        }
    }
    let (first, latin) = stack.first()?.clone();
    if latin.is_none() {
        // An installed family draws the text; a composed one after it is reached only by the
        // characters it lacks, and DirectWrite's own fallback takes those.
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

pub const INITIAL_REM_BASIS: f32 = 16.0;

pub fn rem_basis() -> f32 {
    REM_BASIS.with(|c| c.get())
}

/// Sets the px value of `1rem`, reporting a change. `rem` is flattened at parse time, so
/// already-compiled sheets do not move; out-of-range values (0.0 during boot) are refused.
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

/// Must be non-zero: 0 would collapse every `vh` before the first `set_env`.
pub const INITIAL_VIEWPORT_BASIS: (f32, f32) = (1280.0, 720.0);

pub fn viewport_basis() -> (f32, f32) {
    VIEWPORT_BASIS.with(|c| c.get())
}

pub fn uses_viewport_units() -> bool {
    VIEWPORT_UNITS_SEEN.with(|c| c.get())
}

pub fn reset_viewport_units_seen() {
    VIEWPORT_UNITS_SEEN.with(|c| c.set(false));
}

pub(crate) fn noting_viewport_units<R>(f: impl FnOnce() -> R) -> (R, bool) {
    let before = VIEWPORT_UNITS_SEEN.with(|c| c.replace(false));
    let out = f();
    let seen = VIEWPORT_UNITS_SEEN.with(|c| c.get());
    VIEWPORT_UNITS_SEEN.with(|c| c.set(before || seen));
    (out, seen)
}

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

/// The small/large/dynamic families are the same number on a desktop window (no collapsing browser
/// chrome); resolved rather than dropped because `max-h-[85vh]` is what keeps a dialog inside the
/// window.
fn viewport_value_px(v: &LengthValue) -> Option<f32> {
    let (w, h) = viewport_basis();
    let px = match v {
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
        LengthValue::Vmin(p)
        | LengthValue::Svmin(p)
        | LengthValue::Lvmin(p)
        | LengthValue::Dvmin(p) => p / 100.0 * w.min(h),
        LengthValue::Vmax(p)
        | LengthValue::Svmax(p)
        | LengthValue::Lvmax(p)
        | LengthValue::Dvmax(p) => p / 100.0 * w.max(h),
        _ => return None,
    };
    VIEWPORT_UNITS_SEEN.with(|c| c.set(true));
    Some(px)
}

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontRelativeMetrics {
    pub zero_advance: f32,
    pub x_height: f32,
}

thread_local! {
    static FONT_METRIC_SOURCE: std::cell::RefCell<Option<Box<dyn Fn(&gpui::Font, f32) -> Option<FontRelativeMetrics>>>> = const { std::cell::RefCell::new(None) };
    static FONT_METRIC_GENERATION: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static FONT_FACE: std::cell::RefCell<Option<gpui::Font>> = const { std::cell::RefCell::new(None) };
}

/// Installs the host's font metrics. Replacing the source invalidates cached folds, so a style
/// computed before host setup cannot retain fallback widths.
pub fn set_font_metric_source(
    source: impl Fn(&gpui::Font, f32) -> Option<FontRelativeMetrics> + 'static,
) {
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
    font.and_then(|font| {
        FONT_METRIC_SOURCE.with(|slot| {
            slot.borrow()
                .as_ref()
                .and_then(|source| source(&font, size))
        })
    })
    .map(|metrics| {
        if x_height {
            metrics.x_height
        } else {
            metrics.zero_advance
        }
    })
    .filter(|value| value.is_finite() && *value > 0.0)
    .unwrap_or(HALF_EM * size)
}

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

thread_local! {
    static FONT_BASIS: std::cell::Cell<f32> = const { std::cell::Cell::new(f32::NAN) };
    static FONT_UNITS_SEEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn em_basis() -> f32 {
    FONT_UNITS_SEEN.with(|c| c.set(true));
    let basis = FONT_BASIS.with(|c| c.get());
    if basis.is_nan() {
        rem_basis()
    } else {
        basis
    }
}

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

pub(crate) fn noting_font_units<R>(f: impl FnOnce() -> R) -> (R, bool) {
    let before = FONT_UNITS_SEEN.with(|c| c.replace(false));
    let out = f();
    let seen = FONT_UNITS_SEEN.with(|c| c.replace(before));
    FONT_UNITS_SEEN.with(|c| c.set(before || seen));
    (out, seen)
}

/// Whether `p` sets `font-size`, which resolves its em against the PARENT's font size where every
/// other property uses the element's own.
pub(crate) fn sets_font_size(p: &Property) -> bool {
    matches!(p, Property::FontSize(_))
}

pub(crate) fn apply_at(
    p: &Property,
    basis: f32,
    r: &mut StyleRefinement,
    x: &mut Extras,
    diagnostics: &mut Vec<String>,
) {
    with_font_basis(basis, || apply(p, r, x, diagnostics));
}

pub(crate) fn font_size_px(l: AbsoluteLength) -> f32 {
    match l {
        AbsoluteLength::Pixels(p) => f32::from(p),
        AbsoluteLength::Rems(r) => r.0 * rem_basis(),
    }
}

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

/// Resolves a linear `calc()` to px; needed for shadcn's radius scale (`calc(var(--radius) -
/// 2px)`). A non-finite result is unresolved (`calc(100px / 0)`).
fn calc_px<V>(
    calc: &lightningcss::values::calc::Calc<V>,
    leaf: &dyn Fn(&V) -> Option<f32>,
) -> Option<f32> {
    calc_term(calc, leaf).filter(|px| px.is_finite())
}

fn calc_term<V>(
    calc: &lightningcss::values::calc::Calc<V>,
    leaf: &dyn Fn(&V) -> Option<f32>,
) -> Option<f32> {
    use lightningcss::values::calc::{Calc, MathFunction};
    match calc {
        Calc::Value(v) => leaf(v),
        Calc::Number(n) => Some(*n),
        Calc::Sum(a, b) => Some(calc_term(a, leaf)? + calc_term(b, leaf)?),
        Calc::Product(n, v) => Some(n * calc_term(v, leaf)?),
        Calc::Function(f) => match &**f {
            MathFunction::Calc(c) => calc_term(c, leaf),
            MathFunction::Min(list) => list
                .iter()
                .map(|c| calc_term(c, leaf))
                .try_fold(f32::INFINITY, |acc, v| Some(acc.min(v?))),
            MathFunction::Max(list) => list
                .iter()
                .map(|c| calc_term(c, leaf))
                .try_fold(f32::NEG_INFINITY, |acc, v| Some(acc.max(v?))),
            // `f32::clamp` panics on that, and on a NaN bound.
            MathFunction::Clamp(lo, v, hi) => Some(
                calc_term(v, leaf)?
                    .min(calc_term(hi, leaf)?)
                    .max(calc_term(lo, leaf)?),
            ),
            MathFunction::Abs(v) => Some(calc_term(v, leaf)?.abs()),
            MathFunction::Sign(v) => Some(calc_term(v, leaf)?.signum()),
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

fn lp_leaf_px(lp: &LengthPercentage) -> Option<f32> {
    match lp {
        DimensionPercentage::Dimension(v) => length_value_px(v),
        // A percentage needs the containing block; a calc() that mixes one in is left unresolved
        // rather than guessed at.
        DimensionPercentage::Percentage(_) => None,
        DimensionPercentage::Calc(c) => calc_px(c, &lp_leaf_px),
    }
}

/// `calc_px` for properties whose percentage is not resolved here: a `calc()` mixing one in is
/// dropped and advised (`calc_dropped`).
fn lp_calc_px(c: &lightningcss::values::calc::Calc<LengthPercentage>) -> Option<f32> {
    let v = calc_px(c, &lp_leaf_px);
    if v.is_none() {
        calc_dropped();
    }
    v
}

fn calc_dropped() {
    CALC_DROPPED.with(|c| c.set(true));
}

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
            _ => calc_px(c, &lp_leaf_px).map(|p| (0.0, p)),
        },
    }
}

fn lp_definite(lp: &LengthPercentage) -> Option<DefiniteLength> {
    match lp {
        DimensionPercentage::Dimension(v) => length_value_px(v).map(|p| px(p).into()),
        DimensionPercentage::Percentage(p) => Some(relative(p.0)),
        DimensionPercentage::Calc(c) => match calc_px(c, &lp_leaf_px) {
            Some(p) => Some(px(p).into()),
            None => match calc_linear(c) {
                Some((fraction, pixels)) if fraction == 0.0 => Some(px(pixels).into()),
                Some((fraction, pixels)) => Some(DefiniteLength::Calc {
                    fraction,
                    pixels: px(pixels),
                }),
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

fn self_position(v: &SelfPosition) -> gpui::AlignItems {
    use gpui::AlignItems as G;
    match v {
        SelfPosition::Center => G::Center,
        SelfPosition::Start | SelfPosition::FlexStart | SelfPosition::SelfStart => G::FlexStart,
        SelfPosition::End | SelfPosition::FlexEnd | SelfPosition::SelfEnd => G::FlexEnd,
    }
}

/// `repeat(N, minmax(<min>, 1fr))` — the ONLY track list gpui's `GridTemplate` can express, and
/// (not by accident) exactly what Tailwind's `grid-cols-N` / `grid-rows-N` emit.
fn grid_template(v: &lightningcss::properties::grid::TrackSizing) -> Option<gpui::GridTemplate> {
    use lightningcss::properties::grid::{
        RepeatCount, TrackBreadth, TrackListItem, TrackSize, TrackSizing,
    };
    let TrackSizing::TrackList(list) = v else {
        return None;
    };
    let [TrackListItem::TrackRepeat(repeat)] = list.items.as_slice() else {
        return None;
    };
    let RepeatCount::Number(count) = repeat.count else {
        return None;
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
        TrackBreadth::Length(DimensionPercentage::Dimension(l))
            if length_value_px(l) == Some(0.0) =>
        {
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

/// `None` is a breadth gpui cannot express; the caller drops the whole list, since a
/// half-understood track list lays out worse than none.
fn track_breadth(b: &lightningcss::properties::grid::TrackBreadth) -> Option<gpui::GridTrackSize> {
    use gpui::GridTrackSize as G;
    use lightningcss::properties::grid::TrackBreadth as B;
    Some(match b {
        B::Auto => G::Auto,
        B::MinContent => G::MinContent,
        B::MaxContent => G::MaxContent,
        B::Flex(f) => G::Fr(*f),
        B::Length(DimensionPercentage::Dimension(l)) => G::Px(length_value_px(l)?),
        B::Length(DimensionPercentage::Percentage(p)) => G::Percent(p.0),
        B::Length(DimensionPercentage::Calc(c)) => G::Px(lp_calc_px(c)?),
    })
}

fn track_size(s: &lightningcss::properties::grid::TrackSize) -> Option<gpui::GridTrack> {
    use lightningcss::properties::grid::{TrackBreadth as B, TrackSize as S};
    match s {
        S::TrackBreadth(B::Flex(f)) => Some(gpui::GridTrack::fr(*f)),
        S::TrackBreadth(b) => Some(gpui::GridTrack::fixed(track_breadth(b)?)),
        S::MinMax { min, max } => Some(gpui::GridTrack {
            min: track_breadth(min)?,
            max: track_breadth(max)?,
        }),
        // Approximating with `auto` would silently change the layout, so the list is refused.
        S::FitContent(_) => None,
    }
}

/// Written with every list: a refinement cannot unset a field, so an earlier rule's auto-repeat
/// would otherwise outlive a later plain list.
const NO_AUTO_REPEAT: gpui::GridAutoRepeat = gpui::GridAutoRepeat {
    fit: false,
    start: 0,
    len: 0,
};

fn grid_tracks(
    v: &lightningcss::properties::grid::TrackSizing,
) -> Option<(Vec<gpui::GridTrack>, gpui::GridAutoRepeat)> {
    use lightningcss::properties::grid::{RepeatCount, TrackListItem, TrackSizing};
    let TrackSizing::TrackList(list) = v else {
        return None;
    };
    let mut out: Vec<gpui::GridTrack> = Vec::new();
    let mut auto: Option<gpui::GridAutoRepeat> = None;
    for item in &list.items {
        match item {
            TrackListItem::TrackSize(s) => out.push(track_size(s)?),
            TrackListItem::TrackRepeat(rep) => {
                let one: Vec<gpui::GridTrack> = rep
                    .track_sizes
                    .iter()
                    .map(track_size)
                    .collect::<Option<_>>()?;
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

fn fixed_track(t: &gpui::GridTrack) -> bool {
    use gpui::GridTrackSize as G;
    let fixed = |s: G| matches!(s, G::Px(_) | G::Percent(_));
    fixed(t.min) || (fixed(t.max) && !matches!(t.min, G::Fr(_)))
}

/// `None` = a NAMED line or area, which needs the named grid taffy does not model here.
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

fn outline_mut(r: &mut StyleRefinement) -> &mut gpui::Outline {
    r.vn_outline.get_or_insert(gpui::Outline {
        width: px(0.0),
        offset: px(0.0),
        color: Hsla::default(),
        style: gpui::BorderStyle::Solid,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMarker {
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
            if upper {
                out.to_uppercase()
            } else {
                out
            }
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

/// CSS matches a background geometry list to the image list by index and repeats the shorter one.
fn cycled<T: Copy>(list: Option<&Rc<Vec<T>>>, i: usize) -> Option<T> {
    let list = list.filter(|l| !l.is_empty())?;
    Some(list[i % list.len()])
}

#[derive(Clone, Debug, PartialEq)]
pub struct BackgroundUrl {
    pub index: u16,
    pub url: Rc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BackgroundSize {
    Explicit(Option<TileLength>, Option<TileLength>),
    Cover,
    Contain,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PaintLayer {
    Gradient(gpui::Background),
    Image {
        url: Rc<str>,
        size: BackgroundSize,
        position: (TileLength, TileLength),
        repeat: (bool, bool),
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BackgroundLayer {
    pub index: u16,
    pub background: gpui::Background,
}

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

#[derive(Clone, Copy, Debug)]
struct Stop {
    color: Hsla,
    position: Option<StopPosition>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum StopPosition {
    Fraction(f32),
    Pixels(f32),
}

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

    // Non-decreasing, per unit: comparing a fraction against a length would be comparing two
    // different quantities, so a change of unit resets the floor.
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

fn hint_stop(before: Hsla, after: Hsla, position: StopPosition) -> Stop {
    Stop {
        color: mix_hsla(before, after),
        position: Some(position),
    }
}

/// The midpoint of two colours, hue averaged the short way round so amber→yellow→amber does not
/// come back cyan.
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

fn stop_position_lp(v: &LengthPercentage) -> Option<StopPosition> {
    match v {
        DimensionPercentage::Percentage(p) => Some(StopPosition::Fraction(p.0)),
        DimensionPercentage::Dimension(d) => length_value_px(d).map(StopPosition::Pixels),
        DimensionPercentage::Calc(c) => lp_calc_px(c).map(StopPosition::Pixels),
    }
}

fn stop_position_angle(
    v: &DimensionPercentage<lightningcss::values::angle::Angle>,
) -> Option<StopPosition> {
    match v {
        DimensionPercentage::Percentage(p) => Some(StopPosition::Fraction(p.0)),
        DimensionPercentage::Dimension(a) => Some(StopPosition::Fraction(a.to_degrees() / 360.0)),
        DimensionPercentage::Calc(_) => {
            calc_dropped();
            None
        }
    }
}

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
                    // A length measured from the FAR side would need the box. `right 10px` is not a
                    // shape stylesheets write; the near side is, and it is exact.
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
    match (x_px, y_px) {
        (true, true) => ((x, y), true),
        (true, false) => ((0.0, y), false),
        (false, true) => ((x, 0.0), false),
        (false, false) => ((x, y), false),
    }
}

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

/// Splits a long stop list into bands, each with transparent hard-stop guards at shared edges: each
/// pixel is opaque in one band only, so there is no seam or double blend.
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

fn mask_clips<'a>(
    clips: impl Iterator<Item = &'a lightningcss::properties::masking::MaskClip>,
) -> MaskClips {
    use lightningcss::properties::masking::{GeometryBox as B, MaskClip as C};
    let clips: Vec<_> = clips.collect();
    match clips.as_slice() {
        [C::GeometryBox(B::ContentBox), C::GeometryBox(B::BorderBox)] => {
            MaskClips::ContentThenBorder
        }
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

fn webkit_mask_compositing(keyword: &str) -> MaskCompositing {
    match keyword.to_ascii_lowercase().as_str() {
        "source-over" => MaskCompositing::Add,
        "xor" | "destination-out" => MaskCompositing::Exclude,
        _ => MaskCompositing::Other,
    }
}

fn advise_mask(x: &Extras, diagnostics: &mut Vec<String>) {
    let unsupported = x.mask_images == Some(MaskImages::Other)
        || x.mask_composite == Some(MaskCompositing::Other)
        || (x.mask_images == Some(MaskImages::TwoOpaque) && x.mask_clips == Some(MaskClips::Other));
    if unsupported {
        warn_once(
            diagnostics,
            "`mask` is not applied (v1), and the element paints unmasked. gpui has no \
             per-element alpha mask. The one mask Skathi cuts is the gradient-border ring: \
             two opaque layers, the top one clipped to `content-box`, with `mask-composite: \
             exclude` (or `-webkit-mask-composite: xor`). That paints the background in the \
             padding and border only",
        );
    }
}

fn collect_background_images<'a>(
    images: impl Iterator<Item = &'a lightningcss::values::image::Image<'a>>,
    x: &mut Extras,
    diagnostics: &mut Vec<String>,
) {
    let mut layers: Vec<BackgroundLayer> = Vec::new();
    let mut urls: Vec<BackgroundUrl> = Vec::new();
    for (index, image) in images.enumerate() {
        if let lightningcss::values::image::Image::Url(u) = image {
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
    // Both still repeat, and both differ from `repeat` only by a sub-tile amount; treating them as
    // `repeat` is the closer of the two available answers.
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

fn gradient_layer(
    img: &lightningcss::values::image::Image<'_>,
    diagnostics: &mut Vec<String>,
) -> Option<(Vec<gpui::Background>, Hsla)> {
    use lightningcss::values::gradient::{
        Circle, Ellipse, EndingShape, Gradient, LineDirection, ShapeExtent,
    };
    use lightningcss::values::image::Image;
    use lightningcss::values::position::{
        HorizontalPositionKeyword as H, VerticalPositionKeyword as V,
    };

    let gradient = match img {
        Image::Gradient(g) => g.as_ref(),
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

    let (background, stops) = match gradient {
        Gradient::Linear(l) | Gradient::RepeatingLinear(l) => {
            let stops = collect_stops(&l.items, &stop_position_lp)?;
            let angle = match &l.direction {
                LineDirection::Angle(a) => Some(a.to_degrees()),
                LineDirection::Vertical(V::Top) => Some(0.0),
                LineDirection::Vertical(V::Bottom) => Some(180.0),
                LineDirection::Horizontal(H::Right) => Some(90.0),
                LineDirection::Horizontal(H::Left) => Some(270.0),
                LineDirection::Corner { .. } => None,
            };
            let background = match angle {
                Some(angle) => gpui::linear_gradient_stops(angle, &[]),
                None => {
                    let LineDirection::Corner {
                        horizontal,
                        vertical,
                    } = &l.direction
                    else {
                        unreachable!("only a corner direction leaves the angle unresolved")
                    };
                    gpui::linear_gradient_stops(0.0, &[]).to_corner(match (horizontal, vertical) {
                        (H::Right, V::Top) => 0,
                        (H::Right, V::Bottom) => 1,
                        (H::Left, V::Bottom) => 2,
                        (H::Left, V::Top) => 3,
                    })
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
                        DimensionPercentage::Calc(c) => lp_calc_px(c).map(|v| (v, true)),
                    };
                    match (axis(x), axis(y)) {
                        // Both axes have to agree on the unit, because one flag covers the pair.
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

pub(crate) fn apply_current_color(
    r: &mut StyleRefinement,
    targets: CurrentColorTargets,
    color: Hsla,
) {
    let sides = [
        r.vn_border_colors.top,
        r.vn_border_colors.right,
        r.vn_border_colors.bottom,
        r.vn_border_colors.left,
    ];
    if sides.iter().all(|side| *side == Some(None))
        || (targets.has(CurrentColorTargets::BORDER)
            && r.border_color.is_none()
            && sides.iter().all(Option::is_none))
    {
        r.border_color = Some(color);
    }
    // A currentColor placeholder belongs to this refinement, not every state that contributed to
    // Extras.
    for slot in [
        &mut r.vn_border_colors.top,
        &mut r.vn_border_colors.right,
        &mut r.vn_border_colors.bottom,
        &mut r.vn_border_colors.left,
    ] {
        if *slot == Some(None) {
            *slot = Some(Some(color));
        }
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
    [
        r.vn_border_colors.top,
        r.vn_border_colors.right,
        r.vn_border_colors.bottom,
        r.vn_border_colors.left,
    ]
    .contains(&Some(None))
}

fn border_side_color(
    slot: &mut Option<Option<Hsla>>,
    bit: u16,
    value: &CssColor,
    extras: &mut Extras,
    diagnostics: &mut Vec<String>,
) {
    let resolved = match value {
        CssColor::CurrentColor => None,
        value => match color(value) {
            Some(color) => Some(color),
            None => {
                unresolved_color(diagnostics, "border-color");
                return;
            }
        },
    };
    *slot = Some(resolved);
    extras.current_color_overrides.0 |= bit | CurrentColorTargets::BORDER;
    extras.current_color.0 &= !(bit | CurrentColorTargets::BORDER);
    if matches!(value, CssColor::CurrentColor) {
        extras.current_color.0 |= bit;
    }
}

fn all_border_colors(
    r: &mut StyleRefinement,
    x: &mut Extras,
    colors: [&CssColor; 4],
    diagnostics: &mut Vec<String>,
) {
    border_side_color(
        &mut r.vn_border_colors.top,
        CurrentColorTargets::BORDER_TOP,
        colors[0],
        x,
        diagnostics,
    );
    border_side_color(
        &mut r.vn_border_colors.right,
        CurrentColorTargets::BORDER_RIGHT,
        colors[1],
        x,
        diagnostics,
    );
    border_side_color(
        &mut r.vn_border_colors.bottom,
        CurrentColorTargets::BORDER_BOTTOM,
        colors[2],
        x,
        diagnostics,
    );
    border_side_color(
        &mut r.vn_border_colors.left,
        CurrentColorTargets::BORDER_LEFT,
        colors[3],
        x,
        diagnostics,
    );
    r.border_color = color(colors[0]);
    if colors
        .iter()
        .all(|value| matches!(value, CssColor::CurrentColor))
    {
        x.current_color.0 |= CurrentColorTargets::BORDER;
    }
}

fn unresolved_color(diagnostics: &mut Vec<String>, what: &str) {
    once(
        diagnostics,
        format!(
            "`{what}` value does not resolve to a concrete color (v1) — `light-dark()` and \
             system colors (`Canvas`, `ButtonText`, …) are not computed; write the color, and \
             switch it with `@media (prefers-color-scheme: dark)` where it must follow the theme"
        ),
    );
}

#[path = "effect.rs"]
pub(crate) mod effect;

fn object_position(p: &Property, diagnostics: &mut Vec<String>) -> Option<gpui::ObjectPosition> {
    use lightningcss::traits::Parse;
    use lightningcss::values::position::{Position, PositionComponent as C};
    fn axis<S: PositionSide + Copy>(c: &C<S>) -> Option<(f32, f32)> {
        let linear = |v: &LengthPercentage| match v {
            DimensionPercentage::Dimension(d) => Some((0.0, length_value_px(d)?)),
            DimensionPercentage::Percentage(p) => Some((p.0, 0.0)),
            DimensionPercentage::Calc(c) => calc_linear(c),
        };
        match c {
            C::Center => Some((0.5, 0.0)),
            C::Length(v) => linear(v),
            C::Side { side, offset } => {
                let (f, px) = offset.as_ref().map_or(Some((0.0, 0.0)), linear)?;
                Some(if side.is_near_edge() {
                    (f, px)
                } else {
                    (1.0 - f, -px)
                })
            }
        }
    }
    let raw = value_text(p)?;
    let resolved = Position::parse_string(&raw)
        .ok()
        .and_then(|pos| Some((axis(&pos.x)?, axis(&pos.y)?)));
    let Some(((x_fraction, x_offset), (y_fraction, y_offset))) = resolved else {
        once(
            diagnostics,
            format!(
                "unsupported value for object-position: {raw} — expected one to four keywords, \
                 lengths or percentages, as in `left 10px top 20%`"
            ),
        );
        return None;
    };
    Some(gpui::ObjectPosition {
        x_fraction,
        x_offset: px(x_offset),
        y_fraction,
        y_offset: px(y_offset),
    })
}

fn object_fit(
    value: &lightningcss::properties::custom::TokenList,
    diagnostics: &mut Vec<String>,
) -> Option<ObjectFit> {
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

pub(crate) const WINDOW_CONTROL: &str = "--window-control";

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

fn clip_path(v: &CssClipPath<'_>, diagnostics: &mut Vec<String>) -> Option<gpui::VnClipPath> {
    use gpui::{VnClipBox, VnClipPath, VnShapeRadius};
    use lightningcss::properties::masking::GeometryBox as G;
    use lightningcss::values::shape::{BasicShape, ShapeRadius};
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
        CssClipPath::Shape {
            shape,
            reference_box,
        } => {
            let reference = reference(reference_box);
            match &**shape {
                BasicShape::Inset(inset) => {
                    let r = &inset.rect;
                    let insets = [
                        lp_definite(&r.0)?,
                        lp_definite(&r.1)?,
                        lp_definite(&r.2)?,
                        lp_definite(&r.3)?,
                    ];
                    let corner = |c: &lightningcss::values::size::Size2D<LengthPercentage>| {
                        Some(gpui::VnCornerRadius {
                            x: lp_definite(&c.0)?,
                            y: lp_definite(&c.1)?,
                        })
                    };
                    let b = &inset.radius;
                    let radii = [
                        corner(&b.top_left)?,
                        corner(&b.top_right)?,
                        corner(&b.bottom_right)?,
                        corner(&b.bottom_left)?,
                    ];
                    Some(VnClipPath::Inset {
                        reference,
                        insets,
                        radii,
                    })
                }
                BasicShape::Circle(c) => {
                    let r = radius(&c.radius)?;
                    let center = shape_center(&c.position)?;
                    Some(VnClipPath::Ellipse {
                        reference,
                        center,
                        radius: [r, r],
                        circle: true,
                    })
                }
                BasicShape::Ellipse(e) => {
                    let (rx, ry) = (radius(&e.radius_x)?, radius(&e.radius_y)?);
                    let center = shape_center(&e.position)?;
                    Some(VnClipPath::Ellipse {
                        reference,
                        center,
                        radius: [rx, ry],
                        circle: false,
                    })
                }
                BasicShape::Polygon(polygon) => match clip_path_polygon(polygon) {
                    Some(points) => Some(VnClipPath::Polygon {
                        reference,
                        fill_rule: fill_rule(polygon.fill_rule),
                        points: points.into(),
                    }),
                    None => unsupported(
                        diagnostics,
                        "polygon() with a vertex that is no length or percentage",
                    ),
                },
            }
        }
    }
}

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

fn clip_path_polygon(
    polygon: &lightningcss::values::shape::Polygon,
) -> Option<Vec<[DefiniteLength; 2]>> {
    use lightningcss::traits::{Parse, ToCss};
    polygon
        .points
        .iter()
        .map(|point| {
            let text = point
                .to_css_string(lightningcss::stylesheet::PrinterOptions::default())
                .ok()?;
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

fn clip_path_path(
    tokens: &lightningcss::properties::custom::TokenList<'_>,
) -> Option<Result<gpui::VnClipPath, String>> {
    use gpui::{VnClipBox, VnFillRule};
    use lightningcss::properties::custom::{Token, TokenOrValue};
    let mut path = None;
    let mut reference = None;
    for t in &tokens.0 {
        match t {
            TokenOrValue::Function(f)
                if f.name.0.as_ref().eq_ignore_ascii_case("path") && path.is_none() =>
            {
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
            TokenOrValue::Token(Token::String(d))
                if data.is_none() && (fill_rule.is_none() || comma) =>
            {
                data = Some(d.as_ref().to_string());
            }
            _ => {
                return Some(Err(
                    "`path()` takes an optional fill rule, a comma and a string".into(),
                ))
            }
        }
    }
    let Some(data) = data else {
        return Some(Err("`path()` has no path string".into()));
    };
    Some(
        crate::path_data::parse(&data).map(|verbs| gpui::VnClipPath::Path {
            reference: reference.unwrap_or_default(),
            fill_rule: fill_rule.unwrap_or_default(),
            verbs: verbs.into(),
        }),
    )
}

fn shape_center(
    position: &lightningcss::values::position::Position,
) -> Option<[DefiniteLength; 2]> {
    use lightningcss::values::position::PositionComponent as P;
    fn component<S: PositionSide + Copy>(c: &P<S>) -> Option<DefiniteLength> {
        match c {
            P::Center => Some(relative(0.5)),
            P::Length(lp) => lp_definite(lp),
            P::Side { side, offset } => {
                let near = side.is_near_edge();
                let Some(offset) = offset else {
                    return Some(relative(if near { 0.0 } else { 1.0 }));
                };
                let offset = lp_definite(offset)?;
                if near {
                    return Some(offset);
                }
                match offset {
                    DefiniteLength::Fraction(f) => Some(relative(1.0 - f)),
                    DefiniteLength::Absolute(AbsoluteLength::Pixels(p)) => {
                        Some(DefiniteLength::Calc {
                            fraction: 1.0,
                            pixels: -p,
                        })
                    }
                    DefiniteLength::Calc { fraction, pixels } => Some(DefiniteLength::Calc {
                        fraction: 1.0 - fraction,
                        pixels: -pixels,
                    }),
                    DefiniteLength::Absolute(AbsoluteLength::Rems(_)) => None,
                }
            }
        }
    }
    Some([component(&position.x)?, component(&position.y)?])
}

fn clip_rect_is_empty(value: &lightningcss::properties::custom::TokenList<'_>) -> bool {
    use lightningcss::properties::custom::{Token, TokenOrValue};

    fn scan(
        tokens: &[lightningcss::properties::custom::TokenOrValue<'_>],
        saw_rect: &mut bool,
        numbers: &mut Vec<f32>,
    ) {
        for t in tokens {
            match t {
                // lightningcss keeps a function's arguments NESTED rather than flat, so `rect(0, 0,
                // 0, 0)` is one `Function` whose arguments are the four numbers.
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
    saw_rect
        && numbers.len() == 4
        && ((numbers[0] - numbers[2]).abs() < f32::EPSILON
            || (numbers[3] - numbers[1]).abs() < f32::EPSILON)
}

fn is_css_wide_keyword(value: &lightningcss::properties::custom::TokenList<'_>) -> bool {
    single_ident_raw(value).is_some_and(|v| {
        matches!(
            v.to_ascii_lowercase().as_str(),
            "inherit" | "initial" | "unset" | "revert" | "revert-layer"
        )
    })
}

fn single_ident_is(
    value: &lightningcss::properties::custom::TokenList<'_>,
    expected: &str,
) -> bool {
    single_ident_raw(value).is_some_and(|v| v.eq_ignore_ascii_case(expected))
}

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
            let TokenOrValue::Token(Token::Ident(name)) = t else {
                continue;
            };
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
    if property == "font-kerning" {
        return match single_ident_raw(value)?.to_ascii_lowercase().as_str() {
            "auto" | "normal" => Some(gpui::FontFeatures::default()),
            "none" => Some(gpui::FontFeatures(std::sync::Arc::new(vec![(
                "kern".into(),
                0,
            )]))),
            _ => None,
        };
    }
    if property == "font-variant-ligatures" {
        for t in value.0.iter() {
            let TokenOrValue::Token(Token::Ident(name)) = t else {
                continue;
            };
            let set: &[(&str, u32)] = match name.as_ref() {
                "normal" => continue,
                "none" => &[
                    ("liga", 0),
                    ("clig", 0),
                    ("dlig", 0),
                    ("hlig", 0),
                    ("calt", 0),
                ],
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
                    other => match font_unit_px(other, *value)
                        .or_else(|| viewport_unit_px(other, *value))
                    {
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

fn has_substitution(value: &lightningcss::properties::custom::TokenList<'_>) -> bool {
    use lightningcss::properties::custom::{TokenOrValue, UnresolvedColor};
    value.0.iter().any(|token| match token {
        TokenOrValue::Var(_) | TokenOrValue::Env(_) => true,
        TokenOrValue::Function(function) => has_substitution(&function.arguments),
        TokenOrValue::UnresolvedColor(
            UnresolvedColor::RGB { alpha, .. } | UnresolvedColor::HSL { alpha, .. },
        ) => has_substitution(alpha),
        TokenOrValue::UnresolvedColor(UnresolvedColor::LightDark { light, dark }) => {
            has_substitution(light) || has_substitution(dark)
        }
        _ => false,
    })
}

/// `color-mix(in srgb, currentColor 40%, red)` and `oklch(from currentColor l c h)` depend on the
/// element's computed `color`, so they are resolved per element like a `var()`.
fn has_current_color_function(value: &lightningcss::properties::custom::TokenList<'_>) -> bool {
    use lightningcss::properties::custom::{Token, TokenOrValue};
    fn mentions(value: &lightningcss::properties::custom::TokenList<'_>) -> bool {
        value.0.iter().any(|token| match token {
            TokenOrValue::Token(Token::Ident(id)) => id.eq_ignore_ascii_case("currentcolor"),
            TokenOrValue::Color(lightningcss::values::color::CssColor::CurrentColor) => true,
            TokenOrValue::Function(f) => mentions(&f.arguments),
            _ => false,
        })
    }
    value.0.iter().any(|token| match token {
        TokenOrValue::Function(f) => mentions(&f.arguments),
        _ => false,
    })
}

enum ContentValue {
    Text(Rc<str>),
    None,
    Unsupported(String),
}

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

fn single_ident_raw<'a>(
    value: &'a lightningcss::properties::custom::TokenList<'_>,
) -> Option<&'a str> {
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

/// Unknown properties arrive as a raw token list, so there is no typed `CssColor` for `red`;
/// re-parse it as one.
fn named_color(ident: &str) -> Option<Hsla> {
    match Property::parse_string(PropertyId::from("color"), ident, ParserOptions::default()) {
        Ok(Property::Color(c)) => color(&c),
        _ => None,
    }
}

fn scrollbar_color(
    value: &lightningcss::properties::custom::TokenList,
    diagnostics: &mut Vec<String>,
) -> Option<(Hsla, Hsla)> {
    use lightningcss::properties::custom::{
        CustomProperty, CustomPropertyName, Token, TokenOrValue,
    };
    let tokens: Vec<_> = value
        .0
        .iter()
        .filter(|token| !token.is_whitespace())
        .collect();
    if matches!(tokens.as_slice(), [TokenOrValue::Token(Token::Ident(id))] if id.as_ref().eq_ignore_ascii_case("auto"))
    {
        return None;
    }
    let property = Property::Custom(CustomProperty {
        name: CustomPropertyName::Unknown(lightningcss::values::ident::Ident(
            "scrollbar-color".into(),
        )),
        value: value.clone(),
    });
    if tokens.len() == 2 {
        if let Some(raw) = value_text(&property) {
            if let Ok(Property::BorderColor(pair)) = Property::parse_string(
                PropertyId::from("border-color"),
                &raw,
                ParserOptions::default(),
            ) {
                if let (Some(thumb), Some(track)) = (color(&pair.top), color(&pair.right)) {
                    return Some((thumb, track));
                }
            }
        }
    }
    once(
        diagnostics,
        "`scrollbar-color` requires auto or two valid colours (thumb then track)".to_string(),
    );
    None
}

/// `contain` as (paint containment, size-contained axes): layout and style containment change
/// nothing visible here.
fn contain(
    value: &lightningcss::properties::custom::TokenList,
    diagnostics: &mut Vec<String>,
) -> Option<(bool, u8)> {
    use lightningcss::properties::custom::{Token, TokenOrValue};
    let (mut paint, mut size) = (false, 0);
    for token in value.0.iter().filter(|t| !t.is_whitespace()) {
        let TokenOrValue::Token(Token::Ident(id)) = token else {
            unsupported(diagnostics, "contain");
            return None;
        };
        match id.to_ascii_lowercase().as_str() {
            "none" | "layout" | "style" => {}
            "paint" | "content" => paint = true,
            "strict" => (paint, size) = (true, Extras::CONTAIN_WIDTH | Extras::CONTAIN_HEIGHT),
            "size" => size = Extras::CONTAIN_WIDTH | Extras::CONTAIN_HEIGHT,
            "inline-size" => size |= Extras::CONTAIN_WIDTH,
            _ => {
                unsupported(diagnostics, "contain");
                return None;
            }
        }
    }
    Some((paint, size))
}

/// Size containment sizes a box as if it were empty, which only shows on an axis the box does not
/// size itself.
pub(crate) fn advise_contain_size(r: &StyleRefinement, x: &Extras, diagnostics: &mut Vec<String>) {
    let axes = x.contain_size.unwrap_or(0);
    let auto = |l: Option<Length>| matches!(l, None | Some(Length::Auto));
    if (axes & Extras::CONTAIN_WIDTH != 0 && auto(r.size.width))
        || (axes & Extras::CONTAIN_HEIGHT != 0 && auto(r.size.height))
    {
        warn_once(
            diagnostics,
            "`contain: size` (or `strict`, `inline-size`) does not size the box as if it were \
             empty (v1) — an axis without a width or height is still sized by the content; give \
             the box an explicit size on that axis",
        );
    }
}

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
            // Reset on every `display` declaration, so a later `display: flex` un-does an earlier
            // `display: inline-flex` rather than leaving the flag behind (`refine` is later-wins
            // per FIELD).
            x.inline_level = Some(matches!(
                d,
                D::Pair(p) if matches!(p.outside, DisplayOutside::Inline)
            ));
            r.display = Some(match d {
                D::Keyword(DisplayKeyword::None) => gpui::Display::None,
                D::Pair(pair) => match pair.inside {
                    DisplayInside::Flex(_) => gpui::Display::Flex,
                    DisplayInside::Grid => gpui::Display::Grid,
                    _ => gpui::Display::Block,
                },
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
        Order(v, _) => r.vn_order = Some(*v),
        FlexBasis(v, _) => r.flex_basis = lpa_length(v),
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
                lightningcss::properties::align::AlignItems::BaselinePosition(_) => {
                    Some(G::Baseline)
                }
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
                // `self-auto`: inherit the parent's align-items. gpui spells that as "unset", and a
                // refinement cannot express "clear" — so leave the field alone rather than pin it
                // to Stretch.
                A::Auto => None,
                A::Stretch => Some(gpui::AlignItems::Stretch),
                A::BaselinePosition(_) => Some(gpui::AlignItems::Baseline),
                A::SelfPosition { value, .. } => Some(self_position(value)),
                A::Normal => None,
            };
            match mapped {
                Some(m) => r.align_self = Some(m),
                // `auto`/`normal` mean "defer to the parent", which is already what an unset
                // refinement does — but say so, because a rule that MEANT to undo an earlier
                // `self-center` will not.
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
                lightningcss::properties::align::JustifyContent::ContentPosition {
                    value, ..
                } => match value {
                    ContentPosition::Center => Some(G::Center),
                    ContentPosition::Start | ContentPosition::FlexStart => Some(G::FlexStart),
                    ContentPosition::End | ContentPosition::FlexEnd => Some(G::FlexEnd),
                },
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
                // `fixed` becomes taffy's one out-of-flow mode, resolved against the parent, plus a
                // deferred layer; this matches when the parent is the window (portalled overlays)
                // and diverges inside a smaller ancestor. The renderer advises, since only it sees
                // ancestors.
                P::Fixed => gpui::Position::Absolute,
                P::Static | P::Relative => gpui::Position::Relative,
                _ => gpui::Position::Relative,
            });
        }
        // Recorded as written: `auto` is not `0`, because only an integer makes a stacking context.
        ZIndex(v) => {
            use lightningcss::properties::position::ZIndex as Z;
            match v {
                Z::Integer(n) => x.z_index = Some(Some(*n)),
                Z::Auto => x.z_index = Some(None),
            }
        }
        Transform(list, _) => apply_transform(list, r, diagnostics),
        // Seeded with the initial `50% 50%` so declaring the origin alone does not spell
        // `transform: none`.
        TransformOrigin(pos, _) => {
            use lightningcss::values::position::{
                HorizontalPositionKeyword as H, PositionComponent as P,
                VerticalPositionKeyword as V,
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
        // `-webkit-line-clamp` is honoured on its own (below, in the unknown-property bucket where
        // lightningcss leaves it), so these are genuinely inert rather than a missing feature.
        BoxOrient(..) | BoxAlign(..) | BoxPack(..) => {}
        // gpui draws one unbroken rule, so `none` is what it already does and only `auto` diverges.
        TextDecorationSkipInk(v, _) => {
            use lightningcss::properties::text::TextDecorationSkipInk as Skip;
            if !matches!(v, Skip::None) {
                warn_once(
                    diagnostics,
                    "`text-decoration-skip-ink: auto` is not applied (v1) — gpui draws one                      unbroken underline and does not lift it around descenders. `none` (draw it                      straight through) is what it already does",
                );
            }
        }
        // Colour, style and thickness live on gpui's decoration, so they land here only if this
        // block produced a line; otherwise `Extras::decoration` applies them after the cascade.
        TextDecorationColor(c, _) => match color(c) {
            Some(v) => {
                x.decoration.color = Some(Some(v));
                let mut hit = false;
                if let Some(u) = r.text.underline.as_mut() {
                    u.color = Some(v);
                    hit = true;
                }
                if let Some(s) = r.text.strikethrough.as_mut() {
                    s.color = Some(v);
                    hit = true;
                }
                let _ = hit;
            }
            None => unresolved_color(diagnostics, "text-decoration-color"),
        },
        TextDecorationStyle(s, _) => {
            use lightningcss::properties::text::TextDecorationStyle as S;
            x.decoration.wavy = Some(matches!(s, S::Wavy));
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
                if let Some(w) = length_value_px(l) {
                    x.decoration.thickness = Some(w);
                    if let Some(u) = r.text.underline.as_mut() {
                        u.thickness = px(w);
                    }
                }
            }
        }
        // `text-decoration` shorthand: the line half is the part that renders, and it must run
        // BEFORE colour/style so they have something to land on.
        TextDecoration(d, _) => {
            use lightningcss::properties::text::TextDecorationLine as TDL;
            if d.line.contains(TDL::Underline) {
                r.text.underline = Some(gpui::UnderlineStyle::default());
            }
            if d.line.contains(TDL::LineThrough) {
                r.text.strikethrough = Some(gpui::StrikethroughStyle::default());
            }
            x.decoration.color = Some(color(&d.color));
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
        Fill(p) => x.fill = Some(svg_paint(p, "fill", diagnostics)),
        Stroke(p) => x.stroke = Some(svg_paint(p, "stroke", diagnostics)),
        FillOpacity(_) | FillRule(_) | StrokeOpacity(_) | StrokeWidth(_) | StrokeLinecap(_)
        | StrokeLinejoin(_) | StrokeMiterlimit(_) | StrokeDasharray(_) | StrokeDashoffset(_)
        | ShapeRendering(_) => {
            if let (Some(name), Some(value)) = (presentation_name(p), value_text(p)) {
                let value = presentation_value(name, value);
                let list = x.svg_presentation.get_or_insert_with(Default::default);
                set_presentation(Rc::make_mut(list), name, value.into());
            }
        }
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
        // One axis each: the other axis's computed value depends on this one, known only after the
        // cascade (`compute_overflow`).
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
                V::Collapse => gpui::Visibility::Hidden,
            });
        }
        AspectRatio(v) => match &v.ratio {
            Some(ratio) if !v.auto && ratio.1 != 0.0 => r.aspect_ratio = Some(ratio.0 / ratio.1),
            // `aspect-auto` / `auto <ratio>`: a refinement cannot say "clear", so an earlier ratio
            // survives.
            _ => unsupported(
                diagnostics,
                "aspect-ratio: auto — cannot undo an earlier ratio",
            ),
        },
        Opacity(v) => r.opacity = Some(v.0),
        // `color: currentColor` IS `color: inherit` — the property resolves against itself — and
        // inheriting is what not writing the field does.
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
            if let Some(last) = layers.last() {
                if let Some(c) = color(&last.color) {
                    r.background = Some(gpui::Fill::Color(c.into()));
                    x.background_key_color = Some(c);
                }
            }
            collect_background_images(layers.iter().map(|l| &l.image), x, diagnostics);
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
                layers
                    .iter()
                    .map(|l| background_repeat(&l.repeat))
                    .collect(),
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
        Mask(layers, _) => {
            x.mask_images = Some(mask_images(layers.iter().map(|l| &l.image)));
            x.mask_clips = Some(mask_clips(layers.iter().map(|l| &l.clip)));
            x.mask_composite = Some(
                layers
                    .first()
                    .map_or(MaskCompositing::Add, |l| mask_compositing(l.composite)),
            );
            advise_mask(x, diagnostics);
        }
        MaskImage(images, _) => {
            x.mask_images = Some(mask_images(images.iter()));
            advise_mask(x, diagnostics);
        }
        MaskClip(clips, _) => x.mask_clips = Some(mask_clips(clips.iter())),
        MaskComposite(list) => {
            x.mask_composite = Some(
                list.first()
                    .map_or(MaskCompositing::Add, |c| mask_compositing(*c)),
            );
            advise_mask(x, diagnostics);
        }
        WebKitMaskComposite(list) => {
            use lightningcss::traits::ToCss;
            let first = list
                .first()
                .and_then(|c| c.to_css_string(Default::default()).ok());
            x.mask_composite = Some(webkit_mask_compositing(
                first.as_deref().unwrap_or("source-over"),
            ));
            advise_mask(x, diagnostics);
        }
        BorderColor(v) => {
            all_border_colors(r, x, [&v.top, &v.right, &v.bottom, &v.left], diagnostics)
        }
        Border(b) => {
            if let Some(w) = border_width_px(&b.width) {
                let w: AbsoluteLength = px(w).into();
                r.border_widths.top = Some(w);
                r.border_widths.right = Some(w);
                r.border_widths.bottom = Some(w);
                r.border_widths.left = Some(w);
            }
            all_border_colors(r, x, [&b.color; 4], diagnostics);
            set_border_style(r, b.style, SIDES_ALL, diagnostics);
        }
        BorderWidth(v) => {
            r.border_widths.top = border_width_px(&v.top).map(|w| px(w).into());
            r.border_widths.right = border_width_px(&v.right).map(|w| px(w).into());
            r.border_widths.bottom = border_width_px(&v.bottom).map(|w| px(w).into());
            r.border_widths.left = border_width_px(&v.left).map(|w| px(w).into());
        }
        BorderTopColor(c) => border_side_color(
            &mut r.vn_border_colors.top,
            CurrentColorTargets::BORDER_TOP,
            c,
            x,
            diagnostics,
        ),
        BorderRightColor(c) => border_side_color(
            &mut r.vn_border_colors.right,
            CurrentColorTargets::BORDER_RIGHT,
            c,
            x,
            diagnostics,
        ),
        BorderBottomColor(c) => border_side_color(
            &mut r.vn_border_colors.bottom,
            CurrentColorTargets::BORDER_BOTTOM,
            c,
            x,
            diagnostics,
        ),
        BorderLeftColor(c) => border_side_color(
            &mut r.vn_border_colors.left,
            CurrentColorTargets::BORDER_LEFT,
            c,
            x,
            diagnostics,
        ),
        BorderTop(b) => {
            r.border_widths.top = border_width_px(&b.width).map(|w| px(w).into());
            border_side_color(
                &mut r.vn_border_colors.top,
                CurrentColorTargets::BORDER_TOP,
                &b.color,
                x,
                diagnostics,
            );
            set_border_style(r, b.style, SIDE_TOP, diagnostics);
        }
        BorderRight(b) => {
            r.border_widths.right = border_width_px(&b.width).map(|w| px(w).into());
            border_side_color(
                &mut r.vn_border_colors.right,
                CurrentColorTargets::BORDER_RIGHT,
                &b.color,
                x,
                diagnostics,
            );
            set_border_style(r, b.style, SIDE_RIGHT, diagnostics);
        }
        BorderBottom(b) => {
            r.border_widths.bottom = border_width_px(&b.width).map(|w| px(w).into());
            border_side_color(
                &mut r.vn_border_colors.bottom,
                CurrentColorTargets::BORDER_BOTTOM,
                &b.color,
                x,
                diagnostics,
            );
            set_border_style(r, b.style, SIDE_BOTTOM, diagnostics);
        }
        BorderLeft(b) => {
            r.border_widths.left = border_width_px(&b.width).map(|w| px(w).into());
            border_side_color(
                &mut r.vn_border_colors.left,
                CurrentColorTargets::BORDER_LEFT,
                &b.color,
                x,
                diagnostics,
            );
            set_border_style(r, b.style, SIDE_LEFT, diagnostics);
        }
        BorderTopWidth(v) => r.border_widths.top = border_width_px(v).map(|w| px(w).into()),
        BorderRightWidth(v) => r.border_widths.right = border_width_px(v).map(|w| px(w).into()),
        BorderBottomWidth(v) => r.border_widths.bottom = border_width_px(v).map(|w| px(w).into()),
        BorderLeftWidth(v) => r.border_widths.left = border_width_px(v).map(|w| px(w).into()),
        BorderStyle(v) => {
            set_border_style(r, v.left, SIDE_LEFT, diagnostics);
            set_border_style(r, v.bottom, SIDE_BOTTOM, diagnostics);
            set_border_style(r, v.right, SIDE_RIGHT, diagnostics);
            set_border_style(r, v.top, SIDE_TOP, diagnostics);
        }
        BorderTopStyle(v) => set_border_style(r, *v, SIDE_TOP, diagnostics),
        BorderRightStyle(v) => set_border_style(r, *v, SIDE_RIGHT, diagnostics),
        BorderBottomStyle(v) => set_border_style(r, *v, SIDE_BOTTOM, diagnostics),
        BorderLeftStyle(v) => set_border_style(r, *v, SIDE_LEFT, diagnostics),
        BorderRadius(v, _) => {
            let mut wrote = 0;
            if corner(
                &mut r.corner_radii.top_left,
                &mut r.vn_corner_radii.top_left,
                &v.top_left,
            ) {
                wrote |= RadiusInherit::TOP_LEFT;
            }
            if corner(
                &mut r.corner_radii.top_right,
                &mut r.vn_corner_radii.top_right,
                &v.top_right,
            ) {
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
        BorderTopLeftRadius(v, _) => {
            if corner(
                &mut r.corner_radii.top_left,
                &mut r.vn_corner_radii.top_left,
                v,
            ) {
                x.radius_inherit.write(RadiusInherit::TOP_LEFT, false);
            }
        }
        BorderTopRightRadius(v, _) => {
            if corner(
                &mut r.corner_radii.top_right,
                &mut r.vn_corner_radii.top_right,
                v,
            ) {
                x.radius_inherit.write(RadiusInherit::TOP_RIGHT, false);
            }
        }
        BorderBottomRightRadius(v, _) => {
            if corner(
                &mut r.corner_radii.bottom_right,
                &mut r.vn_corner_radii.bottom_right,
                v,
            ) {
                x.radius_inherit.write(RadiusInherit::BOTTOM_RIGHT, false);
            }
        }
        BorderBottomLeftRadius(v, _) => {
            if corner(
                &mut r.corner_radii.bottom_left,
                &mut r.vn_corner_radii.bottom_left,
                v,
            ) {
                x.radius_inherit.write(RadiusInherit::BOTTOM_LEFT, false);
            }
        }
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
        GridColumn(v) => match (grid_line(&v.start), grid_line(&v.end)) {
            (Some(s), Some(e)) => {
                let loc = r.grid_location.get_or_insert_default();
                loc.column = s..e;
            }
            _ => unsupported(
                diagnostics,
                "grid-column with a NAMED line or area — use line numbers or `span N`",
            ),
        },
        GridRow(v) => match (grid_line(&v.start), grid_line(&v.end)) {
            (Some(s), Some(e)) => {
                let loc = r.grid_location.get_or_insert_default();
                loc.row = s..e;
            }
            _ => unsupported(
                diagnostics,
                "grid-row with a NAMED line or area — use line numbers or `span N`",
            ),
        },
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
        GridAutoColumns(list) | GridAutoRows(list) => {
            match list.0.iter().map(track_size).collect::<Option<Vec<_>>>() {
                Some(tracks) => {
                    if matches!(p, GridAutoColumns(_)) {
                        r.vn_grid_auto_cols = Some(tracks);
                    } else {
                        r.vn_grid_auto_rows = Some(tracks);
                    }
                }
                None => unsupported(
                    diagnostics,
                    "grid-auto-columns / grid-auto-rows — fit-content() and percentage-dependent calc() \
                     track sizes are not modelled; use lengths, percentages, auto, min-content, \
                     max-content, flex, or minmax()",
                ),
            }
        }
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
            let style = out.style;
            // `outline: none` is the whole declaration: there is no width to read and no colour to
            // resolve (the shorthand's is the initial `currentColor`, which would advise for
            // nothing).
            if matches!(&o.style, OS::LineStyle(LineStyle::None | LineStyle::Hidden)) {
                x.outline.hidden = Some(true);
            } else {
                x.outline.hidden = Some(false);
                x.outline.style = Some(style);
                if let Some(w) = border_width_px(&o.width) {
                    r.vn_outline.as_mut().unwrap().width = px(w);
                    x.outline.width = Some(w);
                }
                match &o.color {
                    CssColor::CurrentColor => {
                        x.current_color.0 |= CurrentColorTargets::OUTLINE;
                        x.outline.color = Some(None);
                    }
                    c => match color(c) {
                        Some(c) => {
                            r.vn_outline.as_mut().unwrap().color = c;
                            x.outline.color = Some(Some(c));
                        }
                        None => unresolved_color(diagnostics, "outline-color"),
                    },
                }
            }
        }
        OutlineWidth(w) => {
            if let Some(v) = border_width_px(w) {
                outline_mut(r).width = px(v);
                x.outline.width = Some(v);
            }
        }
        OutlineColor(CssColor::CurrentColor) => {
            outline_mut(r);
            x.current_color.0 |= CurrentColorTargets::OUTLINE;
            x.outline.color = Some(None);
        }
        OutlineColor(c) => match color(c) {
            Some(v) => {
                outline_mut(r).color = v;
                x.outline.color = Some(Some(v));
            }
            None => unresolved_color(diagnostics, "outline-color"),
        },
        OutlineStyle(s) => {
            use lightningcss::properties::outline::OutlineStyle as OS;
            let out = outline_mut(r);
            match s {
                OS::LineStyle(LineStyle::None) | OS::LineStyle(LineStyle::Hidden) => {
                    out.width = px(0.0);
                    x.outline.hidden = Some(true);
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
                    x.outline.hidden = Some(false);
                    x.outline.style = Some(out.style);
                    if out.width == px(0.0) {
                        out.width = px(3.0);
                    }
                }
            }
        }
        BoxShadow(shadows, _) => {
            let mut out = Vec::new();
            for s in shadows.iter() {
                let (Some(c), Some(x), Some(y)) = (
                    color(&s.color),
                    css_length_px(&s.x_offset),
                    css_length_px(&s.y_offset),
                ) else {
                    // One unresolvable shadow drops ITSELF, not the list — but it does not drop in
                    // silence, because a missing ring in a stack of three is invisible to read
                    // back.
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
            use lightningcss::properties::font::{FontSize as FS, RelativeFontSize as R};
            let size = match v {
                FS::Length(DimensionPercentage::Percentage(p)) => Some(p.0 * em_basis()),
                FS::Length(DimensionPercentage::Calc(c)) => match calc_px(c, &lp_leaf_px) {
                    Some(p) => Some(p),
                    None => calc_linear(c).map(|(fraction, pixels)| fraction * em_basis() + pixels),
                },
                FS::Length(lp) => lp_absolute(lp).map(font_size_px),
                FS::Absolute(k) => Some(absolute_font_size_px(k)),
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
                // A number inherits as the number: gpui's fraction is taken of each element's own
                // font size at paint.
                CssLineHeight::Number(n) => Some(relative(*n)),
                CssLineHeight::Length(DimensionPercentage::Percentage(p)) => {
                    Some(px(p.0 * em_basis()).into())
                }
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
                // `break-spaces` differs from `pre-wrap` only inside a run of spaces, which gpui's
                // breaker does not expose.
                WhiteSpaceMode::Nowrap | WhiteSpaceMode::Pre => gpui::WhiteSpace::Nowrap,
                _ => gpui::WhiteSpace::Normal,
            });
        }
        TextOverflow(v, _) => {
            use lightningcss::properties::overflow::TextOverflow as TO;
            match v {
                TO::Ellipsis => {
                    r.text.text_overflow = Some(gpui::TextOverflow::Truncate("…".into()))
                }
                // A refinement cannot express "clear", so `text-clip` after an `text-ellipsis` is a
                // no-op rather than a reset.
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
                // `text-decoration-line: none` must undo an earlier line but a refinement cannot
                // clear a field, so it is a decoration that paints nothing: zero thickness AND
                // transparent, since gpui snaps a hairline up to a device pixel.
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
            use gpui::CursorStyle as C;
            use lightningcss::properties::ui::CursorKeyword as K;
            let mapped = match c.keyword {
                K::Pointer => C::PointingHand,
                K::Default | K::Auto => C::Arrow,
                K::Text => C::IBeam,
                K::VerticalText => C::IBeamCursorForVerticalLayout,
                K::Crosshair | K::Cell => C::Crosshair,
                K::Grab | K::Move | K::AllScroll => C::OpenHand,
                K::Grabbing => C::ClosedHand,
                K::EResize => C::ResizeRight,
                K::WResize => C::ResizeLeft,
                K::NResize => C::ResizeUp,
                K::SResize => C::ResizeDown,
                K::EwResize => C::ResizeLeftRight,
                K::NsResize => C::ResizeUpDown,
                // Use the actual platform shapes; the GPUI enum's CSS doc comments accidentally
                // reverse these two diagonals.
                K::NwResize | K::SeResize | K::NwseResize => C::ResizeUpLeftDownRight,
                K::NeResize | K::SwResize | K::NeswResize => C::ResizeUpRightDownLeft,
                K::ColResize => C::ResizeColumn,
                K::RowResize => C::ResizeRow,
                K::NotAllowed | K::NoDrop => C::OperationNotAllowed,
                K::Alias => C::DragLink,
                K::Copy => C::DragCopy,
                K::ContextMenu => C::ContextualMenu,
                K::None | K::Help | K::Progress | K::Wait | K::ZoomIn | K::ZoomOut => {
                    diagnostics.push(format!(
                        "`cursor: {:?}` has no native platform shape; using the arrow cursor",
                        c.keyword
                    ));
                    C::Arrow
                }
            };
            r.mouse_cursor = Some(mapped);
        }
        Filter(list, _) => apply_filter(list, x, diagnostics),
        BackdropFilter(list, _) => apply_backdrop_filter(list, r, diagnostics),
        Custom(c) => match &c.name {
            // `--shading` is the ONE custom property Skathi interprets: a GPU effect has no web
            // equivalent, so it needs a spelling a browser parses, stores and ignores rather than
            // one a browser DROPS.
            lightningcss::properties::custom::CustomPropertyName::Custom(name)
                if name.0.as_ref() == effect::SHADING =>
            {
                if let Some(spec) = effect::parse(&c.value, diagnostics) {
                    r.vn_effect = Some(spec);
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Custom(name)
                if name.0.as_ref() == WINDOW_CONTROL =>
            {
                if let Some(c) = window_control(&c.value, diagnostics) {
                    x.window_control = Some(c);
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Custom(_) => {}
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "object-fit" =>
            {
                if let Some(fit) = object_fit(&c.value, diagnostics) {
                    x.object_fit = Some(fit);
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "object-position" =>
            {
                if let Some(position) = object_position(p, diagnostics) {
                    r.vn_object_position = Some(position);
                }
            }
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
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "content" =>
            {
                match content_string(&c.value) {
                    ContentValue::Text(t) => {
                        x.content = Some(t);
                        x.content_none = false;
                    }
                    ContentValue::None => {
                        x.content = None;
                        x.content_none = true;
                    }
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
                    Some(v) => {
                        outline_mut(r).offset = px(v);
                        x.outline.offset = Some(v);
                    }
                    None => unsupported(diagnostics, "outline-offset (a <length> is expected)"),
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "-webkit-line-clamp" || name.0.as_ref() == "line-clamp" =>
            {
                match single_number(&c.value) {
                    Some(n) if n >= 1.0 => r.text.line_clamp = Some(n as usize),
                    _ => unsupported(
                        diagnostics,
                        "line-clamp: none — cannot undo an earlier clamp",
                    ),
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name
                    .0
                    .as_ref()
                    .eq_ignore_ascii_case("-webkit-text-fill-color") =>
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
                    None => unsupported(
                        diagnostics,
                        "-webkit-text-fill-color (a <color> is expected)",
                    ),
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "text-underline-offset" =>
            {
                if single_ident_is(&c.value, "auto") || is_css_wide_keyword(&c.value) {
                } else if let Some(offset) = single_length_px(&c.value) {
                    x.underline_offset = Some(offset);
                    if let Some(u) = r.text.underline.as_mut() {
                        u.offset = px(offset);
                    }
                } else {
                    unsupported(
                        diagnostics,
                        "text-underline-offset (a <length> is expected)",
                    );
                }
            }
            // `text-decoration-skip-ink` decides whether an underline BREAKS around descenders.
            // gpui draws one unbroken rule, and the property has no other effect.
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
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if matches!(
                    name.0.as_ref(),
                    "touch-action" | "scroll-behavior" | "will-change"
                ) => {}
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name.0.as_ref() == "contain" =>
            {
                if let Some((paint, size)) = contain(&c.value, diagnostics) {
                    x.contain_paint = Some(paint);
                    x.contain_size = Some(size);
                }
            }
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
                    "border-collapse"
                        | "border-spacing"
                        | "caption-side"
                        | "table-layout"
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
                // A non-degenerate rect would need a per-element content mask in the paint path.
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
            // lightningcss has a `-webkit-mask-composite` property, but its name lookup strips the
            // prefix first, finds the unprefixed `mask-composite` (which takes no prefix), and
            // gives up.
            lightningcss::properties::custom::CustomPropertyName::Unknown(name)
                if name
                    .0
                    .as_ref()
                    .eq_ignore_ascii_case("-webkit-mask-composite") =>
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
            // `vertical-align` applies only to inline-level boxes and table cells, so it is inert
            // on a block; Tailwind's preflight pairs `display: block` with `vertical-align: middle`
            // on that basis.
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
            if lp_definite(&v.value).is_none_or(|l| l != gpui::px(0.0).into()) {
                warn_once(
                    diagnostics,
                    "`text-indent` is not applied (v1) — there is no first-line concept in the                      shaper. `padding-left` indents the whole block",
                );
            }
        }
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
        // A marker IMAGE would be a raster in the marker box, which is the one shape of marker
        // there is no box to put.
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
            // `resize: none` is the initial value and already true of every box here; the others
            // ask for a UA drag affordance that gpui has no equivalent of.
            if !matches!(v, R::None) {
                warn_once(
                    diagnostics,
                    "`resize` is not applied (v1) — there is no user-resize affordance on a gpui                      box. A drag handle has to be a real element with a pointer handler",
                );
            }
        }
        UserSelect(v, _) => {
            use crate::mapper::UserSelect as S;
            use lightningcss::properties::ui::UserSelect as U;
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
        AccentColor(c) => {
            use lightningcss::properties::ui::ColorOrAuto;
            x.accent_color = Some(match c {
                ColorOrAuto::Auto => None,
                ColorOrAuto::Color(c) => color(c),
            });
        }
        Appearance(a, _) => {
            x.appearance_none = Some(matches!(a, lightningcss::properties::ui::Appearance::None))
        }
        // Whitespace is always collapsed here, so a tab is one space and `tab-size` could not
        // change the result.
        TabSize(..) => {}
        // `text-size-adjust` exists to stop MOBILE BROWSERS inflating text.
        TextSizeAdjust(..) => {}
        // The screen-reader-only idiom, `inset(50%)`, encloses nothing and so paints nothing, while
        // the layout box and the CSS `visibility` stay.
        ClipPath(v, _) => {
            if let Some(clip) = clip_path(v, diagnostics) {
                r.vn_clip_path = Some(clip);
            }
        }
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
        ScrollMarginTop(_)
        | ScrollMarginBottom(_)
        | ScrollMarginLeft(_)
        | ScrollMarginRight(_)
        | ScrollMargin(_)
        | ScrollMarginBlock(_)
        | ScrollMarginInline(_)
        | ScrollMarginBlockStart(_)
        | ScrollMarginBlockEnd(_)
        | ScrollMarginInlineStart(_)
        | ScrollMarginInlineEnd(_)
        | ScrollPaddingTop(_)
        | ScrollPaddingBottom(_)
        | ScrollPaddingLeft(_)
        | ScrollPaddingRight(_)
        | ScrollPadding(_) => {}
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
        // Reaching here means the value stayed unparsed after substitution (unbalanced `var(` or an
        // invalid token stream): a defensive advisory.
        Unparsed(u) => {
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
            // lightningcss has no typed `none` for a list property, so `box-shadow: none`
            // (Tailwind's `shadow-none`) always lands here; it means an empty list, which lets it
            // cancel an earlier shadow.
            if matches!(u.property_id, PropertyId::BoxShadow(_))
                && matches!(single_ident_raw(&u.value), Some("none"))
            {
                r.box_shadow = Some(Vec::new());
                return;
            }
            // `x: inherit` parses as Unparsed for every property but `all`; not writing the field
            // is "take the parent's value", so it is honoured by ignoring it. `border-radius` is
            // the exception (it does not inherit): it is recorded and filled after the cascade
            // (`RadiusInherit`).
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
                    TokenOrValue::Token(Token::Function(name)) => {
                        Some(format!("{}()", name.as_ref()))
                    }
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

/// CSS keywords are case-insensitive and lightningcss prints them lowercase; an attribute's are
/// not, and resvg ignores `shape-rendering="crispedges"` (the line is anti-aliased again).
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

fn set_presentation(list: &mut Vec<(&'static str, Rc<str>)>, name: &'static str, value: Rc<str>) {
    match list.iter_mut().find(|(n, _)| *n == name) {
        Some(slot) => slot.1 = value,
        None => list.push((name, value)),
    }
}

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
        // `context-fill`/`context-stroke` only mean anything inside a `<use>` of a marker; treated
        // as "painted", which is what they degrade to.
        P::Url { .. } | P::ContextFill | P::ContextStroke => SvgPaint::Server,
    }
}

fn is_current_color(c: &lightningcss::values::color::CssColor) -> bool {
    matches!(c, lightningcss::values::color::CssColor::CurrentColor)
}

fn apply_transform(
    list: &lightningcss::properties::transform::TransformList,
    r: &mut StyleRefinement,
    diagnostics: &mut Vec<String>,
) {
    use lightningcss::properties::transform::Transform as T;
    use lightningcss::values::percentage::NumberOrPercentage;
    const UNIT: [[f64; 2]; 2] = [[1.0, 0.0], [0.0, 1.0]];
    const ZERO: [[f64; 3]; 2] = [[0.0; 3]; 2];
    fn len(v: &LengthPercentage, axis: usize) -> Option<[f64; 3]> {
        match v {
            DimensionPercentage::Dimension(l) => {
                length_value_px(l).map(|p| [f64::from(p), 0.0, 0.0])
            }
            DimensionPercentage::Percentage(p) => {
                let mut row = [0.0; 3];
                row[axis + 1] = f64::from(p.0);
                Some(row)
            }
            DimensionPercentage::Calc(_) => {
                calc_dropped();
                None
            }
        }
    }
    fn factor(v: &NumberOrPercentage) -> f64 {
        f64::from(match v {
            NumberOrPercentage::Number(n) => *n,
            NumberOrPercentage::Percentage(p) => p.0,
        })
    }
    fn append(
        linear: &mut [[f64; 2]; 2],
        offset: &mut [[f64; 3]; 2],
        m: [[f64; 2]; 2],
        t: [[f64; 3]; 2],
    ) {
        let old = *linear;
        for row in 0..2 {
            for term in 0..3 {
                offset[row][term] += old[row][0] * t[0][term] + old[row][1] * t[1][term];
            }
            for column in 0..2 {
                linear[row][column] = old[row][0] * m[0][column] + old[row][1] * m[1][column];
            }
        }
    }
    fn slot(row: [f32; 3], axis: usize) -> Option<TransformLength> {
        let own = row[axis + 1];
        let cross = row[2 - axis];
        if cross != 0.0 || (row[0] != 0.0 && own != 0.0) {
            None
        } else if own != 0.0 {
            Some(TransformLength::Fraction(own))
        } else {
            Some(TransformLength::Pixels(row[0]))
        }
    }
    let mut linear = UNIT;
    let mut offset = ZERO;
    let mut authored_rotate = 0.0f64;
    let mut authored_scale = (1.0f64, 1.0f64);
    let mut affine = false;
    let mut dropped: Vec<&str> = Vec::new();
    for transform in &list.0 {
        let mut m = UNIT;
        let mut t = ZERO;
        match transform {
            T::Translate(a, b) | T::Translate3d(a, b, _) => {
                let (Some(x), Some(y)) = (len(a, 0), len(b, 1)) else {
                    dropped.push("translate() with a calc() length");
                    continue;
                };
                t = [x, y];
                if let T::Translate3d(_, _, z) = transform {
                    if !matches!(css_length_px(z), Some(0.0)) {
                        dropped.push("the Z axis of a 3D translate");
                    }
                }
            }
            T::TranslateX(v) => {
                let Some(value) = len(v, 0) else {
                    dropped.push("translateX() with a calc() length");
                    continue;
                };
                t[0] = value;
            }
            T::TranslateY(v) => {
                let Some(value) = len(v, 1) else {
                    dropped.push("translateY() with a calc() length");
                    continue;
                };
                t[1] = value;
            }
            T::Rotate(a) | T::RotateZ(a) => {
                let angle = f64::from(a.to_radians());
                let (sin, cos) = angle.sin_cos();
                m = [[cos, -sin], [sin, cos]];
                authored_rotate += angle;
            }
            T::RotateX(a) | T::RotateY(a) if a.to_degrees() == 0.0 => {}
            T::RotateX(_) | T::RotateY(_) | T::Rotate3d(..) => {
                dropped.push("rotate3d()/rotateX()/rotateY()");
                continue;
            }
            T::Scale(a, b) | T::Scale3d(a, b, _) => {
                let (x, y) = (factor(a), factor(b));
                m = [[x, 0.0], [0.0, y]];
                authored_scale = (authored_scale.0 * x, authored_scale.1 * y);
            }
            T::ScaleX(value) => {
                let x = factor(value);
                m[0][0] = x;
                authored_scale.0 *= x;
            }
            T::ScaleY(value) => {
                let y = factor(value);
                m[1][1] = y;
                authored_scale.1 *= y;
            }
            T::ScaleZ(_) => {}
            T::Skew(a, b) => {
                m = [
                    [1.0, f64::from(a.to_radians()).tan()],
                    [f64::from(b.to_radians()).tan(), 1.0],
                ];
                affine |= a.to_degrees() != 0.0 || b.to_degrees() != 0.0;
            }
            T::SkewX(a) => {
                m[0][1] = f64::from(a.to_radians()).tan();
                affine |= a.to_degrees() != 0.0;
            }
            T::SkewY(a) => {
                m[1][0] = f64::from(a.to_radians()).tan();
                affine |= a.to_degrees() != 0.0;
            }
            T::Matrix(value) => {
                m = [
                    [f64::from(value.a), f64::from(value.c)],
                    [f64::from(value.b), f64::from(value.d)],
                ];
                t[0][0] = f64::from(value.e);
                t[1][0] = f64::from(value.f);
                affine = true;
            }
            T::Matrix3d(_) => {
                dropped.push("matrix3d()");
                continue;
            }
            T::Perspective(_) => {
                dropped.push("perspective()");
                continue;
            }
            T::TranslateZ(_) => {
                dropped.push("the Z axis of a 3D translate");
                continue;
            }
        }
        if m.iter()
            .flatten()
            .chain(t.iter().flatten())
            .any(|v| !v.is_finite())
        {
            dropped.push("a non-finite 2D transform");
            continue;
        }
        append(&mut linear, &mut offset, m, t);
    }
    let (sin, cos) = authored_rotate.sin_cos();
    let expected = [
        [cos * authored_scale.0, -sin * authored_scale.1],
        [sin * authored_scale.0, cos * authored_scale.1],
    ];
    affine |= linear
        .iter()
        .flatten()
        .zip(expected.iter().flatten())
        .any(|(a, b)| (a - b).abs() > 1e-6 * a.abs().max(b.abs()).max(1.0));
    let (mut rotate, mut scale, mut shear) = (
        authored_rotate as f32,
        (authored_scale.0 as f32, authored_scale.1 as f32),
        0.0f32,
    );
    if affine {
        let [[a, c], [b, d]] = linear;
        let sx = a.hypot(b);
        if sx != 0.0 {
            rotate = b.atan2(a) as f32;
            scale = (sx as f32, ((a * d - b * c) / sx) as f32);
            shear = ((a * c + b * d) / sx) as f32;
        } else {
            rotate = (-c).atan2(d) as f32;
            scale = (0.0, c.hypot(d) as f32);
        }
    }
    let basis = offset.map(|row| row.map(|value| value as f32));
    let (x, y) = (slot(basis[0], 0), slot(basis[1], 1));
    let origin = r
        .vn_transform
        .map(|t| t.origin)
        .unwrap_or(TransformSpec::CENTRE_ORIGIN);
    let mut spec = TransformSpec {
        translate: (
            x.unwrap_or(TransformLength::ZERO),
            y.unwrap_or(TransformLength::ZERO),
        ),
        rotate,
        scale,
        shear,
        origin,
        translation_basis: (x.is_none() || y.is_none()).then_some(basis),
    };
    if [rotate, scale.0, scale.1, shear]
        .iter()
        .chain(basis.iter().flatten())
        .any(|v| !v.is_finite())
    {
        dropped.push("an out-of-range 2D composition");
        spec = TransformSpec {
            origin,
            ..TransformSpec::IDENTITY
        };
    }
    r.vn_transform = Some(spec);
    if !dropped.is_empty() {
        dropped.sort_unstable();
        dropped.dedup();
        once(diagnostics, format!("`transform` supports ordered 2D translate(), rotate(), scale(), skew(), matrix() and `transform-origin` at paint time — dropped {}", dropped.join(", ")));
    }
}
pub fn transform_css(spec: TransformSpec) -> String {
    if spec.is_identity() {
        return "none".into();
    }
    let mut parts = Vec::new();
    let len = |value: TransformLength| match value {
        TransformLength::Pixels(value) => format!("{value}px"),
        TransformLength::Fraction(value) => format!("{}%", value * 100.0),
    };
    if let Some([[px, width, cross_x], [py, cross_y, height]]) = spec.translation_basis {
        if px != 0.0 || py != 0.0 {
            parts.push(format!("translate({px}px, {py}px)"));
        }
        if width != 0.0 || height != 0.0 {
            parts.push(format!(
                "translate({}%, {}%)",
                width * 100.0,
                height * 100.0
            ));
        }
        if cross_x != 0.0 {
            parts.push(format!(
                "rotate(-90deg) translateY({}%) rotate(90deg)",
                cross_x * 100.0
            ));
        }
        if cross_y != 0.0 {
            parts.push(format!(
                "rotate(90deg) translateX({}%) rotate(-90deg)",
                cross_y * 100.0
            ));
        }
    } else if !spec.translate.0.is_zero() || !spec.translate.1.is_zero() {
        parts.push(format!(
            "translate({}, {})",
            len(spec.translate.0),
            len(spec.translate.1)
        ));
    }
    if spec.rotate != 0.0 {
        parts.push(format!("rotate({}deg)", spec.rotate.to_degrees()));
    }
    if spec.shear != 0.0 {
        parts.push(format!(
            "matrix({},0,{},{},0,0)",
            spec.scale.0, spec.shear, spec.scale.1
        ));
    } else if spec.scale != (1.0, 1.0) {
        parts.push(format!("scale({}, {})", spec.scale.0, spec.scale.1));
    }
    parts.join(" ")
}
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

fn drop_shadow_of(shadow: &lightningcss::properties::effects::DropShadow) -> Option<DropShadow> {
    Some(DropShadow {
        dx: css_length_px(&shadow.x_offset)?,
        dy: css_length_px(&shadow.y_offset)?,
        blur: css_length_px(&shadow.blur)?.max(0.0),
        // `None` here is `currentColor` — the CSS default for the function's colour argument, and
        // the one value `color()` cannot flatten.
        color: color(&shadow.color),
    })
}

const SIDE_TOP: u8 = 1;
const SIDE_RIGHT: u8 = 2;
const SIDE_BOTTOM: u8 = 4;
const SIDE_LEFT: u8 = 8;
const SIDES_ALL: u8 = SIDE_TOP | SIDE_RIGHT | SIDE_BOTTOM | SIDE_LEFT;

fn set_border_style(
    r: &mut StyleRefinement,
    s: LineStyle,
    sides: u8,
    diagnostics: &mut Vec<String>,
) {
    if matches!(s, LineStyle::None | LineStyle::Hidden) {
        let zero: AbsoluteLength = px(0.0).into();
        if sides & SIDE_TOP != 0 {
            r.border_widths.top = Some(zero);
        }
        if sides & SIDE_RIGHT != 0 {
            r.border_widths.right = Some(zero);
        }
        if sides & SIDE_BOTTOM != 0 {
            r.border_widths.bottom = Some(zero);
        }
        if sides & SIDE_LEFT != 0 {
            r.border_widths.left = Some(zero);
        }
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

pub(crate) fn compute_overflow(r: &mut StyleRefinement, x: &mut Extras) {
    use OverflowKeyword as K;
    if x.contain_paint == Some(true) {
        if overflow_keyword(r.overflow.x, x.overflow_x) == K::Visible {
            r.overflow.x = Some(overflow(K::Clip));
            x.overflow_x = Some(K::Clip);
        }
        if overflow_keyword(r.overflow.y, x.overflow_y) == K::Visible {
            r.overflow.y = Some(overflow(K::Clip));
            x.overflow_y = Some(K::Clip);
        }
    }
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
    pub const CONTAIN_WIDTH: u8 = 1;
    pub const CONTAIN_HEIGHT: u8 = 2;

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

    fn map(body: &str) -> (StyleRefinement, Vec<String>) {
        let mut diagnostics = Vec::new();
        let r = parse_inline(body, &mut diagnostics);
        (r, diagnostics)
    }

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

    #[test]
    fn overflow_longhands_set_one_axis_each() {
        let r = clean("overflow-x: hidden");
        assert_eq!(r.overflow.x, Some(Overflow::Hidden));
        assert_eq!(r.overflow.y, None, "overflow-x must not touch the y axis");

        let r = clean("overflow-y: scroll");
        assert_eq!(r.overflow.y, Some(Overflow::Scroll));
        assert_eq!(r.overflow.x, None);

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

    fn computed_overflow(body: &str) -> (StyleRefinement, [&'static str; 2]) {
        let (mut r, mut x, d) = map_full(body);
        assert!(d.is_empty(), "{body:?} produced {d:?}");
        compute_overflow(&mut r, &mut x);
        let names = x.overflow_names(&r);
        (r, names)
    }

    #[test]
    fn visible_beside_a_scrolling_axis_computes_to_auto() {
        let (r, names) = computed_overflow("overflow-y: auto");
        assert_eq!(names, ["auto", "auto"]);
        assert_eq!(
            r.overflow.x,
            Some(Overflow::Scroll),
            "x scrolls too, it is not just clipped"
        );
        assert_eq!(r.overflow.y, Some(Overflow::Scroll));

        let (r, names) = computed_overflow("overflow-x: hidden");
        assert_eq!(names, ["hidden", "auto"]);
        assert_eq!(r.overflow.y, Some(Overflow::Scroll));

        assert_eq!(
            computed_overflow("overflow-x: scroll").1,
            ["scroll", "auto"]
        );
        assert_eq!(computed_overflow("overflow: auto").1, ["auto", "auto"]);
        assert_eq!(
            computed_overflow("overflow: hidden scroll").1,
            ["hidden", "scroll"]
        );
    }

    #[test]
    fn clip_beside_a_scrolling_axis_computes_to_hidden_and_beside_visible_stays() {
        assert_eq!(
            computed_overflow("overflow-x: clip; overflow-y: auto").1,
            ["hidden", "auto"]
        );
        let (r, names) = computed_overflow("overflow-x: clip");
        assert_eq!(names, ["clip", "visible"]);
        assert_eq!(r.overflow.y, None);
        assert_eq!(computed_overflow("overflow: clip").1, ["clip", "clip"]);
        assert_eq!(computed_overflow("color: red").1, ["visible", "visible"]);
    }

    #[test]
    fn the_refinement_decides_and_the_keyword_only_qualifies_it() {
        let (mut base, mut x, _) = map_full("color: red");
        x.refine(&map_full("overflow-y: auto").1);
        compute_overflow(&mut base, &mut x);
        assert_eq!(x.overflow_names(&base), ["visible", "visible"]);
        assert_eq!(base.overflow.x, None);
    }

    #[test]
    fn gap_longhands_map_to_the_matching_axis() {
        let r = clean("row-gap: 4px; column-gap: 8px");
        assert_eq!(r.gap.height, Some(px(4.0).into()));
        assert_eq!(r.gap.width, Some(px(8.0).into()));
    }

    #[test]
    fn border_style_none_computes_the_widths_to_zero() {
        let r = clean("border: 3px solid red; border-style: none");
        let zero: AbsoluteLength = px(0.0).into();
        assert_eq!(r.border_widths.top, Some(zero));
        assert_eq!(r.border_widths.right, Some(zero));
        assert_eq!(r.border_widths.bottom, Some(zero));
        assert_eq!(r.border_widths.left, Some(zero));
    }

    #[test]
    fn border_style_none_on_one_side_leaves_the_others() {
        let zero: AbsoluteLength = px(0.0).into();
        let one: AbsoluteLength = px(1.0).into();
        for body in [
            "border: 1px solid red; border-bottom: none",
            "border: 1px solid red; border-bottom: 0",
            "border: 1px solid red; border-bottom-style: none",
            "border: 1px solid red; border-style: solid solid none",
        ] {
            let r = clean(body);
            assert_eq!(r.border_widths.bottom, Some(zero), "{body}");
            assert_eq!(r.border_widths.top, Some(one), "{body}");
            assert_eq!(r.border_widths.right, Some(one), "{body}");
            assert_eq!(r.border_widths.left, Some(one), "{body}");
            assert_eq!(r.border_style, Some(gpui::BorderStyle::Solid), "{body}");
        }
        let r = clean("border-right: none");
        assert_eq!(r.border_widths.right, Some(zero));
        assert_eq!(r.border_widths.top, None);
    }

    #[test]
    fn border_style_dashed_and_solid_for_the_rest() {
        assert_eq!(
            clean("border-style: dashed").border_style,
            Some(gpui::BorderStyle::Dashed)
        );
        let (r, _) = map("border-style: dotted");
        assert_eq!(r.border_style, Some(gpui::BorderStyle::Solid));
    }

    #[test]
    fn corner_radius_longhands() {
        let r = clean("border-top-left-radius: 8px; border-bottom-right-radius: 2px");
        assert_eq!(r.corner_radii.top_left, Some(px(8.0).into()));
        assert_eq!(r.corner_radii.bottom_right, Some(px(2.0).into()));
        assert_eq!(r.corner_radii.top_right, None);
        assert_eq!(r.vn_corner_radii.top_left, Some(None));
        assert_eq!(r.vn_corner_radii.top_right, None);
    }

    #[test]
    fn relative_and_elliptical_radii_resolve_at_paint() {
        let half = gpui::VnCornerRadius {
            x: relative(0.5),
            y: relative(0.5),
        };
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
        let calc = DefiniteLength::Calc {
            fraction: 0.5,
            pixels: px(-2.),
        };
        assert_eq!(
            r.vn_corner_radii.top_left,
            Some(Some(gpui::VnCornerRadius { x: calc, y: calc }))
        );

        let r = clean("border-radius: 10px / 20px");
        assert_eq!(
            r.vn_corner_radii.bottom_left,
            Some(Some(gpui::VnCornerRadius {
                x: px(10.).into(),
                y: px(20.).into()
            }))
        );

        let r = clean("border-radius: 6px / 6px");
        assert_eq!(r.corner_radii.top_right, Some(px(6.).into()));
        assert_eq!(r.vn_corner_radii.top_right, Some(None));
    }

    #[test]
    fn flex_shorthand_fills_all_three_fields() {
        let r = clean("flex: 1 1 0%");
        assert_eq!(r.flex_grow, Some(1.0));
        assert_eq!(r.flex_shrink, Some(1.0));
        assert_eq!(r.flex_basis, Some(Length::Definite(relative(0.0))));

        let r = clean("flex: none");
        assert_eq!(r.flex_grow, Some(0.0));
        assert_eq!(r.flex_shrink, Some(0.0));
        assert_eq!(r.flex_basis, Some(Length::Auto));
    }

    #[test]
    fn order_reaches_the_layout_in_every_spelling() {
        assert_eq!(clean("order: -3").vn_order, Some(-3));
        assert_eq!(clean("-webkit-order: 2").vn_order, Some(2));
        let mut d = Vec::new();
        let vars = crate::vars::resolve(
            &Rc::new(Vars::default()),
            &[&("--o".to_string(), "7".to_string())],
            &crate::Env::default(),
            &mut d,
        );
        let mut r = StyleRefinement::default();
        apply_dynamic(
            "order",
            "var(--o)",
            &vars,
            &crate::Env::default(),
            &mut r,
            &mut Extras::default(),
            &mut d,
        );
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(r.vn_order, Some(7));
    }

    fn rgb8(c: Option<Hsla>) -> [u8; 4] {
        let c = gpui::Rgba::from(c.expect("a concrete colour"));
        [c.r, c.g, c.b, c.a].map(|v| (v * 255.0).round() as u8)
    }

    #[test]
    fn css_color_4_and_5_functions_compute_to_srgb() {
        let text = |value: &str| rgb8(clean(&format!("color: {value}")).text.color);
        assert_eq!(text("oklch(62.8% 0.2577 29.23)"), [255, 0, 0, 255]);
        assert_eq!(text("oklab(0.5 0 0)"), [99, 99, 99, 255]);
        assert_eq!(text("lab(54.29% 80.82 69.91)"), [255, 0, 0, 255]);
        assert_eq!(text("lch(54.29% 106.84 40.85 / 50%)"), [255, 0, 0, 128]);
        assert_eq!(text("hwb(120 0% 0%)"), [0, 255, 0, 255]);
        assert_eq!(text("color(srgb 0 0.5 1)"), [0, 128, 255, 255]);
        assert_eq!(
            text("color(srgb-linear 0.2158 0.2158 0.2158)"),
            [128, 128, 128, 255]
        );
        assert_eq!(text("color-mix(in srgb, red, blue)"), [128, 0, 128, 255]);
        assert_eq!(
            text("color-mix(in srgb, red 25%, transparent)"),
            [255, 0, 0, 64]
        );
        assert_eq!(text("rgb(from #ff0000 r g 255)"), [255, 0, 255, 255]);
        assert_eq!(text("oklch(from #00ff00 l c h / 0.5)"), [0, 255, 0, 128]);
    }

    #[test]
    fn a_wide_gamut_colour_is_clipped_into_srgb() {
        let [r, g, b, _] = rgb8(clean("color: color(display-p3 1 0 0)").text.color);
        assert!(r == 255 && g < 40 && b < 40, "{r} {g} {b}");
        let [r, g, b, _] = rgb8(clean("color: oklch(90% 0.4 145)").text.color);
        assert!(
            g > r && g > b && g > 220,
            "a too-vivid green stays green: {r} {g} {b}"
        );
    }

    #[test]
    fn every_colour_valued_property_takes_a_css_color_4_value() {
        let red = "oklch(62.8% 0.2577 29.23)";
        let solid = |r: &StyleRefinement| match &r.background {
            Some(gpui::Fill::Color(bg)) => bg.as_solid(),
            _ => None,
        };
        let want = [255, 0, 0, 255];
        assert_eq!(
            rgb8(solid(&clean(&format!("background-color: {red}")))),
            want
        );
        assert_eq!(
            rgb8(clean(&format!("border-color: {red}")).border_color),
            want
        );
        assert_eq!(
            rgb8(
                clean(&format!("border-left: 1px solid {red}"))
                    .vn_border_colors
                    .left
                    .flatten()
            ),
            want
        );
        let outline = clean(&format!("outline: 2px solid {red}"))
            .vn_outline
            .expect("ring");
        assert_eq!(rgb8(Some(outline.color)), want);
        let shadow = clean(&format!("box-shadow: 0 0 4px {red}"))
            .box_shadow
            .expect("shadow");
        assert_eq!(rgb8(Some(shadow[0].color)), want);
        let decoration = clean(&format!("text-decoration: underline {red}"))
            .text
            .underline
            .expect("line");
        assert_eq!(rgb8(decoration.color), want);

        let (x, d) = extras(&format!(
            "accent-color: {red}; fill: {red}; overflow: auto; scrollbar-color: {red} lab(0% 0 0)"
        ));
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(rgb8(x.accent_color.flatten()), want);
        let Some(SvgPaint::Color(fill)) = x.fill else {
            panic!("{:?}", x.fill)
        };
        assert_eq!(rgb8(Some(fill)), want);
        let (thumb, track) = x.scrollbar_color.expect("pair");
        assert_eq!(
            (rgb8(Some(thumb)), rgb8(Some(track))),
            (want, [0, 0, 0, 255])
        );

        let (x, d) = extras(&format!(
            "background-image: linear-gradient({red}, color-mix(in oklab, {red}, {red}))"
        ));
        assert!(d.is_empty(), "{d:?}");
        assert!(x.background_images.is_some_and(|l| !l.is_empty()));
    }

    #[test]
    fn object_position_is_a_fraction_of_the_free_space_plus_an_offset() {
        let at = |body: &str| {
            let p = clean(body).vn_object_position.expect("mapped");
            (
                p.x_fraction,
                f32::from(p.x_offset),
                p.y_fraction,
                f32::from(p.y_offset),
            )
        };
        assert_eq!(at("object-position: left"), (0.0, 0.0, 0.5, 0.0));
        assert_eq!(at("object-position: 25% 10px"), (0.25, 0.0, 0.0, 10.0));
        assert_eq!(
            at("object-position: right 10px bottom 20%"),
            (1.0, -10.0, 0.8, 0.0)
        );
        assert_eq!(at("object-position: top right"), (1.0, 0.0, 0.0, 0.0));
        assert_eq!(
            at("object-position: calc(50% + 4px) center"),
            (0.5, 4.0, 0.5, 0.0)
        );
        let (r, d) = map("object-position: 10px sideways");
        assert_eq!(r.vn_object_position, None);
        assert_eq!(d.len(), 1, "{d:?}");
    }

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

    #[test]
    fn an_arbitrary_track_list_becomes_an_explicit_track_list() {
        use gpui::{GridTrack, GridTrackSize as G};
        let r = clean("grid-template-columns: 200px 1fr");
        assert_eq!(r.grid_cols, None);
        assert_eq!(
            r.vn_grid_cols,
            Some(vec![GridTrack::fixed(G::Px(200.0)), GridTrack::fr(1.0)])
        );
        assert_eq!(
            clean("grid-template-columns: 1fr auto").vn_grid_cols,
            Some(vec![GridTrack::fr(1.0), GridTrack::fixed(G::Auto)])
        );
        assert_eq!(
            clean("grid-template-rows: repeat(2, minmax(min-content, 1fr)) 40px").vn_grid_rows,
            Some(vec![
                GridTrack {
                    min: G::MinContent,
                    max: G::Fr(1.0)
                },
                GridTrack {
                    min: G::MinContent,
                    max: G::Fr(1.0)
                },
                GridTrack::fixed(G::Px(40.0)),
            ])
        );
        assert_eq!(
            clean("grid-template-columns: 25% 75%").vn_grid_cols,
            Some(vec![
                GridTrack::fixed(G::Percent(0.25)),
                GridTrack::fixed(G::Percent(0.75)),
            ])
        );
    }

    #[test]
    fn an_auto_repeat_is_counted_by_the_layout() {
        use gpui::{GridAutoRepeat, GridTrack, GridTrackSize as G};
        let r = clean("grid-template-columns: repeat(auto-fill, minmax(120px, 1fr))");
        assert_eq!(r.grid_cols, None);
        assert_eq!(
            r.vn_grid_cols,
            Some(vec![GridTrack {
                min: G::Px(120.0),
                max: G::Fr(1.0)
            }])
        );
        assert_eq!(
            r.vn_grid_cols_auto,
            Some(GridAutoRepeat {
                fit: false,
                start: 0,
                len: 1
            })
        );

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
        assert_eq!(
            r.vn_grid_rows_auto,
            Some(GridAutoRepeat {
                fit: true,
                start: 1,
                len: 2
            })
        );

        assert_eq!(
            clean("grid-template-columns: 1fr auto").vn_grid_cols_auto,
            Some(GridAutoRepeat {
                fit: false,
                start: 0,
                len: 0
            })
        );
    }

    #[test]
    fn an_unmodelable_track_list_is_dropped_not_a_wrong_grid() {
        for body in [
            "grid-template-columns: repeat(auto-fill, minmax(120px, 1fr)) 1fr",
            "grid-template-columns: repeat(auto-fill, auto)",
            "grid-template-columns: fit-content(200px) 1fr",
        ] {
            let (r, _) = map(body);
            assert_eq!(r.grid_cols, None, "{body}");
            assert_eq!(r.vn_grid_cols, None, "{body}");
        }
    }

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
    }

    #[test]
    fn implicit_track_lists_map_fractional_and_repeating_patterns() {
        use gpui::{GridTrack, GridTrackSize as Track};
        assert_eq!(
            clean("grid-auto-columns: 1fr").vn_grid_auto_cols,
            Some(vec![GridTrack::fr(1.0)])
        );
        assert_eq!(
            clean("grid-auto-rows: 20px 2rem 25% minmax(min-content, 1fr)").vn_grid_auto_rows,
            Some(vec![
                GridTrack::fixed(Track::Px(20.0)),
                GridTrack::fixed(Track::Px(32.0)),
                GridTrack::fixed(Track::Percent(0.25)),
                GridTrack {
                    min: Track::MinContent,
                    max: Track::Fr(1.0)
                }
            ])
        );
        // lightningcss canonicalizes a sole `auto` to an empty track list. It must remain Some so
        // the cascade can reset an earlier pattern.
        assert_eq!(
            clean("grid-auto-rows: 20px; grid-auto-rows: auto").vn_grid_auto_rows,
            Some(Vec::new())
        );
        let (style, _) = map("grid-auto-columns: 12px fit-content(30px)");
        assert!(
            style.vn_grid_auto_cols.is_none(),
            "unsupported tracks cannot leave a truncated pattern"
        );
    }

    #[test]
    fn declared_visibility_is_not_the_paint_switch() {
        assert_eq!(extras("visibility: hidden").0.visibility_hidden, Some(true));
        assert_eq!(
            extras("visibility: collapse").0.visibility_hidden,
            Some(true)
        );
        assert_eq!(
            extras("visibility: visible").0.visibility_hidden,
            Some(false)
        );
        assert_eq!(extras("color: red").0.visibility_hidden, None);
        for sr_only in ["clip: rect(0, 0, 0, 0)"] {
            let mut d = Vec::new();
            let inline = parse_inline_style(sr_only, &mut d);
            assert_eq!(
                inline.refinement.and_then(|r| r.visibility),
                Some(gpui::Visibility::Hidden),
                "{sr_only}"
            );
            assert_eq!(inline.extras.visibility_hidden, None, "{sr_only}");
        }
        let mut d = Vec::new();
        let inline = parse_inline_style("clip-path: inset(50%)", &mut d);
        assert_eq!(inline.extras.visibility_hidden, None);
    }

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
        assert!(inline.refinement.is_some_and(|r| r.text.color.is_some()));
    }

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

    #[test]
    fn apply_dynamic_substitutes_then_maps() {
        let mut d = Vec::new();
        let vars = crate::vars::resolve(
            &Rc::new(Vars::default()),
            &[&(
                "--tw-shadow".to_string(),
                "0 1px 2px 0 rgb(0 0 0 / 0.5)".to_string(),
            )],
            &crate::Env::default(),
            &mut d,
        );
        let mut r = StyleRefinement::default();
        let mut x = Extras::default();
        apply_dynamic(
            "box-shadow",
            "var(--tw-shadow)",
            &vars,
            &crate::Env::default(),
            &mut r,
            &mut x,
            &mut d,
        );
        assert!(d.is_empty(), "{d:?}");
        let shadows = r.box_shadow.expect("box-shadow mapped");
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].blur_radius, px(2.0));
    }

    #[test]
    fn currentcolor_is_recorded_for_substitution_after_the_cascade() {
        let (r, x, d) = map_full("background-color: currentColor");
        assert_eq!(r.background, None, "not resolvable yet");
        assert!(x.current_color.has(CurrentColorTargets::BACKGROUND));
        assert!(d.is_empty(), "{d:?}");

        let (_, x, d) = map_full("border-color: currentColor");
        assert!(x.current_color.has(CurrentColorTargets::BORDER));
        assert!(d.is_empty(), "{d:?}");

        let (r, x, d) = map_full("color: currentColor");
        assert_eq!(r.text.color, None);
        assert_eq!(x.current_color, CurrentColorTargets::default());
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn calc_that_is_not_a_finite_length_does_not_reach_the_layout() {
        let (r, _) = map("width: clamp(2rem, 5px, 1rem)");
        assert_eq!(r.size.width, Some(Length::Definite(px(32.0).into())));
        let (r, _) = map("width: calc(100px / 0)");
        assert_eq!(r.size.width, None);
        let (r, _) = map("padding-top: calc(1px * (1 / 0))");
        assert_eq!(r.padding.top, None);
    }

    #[test]
    fn calc_sums_and_products_resolve_to_pixels() {
        assert_eq!(
            clean("border-radius: calc(0.5rem - 2px)")
                .corner_radii
                .top_left,
            Some(px(6.0).into())
        );
        assert_eq!(
            clean("padding-top: calc(1rem + 4px)").padding.top,
            Some(px(20.0).into())
        );
        assert_eq!(
            clean("width: calc(2 * 8px)").size.width,
            Some(Length::Definite(px(16.0).into()))
        );
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

    #[test]
    fn a_percentage_inside_calc_is_resolved_by_the_layout() {
        let calc = |fraction: f32, pixels: f32| {
            Some(Length::Definite(DefiniteLength::Calc {
                fraction,
                pixels: px(pixels),
            }))
        };
        assert_eq!(
            clean("left: calc(38.89% + 13.33px)").inset.left,
            calc(0.3889, 13.33)
        );
        assert_eq!(clean("width: calc(100% - 8px)").size.width, calc(1.0, -8.0));
        assert_eq!(
            clean("max-width: calc((100% - 1rem) * 0.5)").max_size.width,
            calc(0.5, -8.0)
        );
        assert_eq!(
            clean("margin-left: calc(-50% + 2px)").margin.left,
            calc(-0.5, 2.0)
        );
        assert_eq!(
            clean("padding-top: calc(10% - 1px)").padding.top,
            Some(DefiniteLength::Calc {
                fraction: 0.1,
                pixels: px(-1.0)
            })
        );
        assert_eq!(
            clean("width: calc(50% - 50% + 3px)").size.width,
            Some(Length::Definite(px(3.0).into()))
        );
    }

    #[test]
    fn a_calc_that_cannot_be_laid_out_is_dropped() {
        let (r, _) = map("width: min(100%, 10px)");
        assert_eq!(r.size.width, None);
        let (r, _) = map("left: clamp(0px, 50%, 100px)");
        assert_eq!(r.inset.left, None);
    }

    #[test]
    fn a_cursor_image_falls_back_to_its_keyword() {
        assert_eq!(
            clean("cursor: url(brush.svg) 8 8, crosshair").mouse_cursor,
            Some(gpui::CursorStyle::Crosshair)
        );
    }

    fn extras(body: &str) -> (Extras, Vec<String>) {
        let mut d = Vec::new();
        let inline = parse_inline_style(body, &mut d);
        (inline.extras, d)
    }

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

    #[test]
    fn appearance_none_takes_the_select_skin_away() {
        for (body, none) in [
            ("appearance: none", Some(true)),
            ("-webkit-appearance: none", Some(true)),
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

    #[test]
    fn fixed_is_absolute_plus_a_deferred_layer_and_no_advisory() {
        let (x, d) = extras("position: fixed");
        assert_eq!(x.fixed, Some(true));
        assert!(x.may_defer());
        assert!(d.is_empty(), "{d:?}");
        let (x, _) = extras("position: fixed; position: relative");
        assert_eq!(x.fixed, Some(false));
        assert!(!x.may_defer());
    }

    #[test]
    fn z_index_auto_is_not_zero() {
        assert_eq!(extras("z-index: auto").0.z_index, Some(None));
        assert_eq!(extras("z-index: 0").0.z_index, Some(Some(0)));
        assert_eq!(extras("color: red").0.z_index, None);
        let mut folded = extras("z-index: 10").0;
        folded.refine(&extras("z-index: auto").0);
        assert_eq!(folded.z(), None);
        folded.refine(&extras("color: red").0);
        assert_eq!(
            folded.z_index,
            Some(None),
            "a block that says nothing keeps it"
        );
    }

    #[test]
    fn translate_is_kept_in_its_authored_unit() {
        let (r, d) = transform("transform: translate(-50%, -50%)");
        assert!(d.is_empty(), "{d:?}");
        let t = r.expect("declared");
        assert_eq!(
            t.translate,
            (
                TransformLength::Fraction(-0.5),
                TransformLength::Fraction(-0.5)
            )
        );
        assert_eq!(TransformLength::Fraction(-0.5).resolve(400.0), -200.0);

        let t = transform("transform: translateX(1.25rem)")
            .0
            .expect("declared");
        assert_eq!(
            t.translate,
            (TransformLength::Pixels(20.0), TransformLength::Pixels(0.0))
        );
    }

    #[test]
    fn tailwinds_identity_transform_chain_is_silent() {
        let (r, d) = transform(
            "transform: translate(-50%, -50%) rotate(0) skewX(0) skewY(0) scaleX(1) scaleY(1)",
        );
        assert!(d.is_empty(), "{d:?}");
        let t = r.expect("declared");
        assert_eq!(
            t.translate,
            (
                TransformLength::Fraction(-0.5),
                TransformLength::Fraction(-0.5)
            )
        );
        assert_eq!(t.rotate, 0.0);
        assert_eq!(t.scale, (1.0, 1.0));
    }

    #[test]
    fn transform_none_is_a_value_not_an_absence() {
        let (r, d) = transform("transform: rotate(0deg)");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(r, Some(TransformSpec::IDENTITY));
        assert!(r.unwrap().is_identity());
        assert_eq!(transform("color: red").0, None);
    }

    #[test]
    fn rotate_and_scale_reach_the_computed_value() {
        let (r, d) = transform("transform: translateY(4px) rotate(45deg) scale(1.5)");
        assert!(d.is_empty(), "{d:?}");
        let t = r.expect("declared");
        assert_eq!(
            t.translate,
            (TransformLength::Pixels(0.0), TransformLength::Pixels(4.0))
        );
        assert!(
            (t.rotate - std::f32::consts::FRAC_PI_4).abs() < 1e-5,
            "{}",
            t.rotate
        );
        assert_eq!(t.scale, (1.5, 1.5));

        let spin = transform("transform: rotate(360deg)").0.expect("declared");
        assert!(
            (spin.rotate - std::f32::consts::TAU).abs() < 1e-4,
            "{}",
            spin.rotate
        );
        assert_eq!(
            transform("transform: scaleX(-1)")
                .0
                .expect("declared")
                .scale,
            (-1.0, 1.0)
        );

        let (r, _) = transform("transform: rotateY(30deg) skewX(10deg)");
        assert_eq!(r.expect("declared").rotate, 0.0);
    }

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

        let right = one("background-image: linear-gradient(to right, #ff0000, #0000ff)");
        let ninety = one("background: linear-gradient(90deg, #ff0000 0%, #0000ff 100%)");
        assert_eq!(
            format!("{right:?}"),
            format!("{ninety:?}"),
            "`to right` is 90deg"
        );
        let down = one("background-image: linear-gradient(#fff, #000)");
        let explicit = one("background-image: linear-gradient(180deg, #fff, #000)");
        assert_eq!(
            format!("{down:?}"),
            format!("{explicit:?}"),
            "the default is `to bottom`"
        );

        assert!(
            format!("{:?}", one("background-image: radial-gradient(#fff, #000)"))
                .starts_with("RadialGradient")
        );
        assert!(
            format!("{:?}", one("background-image: conic-gradient(#fff, #000)"))
                .starts_with("ConicGradient")
        );

        let three = one("background-image: linear-gradient(#f00, #0f0, #00f)");
        assert_eq!(three.live_stops().len(), 3);

        let repeating = one("background-image: repeating-linear-gradient(#f00 0px, #00f 10px)");
        assert_eq!(repeating.live_stops().len(), 2);
        assert_ne!(
            format!("{repeating:?}"),
            format!(
                "{:?}",
                one("background-image: linear-gradient(#f00 0px, #00f 10px)")
            )
        );

        let stack =
            layers("background-image: linear-gradient(#f00, #f00), linear-gradient(#00f, #00f)");
        assert_eq!(stack.len(), 2);
    }

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
            assert_eq!(layer.live_stops()[0].percentage, 1.0);
        }
        assert!(format!("{:?}", layers[0]).contains("LinearGradient(90"));

        let (_, x, _) = map_full("background-image: linear-gradient(#f00, #00f)");
        assert!(!x.resolve_background_layers()[0].is_tiled());
    }

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

        let (_, x, _) = map_full(
            "background-image: url(\"a.png\"); background-size: 1342px 268px; \
             background-position: -88px -44px",
        );
        let stack = x.resolve_background_stack().unwrap();
        let [PaintLayer::Image {
            size,
            position,
            repeat,
            ..
        }] = stack.as_slice()
        else {
            panic!("one image layer: {stack:?}");
        };
        assert_eq!(
            *size,
            BackgroundSize::Explicit(
                Some(TileLength::Pixels(1342.0)),
                Some(TileLength::Pixels(268.0))
            )
        );
        assert_eq!(
            *position,
            (TileLength::Pixels(-88.0), TileLength::Pixels(-44.0))
        );
        assert_eq!(*repeat, (true, true));
        let size_of = |body: &str| match map_full(body).1.resolve_background_stack().unwrap()[0] {
            PaintLayer::Image { size, .. } => size,
            ref other => panic!("{other:?}"),
        };
        assert_eq!(
            size_of("background: url(b.png) center / cover no-repeat"),
            BackgroundSize::Cover
        );
        assert_eq!(
            size_of("background-image: url(b.png); background-size: contain"),
            BackgroundSize::Contain
        );

        let (_, x, _) =
            map_full("background-image: linear-gradient(#0008, #0008), url(banner.png)");
        let stack = x.resolve_background_stack().unwrap();
        assert!(matches!(stack[0], PaintLayer::Image { .. }), "{stack:?}");
        assert!(matches!(stack[1], PaintLayer::Gradient(_)), "{stack:?}");

        let (_, x, _) = map_full("background-image: linear-gradient(#f00, #00f)");
        assert_eq!(x.resolve_background_stack(), None);
        assert_eq!(
            map_full("background-image: url(\"\")")
                .1
                .resolve_background_stack(),
            None
        );
    }

    #[test]
    fn a_later_background_image_replaces_an_earlier_url() {
        let (_, mut x, _) = map_full("background-image: url(a.png)");
        let (_, later, _) = map_full("background-image: linear-gradient(#f00, #00f)");
        x.refine(&later);
        assert_eq!(x.resolve_background_stack(), None);
        assert_eq!(x.resolve_background_layers().len(), 1);
    }

    #[test]
    fn letter_and_word_spacing_reach_the_text_style() {
        assert_eq!(
            clean("letter-spacing: 6.3px").text.letter_spacing,
            Some(px(6.3))
        );
        assert_eq!(clean("word-spacing: 2px").text.word_spacing, Some(px(2.0)));
        assert_eq!(
            clean("letter-spacing: normal").text.letter_spacing,
            Some(px(0.0))
        );
        assert_eq!(
            clean("letter-spacing: 0.5em").text.letter_spacing,
            Some(px(0.5 * crate::rem_basis()))
        );
    }

    #[test]
    fn no_underline_cancels_an_earlier_one() {
        let r = clean("text-decoration-line: underline; text-decoration-line: none");
        let u = r
            .text
            .underline
            .expect("declared, so it wins over the earlier one");
        assert_eq!(u.thickness, px(0.0));
        assert_eq!(u.color.map(|c| c.a), Some(0.0));
        let strike = r.text.strikethrough.expect("cleared too");
        assert_eq!(strike.thickness, px(0.0));
    }

    #[test]
    fn underline_offset_lands_on_the_decoration() {
        let (r, x, d) = map_full("text-decoration-line: underline; text-underline-offset: 4px");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(r.text.underline.expect("declared").offset, px(4.0));
        assert_eq!(x.underline_offset, Some(4.0));

        let (_, x, d) = map_full("text-underline-offset: 4px");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(x.underline_offset, Some(4.0));
    }

    #[test]
    fn font_features_reach_the_shaper_and_axes_do_not() {
        let features = clean("font-feature-settings: \"ss01\" 1, \"liga\" 0")
            .text
            .font_features
            .expect("declared");
        assert_eq!(
            &*features.0,
            &[("ss01".into(), 1u32), ("liga".into(), 0u32)]
        );

        let numeric = clean("font-variant-numeric: tabular-nums")
            .text
            .font_features
            .expect("declared");
        assert_eq!(&*numeric.0, &[("tnum".into(), 1u32)]);
    }

    #[test]
    fn kerning_and_ligature_keywords_become_features() {
        let features = |css: &str| {
            clean(css)
                .text
                .font_features
                .map(|f| (*f.0).clone())
                .unwrap_or_default()
        };
        let pairs = |p: &[(&str, u32)]| -> Vec<(String, u32)> {
            p.iter().map(|(t, v)| (t.to_string(), *v)).collect()
        };
        assert_eq!(features("font-kerning: none"), pairs(&[("kern", 0)]));
        assert!(features("font-kerning: auto").is_empty());
        assert!(features("font-kerning: normal").is_empty());
        assert!(features("font-variant-ligatures: normal").is_empty());
        assert_eq!(
            features("font-variant-ligatures: none"),
            pairs(&[
                ("liga", 0),
                ("clig", 0),
                ("dlig", 0),
                ("hlig", 0),
                ("calt", 0)
            ])
        );
        assert_eq!(
            features("font-variant-ligatures: no-common-ligatures discretionary-ligatures"),
            pairs(&[("liga", 0), ("clig", 0), ("dlig", 1)])
        );
        assert_eq!(
            features("font-variant-ligatures: no-contextual"),
            pairs(&[("calt", 0)])
        );
        assert_eq!(
            features(
                "font-variant-numeric: tabular-nums; font-kerning: none; \
                 font-variant-ligatures: no-common-ligatures; font-feature-settings: \"liga\" 1"
            ),
            pairs(&[("tnum", 1), ("kern", 0), ("liga", 1), ("clig", 0)])
        );
    }

    #[test]
    fn font_family_takes_the_first_installed_family() {
        let family = |css: &str| clean(css).text.font_family.map(|f| f.to_string());
        assert_eq!(
            family("font-family: Inter, sans-serif").as_deref(),
            Some("Inter")
        );

        set_font_source(|| vec!["Segoe UI".into(), "Consolas".into(), "Arial".into()]);
        assert_eq!(
            family("font-family: Inter, \"Segoe UI\", sans-serif").as_deref(),
            Some("Segoe UI")
        );
        assert_eq!(
            family(
                "font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, \
                 \"Liberation Mono\", \"Courier New\", monospace"
            )
            .as_deref(),
            Some("Consolas")
        );
        assert_eq!(
            family(
                "font-family: ui-sans-serif, system-ui, sans-serif, \"Apple Color Emoji\", \
                 \"Segoe UI Emoji\""
            )
            .as_deref(),
            Some(".SystemUIFont")
        );
        assert_eq!(
            family("font-family: monospace").as_deref(),
            Some("Consolas")
        );
        assert_eq!(family("font-family: sans-serif").as_deref(), Some("Arial"));
        assert_eq!(
            family("font-family: serif").as_deref(),
            Some(".SystemUIFont")
        );
        assert_eq!(
            family("font-family: Roboto, Helvetica").as_deref(),
            Some("Roboto")
        );
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
        assert_eq!(font("font-family: Mix, Arial"), (Some("Mix".into()), None));

        set_composed_font_families([("Mix".to_string(), true), ("SK PUA".to_string(), false)]);
        let some = |f: &str, rest: &str| (Some(f.to_string()), Some(rest.to_string()));
        assert_eq!(
            font("font-family: Mix, \"Segoe UI\", sans-serif"),
            some("Mix", "Segoe UI|Arial")
        );
        assert_eq!(
            font("font-family: \"SK PUA\", Nowhere, mix, sans-serif"),
            some("SK PUA", "mix|Arial")
        );
        assert_eq!(
            font("font-family: Nowhere, \"SK PUA\", Mix"),
            some("SK PUA", "Mix")
        );
        assert!(take_unreached_fonts().is_empty());
        assert_eq!(
            font("font-family: Arial, Mix, \"SK PUA\""),
            some("Arial", "")
        );
        assert_eq!(
            take_unreached_fonts(),
            vec![
                ("Mix".to_string(), "Arial".to_string()),
                ("SK PUA".to_string(), "Arial".to_string())
            ]
        );
        assert_eq!(font("font-family: Arial"), some("Arial", ""));
        assert!(take_unreached_fonts().is_empty());
        set_composed_font_families([]);
        assert_eq!(font("font-family: Mix"), (Some("Mix".into()), None));
    }

    #[test]
    fn waiting_web_fonts_are_recorded_for_the_host() {
        let web = |css: &str| extras(css).0.web_font.map(|f| f.join("|"));
        set_font_source(|| vec!["Arial".into(), "SK Half".into()]);
        assert_eq!(web("font-family: \"SK Web\", Arial"), None);

        set_web_font_families([
            "SK Web".to_string(),
            "sk half".to_string(),
            "SK PUA".to_string(),
        ]);
        assert_eq!(
            web("font-family: \"SK Web\", Arial").as_deref(),
            Some("SK Web")
        );
        assert_eq!(
            clean("font-family: \"SK Web\", Arial")
                .text
                .font_family
                .as_deref(),
            Some("Arial")
        );
        assert_eq!(
            web("font-family: Nowhere, sk web, sans-serif").as_deref(),
            Some("sk web")
        );
        assert_eq!(
            web("font-family: \"SK PUA\", \"SK Web\", serif").as_deref(),
            Some("SK PUA|SK Web")
        );
        assert_eq!(web("font-family: Arial, \"SK Web\"").as_deref(), Some(""));
        assert_eq!(
            web("font-family: sans-serif, \"SK Web\"").as_deref(),
            Some("")
        );
        assert_eq!(
            web("font-family: \"SK Half\", \"SK Web\"").as_deref(),
            Some("SK Half")
        );
        assert_eq!(web("color: red"), None);
        set_web_font_families(Vec::new());
    }

    #[test]
    fn the_visually_hidden_clip_idiom_hides_the_box() {
        assert_eq!(
            clean("clip: rect(0, 0, 0, 0)").visibility,
            Some(gpui::Visibility::Hidden)
        );
        assert_eq!(
            clean("clip: rect(0 0 0 0)").visibility,
            Some(gpui::Visibility::Hidden)
        );
        let (r, _) = map("clip: rect(0, 20px, 20px, 0)");
        assert_eq!(r.visibility, None);
    }

    #[test]
    fn ordered_markers_count_from_one() {
        use crate::ListMarker;
        assert_eq!(ListMarker::Disc.render(3).as_deref(), Some("\u{2022}"));
        assert_eq!(ListMarker::None.render(3), None);
        assert_eq!(ListMarker::Decimal.render(1).as_deref(), Some("1."));
        assert_eq!(
            ListMarker::DecimalLeadingZero.render(7).as_deref(),
            Some("07.")
        );
        assert_eq!(ListMarker::LowerAlpha.render(1).as_deref(), Some("a."));
        assert_eq!(ListMarker::LowerAlpha.render(26).as_deref(), Some("z."));
        assert_eq!(ListMarker::LowerAlpha.render(27).as_deref(), Some("aa."));
        assert_eq!(ListMarker::UpperAlpha.render(28).as_deref(), Some("AB."));
        assert_eq!(ListMarker::LowerRoman.render(4).as_deref(), Some("iv."));
        assert_eq!(
            ListMarker::UpperRoman.render(1994).as_deref(),
            Some("MCMXCIV.")
        );
    }

    #[test]
    fn outline_folds_the_shorthand_and_the_longhands_into_one_ring() {
        let r = clean("outline: 2px solid #ff0000; outline-offset: 3px");
        let o = r.vn_outline.expect("an outline");
        assert_eq!(o.width, px(2.0));
        assert_eq!(o.offset, px(3.0));
        assert_eq!(o.style, gpui::BorderStyle::Solid);
        assert!((o.color.a - 1.0).abs() < 1e-3, "{o:?}");

        let r = clean("outline: 2px solid #ff0000; outline: none");
        assert_eq!(r.vn_outline.unwrap().width, px(0.0));

        let r = clean("outline: 2px solid transparent; outline-offset: 2px");
        let o = r.vn_outline.unwrap();
        assert_eq!(o.width, px(2.0));
        assert_eq!(o.color.a, 0.0, "transparent paints nothing");

        assert_eq!(
            clean("outline-color: #ff0000").vn_outline.unwrap().width,
            px(0.0)
        );
        assert_eq!(
            clean("outline-style: solid").vn_outline.unwrap().width,
            px(3.0)
        );
        assert_eq!(
            clean("outline-style: dashed").vn_outline.unwrap().style,
            gpui::BorderStyle::Dashed
        );
    }

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
        let mut d = Vec::new();
        let mut r = StyleRefinement::default();
        r.box_shadow = Some(vec![gpui::BoxShadow {
            color: Hsla::default(),
            offset: gpui::point(px(0.0), px(1.0)),
            blur_radius: px(2.0),
            spread_radius: px(0.0),
            inset: false,
        }]);
        apply_dynamic(
            "box-shadow",
            "none",
            &Vars::default(),
            &crate::Env::default(),
            &mut r,
            &mut Extras::default(),
            &mut d,
        );
        assert_eq!(r.box_shadow, Some(Vec::new()), "{d:?}");
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn content_keeps_literal_strings_and_drops_the_rest() {
        let (x, d) = extras(r#"content: "->""#);
        assert_eq!(x.content.as_deref(), Some("->"));
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(extras(r#"content: """#).0.content.as_deref(), Some(""));
        assert_eq!(extras("content: none").0.content, None);
        for body in [
            "content: counter(x)",
            "content: attr(data-x)",
            "content: url(a.png)",
        ] {
            assert_eq!(extras(body).0.content, None, "{body}");
        }
    }

    #[test]
    fn text_transform_rewrites_the_text() {
        use crate::mapper::TextTransform as TT;
        assert_eq!(TT::Uppercase.apply("Sign in").as_deref(), Some("SIGN IN"));
        assert_eq!(TT::Lowercase.apply("Sign In").as_deref(), Some("sign in"));
        assert_eq!(
            TT::Capitalize.apply("sign iN now").as_deref(),
            Some("Sign IN Now")
        );
        assert_eq!(TT::None.apply("Sign in"), None);
    }

    #[test]
    fn referenced_color_filter_is_replaced_by_later_none_in_one_rule_or_cascade() {
        let (mut referenced, diagnostics) = extras("filter:url('#color-matrix')");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(referenced.filter_url.as_deref(), Some("#color-matrix"));
        referenced.refine(&extras("filter:none").0);
        assert!(referenced.filter_url.is_none());
        assert!(extras("filter:url('#color-matrix'); filter:none")
            .0
            .filter_url
            .is_none());
        assert!(extras("filter:drop-shadow(1px 2px red); filter:none")
            .0
            .drop_shadow
            .is_none());
    }

    #[test]
    fn a_blur_is_recorded_even_beside_other_filters() {
        let (x, d) = extras("filter: blur(64px)");
        assert_eq!(x.blur, Some(64.0));
        assert!(
            d.is_empty(),
            "a lone blur() is implemented, not advised: {d:?}"
        );

        assert_eq!(extras("filter: none").0.blur, Some(0.0));

        assert_eq!(extras("filter: blur(8px)  ").0.blur, Some(8.0));

        let (x, _) = extras("filter: blur(4px) grayscale(1)");
        assert_eq!(x.blur, Some(4.0), "the blur still lands");
    }

    #[test]
    fn nickname_six_color_and_thirteen_stop_travelling_gradients_keep_every_stop() {
        let palette = [0xff3355, 0xffaa00, 0x00ff44, 0x0088ff, 0x9933ff, 0xff55bb];
        for travelling in [false, true] {
            let colors: Vec<_> = if travelling {
                palette
                    .iter()
                    .chain(palette.iter())
                    .chain(palette[..1].iter())
                    .copied()
                    .collect()
            } else {
                palette.to_vec()
            };
            let css = format!("background-image:linear-gradient(90deg,{});background-clip:text;color:transparent;background-size:200% 100%;background-position:-100% 0",
                colors.iter().map(|color| format!("#{color:06x}")).collect::<Vec<_>>().join(","));
            let (extras, diagnostics) = extras(&css);
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            let layers = extras
                .background_images
                .as_ref()
                .expect("gradient brush stack");
            for (index, color) in colors.iter().enumerate() {
                let expected: Hsla = gpui::rgb(*color).into();
                let position = index as f32 / (colors.len() - 1) as f32;
                assert!(
                    layers
                        .iter()
                        .flat_map(|layer| layer.background.live_stops())
                        .any(|stop| stop.color == expected
                            && (stop.percentage - position).abs() < 1e-5),
                    "stop {index}/{} must survive transparent-band packing",
                    colors.len()
                );
            }
            assert!(layers
                .iter()
                .all(|layer| layer.background.live_stops().len() <= gpui::GRADIENT_STOPS));
            assert_eq!(extras.background_clip_text, Some(true));
            assert_eq!(
                extras.background_position.as_ref().unwrap()[0].0,
                TileLength::Fraction(-1.)
            );
        }
    }

    #[test]
    fn background_clip_text_records_the_clip_and_background_metadata() {
        let (x, _) = extras(
            "background-image: linear-gradient(to right, #fbbf24, #eab308); \
             background-clip: text; color: transparent",
        );
        assert_eq!(x.background_clip_text, Some(true));
        let key = x
            .background_key_color
            .expect("a gradient contributes a key colour");
        assert!(key.s > 0.8 && key.l > 0.4 && key.l < 0.7, "{key:?}");
        assert!(
            key.h < 0.15,
            "an amber ramp must not average to a cold hue: {key:?}"
        );

        let (x, _) = extras("background-color: #ff0000; -webkit-background-clip: text");
        assert_eq!(x.background_clip_text, Some(true));
        assert_eq!(x.background_key_color.map(|c| c.h), Some(0.0));
    }

    #[test]
    fn rem_resolves_against_the_root_font_size() {
        assert_eq!(rem_basis(), INITIAL_REM_BASIS);
        assert_eq!(
            clean("width: 32rem").size.width,
            Some(Length::Definite(px(512.0).into()))
        );

        assert!(set_rem_basis(14.0), "a new basis is a change");
        assert!(!set_rem_basis(14.0), "the same basis is not");
        assert_eq!(
            clean("width: 32rem").size.width,
            Some(Length::Definite(px(448.0).into()))
        );

        // A basis that would silently rescale the whole app is refused, and "the root has no
        // font-size yet" is a normal state, not a change.
        assert!(!set_rem_basis(0.0));
        assert!(!set_rem_basis(f32::NAN));
        assert_eq!(rem_basis(), 14.0);

        set_rem_basis(INITIAL_REM_BASIS);
    }

    #[test]
    fn ch_and_ex_compile_to_half_an_em_of_the_root() {
        let w = |body: &str| clean(body).size.width;
        assert_eq!(w("width: 1ch"), Some(Length::Definite(px(8.0).into())));
        assert_eq!(w("width: 3ex"), Some(Length::Definite(px(24.0).into())));
        assert_eq!(w("width: 2rch"), Some(Length::Definite(px(16.0).into())));
        assert_eq!(w("width: 2rex"), Some(Length::Definite(px(16.0).into())));
        assert_eq!(
            w("width: calc(2ch + 1px)"),
            Some(Length::Definite(px(17.0).into()))
        );

        assert!(set_rem_basis(14.0));
        assert_eq!(w("width: 1ch"), Some(Length::Definite(px(7.0).into())));
        set_rem_basis(INITIAL_REM_BASIS);
    }

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

    #[test]
    fn border_radius_inherit_is_recorded_per_corner() {
        let inherit = |name: &str| {
            let (mut r, mut x, mut d) = (StyleRefinement::default(), Extras::default(), Vec::new());
            apply_dynamic(
                name,
                "inherit",
                &Vars::default(),
                &crate::Env::default(),
                &mut r,
                &mut x,
                &mut d,
            );
            assert!(d.is_empty(), "{name}: {d:?}");
            (r, x)
        };
        let (r, x) = inherit("border-radius");
        assert_eq!(
            x.radius_inherit,
            RadiusInherit {
                declared: 15,
                inherit: 15
            }
        );
        assert_eq!(
            r.corner_radii.top_left, None,
            "nothing to write until the parent is known"
        );
        let (_, x) = inherit("border-bottom-left-radius");
        let bl = RadiusInherit::BOTTOM_LEFT;
        assert_eq!(
            x.radius_inherit,
            RadiusInherit {
                declared: bl,
                inherit: bl
            }
        );
        assert_eq!(inherit("color").1.radius_inherit, RadiusInherit::default());

        let (_, x, d) = map_full("border-radius: 4px; border-top-left-radius: 50%");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            x.radius_inherit,
            RadiusInherit {
                declared: 15,
                inherit: 0
            }
        );
        let mut folded = inherit("border-radius").1;
        folded.refine(&map_full("border-top-right-radius: 2px").1);
        assert_eq!(
            folded.radius_inherit.inherit,
            RadiusInherit::ALL & !RadiusInherit::TOP_RIGHT
        );
        folded.refine(&inherit("border-top-right-radius").1);
        assert_eq!(folded.radius_inherit.inherit, RadiusInherit::ALL);
    }

    #[test]
    fn a_property_with_no_divergence_says_nothing() {
        for body in [
            "user-select: none",
            "user-select: text",
            "user-select: auto",
            "user-select: all",
            "appearance: none",
            "appearance: textfield",
            "tab-size: 4",
            "text-size-adjust: 100%",
            "touch-action: none",
            "-webkit-font-smoothing: antialiased",
            "-webkit-box-orient: vertical",
            "vertical-align: baseline",
            "text-indent: 0",
            "resize: none",
            "overflow-wrap: normal",
            "word-break: normal",
            "overflow-wrap: break-word",
            "overflow-wrap: anywhere",
            "word-wrap: break-word",
            "word-break: break-all",
            "word-break: keep-all",
            "word-break: break-word",
            "hyphens: manual",
            "hyphens: none",
            "list-style: none",
            "list-style-type: none",
            "list-style-type: disc",
            "list-style: decimal inside",
            "list-style-position: inside",
            "text-decoration-color: #f00",
            "visibility: collapse",
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
            "font-optical-sizing: auto",
            "text-underline-offset: 4px",
            "text-decoration-skip-ink: none",
            "clip: rect(0, 0, 0, 0)",
            "scroll-margin-top: 4rem",
            "scroll-padding: 1rem",
            "background-size: cover",
            "background-image: linear-gradient(#f00, #00f); background-size: 64px 64px",
            "background-repeat: no-repeat",
            "background-position: center",
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
        assert_eq!(
            r.vn_border_colors.top,
            Some(Some(gpui::rgb(0xff0000).into()))
        );
        assert_eq!(
            r.vn_border_colors.left,
            Some(Some(gpui::rgb(0x00ff00).into()))
        );
        assert_eq!(r.vn_border_colors.right, None);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn border_color_shorthand_retains_four_colours_and_resets_side_overrides() {
        let (r, diagnostics) = map("border-color: red green blue transparent");
        assert_eq!(
            r.vn_border_colors.top,
            Some(Some(gpui::rgb(0xff0000).into()))
        );
        assert_eq!(
            r.vn_border_colors.right,
            Some(Some(gpui::rgb(0x008000).into()))
        );
        assert_eq!(
            r.vn_border_colors.bottom,
            Some(Some(gpui::rgb(0x0000ff).into()))
        );
        assert!(r
            .vn_border_colors
            .left
            .flatten()
            .is_some_and(|c| c.is_transparent()));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let (r, diagnostics) = map("border-left-color: blue; border-color: red");
        assert_eq!(
            r.vn_border_colors.left,
            Some(Some(gpui::rgb(0xff0000).into()))
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    #[test]
    fn scrollbar_color_accepts_functional_colours() {
        for body in [
            "scrollbar-color: hsl(240 5% 64% / 0.35) transparent",
            "scrollbar-color: rgba(128, 24, 255, 0.5) #0000",
            "scrollbar-color: hsl(0 0% 10%) transparent",
        ] {
            let (_, extras, diagnostics) = map_full(body);
            assert!(extras.scrollbar_color.is_some(), "{body}: {diagnostics:?}");
            assert!(diagnostics.is_empty(), "{body}: {diagnostics:?}");
        }
    }

    #[test]
    fn transform_origin_survives_beside_the_transform() {
        let spec = transform("transform-origin: left top; transform: rotate(45deg)")
            .0
            .expect("declared");
        assert_eq!(
            spec.origin,
            (
                TransformLength::Fraction(0.0),
                TransformLength::Fraction(0.0)
            )
        );
        let spec = transform("transform: rotate(45deg); transform-origin: 100% 0")
            .0
            .expect("declared");
        assert_eq!(
            spec.origin,
            (TransformLength::Fraction(1.0), TransformLength::Pixels(0.0))
        );
        assert_eq!(
            transform("transform: scale(2)").0.unwrap().origin,
            TransformSpec::CENTRE_ORIGIN
        );
        let alone = transform("transform-origin: left top").0.expect("declared");
        assert!(alone.is_identity(), "an origin alone transforms nothing");
    }

    #[test]
    fn a_corner_transform_origin_pins_that_corner() {
        use gpui::{Bounds, Point, Size};
        let bounds = Bounds {
            origin: Point {
                x: px(100.0),
                y: px(50.0),
            },
            size: Size {
                width: px(200.0),
                height: px(100.0),
            },
        };
        let spec = transform("transform-origin: left top; transform: scale(2)")
            .0
            .expect("declared");
        let m = spec.to_matrix(bounds, 1.0);
        let pinned = m.apply(bounds.origin);
        assert!((f32::from(pinned.x) - 100.0).abs() < 1e-3, "{pinned:?}");
        assert!((f32::from(pinned.y) - 50.0).abs() < 1e-3, "{pinned:?}");
        let far = m.apply(Point {
            x: px(300.0),
            y: px(150.0),
        });
        assert!((f32::from(far.x) - 500.0).abs() < 1e-3, "{far:?}");
        assert!((f32::from(far.y) - 250.0).abs() < 1e-3, "{far:?}");
    }

    #[test]
    fn a_matrix_decomposes_into_translate_rotate_scale() {
        let spec = transform("transform: matrix(2, 0, 0, 2, 10, 20)")
            .0
            .expect("declared");
        assert_eq!(
            spec.translate,
            (TransformLength::Pixels(10.0), TransformLength::Pixels(20.0))
        );
        assert!((spec.scale.0 - 2.0).abs() < 1e-4, "{spec:?}");
        assert!((spec.scale.1 - 2.0).abs() < 1e-4, "{spec:?}");
        assert!(spec.rotate.abs() < 1e-4, "{spec:?}");
        let spec = transform("transform: matrix(0, 1, -1, 0, 0, 0)")
            .0
            .expect("declared");
        assert!(
            (spec.rotate - std::f32::consts::FRAC_PI_2).abs() < 1e-4,
            "{spec:?}"
        );
        assert!((spec.scale.0 - 1.0).abs() < 1e-4 && (spec.scale.1 - 1.0).abs() < 1e-4);
        let (_, d) = map("transform: matrix(1, 0, 0.5, 1, 0, 0)");
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn finite_affine_matrices_keep_every_coefficient_including_singular_ones() {
        use gpui::{Bounds, Point, Size};
        let bounds = Bounds {
            origin: Point::new(px(100.0), px(50.0)),
            size: Size::new(px(200.0), px(100.0)),
        };
        for [a, b, c, d, e, f] in [
            [1.0, 0.25, 0.5, 1.0, 10.0, 20.0],
            [-2.0, 0.0, 0.5, 3.0, -10.0, 4.0],
            [0.0, 0.0, 2.0, 3.0, 0.0, 0.0],
            [1.0, 2.0, 2.0, 4.0, 0.0, 0.0],
            [0.0; 6],
        ] {
            let (spec, diagnostics) = transform(&format!(
                "transform-origin:0 0; transform:matrix({a},{b},{c},{d},{e},{f})"
            ));
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            let matrix = spec.unwrap().to_matrix(bounds, 1.0);
            for (x, y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (30.0, -17.0)] {
                let point = matrix.apply(Point::new(px(100.0 + x), px(50.0 + y)));
                assert!(
                    (f32::from(point.x) - (100.0 + a * x + c * y + e)).abs() < 0.001,
                    "{matrix:?} {point:?}"
                );
                assert!(
                    (f32::from(point.y) - (50.0 + b * x + d * y + f)).abs() < 0.001,
                    "{matrix:?} {point:?}"
                );
            }
        }
    }

    #[test]
    fn projected_editor_matrix_preserves_centring_and_post_rotation_scale() {
        use gpui::{Bounds, Point, Size};
        let bounds = Bounds {
            origin: Point::new(px(0.0), px(0.0)),
            size: Size::new(px(200.0), px(100.0)),
        };
        let (spec, diagnostics) = transform(
            "transform:translate(-50%,-50%) matrix(1,.25,.5,1,0,0) rotate(90deg) scale(2,3)",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let spec = spec.unwrap();
        assert!(spec.translation_basis.is_none());
        let matrix = spec.to_matrix(bounds, 1.0);
        for (x, y, ex, ey) in [
            (100.0, 50.0, 0.0, 0.0),
            (101.0, 50.0, 1.0, 2.0),
            (100.0, 51.0, -3.0, -0.75),
        ] {
            let point = matrix.apply(Point::new(px(x), px(y)));
            assert!(
                (f32::from(point.x) - ex).abs() < 0.001 && (f32::from(point.y) - ey).abs() < 0.001,
                "{point:?} vs {ex},{ey}"
            );
        }
    }

    #[test]
    fn ordered_translation_can_mix_pixels_and_both_box_axes_at_any_dpi() {
        use gpui::{Bounds, Point, Size};
        let bounds = Bounds {
            origin: Point::new(px(0.0), px(0.0)),
            size: Size::new(px(200.0), px(100.0)),
        };
        for (css, x, y) in [
            ("rotate(90deg) translate(50%,25%)", -25.0, 100.0),
            ("translate(-50%,25%) matrix(1,0,0,1,10,-4)", -90.0, 21.0),
            ("matrix(1,.25,.5,1,10,20) translate(8px,4px)", 20.0, 26.0),
        ] {
            let (spec, diagnostics) = transform(&format!("transform-origin:0 0; transform:{css}"));
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            for dpi in [1.0, 2.0] {
                let point = spec
                    .unwrap()
                    .to_matrix(bounds, dpi)
                    .apply(Point::new(px(0.0), px(0.0)));
                assert!(
                    (f32::from(point.x) - x * dpi).abs() < 0.001
                        && (f32::from(point.y) - y * dpi).abs() < 0.001,
                    "{css}: {point:?}"
                );
            }
        }
    }

    #[test]
    fn shear_and_mixed_translation_survive_cssom_and_waapi_serialization() {
        use gpui::{Bounds, Point, Size};
        let bounds = Bounds {
            origin: Point::new(px(0.0), px(0.0)),
            size: Size::new(px(200.0), px(100.0)),
        };
        for css in [
            "matrix(1,.25,.5,1,10,20)",
            "rotate(90deg) translate(50%,25%) skewX(20deg)",
            "translate(-50%,25%) matrix(1,0,.5,1,10,-4)",
        ] {
            let original = transform(&format!("transform:{css}")).0.unwrap();
            let text = transform_css(original);
            let replayed = transform(&format!("transform:{text}")).0.unwrap();
            for (x, y) in [(0.0, 0.0), (20.0, 30.0), (200.0, 100.0)] {
                let p = Point::new(px(x), px(y));
                let a = original.to_matrix(bounds, 1.0).apply(p);
                let b = replayed.to_matrix(bounds, 1.0).apply(p);
                assert!(
                    (f32::from(a.x) - f32::from(b.x)).abs() < 0.001
                        && (f32::from(a.y) - f32::from(b.y)).abs() < 0.001,
                    "{css} -> {text}: {a:?} vs {b:?}"
                );
            }
        }
        let spin = transform("transform:rotate(360deg) skew(0deg,0deg)")
            .0
            .unwrap();
        assert!(
            (TransformSpec::IDENTITY.interpolate(spin, 0.5).rotate - std::f32::consts::PI).abs()
                < 0.0001
        );
        let shear = transform("transform:matrix(1,0,1,1,0,0)").0.unwrap();
        assert!((TransformSpec::IDENTITY.interpolate(shear, 0.5).shear - 0.5).abs() < 0.0001);
    }

    #[test]
    fn the_matrix_turns_about_the_box_centre() {
        use gpui::{Bounds, Point, Size};
        let bounds = Bounds {
            origin: Point {
                x: px(100.0),
                y: px(50.0),
            },
            size: Size {
                width: px(200.0),
                height: px(100.0),
            },
        };
        let quarter = transform("transform: rotate(90deg)").0.expect("declared");
        let m = quarter.to_matrix(bounds, 1.0);
        let centre = m.apply(bounds.center());
        assert!((f32::from(centre.x) - 200.0).abs() < 1e-3, "{centre:?}");
        assert!((f32::from(centre.y) - 100.0).abs() < 1e-3, "{centre:?}");
        let corner = m.apply(bounds.origin);
        assert!((f32::from(corner.x) - 250.0).abs() < 1e-3, "{corner:?}");
        assert!(f32::from(corner.y).abs() < 1e-3, "{corner:?}");

        let half = transform("transform: translate(-50%, -50%)")
            .0
            .expect("declared");
        let m = half.to_matrix(bounds, 1.0);
        let moved = m.apply(bounds.origin);
        assert!((f32::from(moved.x) - 0.0).abs() < 1e-3, "{moved:?}");
        assert!((f32::from(moved.y) - 0.0).abs() < 1e-3, "{moved:?}");
    }

    #[test]
    fn svg_paint_keeps_the_concrete_colour() {
        assert_eq!(extras("fill: none").0.fill, Some(SvgPaint::None));
        assert_eq!(extras("fill: currentColor").0.fill, Some(SvgPaint::Current));
        let red = extras("fill: #ff0000").0.fill;
        assert!(matches!(red, Some(SvgPaint::Color(_))), "{red:?}");
        assert_eq!(red.unwrap().as_attr(), "rgb(255,0,0)");
        let faint = extras("stroke: rgba(219, 232, 255, 0.17)")
            .0
            .stroke
            .expect("declared");
        assert_eq!(faint.as_attr(), "rgba(219,232,255,0.169)");
        assert_eq!(
            extras("stroke: currentColor").0.stroke,
            Some(SvgPaint::Current)
        );
        assert_eq!(SvgPaint::None.as_attr(), "none");
        assert_eq!(extras("fill: url(#g)").0.fill, Some(SvgPaint::Server));
        assert_eq!(SvgPaint::Server.as_attr(), "currentColor");
        assert_eq!(extras("color: red").0.fill, None);
    }

    #[test]
    fn svg_presentation_properties_travel_as_text_and_merge_per_name() {
        let (x, d) = extras(
            "fill: none; stroke: currentColor; stroke-opacity: 0.28; stroke-width: 1; \
             shape-rendering: crispEdges",
        );
        assert!(d.is_empty(), "{d:?}");
        let list = x.svg_presentation.clone().expect("carried");
        let get = |l: &[(&'static str, Rc<str>)], n: &str| {
            l.iter().find(|(k, _)| *k == n).map(|(_, v)| v.to_string())
        };
        assert_eq!(get(&list, "stroke-opacity").as_deref(), Some(".28")); // lightningcss's minified text, a valid SVG <number>
        assert_eq!(get(&list, "stroke-width").as_deref(), Some("1px"));
        assert_eq!(get(&list, "shape-rendering").as_deref(), Some("crispEdges"));
        let (dash, d) = extras("stroke-dasharray: 5 5; stroke-dashoffset: 10px");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(dash.svg_presentation.as_ref().map(|l| l.len()), Some(2));
        let mut merged = x.clone();
        merged.refine(&extras("stroke-width: 2").0);
        let list = merged.svg_presentation.expect("merged");
        assert_eq!(get(&list, "stroke-width").as_deref(), Some("2px"));
        assert_eq!(get(&list, "stroke-opacity").as_deref(), Some(".28"));
    }

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
        assert_eq!(
            later("margin-left: 3px; margin-inline-start: 5px"),
            Some(px(5.0).into())
        );
        assert_eq!(
            later("margin-inline-start: 5px; margin-left: 3px"),
            Some(px(3.0).into())
        );
        assert_eq!(
            clean("margin: 1px; margin-inline-end: 9px").margin.right,
            Some(px(9.0).into())
        );
    }

    #[test]
    fn the_demo_cards_utilities_all_render() {
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

    #[test]
    fn drop_shadow_parses_into_extras_and_says_nothing() {
        let (_, x, d) = map_full("filter: drop-shadow(2px 4px 6px rgb(0 0 0 / 0.5))");
        assert!(
            d.is_empty(),
            "the shape gpui can draw exactly must not advise: {d:?}"
        );
        let s = x.drop_shadow.expect("drop-shadow() reached Extras");
        assert_eq!((s.dx, s.dy, s.blur), (2.0, 4.0, 6.0));
        assert!(s.color.is_some(), "an explicit colour is flattened here");

        // No colour argument is `currentColor`, which only the renderer can resolve — so the mapper
        // records `None` rather than guessing black.
        let (_, x, d) = map_full("filter: drop-shadow(1px 2px 3px)");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(x.drop_shadow.map(|s| s.color), Some(None));

        let (_, x, _) =
            map_full("filter: drop-shadow(1px 1px 1px red) drop-shadow(2px 2px 2px blue)");
        assert!(x.drop_shadow.is_some(), "the first one is still honoured");
    }

    #[test]
    fn the_gradient_border_mask_is_a_ring_and_others_are_not() {
        let ring = "-webkit-mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0);
                    mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0);
                    -webkit-mask-composite: xor;
                    mask-composite: exclude";
        let (_, x, d) = map_full(ring);
        assert!(d.is_empty(), "{d:?}");
        assert!(x.mask_ring());
        for composite in [
            "-webkit-mask-composite: xor",
            "-webkit-mask-composite: destination-out",
        ] {
            let (_, x, d) = map_full(&format!(
                "mask: linear-gradient(#000 0 0) content-box, linear-gradient(#000 0 0); {composite}"
            ));
            assert!(d.is_empty() && x.mask_ring(), "{composite}: {d:?}");
        }
        let (_, x, d) =
            map_full("mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0)");
        assert!(d.is_empty(), "{d:?}");
        assert!(!x.mask_ring());
        let (_, x, d) = map_full(
            "mask-image: linear-gradient(red, blue), linear-gradient(#fff 0 0);
             mask-clip: content-box, border-box; mask-composite: exclude",
        );
        assert!(d.is_empty() && x.mask_ring(), "{d:?}");
        // A shorthand AFTER the composite resets it to `add`.
        let (_, x, _) = map_full(&format!(
            "{ring}; mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0)"
        ));
        assert!(!x.mask_ring());

        for other in [
            "mask-image: linear-gradient(to right, black 90%, transparent)",
            "mask-image: url(icon.svg)",
            "mask: linear-gradient(#fff 0 0) content-box, linear-gradient(#fff 0 0); mask-composite: intersect",
        ] {
            assert!(!map_full(other).1.mask_ring(), "{other}");
        }
        let (_, x, d) = map_full("mask: none");
        assert!(d.is_empty() && !x.mask_ring(), "{d:?}");
    }

    #[test]
    fn clip_path_is_a_rounded_mask_and_the_other_shapes_are_dropped() {
        use gpui::{VnClipBox as B, VnClipPath as C, VnCornerRadius, VnShapeRadius as R};
        let at = |x: f32, y: f32| VnCornerRadius {
            x: px(x).into(),
            y: px(y).into(),
        };
        let half = relative(0.5);
        for sr_only in ["clip-path: inset(50%)", "clip-path: inset(50% 50% 50% 50%)"] {
            let r = clean(sr_only);
            assert_eq!(r.visibility, None, "{sr_only}");
            assert_eq!(
                r.vn_clip_path,
                Some(C::Inset {
                    reference: B::Border,
                    insets: [half; 4],
                    radii: [at(0., 0.); 4]
                }),
                "{sr_only}"
            );
        }
        assert_eq!(
            clean("clip-path: inset(10px 20% round 8px 8px 8px 4px / 8px 8px 8px 6px) padding-box")
                .vn_clip_path,
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
        assert_eq!(
            clean("clip-path: circle(at right 10px top) content-box").vn_clip_path,
            Some(C::Ellipse {
                reference: B::Content,
                center: [
                    DefiniteLength::Calc {
                        fraction: 1.0,
                        pixels: px(-10.)
                    },
                    relative(0.)
                ],
                radius: [R::ClosestSide; 2],
                circle: true,
            })
        );
        assert_eq!(
            clean("clip-path: ellipse(30% farthest-side at 25% 75%)").vn_clip_path,
            Some(C::Ellipse {
                reference: B::Border,
                center: [relative(0.25), relative(0.75)],
                radius: [R::Length(relative(0.3)), R::FarthestSide],
                circle: false,
            })
        );
        assert!(clean("clip-path: ellipse(20px 20px)")
            .vn_clip_path
            .is_some());
        assert_eq!(
            clean("clip-path: margin-box").vn_clip_path,
            Some(C::Box(B::Margin))
        );
        assert_eq!(
            clean("clip-path: fill-box").vn_clip_path,
            Some(C::Box(B::Content))
        );
        assert_eq!(
            clean("clip-path: stroke-box").vn_clip_path,
            Some(C::Box(B::Border))
        );
        for other in [
            "url(#c)",
            "rect(0 10px 10px 0)",
            "xywh(0 0 10px 10px)",
            "shape(from 0 0, line to 10px 10px)",
        ] {
            let (r, _) = map(&format!("clip-path: {other}"));
            assert_eq!(r.vn_clip_path, None, "{other}");
        }
        let r = clean("clip-path: none");
        assert_eq!((r.vn_clip_path, r.visibility), (Some(C::None), None));
    }

    #[test]
    fn clip_path_polygon_and_path_are_shapes() {
        use gpui::{VnClipBox as B, VnClipPath as C, VnFillRule as F, VnPathVerb::*};
        let polygon = |r: &StyleRefinement| match &r.vn_clip_path {
            Some(C::Polygon {
                reference,
                fill_rule,
                points,
            }) => (*reference, *fill_rule, points.to_vec()),
            other => panic!("{other:?}"),
        };
        assert_eq!(
            polygon(&clean("clip-path: polygon(50% 0, 100% 100%, 0 100%)")),
            (
                B::Border,
                F::NonZero,
                vec![
                    [relative(0.5), px(0.).into()],
                    [relative(1.), relative(1.)],
                    [px(0.).into(), relative(1.)]
                ]
            )
        );
        let (rb, rule, points) = polygon(&clean(
            "clip-path: polygon(evenodd, calc(100% - 8px) 0, 8px 100%) content-box",
        ));
        assert_eq!((rb, rule), (B::Content, F::EvenOdd));
        assert_eq!(
            points[0],
            [
                DefiniteLength::Calc {
                    fraction: 1.,
                    pixels: px(-8.)
                },
                px(0.).into()
            ]
        );
        assert_eq!(points[1], [px(8.).into(), relative(1.)]);

        let path = |r: &StyleRefinement| match &r.vn_clip_path {
            Some(C::Path {
                reference,
                fill_rule,
                verbs,
            }) => (*reference, *fill_rule, verbs.to_vec()),
            other => panic!("{other:?}"),
        };
        let notch = "clip-path: path('M0 0 C12.1506 0 22 9.8494 22 22 V42 Q22 64 44 64 H420 Q442 64 442 42 V22 C442 9.8494 451.8494 0 464 0 Z')";
        let (rb, rule, verbs) = path(&clean(notch));
        assert_eq!((rb, rule, verbs.len()), (B::Border, F::NonZero, 9));
        assert_eq!(verbs[3], QuadTo([22., 64.], [44., 64.]));
        let mut d = Vec::new();
        let inline = parse_inline_style(notch, &mut d);
        assert!(d.is_empty(), "{d:?}");
        assert!(format!("{inline:?}").contains("Path"), "{inline:?}");
        let (rb, rule, verbs) = path(&clean(
            r#"clip-path: path(evenodd, "M0 0 h10 v10 h-10 z M2 2 h6 v6 h-6 z") padding-box"#,
        ));
        assert_eq!((rb, rule, verbs.len()), (B::Padding, F::EvenOdd, 10));
        assert_eq!(verbs[5], MoveTo([2., 2.]));
        assert_eq!(path(&clean("clip-path: path('')")).2, vec![]);
        for bad in [
            "path('M0 0 L')",
            "path('L10 10')",
            "path(round, 'M0 0')",
            "path(evenodd)",
            "path('M0 0') blue",
        ] {
            let (r, _) = map(&format!("clip-path: {bad}"));
            assert_eq!(r.vn_clip_path, None, "{bad}");
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
    fn scrollbar_color_takes_two_colours_thumb_first() {
        let x = extras_of("scrollbar-color: #ff0000 #0000ff");
        let (thumb, track) = x.scrollbar_color.expect("a pair");
        assert!(
            thumb.h < 0.02,
            "thumb is the first colour (red), got {thumb:?}"
        );
        assert!(
            (track.h - 0.6667).abs() < 0.02,
            "track is the second (blue), got {track:?}"
        );
        assert_eq!(extras_of("scrollbar-color: auto").scrollbar_color, None);
        assert_eq!(
            extras_of("scrollbar-color: red").scrollbar_color,
            None,
            "one colour is not a pair"
        );
        let named = extras_of("scrollbar-color: red blue");
        let (thumb, track) = named.scrollbar_color.expect("a pair of named colours");
        assert!(thumb.h < 0.02, "thumb is red, got {thumb:?}");
        assert!(
            (track.h - 0.6667).abs() < 0.02,
            "track is blue, got {track:?}"
        );
    }

    #[test]
    fn overscroll_behavior_sets_both_axes_or_one() {
        let both = extras_of("overscroll-behavior: contain");
        assert_eq!(both.overscroll_x, Some(OverscrollBehavior::Contain));
        assert_eq!(both.overscroll_y, Some(OverscrollBehavior::Contain));
        let y = extras_of("overscroll-behavior-y: none");
        assert_eq!(y.overscroll_x, None);
        assert_eq!(y.overscroll_y, Some(OverscrollBehavior::None));
    }

    #[test]
    fn a_malformed_inline_declaration_is_skipped_and_the_rest_applies() {
        let mut d = Vec::new();
        let inline = parse_inline_style("0: d; display: flex; }; gap: 4px", &mut d);
        let r = inline
            .refinement
            .expect("the valid declarations still apply");
        assert_eq!(r.display, Some(gpui::Display::Flex));
        assert!(
            r.gap.width.is_some(),
            "gap after the bad declarations applies too"
        );
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
        // A prefix Chromium ignores is not accepted, a `var()` value included; the `-webkit-` alias
        // it honours is.
        assert!(!accepts_declaration("-moz-appearance", "none"));
        assert!(!accepts_declaration("-moz-transition", "var(--t)"));
        assert!(accepts_declaration("-webkit-user-select", "none"));
    }
}

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
        assert_eq!(extras("-moz-user-select: none").0.user_select, None);
        assert_eq!(
            extras("-webkit-user-select: none").0.user_select,
            Some(UserSelect::None)
        );
        fn block(body: &str) -> DeclarationBlock<'_> {
            DeclarationBlock::parse_string(body, ParserOptions::default()).expect("parses")
        }
        let c = convert_block(&block(
            "-moz-transition: opacity 1s; -ms-transform: scale(2); -webkit-touch-callout: none",
        ));
        assert!(c.normal.is_none() && c.normal_transitions.is_none());
        assert!(c.diagnostics.is_none(), "{:?}", c.diagnostics);
        let c = convert_block(&block(
            "-webkit-transition: opacity 1s; -webkit-transform: scale(2)",
        ));
        assert!(c.normal.is_some() && c.normal_transitions.is_some());
        assert!(c.diagnostics.is_none(), "{:?}", c.diagnostics);
    }

    #[test]
    fn webkit_text_fill_color_is_a_glyph_colour() {
        let is_red = |f: Option<TextFill>| match f {
            Some(TextFill::Color(c)) => {
                c.h.abs() < 1e-3 && (c.s - 1.0).abs() < 1e-3 && (c.l - 0.5).abs() < 1e-3
            }
            _ => false,
        };
        assert!(is_red(
            extras("-webkit-text-fill-color: #ff0000").0.text_fill_color
        ));
        assert!(is_red(
            extras("-webkit-text-fill-color: red").0.text_fill_color
        ));
        assert_eq!(
            extras("-webkit-text-fill-color: currentColor")
                .0
                .text_fill_color,
            Some(TextFill::CurrentColor)
        );
        match extras("-webkit-text-fill-color: transparent")
            .0
            .text_fill_color
        {
            Some(TextFill::Color(c)) => assert_eq!(c.a, 0.0),
            other => panic!("{other:?}"),
        }
        let (x, d) = extras(
            "background-image: linear-gradient(90deg, #f00, #00f); -webkit-background-clip: text; \
             background-clip: text; -webkit-text-fill-color: transparent",
        );
        assert_eq!(x.background_clip_text, Some(true));
        assert!(x.text_fill_color.is_some());
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            extras("-webkit-text-fill-color: 3px").0.text_fill_color,
            None
        );
    }
}
