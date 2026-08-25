//! `lightningcss::Property` → `gpui::StyleRefinement`. The load-bearing match
//! statement: this file decides what CSS vue-native renders. Unsupported
//! declarations produce a DIAGNOSTIC, never silence (the redcss `Coverage`
//! philosophy) — a property that parses but does nothing is the worst DX bug
//! a styling system can have.

use std::rc::Rc;

use gpui::{px, relative, AbsoluteLength, DefiniteLength, Hsla, Length, Overflow, StyleRefinement};
use lightningcss::declaration::DeclarationBlock;
use lightningcss::properties::align::{ContentPosition, SelfPosition};
use lightningcss::properties::border::LineStyle;
use lightningcss::properties::display::{DisplayInside, DisplayKeyword};
use lightningcss::properties::flex::FlexDirection as CssFlexDirection;
use lightningcss::properties::font::{AbsoluteFontWeight, FontFamily as CssFontFamily, FontWeight as CssFontWeight, LineHeight as CssLineHeight};
use lightningcss::properties::overflow::OverflowKeyword;
use lightningcss::properties::size::Size as CssSize;
use lightningcss::properties::text::TextAlign as CssTextAlign;
use lightningcss::properties::Property;
use lightningcss::stylesheet::ParserOptions;
use lightningcss::values::color::CssColor;
use lightningcss::values::length::{Length as CssLength, LengthPercentage, LengthPercentageOrAuto, LengthValue};
use lightningcss::values::percentage::DimensionPercentage;

use crate::transition::{Collector, TransitionSpec};

/// One declaration block, split into the normal/important refinements and the
/// transition lists each half declares (`None` = the half declared none, which
/// is what lets a later rule's `transition` override an earlier one without an
/// empty list being mistaken for `transition: none`).
pub(crate) struct Converted {
    pub normal: Option<Rc<StyleRefinement>>,
    pub important: Option<Rc<StyleRefinement>>,
    pub normal_transitions: Option<Rc<Vec<TransitionSpec>>>,
    pub important_transitions: Option<Rc<Vec<TransitionSpec>>>,
}

/// Converts a declaration block into (normal, important) refinements plus the
/// transitions each half declares.
pub(crate) fn convert_block(block: &DeclarationBlock, diagnostics: &mut Vec<String>) -> Converted {
    let mut convert =
        |props: &[Property]| -> (Option<Rc<StyleRefinement>>, Option<Rc<Vec<TransitionSpec>>>) {
            if props.is_empty() {
                return (None, None);
            }
            let mut r = StyleRefinement::default();
            let mut transitions = Collector::default();
            let mut styled = false;
            for p in props {
                if transitions.take(p, diagnostics) {
                    continue;
                }
                apply(p, &mut r, diagnostics);
                styled = true;
            }
            (
                styled.then(|| Rc::new(r)),
                transitions.finish().map(Rc::new),
            )
        };
    let (normal, normal_transitions) = convert(&block.declarations);
    let (important, important_transitions) = convert(&block.important_declarations);
    Converted {
        normal,
        important,
        normal_transitions,
        important_transitions,
    }
}

