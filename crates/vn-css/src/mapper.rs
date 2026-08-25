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

// ---------------------------------------------------------------------------
// the mapper
// ---------------------------------------------------------------------------

fn apply(p: &Property, r: &mut StyleRefinement, diagnostics: &mut Vec<String>) {
    use Property::*;
    let mut unsupported_value = |what: &str| {
        diagnostics.push(format!("unsupported value for {what} (v1)"));
    };
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
        AlignItems(v, _) => {
            use gpui::AlignItems as G;
            let mapped = match v {
                lightningcss::properties::align::AlignItems::Stretch => Some(G::Stretch),
                lightningcss::properties::align::AlignItems::SelfPosition { value, .. } => match value {
                    SelfPosition::Center => Some(G::Center),
                    SelfPosition::Start | SelfPosition::FlexStart | SelfPosition::SelfStart => {
                        Some(G::FlexStart)
                    }
                    SelfPosition::End | SelfPosition::FlexEnd | SelfPosition::SelfEnd => {
                        Some(G::FlexEnd)
                    }
                },
                lightningcss::properties::align::AlignItems::BaselinePosition(_) => Some(G::Baseline),
                _ => None,
            };
            match mapped {
                Some(m) => r.align_items = Some(m),
                None => unsupported_value("align-items"),
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
                None => unsupported_value("justify-content"),
            }
        }
        Gap(gap) => {
            use lightningcss::properties::align::GapValue;
            if let GapValue::LengthPercentage(lp) = &gap.row {
                if let Some(v) = lp_definite(lp) {
                    r.gap.height = Some(v);
                }
            }
            if let GapValue::LengthPercentage(lp) = &gap.column {
                if let Some(v) = lp_definite(lp) {
                    r.gap.width = Some(v);
                }
            }
        }
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
                    unsupported_value("position (fixed/sticky)");
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
        Opacity(v) => r.opacity = Some(v.0),
        Color(c) => {
            if let Some(c) = color(c) {
                r.text.color = Some(c);
            }
        }
        BackgroundColor(c) => {
            if let Some(c) = color(c) {
                r.background = Some(gpui::Fill::Color(c.into()));
            }
        }
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
            if let Some(c) = color(&v.top) {
                r.border_color = Some(c);
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
            r.border_style = Some(match b.style {
                LineStyle::Dashed => gpui::BorderStyle::Dashed,
                _ => gpui::BorderStyle::Solid,
            });
        }
        BorderRadius(v, _) => {
            r.corner_radii.top_left = lp_absolute(&v.top_left.0);
            r.corner_radii.top_right = lp_absolute(&v.top_right.0);
            r.corner_radii.bottom_left = lp_absolute(&v.bottom_left.0);
            r.corner_radii.bottom_right = lp_absolute(&v.bottom_right.0);
        }
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
                None => unsupported_value("font-weight (bolder/lighter)"),
            }
        }
        LineHeight(v) => {
            r.text.line_height = match v {
                CssLineHeight::Number(n) => Some(relative(*n)),
                CssLineHeight::Length(lp) => lp_definite(lp),
                CssLineHeight::Normal => Some(relative(1.2)),
            };
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
                None => unsupported_value("text-align"),
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
        Custom(_) | Unparsed(_) => {
            diagnostics.push(format!(
                "custom properties / var() not resolved yet (v1): {}",
                prop_name(p)
            ));
        }
        other => {
            diagnostics.push(format!("property not mapped (v1): {}", prop_name(other)));
        }
    }
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