/// Parses an inline `style` body ("color: red; padding: 4px") into a
/// refinement — the transport for Vue `:style` objects and `style=""` strings.
///
/// Transitions declared inline are DROPPED here (the refinement has nowhere to
/// carry them); use [`parse_inline_with_transitions`] to keep them.
pub fn parse_inline(body: &str, diagnostics: &mut Vec<String>) -> StyleRefinement {
    let (refinement, transitions) = parse_inline_with_transitions(body, diagnostics);
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
    let mut r = StyleRefinement::default();
    let mut transitions = Collector::default();
    match DeclarationBlock::parse_string(body, ParserOptions::default()) {
        Ok(block) => {
            for p in block.declarations.iter().chain(block.important_declarations.iter()) {
                if transitions.take(p, diagnostics) {
                    continue;
                }
                apply(p, &mut r, diagnostics);
            }
        }
        Err(e) => diagnostics.push(format!("inline style parse error: {e} in {body:?}")),
    }
    (r, transitions.finish().unwrap_or_default())
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

fn length_value_px(v: &LengthValue) -> Option<f32> {
    match v {
        LengthValue::Px(v) => Some(*v),
        LengthValue::Rem(v) => Some(v * 16.0),
        LengthValue::Em(v) => Some(v * 16.0), // v1: no parent font-size context
        _ => None,
    }
}

fn css_length_px(l: &CssLength) -> Option<f32> {
    match l {
        CssLength::Value(v) => length_value_px(v),
        CssLength::Calc(_) => None,
    }
}

fn lp_definite(lp: &LengthPercentage) -> Option<DefiniteLength> {
    match lp {
        DimensionPercentage::Dimension(v) => length_value_px(v).map(|p| px(p).into()),
        DimensionPercentage::Percentage(p) => Some(relative(p.0)),
        DimensionPercentage::Calc(_) => None,
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

fn lp_absolute(lp: &LengthPercentage) -> Option<AbsoluteLength> {
    match lp {
        DimensionPercentage::Dimension(v) => length_value_px(v).map(|p| px(p).into()),
        _ => None,
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

/// [`color`] only understands a resolved RGBA. `currentColor`, `color-mix()`,
/// `light-dark()` and friends land here — and USED to fall through silently,
/// which meant Tailwind's `bg-current` / `text-current` did nothing at all with
/// no way to find out.
fn unresolved_color(diagnostics: &mut Vec<String>, what: &str) {
    once(
        diagnostics,
        format!(
            "`{what}` value does not resolve to a concrete color (v1) — `currentColor`, \
             `color-mix()` and system colors are not computed"
        ),
    );
}

/// The single advisory every `--*` declaration folds into.
const CUSTOM_PROPS: &str = "custom properties are stored nowhere yet (v1) — var()-composed \
                            utilities like shadows/transforms/rings degrade";

// `--shading`, the one custom property that IS mapped. Declared here rather
// than in lib.rs so the whole effect surface — the module, its interception in
// the `Custom(_)` branch below, and nothing else — lands in one place.
#[path = "effect.rs"]
pub(crate) mod effect;

// ---------------------------------------------------------------------------
// the mapper
// ---------------------------------------------------------------------------

fn apply(p: &Property, r: &mut StyleRefinement, diagnostics: &mut Vec<String>) {
    use Property::*;
    match p {
        Display(d) => {
            use lightningcss::properties::display::Display as D;
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
        Position(v) => {
            use lightningcss::properties::position::Position as P;
            r.position = Some(match v {
                P::Absolute => gpui::Position::Absolute,
                P::Static | P::Relative => gpui::Position::Relative,
                _ => {
                    unsupported(diagnostics, "position (fixed/sticky)");
                    gpui::Position::Relative
                }
            });
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
        Overflow(v) => {
            r.overflow.x = Some(overflow(v.x));
            r.overflow.y = Some(overflow(v.y));
        }
        // The longhands are what Tailwind's `overflow-x-*` / `overflow-y-*`
        // emit; the shorthand only covers `overflow-*`.
        OverflowX(v) => r.overflow.x = Some(overflow(*v)),
        OverflowY(v) => r.overflow.y = Some(overflow(*v)),
        Visibility(v) => {
            use lightningcss::properties::display::Visibility as V;
            r.visibility = Some(match v {
                V::Visible => gpui::Visibility::Visible,
                V::Hidden => gpui::Visibility::Hidden,
                V::Collapse => {
                    // `collapse` only differs from `hidden` inside tables and
                    // flex containers, neither of which gpui models.
                    unsupported(diagnostics, "visibility: collapse (treated as hidden)");
                    gpui::Visibility::Hidden
                }
            });
        }
        AspectRatio(v) => match &v.ratio {
            Some(ratio) if !v.auto && ratio.1 != 0.0 => r.aspect_ratio = Some(ratio.0 / ratio.1),
            // `aspect-auto` / `auto <ratio>`: a refinement cannot say "clear",
            // so an earlier ratio survives.
            _ => unsupported(diagnostics, "aspect-ratio: auto — cannot undo an earlier ratio"),
        },
        Opacity(v) => r.opacity = Some(v.0),
        Color(c) => match color(c) {
            Some(c) => r.text.color = Some(c),
            None => unresolved_color(diagnostics, "color"),
        },
        BackgroundColor(c) => match color(c) {
            Some(c) => r.background = Some(gpui::Fill::Color(c.into())),
            None => unresolved_color(diagnostics, "background-color"),
        },
        Background(layers) => {
            // Shorthand: per CSS the color lives on the LAST layer. v1 takes
            // the color only; gradients/images are a later mapper stage.
            if let Some(last) = layers.last() {
                if let Some(c) = color(&last.color) {
                    r.background = Some(gpui::Fill::Color(c.into()));
                }
            }
        }
        BorderColor(v) => {
            // gpui has one border color for all four sides — take the top.
            match color(&v.top) {
                Some(c) => r.border_color = Some(c),
                None => unresolved_color(diagnostics, "border-color"),
            }
        }
        Border(b) => {
            if let Some(w) = border_width_px(&b.width) {
                let w: AbsoluteLength = px(w).into();
                r.border_widths.top = Some(w);
                r.border_widths.right = Some(w);
                r.border_widths.bottom = Some(w);
                r.border_widths.left = Some(w);
            }
            if let Some(c) = color(&b.color) {
                r.border_color = Some(c);
            }
            set_border_style(r, b.style, diagnostics);
        }
        // Tailwind emits the `border-width` shorthand for `border-2` and the
        // per-side longhands for `border-t` / `border-x-2`.
        BorderWidth(v) => {
            r.border_widths.top = border_width_px(&v.top).map(|w| px(w).into());
            r.border_widths.right = border_width_px(&v.right).map(|w| px(w).into());
            r.border_widths.bottom = border_width_px(&v.bottom).map(|w| px(w).into());
            r.border_widths.left = border_width_px(&v.left).map(|w| px(w).into());
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
            r.corner_radii.top_left = lp_absolute(&v.top_left.0);
            r.corner_radii.top_right = lp_absolute(&v.top_right.0);
            r.corner_radii.bottom_left = lp_absolute(&v.bottom_left.0);
            r.corner_radii.bottom_right = lp_absolute(&v.bottom_right.0);
        }
        // `rounded-t` / `rounded-tl-lg` never use the shorthand.
        BorderTopLeftRadius(v, _) => r.corner_radii.top_left = lp_absolute(&v.0),
        BorderTopRightRadius(v, _) => r.corner_radii.top_right = lp_absolute(&v.0),
        BorderBottomRightRadius(v, _) => r.corner_radii.bottom_right = lp_absolute(&v.0),
        BorderBottomLeftRadius(v, _) => r.corner_radii.bottom_left = lp_absolute(&v.0),
        // `grid-cols-N` / `grid-rows-N` -> `repeat(N, minmax(0, 1fr))`, which is
        // exactly (and only) what gpui's GridTemplate models.
        GridTemplateColumns(v) => match grid_template(v) {
            Some(t) => r.grid_cols = Some(t),
            None => unsupported(
                diagnostics,
                "grid-template-columns (only `repeat(N, minmax(0|min-content|max-content, 1fr))`)",
            ),
        },
        GridTemplateRows(v) => match grid_template(v) {
            Some(t) => r.grid_rows = Some(t),
            None => unsupported(
                diagnostics,
                "grid-template-rows (only `repeat(N, minmax(0|min-content|max-content, 1fr))`)",
            ),
        },
        BoxShadow(shadows, _) => {
            let mut out = Vec::new();
            for s in shadows.iter() {
                let (Some(c), Some(x), Some(y)) =
                    (color(&s.color), css_length_px(&s.x_offset), css_length_px(&s.y_offset))
                else {
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
            for f in families {
                if let CssFontFamily::FamilyName(name) = f {
                    use lightningcss::traits::ToCss;
                    if let Ok(css) = name.to_css_string(Default::default()) {
                        let clean = css.trim_matches('"').to_string();
                        r.text.font_family = Some(gpui::SharedString::from(clean));
                    }
                    break;
                }
            }
        }
        FontSize(v) => {
            use lightningcss::properties::font::FontSize as FS;
            if let FS::Length(lp) = v {
                r.text.font_size = lp_absolute(lp);
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
                None => unsupported(diagnostics, "font-weight (bolder/lighter)"),
            }
        }
        LineHeight(v) => {
            r.text.line_height = match v {
                CssLineHeight::Number(n) => Some(relative(*n)),
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
            r.text.white_space = Some(match v {
                WS::Normal => gpui::WhiteSpace::Normal,
                WS::NoWrap => gpui::WhiteSpace::Nowrap,
                WS::Pre => {
                    // gpui collapses runs of whitespace unconditionally; only
                    // the WRAP half of `pre` survives the trip.
                    unsupported(diagnostics, "white-space: pre (whitespace is still collapsed)");
                    gpui::WhiteSpace::Nowrap
                }
                WS::PreWrap | WS::PreLine | WS::BreakSpaces => {
                    unsupported(
                        diagnostics,
                        "white-space: pre-wrap/pre-line/break-spaces (whitespace is still collapsed)",
                    );
                    gpui::WhiteSpace::Normal
                }
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
                unsupported(
                    diagnostics,
                    "text-decoration-line: none — cannot undo an earlier underline",
                );
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
        // `backdrop-filter` — `blur()` is the one filter function the renderer
        // implements (gpui `BackdropBlurRect`, D3D11 Dual-Kawase pyramid).
        // Everything else in the list is advised rather than silently dropped.
        BackdropFilter(list, _) => apply_backdrop_filter(list, r, diagnostics),
        // `--tw-shadow`, `--tw-translate-x`, … A generated sheet declares these
        // by the hundred, so EVERY `--*` folds into one advisory: the story is
        // identical for all of them and the per-name list is pure noise.
        Custom(c) => match &c.name {
            // `--shading` is the ONE custom property vue-native interprets: a
            // GPU effect has no web equivalent, so it needs a spelling a
            // browser parses, stores and ignores rather than one a browser
            // DROPS. Intercepted here, ahead of the fold-everything advisory,
            // because it is a real mapped property that merely happens to be
            // spelled as a custom property. Everything about it — grammar,
            // schemas, advisories, the inheritance divergence — lives in
            // `crate::effect`.
            lightningcss::properties::custom::CustomPropertyName::Custom(name)
                if name.0.as_ref() == effect::SHADING =>
            {
                if let Some(spec) = effect::parse(&c.value, diagnostics) {
                    r.vn_effect = Some(spec);
                }
            }
            lightningcss::properties::custom::CustomPropertyName::Custom(_) => {
                once(diagnostics, CUSTOM_PROPS.to_string());
            }
            // Not a custom property at all — an unknown/vendor-prefixed one
            // (`-webkit-line-clamp`), which is a real "we do not map this".
            lightningcss::properties::custom::CustomPropertyName::Unknown(name) => {
                once(
                    diagnostics,
                    format!("property not mapped (v1): {}", name.0.as_ref()),
                );
            }
        },
        // A KNOWN property whose value did not parse into a typed value —
        // almost always `var()`-composed (`box-shadow: var(--tw-shadow)`).
        // One advisory per property name: which property degraded is the
        // useful half, how many rules did it is not.
        Unparsed(u) => {
            once(
                diagnostics,
                format!(
                    "`{}` value is unresolved (v1) — {CUSTOM_PROPS}",
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

    #[test]
    fn an_unmodelable_track_list_is_an_advisory_not_a_wrong_grid() {
        let (r, d) = map("grid-template-columns: 200px 1fr");
        assert_eq!(r.grid_cols, None);
        assert!(d.iter().any(|m| m.contains("grid-template-columns")), "{d:?}");
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

    // ---- diagnostics are ONE-SHOT ----------------------------------------

    #[test]
    fn every_custom_property_folds_into_one_advisory() {
        let (_, d) = map(
            "--tw-shadow: 0 0 #0000; --tw-ring-color: red; --tw-translate-x: 0; \
             --tw-scale-x: 1; --whatever: 3",
        );
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(
            d[0].contains("custom properties are stored nowhere yet"),
            "{d:?}"
        );
    }

    #[test]
    fn a_var_composed_value_names_the_property_it_broke() {
        let (r, d) = map("box-shadow: var(--tw-shadow)");
        assert_eq!(r.box_shadow, None);
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(d[0].contains("box-shadow"), "{d:?}");
    }

    #[test]
    fn a_repeated_unmapped_property_is_reported_once() {
        let (_, d) = map("z-index: 1; z-index: 2; z-index: 3");
        assert_eq!(d.len(), 1, "{d:?}");
    }

    #[test]
    fn currentcolor_is_advised_rather_than_dropped_in_silence() {
        let (r, d) = map("background-color: currentColor");
        assert_eq!(r.background, None);
        assert!(d.iter().any(|m| m.contains("background-color")), "{d:?}");
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
}
