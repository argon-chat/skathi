//! The transition animator: diffs the cascade's output, runs the clock and produces a per-frame
//! overlay `StyleRefinement` applied on top of the computed base. An interrupted transition
//! retargets from the value it is showing, not from the old computed value.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    AbsoluteLength, DefiniteLength, EffectCrossFade, EffectSpec, Hsla, Interpolate, Length, Pixels,
    Rems, StyleRefinement, TransformSpec,
};

use crate::keyframes::{AnimationSpec, FillMode, Keyframes, Phase, PlayState};
use crate::transition::{covered, spec_for, Delay, Easing, TransitionProperty, TransitionSpec};
use crate::ComputedStates;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Animatable {
    Number(f32),
    Color(Hsla),
    BorderColors([Hsla; 4]),
    Pixels(Pixels),
    Rems(Rems),
    /// The whole spec travels together because only the `params` interpolate; between two specs of
    /// different effects [`lerp`] cross-fades instead.
    Effect(EffectSpec),
    /// A cross-fade between different effects: the first at `1 - t` beneath the second at `t`.
    EffectFade(EffectSpec, EffectSpec, f32),
    Transform(TransformSpec),
    Overflow(gpui::Overflow),
    ObjectPosition(gpui::ObjectPosition),
    TextShadows(TextShadows),
    /// A sizing length with a `calc-size()` or an intrinsic keyword on one side.
    Size(SizeForm),
}

/// `scale * size + fraction% + pixels`, `size` being the basis's intrinsic size: the form both a
/// `calc-size()` and a plain length take, so the two interpolate.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SizeForm {
    basis: Option<gpui::VnSizeBasis>,
    scale: f32,
    fraction: f32,
    pixels: f32,
}

impl SizeForm {
    fn of(length: Length) -> Self {
        let plain = |fraction: f32, pixels: f32| SizeForm {
            basis: None,
            scale: 0.,
            fraction,
            pixels,
        };
        let definite = |d: DefiniteLength| match d {
            DefiniteLength::Absolute(a) => {
                plain(0., a.to_pixels(gpui::px(crate::rem_basis())).into())
            }
            DefiniteLength::Fraction(f) => plain(f, 0.),
            DefiniteLength::Calc { fraction, pixels } => plain(fraction, pixels.into()),
        };
        match length {
            Length::Definite(d) => definite(d),
            Length::Auto => SizeForm {
                basis: Some(gpui::VnSizeBasis::Auto),
                scale: 1.,
                ..plain(0., 0.)
            },
            Length::VnCalcSize(c) => SizeForm {
                basis: Some(c.basis),
                scale: c.scale,
                ..definite(c.offset)
            },
        }
    }

    fn length(self) -> Length {
        let offset = match (self.fraction, self.pixels) {
            (f, p) if f == 0. => DefiniteLength::Absolute(gpui::px(p).into()),
            (f, p) if p == 0. => DefiniteLength::Fraction(f),
            (fraction, p) => DefiniteLength::Calc {
                fraction,
                pixels: gpui::px(p),
            },
        };
        match self.basis {
            Some(basis) if self.scale != 0. => Length::VnCalcSize(gpui::VnCalcSize {
                basis,
                scale: self.scale,
                offset,
            }),
            _ => Length::Definite(offset),
        }
    }

    fn lerp(a: Self, b: Self, t: f32) -> Self {
        SizeForm {
            basis: a.basis.or(b.basis),
            scale: f32::interpolate(a.scale, b.scale, t),
            fraction: f32::interpolate(a.fraction, b.fraction, t),
            pixels: f32::interpolate(a.pixels, b.pixels, t),
        }
    }
}

fn is_sizing(p: &TransitionProperty) -> bool {
    use TransitionProperty::*;
    matches!(
        p,
        Width | Height | MinWidth | MinHeight | MaxWidth | MaxHeight
    )
}

/// Turns a pair of sizing slots into a pair that interpolates when one side is a `calc-size()`,
/// or, with `interpolate-size: allow-keywords`, `auto`. `None` leaves the pair as it was.
fn size_pair(from: Slot, to: Slot, p: &TransitionProperty, keywords: bool) -> Option<(Slot, Slot)> {
    if !is_sizing(p) {
        return None;
    }
    use TransitionProperty::*;
    let auto_is_initial = matches!(p, Width | Height | MinWidth | MinHeight);
    let lift = |s: Slot| -> Option<SizeForm> {
        match s {
            Slot::Value(Animatable::Size(form)) => Some(form),
            Slot::Value(Animatable::Pixels(px)) => Some(SizeForm::of(Length::Definite(px.into()))),
            Slot::Value(Animatable::Rems(r)) => Some(SizeForm::of(Length::Definite(r.into()))),
            Slot::Opaque(Opaque::Fraction(f)) => {
                Some(SizeForm::of(Length::Definite(DefiniteLength::Fraction(f))))
            }
            Slot::Opaque(Opaque::Calc(fraction, pixels)) => {
                Some(SizeForm::of(Length::Definite(DefiniteLength::Calc {
                    fraction,
                    pixels: gpui::px(pixels),
                })))
            }
            Slot::Opaque(Opaque::Auto) if keywords && auto_is_initial => {
                Some(SizeForm::of(Length::Auto))
            }
            Slot::Missing if keywords && auto_is_initial => Some(SizeForm::of(Length::Auto)),
            _ => None,
        }
    };
    let sized = |s: &Slot| match s {
        Slot::Value(Animatable::Size(_)) => true,
        Slot::Opaque(Opaque::Auto) | Slot::Missing => keywords && auto_is_initial,
        _ => false,
    };
    if !sized(&from) && !sized(&to) {
        return None;
    }
    let (a, b) = (lift(from)?, lift(to)?);
    if a.basis.is_some() && b.basis.is_some() && a.basis != b.basis {
        return None;
    }
    Some((
        Slot::Value(Animatable::Size(a)),
        Slot::Value(Animatable::Size(b)),
    ))
}

/// A longer `text-shadow` list snaps instead of interpolating.
const MAX_ANIMATED_TEXT_SHADOWS: usize = 4;

/// A `text-shadow` list as a `Copy` value, with the `color` its `currentColor` entries take when
/// one is interpolated against a concrete colour.
#[derive(Clone, Copy, Debug)]
struct TextShadows {
    shadows: [gpui::TextShadow; MAX_ANIMATED_TEXT_SHADOWS],
    len: usize,
    current: Hsla,
}

impl TextShadows {
    fn new(list: &[gpui::TextShadow], current: Hsla) -> Option<Self> {
        if list.len() > MAX_ANIMATED_TEXT_SHADOWS {
            return None;
        }
        let mut shadows = [gpui::TextShadow::default(); MAX_ANIMATED_TEXT_SHADOWS];
        shadows[..list.len()].copy_from_slice(list);
        Some(Self {
            shadows,
            len: list.len(),
            current,
        })
    }

    fn list(&self) -> &[gpui::TextShadow] {
        &self.shadows[..self.len]
    }
}

/// `current` is not part of the value: a change of `color` alone does not transition `text-shadow`.
impl PartialEq for TextShadows {
    fn eq(&self, other: &Self) -> bool {
        self.list() == other.list()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Opaque {
    Auto,
    Fraction(f32),
    Calc(f32, f32),
    NonSolidFill,
    /// CSS does not interpolate `none` with a paint either, so the pair is discrete and snaps.
    UnpaintedSvg,
    /// A concrete `fill` / `stroke` colour. sk-tree draws it into the serialized `<svg>` document,
    /// which an overlay refinement cannot reach, so a change of one snaps (and says so).
    SvgColour(Hsla),
    /// A `text-shadow` list longer than [`MAX_ANIMATED_TEXT_SHADOWS`].
    LongTextShadowList,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
    Missing,
    Value(Animatable),
    Opaque(Opaque),
}

fn from_absolute(a: AbsoluteLength) -> Animatable {
    match a {
        AbsoluteLength::Pixels(p) => Animatable::Pixels(p),
        AbsoluteLength::Rems(r) => Animatable::Rems(r),
    }
}

fn to_absolute(v: Animatable) -> Option<AbsoluteLength> {
    match v {
        Animatable::Pixels(p) => Some(AbsoluteLength::Pixels(p)),
        Animatable::Rems(r) => Some(AbsoluteLength::Rems(r)),
        _ => None,
    }
}

fn slot_absolute(v: Option<AbsoluteLength>) -> Slot {
    v.map_or(Slot::Missing, |a| Slot::Value(from_absolute(a)))
}

fn slot_definite(v: Option<DefiniteLength>) -> Slot {
    match v {
        None => Slot::Missing,
        Some(DefiniteLength::Absolute(a)) => Slot::Value(from_absolute(a)),
        Some(DefiniteLength::Fraction(f)) => Slot::Opaque(Opaque::Fraction(f)),
        Some(DefiniteLength::Calc { fraction, pixels }) => {
            Slot::Opaque(Opaque::Calc(fraction, f32::from(pixels)))
        }
    }
}

fn slot_length(v: Option<Length>) -> Slot {
    match v {
        None => Slot::Missing,
        Some(Length::Auto) => Slot::Opaque(Opaque::Auto),
        Some(Length::Definite(d)) => slot_definite(Some(d)),
        Some(c @ Length::VnCalcSize(_)) => Slot::Value(Animatable::Size(SizeForm::of(c))),
    }
}

fn slot_color(v: Option<Hsla>) -> Slot {
    v.map_or(Slot::Missing, |c| Slot::Value(Animatable::Color(c)))
}

fn slot_text_shadow(r: &StyleRefinement, current: Hsla) -> Slot {
    match &r.text.text_shadow {
        None => Slot::Missing,
        Some(list) => TextShadows::new(list, current)
            .map_or(Slot::Opaque(Opaque::LongTextShadowList), |shadows| {
                Slot::Value(Animatable::TextShadows(shadows))
            }),
    }
}

fn slot_of(r: &StyleRefinement, p: &TransitionProperty) -> Slot {
    use TransitionProperty::*;
    match p {
        Opacity => r
            .opacity
            .map_or(Slot::Missing, |o| Slot::Value(Animatable::Number(o))),
        Color => slot_color(r.text.color),
        BackgroundColor => match &r.background {
            None => Slot::Missing,
            Some(gpui::Fill::Color(bg)) => match bg.as_solid() {
                Some(c) => Slot::Value(Animatable::Color(c)),
                None => Slot::Opaque(Opaque::NonSolidFill),
            },
        },
        BorderColor => {
            let side_colors = [
                &r.vn_border_colors.top,
                &r.vn_border_colors.right,
                &r.vn_border_colors.bottom,
                &r.vn_border_colors.left,
            ];
            if r.border_color.is_none() && side_colors.iter().all(|color| color.is_none()) {
                Slot::Missing
            } else {
                let fallback = r.border_color.unwrap_or_default();
                let colors = side_colors.map(|color| color.flatten().unwrap_or(fallback));
                if colors.iter().all(|color| *color == colors[0]) {
                    Slot::Value(Animatable::Color(colors[0]))
                } else {
                    Slot::Value(Animatable::BorderColors(colors))
                }
            }
        }
        TextDecorationColor => slot_color(
            r.text
                .underline
                .as_ref()
                .and_then(|u| u.color)
                .or_else(|| r.text.strikethrough.as_ref().and_then(|st| st.color)),
        ),
        TextShadow => slot_text_shadow(r, r.text.color.unwrap_or_else(gpui::black)),
        // A refinement carries no SVG paint; `slot_inherited` answers these from the node's
        // `Extras`.
        Fill | Stroke => Slot::Missing,
        Width => slot_length(r.size.width),
        Height => slot_length(r.size.height),
        MinWidth => slot_length(r.min_size.width),
        MinHeight => slot_length(r.min_size.height),
        MaxWidth => slot_length(r.max_size.width),
        MaxHeight => slot_length(r.max_size.height),
        PaddingTop => slot_definite(r.padding.top),
        PaddingRight => slot_definite(r.padding.right),
        PaddingBottom => slot_definite(r.padding.bottom),
        PaddingLeft => slot_definite(r.padding.left),
        MarginTop => slot_length(r.margin.top),
        MarginRight => slot_length(r.margin.right),
        MarginBottom => slot_length(r.margin.bottom),
        MarginLeft => slot_length(r.margin.left),
        BorderTopLeftRadius => slot_absolute(r.corner_radii.top_left),
        BorderTopRightRadius => slot_absolute(r.corner_radii.top_right),
        BorderBottomRightRadius => slot_absolute(r.corner_radii.bottom_right),
        BorderBottomLeftRadius => slot_absolute(r.corner_radii.bottom_left),
        BorderTopWidth => slot_absolute(r.border_widths.top),
        BorderRightWidth => slot_absolute(r.border_widths.right),
        BorderBottomWidth => slot_absolute(r.border_widths.bottom),
        BorderLeftWidth => slot_absolute(r.border_widths.left),
        OverflowX => r
            .overflow
            .x
            .map_or(Slot::Missing, |o| Slot::Value(Animatable::Overflow(o))),
        OverflowY => r
            .overflow
            .y
            .map_or(Slot::Missing, |o| Slot::Value(Animatable::Overflow(o))),
        RowGap => slot_definite(r.gap.height),
        ColumnGap => slot_definite(r.gap.width),
        FontSize => slot_absolute(r.text.font_size),
        ObjectPosition => r.vn_object_position.map_or(Slot::Missing, |o| {
            Slot::Value(Animatable::ObjectPosition(o))
        }),
        // `auto` is 0 and `auto <ratio>` negative (the mapper's encoding); neither interpolates.
        AspectRatio => match r.aspect_ratio {
            None => Slot::Missing,
            Some(ratio) if ratio > 0.0 => Slot::Value(Animatable::Number(ratio.ln())),
            Some(_) => Slot::Opaque(Opaque::Auto),
        },
        Shading => r
            .vn_effect
            .map_or(Slot::Missing, |e| Slot::Value(Animatable::Effect(e))),
        Transform => r
            .vn_transform
            .map_or(Slot::Missing, |t| Slot::Value(Animatable::Transform(t))),
        All | Other(_) => Slot::Missing,
    }
}

/// `color` is the one animatable property that inherits, so "this rung declares none" is the
/// parent's colour, not unset; the cascade threads it down (`ComputedStates::color`). Tailwind's
/// `transition-colors` on a wrapper relies on it.
fn slot_inherited(
    r: &StyleRefinement,
    p: &TransitionProperty,
    inherited: Option<Hsla>,
    extras: &crate::Extras,
) -> Slot {
    use TransitionProperty::*;
    match p {
        Color => match slot_of(r, p) {
            Slot::Missing => slot_color(inherited),
            slot => slot,
        },
        // `fill` / `stroke` live in `Extras`, and what they animate is the colour the sprite is
        // tinted with — which is the element's own `color`, because that is the only colour an SVG
        // has here.
        Fill => slot_paint(extras.fill, r.text.color.or(inherited)),
        Stroke => slot_paint(extras.stroke, r.text.color.or(inherited)),
        TextDecorationColor => match slot_of(r, p) {
            Slot::Missing if r.text.underline.is_some() || r.text.strikethrough.is_some() => {
                slot_color(r.text.color.or(inherited))
            }
            slot => slot,
        },
        TextShadow => slot_text_shadow(r, r.text.color.or(inherited).unwrap_or_else(gpui::black)),
        _ => slot_of(r, p),
    }
}

fn slot_paint(paint: Option<crate::SvgPaint>, color: Option<Hsla>) -> Slot {
    match paint {
        None => Slot::Missing,
        Some(crate::SvgPaint::None) => Slot::Opaque(Opaque::UnpaintedSvg),
        Some(crate::SvgPaint::Color(c)) => Slot::Opaque(Opaque::SvgColour(c)),
        Some(_) => slot_color(color),
    }
}

fn install(r: &mut StyleRefinement, base: &StyleRefinement, p: &TransitionProperty, v: Animatable) {
    use TransitionProperty::*;
    let definite = || to_absolute(v).map(DefiniteLength::Absolute);
    let length = || match v {
        Animatable::Size(form) => Some(form.length()),
        _ => to_absolute(v).map(|a| Length::Definite(DefiniteLength::Absolute(a))),
    };
    let color = || match v {
        Animatable::Color(c) => Some(c),
        _ => None,
    };
    match p {
        Opacity => {
            if let Animatable::Number(n) = v {
                // CSS clamps the used value; an overshooting easing must not paint a negative or >1
                // alpha.
                r.opacity = Some(n.clamp(0.0, 1.0));
            }
        }
        Color => r.text.color = color(),
        BackgroundColor => r.background = color().map(|c| gpui::Fill::Color(c.into())),
        BorderColor => match v {
            Animatable::BorderColors(colors) => {
                r.border_color = Some(colors[0]);
                r.vn_border_colors.top = Some(Some(colors[0]));
                r.vn_border_colors.right = Some(Some(colors[1]));
                r.vn_border_colors.bottom = Some(Some(colors[2]));
                r.vn_border_colors.left = Some(Some(colors[3]));
            }
            Animatable::Color(color) => {
                r.border_color = Some(color);
                r.vn_border_colors = gpui::EdgesRefinement {
                    top: Some(None),
                    right: Some(None),
                    bottom: Some(None),
                    left: Some(None),
                };
            }
            _ => {}
        },
        // The decoration is a whole struct in gpui: carry a complete one (thickness, waviness and
        // offset from the base, colour replaced), since a bare `UnderlineStyle { color }` would
        // paint a hairline.
        TextDecorationColor => {
            if let Some(c) = color() {
                if let Some(mut u) = r
                    .text
                    .underline
                    .clone()
                    .or_else(|| base.text.underline.clone())
                {
                    u.color = Some(c);
                    r.text.underline = Some(u);
                }
                if let Some(mut st) = r
                    .text
                    .strikethrough
                    .clone()
                    .or_else(|| base.text.strikethrough.clone())
                {
                    st.color = Some(c);
                    r.text.strikethrough = Some(st);
                }
            }
        }
        TextShadow => {
            if let Animatable::TextShadows(shadows) = v {
                r.text.text_shadow = Some(shadows.list().to_vec());
            }
        }
        Fill | Stroke => r.text.color = color().or(r.text.color),
        Width => r.size.width = length(),
        Height => r.size.height = length(),
        MinWidth => r.min_size.width = length(),
        MinHeight => r.min_size.height = length(),
        MaxWidth => r.max_size.width = length(),
        MaxHeight => r.max_size.height = length(),
        PaddingTop => r.padding.top = definite(),
        PaddingRight => r.padding.right = definite(),
        PaddingBottom => r.padding.bottom = definite(),
        PaddingLeft => r.padding.left = definite(),
        MarginTop => r.margin.top = length(),
        MarginRight => r.margin.right = length(),
        MarginBottom => r.margin.bottom = length(),
        MarginLeft => r.margin.left = length(),
        BorderTopLeftRadius => r.corner_radii.top_left = to_absolute(v),
        BorderTopRightRadius => r.corner_radii.top_right = to_absolute(v),
        BorderBottomRightRadius => r.corner_radii.bottom_right = to_absolute(v),
        BorderBottomLeftRadius => r.corner_radii.bottom_left = to_absolute(v),
        BorderTopWidth => r.border_widths.top = to_absolute(v),
        BorderRightWidth => r.border_widths.right = to_absolute(v),
        BorderBottomWidth => r.border_widths.bottom = to_absolute(v),
        BorderLeftWidth => r.border_widths.left = to_absolute(v),
        OverflowX | OverflowY => {
            if let Animatable::Overflow(o) = v {
                if matches!(p, OverflowX) {
                    r.overflow.x = Some(o);
                } else {
                    r.overflow.y = Some(o);
                }
            }
        }
        RowGap => r.gap.height = definite(),
        ColumnGap => r.gap.width = definite(),
        FontSize => r.text.font_size = to_absolute(v),
        ObjectPosition => {
            if let Animatable::ObjectPosition(o) = v {
                r.vn_object_position = Some(o);
            }
        }
        AspectRatio => {
            if let Animatable::Number(log) = v {
                r.aspect_ratio = Some(log.exp());
            }
        }
        Shading => match v {
            Animatable::Effect(spec) => {
                r.vn_effect = Some(spec);
                r.vn_effect_fade = None;
            }
            Animatable::EffectFade(from, to, progress) => {
                r.vn_effect = Some(to);
                r.vn_effect_fade = Some(EffectCrossFade { from, progress });
            }
            _ => {}
        },
        Transform => {
            if let Animatable::Transform(spec) = v {
                r.vn_transform = Some(spec);
            }
        }
        All | Other(_) => {}
    }
}

fn clear(r: &mut StyleRefinement, p: &TransitionProperty) {
    use TransitionProperty::*;
    match p {
        Opacity => r.opacity = None,
        Color => r.text.color = None,
        TextDecorationColor => {
            if let Some(u) = r.text.underline.as_mut() {
                u.color = None;
            }
            if let Some(st) = r.text.strikethrough.as_mut() {
                st.color = None;
            }
        }
        TextShadow => r.text.text_shadow = None,
        // Take a transitioned property out of the `:hover` rung so gpui cannot snap it.
        // `fill`/`stroke` paint `color`, which only `transition: color` may animate, so stripping
        // it here would leave a hover colour painted by nobody.
        Fill | Stroke => {}
        BackgroundColor => r.background = None,
        BorderColor => {
            r.border_color = None;
            r.vn_border_colors = Default::default();
        }
        Width => r.size.width = None,
        Height => r.size.height = None,
        MinWidth => r.min_size.width = None,
        MinHeight => r.min_size.height = None,
        MaxWidth => r.max_size.width = None,
        MaxHeight => r.max_size.height = None,
        PaddingTop => r.padding.top = None,
        PaddingRight => r.padding.right = None,
        PaddingBottom => r.padding.bottom = None,
        PaddingLeft => r.padding.left = None,
        MarginTop => r.margin.top = None,
        MarginRight => r.margin.right = None,
        MarginBottom => r.margin.bottom = None,
        MarginLeft => r.margin.left = None,
        BorderTopLeftRadius => r.corner_radii.top_left = None,
        BorderTopRightRadius => r.corner_radii.top_right = None,
        BorderBottomRightRadius => r.corner_radii.bottom_right = None,
        BorderBottomLeftRadius => r.corner_radii.bottom_left = None,
        BorderTopWidth => r.border_widths.top = None,
        BorderRightWidth => r.border_widths.right = None,
        BorderBottomWidth => r.border_widths.bottom = None,
        BorderLeftWidth => r.border_widths.left = None,
        OverflowX => r.overflow.x = None,
        OverflowY => r.overflow.y = None,
        RowGap => r.gap.height = None,
        ColumnGap => r.gap.width = None,
        FontSize => r.text.font_size = None,
        ObjectPosition => r.vn_object_position = None,
        AspectRatio => r.aspect_ratio = None,
        Shading => {
            r.vn_effect = None;
            r.vn_effect_fade = None;
        }
        Transform => r.vn_transform = None,
        All | Other(_) => {}
    }
}

fn copy_property(dst: &mut StyleRefinement, src: &StyleRefinement, p: &TransitionProperty) {
    use TransitionProperty::*;
    match p {
        Opacity => dst.opacity = src.opacity,
        Color => dst.text.color = src.text.color,
        TextDecorationColor => {
            dst.text.underline = src.text.underline.clone();
            dst.text.strikethrough = src.text.strikethrough.clone();
        }
        TextShadow => dst.text.text_shadow = src.text.text_shadow.clone(),
        Fill | Stroke => {}
        BackgroundColor => dst.background = src.background.clone(),
        BorderColor => {
            dst.border_color = src.border_color;
            dst.vn_border_colors = src.vn_border_colors.clone();
        }
        Width => dst.size.width = src.size.width,
        Height => dst.size.height = src.size.height,
        MinWidth => dst.min_size.width = src.min_size.width,
        MinHeight => dst.min_size.height = src.min_size.height,
        MaxWidth => dst.max_size.width = src.max_size.width,
        MaxHeight => dst.max_size.height = src.max_size.height,
        PaddingTop => dst.padding.top = src.padding.top,
        PaddingRight => dst.padding.right = src.padding.right,
        PaddingBottom => dst.padding.bottom = src.padding.bottom,
        PaddingLeft => dst.padding.left = src.padding.left,
        MarginTop => dst.margin.top = src.margin.top,
        MarginRight => dst.margin.right = src.margin.right,
        MarginBottom => dst.margin.bottom = src.margin.bottom,
        MarginLeft => dst.margin.left = src.margin.left,
        BorderTopLeftRadius => dst.corner_radii.top_left = src.corner_radii.top_left,
        BorderTopRightRadius => dst.corner_radii.top_right = src.corner_radii.top_right,
        BorderBottomRightRadius => dst.corner_radii.bottom_right = src.corner_radii.bottom_right,
        BorderBottomLeftRadius => dst.corner_radii.bottom_left = src.corner_radii.bottom_left,
        BorderTopWidth => dst.border_widths.top = src.border_widths.top,
        BorderRightWidth => dst.border_widths.right = src.border_widths.right,
        BorderBottomWidth => dst.border_widths.bottom = src.border_widths.bottom,
        BorderLeftWidth => dst.border_widths.left = src.border_widths.left,
        OverflowX => dst.overflow.x = src.overflow.x,
        OverflowY => dst.overflow.y = src.overflow.y,
        RowGap => dst.gap.height = src.gap.height,
        ColumnGap => dst.gap.width = src.gap.width,
        FontSize => dst.text.font_size = src.text.font_size,
        ObjectPosition => dst.vn_object_position = src.vn_object_position,
        AspectRatio => dst.aspect_ratio = src.aspect_ratio,
        Shading => {
            dst.vn_effect = src.vn_effect;
            dst.vn_effect_fade = src.vn_effect_fade;
        }
        Transform => dst.vn_transform = src.vn_transform,
        All | Other(_) => {}
    }
}

pub fn strip_transitioned(
    rung: &StyleRefinement,
    transitions: &[TransitionSpec],
) -> Option<StyleRefinement> {
    if transitions.is_empty() {
        return None;
    }
    let properties = covered(transitions);
    let mut out = rung.clone();
    let mut touched = false;
    for p in &properties {
        if !matches!(slot_of(rung, p), Slot::Missing) {
            clear(&mut out, p);
            touched = true;
        }
    }
    touched.then_some(out)
}

fn initial(p: &TransitionProperty) -> Option<Animatable> {
    use TransitionProperty::*;
    match p {
        Opacity => Some(Animatable::Number(1.0)),
        BackgroundColor => Some(Animatable::Color(Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        })),
        PaddingTop
        | PaddingRight
        | PaddingBottom
        | PaddingLeft
        | MarginTop
        | MarginRight
        | MarginBottom
        | MarginLeft
        | BorderTopLeftRadius
        | BorderTopRightRadius
        | BorderBottomRightRadius
        | BorderBottomLeftRadius
        | BorderTopWidth
        | BorderRightWidth
        | BorderBottomWidth
        | BorderLeftWidth
        | RowGap
        | ColumnGap => Some(Animatable::Pixels(Pixels::from(0.0))),
        OverflowX | OverflowY => Some(Animatable::Overflow(gpui::Overflow::Visible)),
        Transform => Some(Animatable::Transform(TransformSpec::IDENTITY)),
        ObjectPosition => Some(Animatable::ObjectPosition(gpui::ObjectPosition::default())),
        Shading => Some(Animatable::Effect(EffectSpec::NONE)),
        TextShadow => TextShadows::new(&[], gpui::black()).map(Animatable::TextShadows),
        _ => None,
    }
}

fn unify(a: Animatable, b: Animatable) -> Option<(Animatable, Animatable)> {
    use Animatable::*;
    match (a, b) {
        (Number(_), Number(_))
        | (Color(_), Color(_))
        | (Pixels(_), Pixels(_))
        | (Rems(_), Rems(_)) => Some((a, b)),
        (BorderColors(_), BorderColors(_)) => Some((a, b)),
        (Color(color), BorderColors(colors)) => {
            Some((BorderColors([color; 4]), BorderColors(colors)))
        }
        (BorderColors(colors), Color(color)) => {
            Some((BorderColors(colors), BorderColors([color; 4])))
        }
        (Effect(_), Effect(_)) | (EffectFade(..), Effect(_)) => Some((a, b)),
        (Transform(_), Transform(_)) => Some((a, b)),
        (ObjectPosition(_), ObjectPosition(_)) => Some((a, b)),
        (TextShadows(_), TextShadows(_)) => Some((a, b)),
        (Size(x), Size(y)) if x.basis.is_none() || y.basis.is_none() || x.basis == y.basis => {
            Some((a, b))
        }
        (Pixels(p), Rems(r)) if r.0 == 0.0 => Some((Pixels(p), Pixels(gpui::Pixels::from(0.0)))),
        (Pixels(p), Rems(r)) if p.as_f32() == 0.0 => Some((Rems(gpui::Rems(0.0)), Rems(r))),
        (Rems(r), Pixels(p)) if p.as_f32() == 0.0 => Some((Rems(r), Rems(gpui::Rems(0.0)))),
        (Rems(r), Pixels(p)) if r.0 == 0.0 => Some((Pixels(gpui::Pixels::from(0.0)), Pixels(p))),
        _ => None,
    }
}

fn lerp(from: Animatable, to: Animatable, t: f32) -> Animatable {
    use Animatable::*;
    match (from, to) {
        (Number(a), Number(b)) => Number(f32::interpolate(a, b, t)),
        (Color(a), Color(b)) => Color(lerp_color(a, b, t)),
        (BorderColors(a), BorderColors(b)) => BorderColors(std::array::from_fn(|index| {
            lerp_color(a[index], b[index], t)
        })),
        (Pixels(a), Pixels(b)) => Pixels(gpui::Pixels::interpolate(a, b, t)),
        (Rems(a), Rems(b)) => Rems(gpui::Rems::interpolate(a, b, t)),
        (Effect(a), Effect(b)) if a.interpolates_with(&b) => Effect(lerp_effect(a, b, t)),
        (Effect(a), Effect(b)) => EffectFade(a, b, t),
        // Retargeted mid-fade: back to (or on to) one of the two keeps fading from where it was;
        // a third effect fades in over the one showing more.
        (EffectFade(x, y, s), Effect(c)) if c.interpolates_with(&y) => {
            EffectFade(x, lerp_effect(y, c, t), s + (1.0 - s) * t)
        }
        (EffectFade(x, y, s), Effect(c)) if c.interpolates_with(&x) => {
            EffectFade(y, lerp_effect(x, c, t), (1.0 - s) + s * t)
        }
        (EffectFade(x, y, s), Effect(c)) => {
            lerp(Effect(if s >= 0.5 { y } else { x }), Effect(c), t)
        }
        (Transform(a), Transform(b)) => Transform(a.interpolate(b, t)),
        (ObjectPosition(a), ObjectPosition(b)) => ObjectPosition(gpui::ObjectPosition {
            x_fraction: f32::interpolate(a.x_fraction, b.x_fraction, t),
            x_offset: gpui::Pixels::interpolate(a.x_offset, b.x_offset, t),
            y_fraction: f32::interpolate(a.y_fraction, b.y_fraction, t),
            y_offset: gpui::Pixels::interpolate(a.y_offset, b.y_offset, t),
        }),
        (TextShadows(a), TextShadows(b)) => TextShadows(lerp_text_shadows(a, b, t)),
        (Size(a), Size(b)) => Size(SizeForm::lerp(a, b, t)),
        _ => to,
    }
}

/// CSS pads the shorter list with transparent shadows at zero offset and blur.
fn lerp_text_shadows(from: TextShadows, to: TextShadows, t: f32) -> TextShadows {
    let len = from.len.max(to.len);
    let mut out = to;
    out.len = len;
    for index in 0..len {
        let pick = |shadows: &TextShadows, other: &TextShadows| match shadows.list().get(index) {
            Some(shadow) => (*shadow, shadow.color.unwrap_or(shadows.current)),
            None => {
                let color = other.list()[index]
                    .color
                    .unwrap_or(other.current)
                    .opacity(0.0);
                (gpui::TextShadow::default(), color)
            }
        };
        let (a, a_color) = pick(&from, &to);
        let (b, b_color) = pick(&to, &from);
        out.shadows[index] = gpui::TextShadow {
            color: match (from.list().get(index), to.list().get(index)) {
                (Some(x), Some(y)) if x.color.is_none() && y.color.is_none() => None,
                _ => Some(lerp_color(a_color, b_color, t)),
            },
            offset: gpui::point(
                Pixels::interpolate(a.offset.x, b.offset.x, t),
                Pixels::interpolate(a.offset.y, b.offset.y, t),
            ),
            blur_radius: Pixels::interpolate(a.blur_radius, b.blur_radius, t).max(gpui::px(0.)),
        };
    }
    out
}

fn lerp_effect(a: EffectSpec, b: EffectSpec, t: f32) -> EffectSpec {
    let mut spec = b;
    for (slot, value) in spec.params.iter_mut().enumerate() {
        *value = f32::interpolate(a.params[slot], b.params[slot], t);
    }
    // Keep repainting for as long as EITHER endpoint wants to, or an animated grain would freeze
    // the moment a transition starts.
    spec.animated = a.animated || b.animated;
    spec
}

/// Mixes premultiplied: mixing raw HSLA swings through the hues in between and makes `transparent`
/// -> white pass through gray. Channels are clamped because an overshooting easing can leave range.
fn lerp_color(from: Hsla, to: Hsla, t: f32) -> Hsla {
    if from == to {
        return to;
    }
    let (a, b) = (gpui::Rgba::from(from), gpui::Rgba::from(to));
    let alpha = f32::interpolate(a.a, b.a, t).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return Hsla {
            a: 0.0,
            ..if t < 0.5 { from } else { to }
        };
    }
    let channel = |x: f32, y: f32| (f32::interpolate(x * a.a, y * b.a, t) / alpha).clamp(0.0, 1.0);
    Hsla::from(gpui::Rgba {
        r: channel(a.r, b.r),
        g: channel(a.g, b.g),
        b: channel(a.b, b.b),
        a: alpha,
    })
}

#[derive(Clone, Debug)]
struct Track {
    property: TransitionProperty,
    from: Animatable,
    to: Animatable,
    start: Instant,
    delay: Delay,
    duration: Duration,
    easing: Easing,
}

impl Track {
    fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.start);
        let Some(running) = self.delay.active(elapsed) else {
            return 0.0;
        };
        if self.duration.is_zero() {
            return 1.0;
        }
        (running.as_secs_f32() / self.duration.as_secs_f32()).clamp(0.0, 1.0)
    }

    fn value(&self, now: Instant) -> Animatable {
        lerp(self.from, self.to, self.easing.eval(self.progress(now)))
    }

    fn end(&self) -> Instant {
        self.start + self.delay.end_after(self.duration).unwrap_or(self.duration)
    }

    fn finished(&self, now: Instant) -> bool {
        now >= self.end()
    }
}

#[derive(Default)]
pub struct Animator {
    nodes: HashMap<u32, Vec<Track>>,
    anims: HashMap<u32, Vec<AnimRun>>,
    registered_samples: RefCell<HashMap<u32, (Instant, Rc<(StyleRefinement, crate::Extras)>)>>,
    events: Vec<AnimEvent>,
    /// The held value of a settled state-driven track: the host strips the transitioned properties
    /// out of the rung it hands gpui, so this is all that keeps the hovered value on screen.
    states: HashMap<u32, StyleRefinement>,
    scripts: HashMap<u32, Vec<script::ScriptAnim>>,
    script_nodes: HashMap<u32, u32>,
    /// A script animation's clock is in timeline milliseconds, so without it nothing a script
    /// started can be sampled.
    timeline_origin: Option<Instant>,
    warned: HashSet<String>,
    diagnostics: Vec<String>,
    reduce_motion: bool,
    /// Nodes under `interpolate-size: allow-keywords`.
    keywords: HashSet<u32>,
}

mod script;
pub use script::{
    script_keyframes, ScriptClock, ScriptFrame, ScriptKeyframeNotes, ScriptSample, ScriptTiming,
};

impl Animator {
    pub fn new() -> Animator {
        Animator::default()
    }

    /// Mirrors `gpui::App::reduce_motion()`. While set, values snap instead of animating (gpui's
    /// own animations behave the same way); turning it on also cancels whatever is running.
    pub fn set_reduce_motion(&mut self, reduce_motion: bool) {
        if reduce_motion && !self.reduce_motion {
            self.registered_samples.get_mut().clear();
            self.nodes.clear();
            let now = Instant::now();
            for (node, runs) in std::mem::take(&mut self.anims) {
                for run in runs {
                    self.cancel_event(node, &run, now);
                }
            }
        }
        self.reduce_motion = reduce_motion;
    }

    /// The node's inherited `interpolate-size`: whether `auto` sizes interpolate with lengths.
    pub fn set_interpolate_keywords(&mut self, node: u32, allow: bool) {
        if allow {
            self.keywords.insert(node);
        } else {
            self.keywords.remove(&node);
        }
    }

    /// `old = None` is the first styling of the node: CSS does not transition initial style, so
    /// leftover tracks are dropped.
    pub fn on_computed(
        &mut self,
        node: u32,
        old: Option<&ComputedStates>,
        new: &ComputedStates,
        now: Instant,
    ) {
        if self.reduce_motion {
            self.nodes.remove(&node);
            return;
        }
        let Some(old) = old else {
            self.nodes.remove(&node);
            return;
        };

        for spec in new.transitions.iter() {
            if let TransitionProperty::Other(name) = &spec.property {
                self.warn_once(format!("transition of {name} not animatable yet"));
            }
        }

        if new.transitions.is_empty() {
            // The state hold goes with them: with nothing left to strip, the whole rung is gpui's
            // again and a leftover hold would double-apply it.
            self.nodes.remove(&node);
            self.states.remove(&node);
            return;
        }

        let properties = covered(&new.transitions);
        let mut tracks = self.nodes.remove(&node).unwrap_or_default();
        tracks.retain(|t| properties.contains(&t.property));

        for property in &properties {
            let Some(spec) = spec_for(&new.transitions, property) else {
                continue;
            };
            let index = tracks.iter().position(|t| t.property == *property);
            let running = index.map(|i| tracks[i].value(now));

            let from_slot = match running {
                Some(v) => Slot::Value(v),
                None => slot_inherited(&old.base, property, old.color, &old.extras),
            };
            let to_slot = slot_inherited(&new.base, property, new.color, &new.extras);

            match self.endpoints(node, from_slot, to_slot, property) {
                Endpoints::Pair(from, to) => {
                    if let Some(i) = index {
                        if tracks[i].to == to {
                            continue;
                        }
                    }
                    if from == to || spec.is_instant() {
                        if let Some(i) = index {
                            tracks.remove(i);
                        }
                        continue;
                    }
                    let track = Track {
                        property: property.clone(),
                        from,
                        to,
                        start: now,
                        delay: spec.delay,
                        duration: spec.duration,
                        easing: spec.easing,
                    };
                    match index {
                        Some(i) => tracks[i] = track,
                        None => tracks.push(track),
                    }
                }
                Endpoints::Snap => {
                    if let Some(i) = index {
                        tracks.remove(i);
                    }
                }
            }
        }

        if tracks.is_empty() {
            self.nodes.remove(&node);
        } else {
            self.nodes.insert(node, tracks);
        }
    }

    pub fn on_state(
        &mut self,
        node: u32,
        computed: &ComputedStates,
        target: Option<&StyleRefinement>,
        now: Instant,
    ) {
        if computed.transitions.is_empty() {
            self.states.remove(&node);
            self.nodes.remove(&node);
            return;
        }
        let properties = covered(&computed.transitions);
        let base = &*computed.base;

        // Built first so that the reduce-motion path can take it and skip the animation entirely —
        // the state style still has to APPLY, it just applies at once.
        let mut hold = StyleRefinement::default();
        let mut holds_anything = false;
        if let Some(target) = target {
            for p in &properties {
                let to = slot_of(target, p);
                if matches!(to, Slot::Missing) {
                    continue;
                }
                if to == slot_inherited(base, p, computed.color, &computed.extras) {
                    continue;
                }
                match to {
                    Slot::Value(v) => {
                        install(&mut hold, base, p, v);
                        holds_anything = true;
                    }
                    _ => {
                        copy_property(&mut hold, target, p);
                        holds_anything = true;
                    }
                }
            }
        }

        if self.reduce_motion {
            self.nodes.remove(&node);
            if holds_anything {
                self.states.insert(node, hold);
            } else {
                self.states.remove(&node);
            }
            return;
        }

        let previous = self.states.get(&node).cloned();
        let mut tracks = self.nodes.remove(&node).unwrap_or_default();
        tracks.retain(|t| properties.contains(&t.property));

        for property in &properties {
            let Some(spec) = spec_for(&computed.transitions, property) else {
                continue;
            };
            let index = tracks.iter().position(|t| t.property == *property);
            let running = index.map(|i| tracks[i].value(now));
            let from_slot = match running {
                Some(v) => Slot::Value(v),
                None => match previous.as_ref().map(|p| slot_of(p, property)) {
                    Some(Slot::Missing) | None => {
                        slot_inherited(base, property, computed.color, &computed.extras)
                    }
                    Some(s) => s,
                },
            };
            let to_slot = match slot_of(&hold, property) {
                Slot::Missing => slot_inherited(base, property, computed.color, &computed.extras),
                s => s,
            };

            match self.endpoints(node, from_slot, to_slot, property) {
                Endpoints::Pair(from, to) => {
                    if let Some(i) = index {
                        if tracks[i].to == to {
                            continue;
                        }
                    }
                    if from == to || spec.is_instant() {
                        if let Some(i) = index {
                            tracks.remove(i);
                        }
                        continue;
                    }
                    let track = Track {
                        property: property.clone(),
                        from,
                        to,
                        start: now,
                        delay: spec.delay,
                        duration: spec.duration,
                        easing: spec.easing,
                    };
                    match index {
                        Some(i) => tracks[i] = track,
                        None => tracks.push(track),
                    }
                }
                Endpoints::Snap => {
                    if let Some(i) = index {
                        tracks.remove(i);
                    }
                }
            }
        }

        if tracks.is_empty() {
            self.nodes.remove(&node);
        } else {
            self.nodes.insert(node, tracks);
        }
        if holds_anything {
            self.states.insert(node, hold);
        } else {
            self.states.remove(&node);
        }
    }

    pub fn holds_state(&self, node: u32) -> bool {
        self.states.contains_key(&node)
    }

    pub fn overlay(
        &self,
        node: u32,
        base: &StyleRefinement,
        now: Instant,
    ) -> Option<StyleRefinement> {
        let mut r = self.animation_overlay(node, base, now);
        if let Some(hold) = self.states.get(&node) {
            use gpui::Refineable as _;
            r.get_or_insert_with(StyleRefinement::default).refine(hold);
        }
        if let Some(tracks) = self.nodes.get(&node).filter(|t| !t.is_empty()) {
            let out = r.get_or_insert_with(StyleRefinement::default);
            for track in tracks {
                install(out, base, &track.property, track.value(now));
            }
        }
        // Script animations last, so above both: Web Animations sorts script animations after CSS
        // animations and transitions in the effect stack.
        self.script_overlay(node, base, now, &mut r);
        r
    }

    pub fn any_active(&self, now: Instant) -> bool {
        self.nodes
            .values()
            .any(|tracks| tracks.iter().any(|t| !t.finished(now)))
    }

    pub fn settles_at(&self) -> Option<Instant> {
        self.nodes
            .values()
            .flat_map(|tracks| tracks.iter())
            .map(Track::end)
            .max()
    }

    /// Drops tracks that have reached their target, and says whether any did: the last frame that
    /// painted one showed a value short of its end, so one more frame must be drawn.
    pub fn finish_settled(&mut self, now: Instant) -> bool {
        let mut events = Vec::new();
        self.nodes.retain(|&node, tracks| {
            tracks.retain(|t| {
                if !t.finished(now) {
                    return true;
                }
                events.push(AnimEvent {
                    node,
                    kind: AnimEventKind::TransitionEnd,
                    name: t.property.name().to_string(),
                    elapsed: t.duration.as_secs_f32(),
                });
                false
            });
            !tracks.is_empty()
        });
        let settled = !events.is_empty();
        self.events.append(&mut events);
        settled
    }

    pub fn gc(&mut self, alive: &dyn Fn(u32) -> bool) {
        self.nodes.retain(|&id, _| alive(id));
        self.anims.retain(|&id, _| alive(id));
        self.registered_samples.get_mut().retain(|&id, _| alive(id));
        self.states.retain(|&id, _| alive(id));
        self.events.retain(|e| alive(e.node));
        // A script animation outlives its target in a browser (its clock and its promises are the
        // script's), and that half lives in JS; only the painting half goes with the node.
        self.scripts.retain(|&id, _| alive(id));
        self.script_nodes.retain(|_, node| alive(*node));
        self.keywords.retain(|&id| alive(id));
    }

    /// Explicitly resets CSS motion state. A stylesheet replacement instead rebinds its keyframes
    /// through `on_computed_animations`, preserving clocks.
    pub fn clear(&mut self) {
        self.registered_samples.get_mut().clear();
        self.nodes.clear();
        self.states.clear();
        self.anims.clear();
        self.events.clear();
    }

    pub fn is_idle(&self) -> bool {
        self.nodes.is_empty() && self.anims.is_empty() && self.scripts.is_empty()
    }

    pub fn take_diagnostics(&mut self) -> Vec<String> {
        std::mem::take(&mut self.diagnostics)
    }

    fn warn_once(&mut self, message: String) {
        if self.warned.insert(message.clone()) {
            self.diagnostics.push(message);
        }
    }

    fn endpoints(
        &mut self,
        node: u32,
        from: Slot,
        to: Slot,
        property: &TransitionProperty,
    ) -> Endpoints {
        if from == to {
            return Endpoints::Snap;
        }
        let keywords = self.keywords.contains(&node);
        let (from, to) = size_pair(from, to, property, keywords).unwrap_or((from, to));
        if from == to {
            return Endpoints::Snap;
        }
        let (from, to) = match (from, to) {
            (Slot::Value(a), Slot::Value(b)) => (a, b),
            (Slot::Missing, Slot::Value(b)) => match initial(property) {
                Some(a) => (a, b),
                None => {
                    self.warn_once(format!(
                        "transition of {} from an unset value is not animatable yet (v1) — snapping",
                        property.name()
                    ));
                    return Endpoints::Snap;
                }
            },
            (Slot::Value(a), Slot::Missing) => match initial(property) {
                Some(b) => (a, b),
                None => {
                    self.warn_once(format!(
                        "transition of {} to an unset value is not animatable yet (v1) — snapping",
                        property.name()
                    ));
                    return Endpoints::Snap;
                }
            },
            (Slot::Opaque(Opaque::LongTextShadowList), _)
            | (_, Slot::Opaque(Opaque::LongTextShadowList)) => {
                self.warn_once(format!(
                    "transition of {} with more than {MAX_ANIMATED_TEXT_SHADOWS} shadows is not \
                     animatable — snapping. Animate a shorter list",
                    property.name()
                ));
                return Endpoints::Snap;
            }
            (Slot::Opaque(Opaque::SvgColour(_)), _) | (_, Slot::Opaque(Opaque::SvgColour(_))) => {
                self.warn_once(format!(
                    "transition of {} to or from a concrete SVG paint colour is not animatable \
                     yet (v1) — snapping",
                    property.name()
                ));
                return Endpoints::Snap;
            }
            _ => {
                self.warn_once(format!(
                    "transition of {} between percentage/auto endpoints is not animatable yet \
                     (v1) — snapping",
                    property.name()
                ));
                return Endpoints::Snap;
            }
        };
        match unify(from, to) {
            Some((from, to)) => Endpoints::Pair(from, to),
            None => {
                self.warn_once(format!(
                    "transition of {} between different length units is not animatable yet (v1) \
                     — snapping",
                    property.name()
                ));
                Endpoints::Snap
            }
        }
    }
}

enum Endpoints {
    Pair(Animatable, Animatable),
    Snap,
}

impl Animator {
    #[cfg(test)]
    fn overlay_test(&self, node: u32, now: Instant) -> Option<StyleRefinement> {
        self.overlay(node, &StyleRefinement::default(), now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, relative, rems, Rgba};
    use std::rc::Rc;

    fn states(base: StyleRefinement, css: &str) -> ComputedStates {
        let transitions = if css.is_empty() {
            Vec::new()
        } else {
            let mut diagnostics = Vec::new();
            let (_, t) = crate::parse_inline_with_transitions(css, &mut diagnostics);
            assert!(diagnostics.is_empty(), "{diagnostics:?}");
            t
        };
        ComputedStates {
            base: Rc::new(base),
            transitions: Rc::new(transitions),
            ..Default::default()
        }
    }

    fn with_opacity(v: f32) -> StyleRefinement {
        let mut r = StyleRefinement::default();
        r.opacity = Some(v);
        r
    }

    fn with_width(w: Length) -> StyleRefinement {
        let mut r = StyleRefinement::default();
        r.size.width = Some(w);
        r
    }

    fn with_radius(radius: AbsoluteLength) -> StyleRefinement {
        let mut r = StyleRefinement::default();
        r.corner_radii.top_left = Some(radius);
        r
    }

    fn with_opacity_and_radius(o: f32, radius: f32) -> StyleRefinement {
        let mut r = with_radius(px(radius).into());
        r.opacity = Some(o);
        r
    }

    fn opacity_of(r: &StyleRefinement) -> f32 {
        r.opacity.expect("overlay carries opacity")
    }

    fn t0() -> Instant {
        Instant::now()
    }

    #[test]
    fn first_computation_does_not_animate() {
        let mut a = Animator::new();
        let now = t0();
        let new = states(with_opacity(0.0), "transition: opacity 1s linear");
        a.on_computed(1, None, &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
        assert!(!a.any_active(now));
    }

    #[test]
    fn diff_start_interpolate_settle() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 1s linear");
        let new = states(with_opacity(1.0), "transition: opacity 1s linear");

        a.on_computed(1, Some(&old), &new, now);
        assert!(a.any_active(now));
        assert!(
            (opacity_of(&a.overlay(1, &StyleRefinement::default(), now).unwrap()) - 0.0).abs()
                < 1e-6
        );

        let half = now + Duration::from_millis(500);
        assert!(
            (opacity_of(&a.overlay(1, &StyleRefinement::default(), half).unwrap()) - 0.5).abs()
                < 1e-3
        );

        let end = now + Duration::from_millis(1000);
        assert!(
            (opacity_of(&a.overlay(1, &StyleRefinement::default(), end).unwrap()) - 1.0).abs()
                < 1e-6
        );
        assert!(!a.any_active(end));

        a.finish_settled(end);
        assert!(a.overlay(1, &StyleRefinement::default(), end).is_none());
        assert!(a.is_idle());
    }

    #[test]
    fn delay_holds_the_start_value() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 1s linear 500ms");
        let new = states(with_opacity(1.0), "transition: opacity 1s linear 500ms");
        a.on_computed(1, Some(&old), &new, now);

        for at in [0, 200, 499] {
            let v = opacity_of(&a.overlay_test(1, now + Duration::from_millis(at)).unwrap());
            assert!(
                v.abs() < 1e-6,
                "value moved during the delay at {at}ms: {v}"
            );
        }
        let v = opacity_of(
            &a.overlay_test(1, now + Duration::from_millis(1000))
                .unwrap(),
        );
        assert!((v - 0.5).abs() < 1e-3, "{v}");
        assert!(a.any_active(now + Duration::from_millis(1000)));
        assert!(!a.any_active(now + Duration::from_millis(1500)));
    }

    #[test]
    fn a_negative_delay_starts_part_way_through() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 1s linear -250ms");
        let new = states(with_opacity(1.0), "transition: opacity 1s linear -250ms");
        a.on_computed(1, Some(&old), &new, now);

        let v = opacity_of(&a.overlay_test(1, now).unwrap());
        assert!((v - 0.25).abs() < 1e-3, "{v}");
        assert!(a.any_active(now + Duration::from_millis(700)));
        assert!(!a.any_active(now + Duration::from_millis(750)));

        let mut b = Animator::new();
        let skipped = "transition: opacity 1s linear -1s";
        b.on_computed(
            1,
            Some(&states(with_opacity(0.0), skipped)),
            &states(with_opacity(1.0), skipped),
            now,
        );
        assert!(b.overlay(1, &StyleRefinement::default(), now).is_none());
    }

    #[test]
    fn zero_combined_duration_snaps() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 0s");
        let new = states(with_opacity(1.0), "transition: opacity 0s");
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
    }

    #[test]
    fn zero_duration_with_delay_jumps_after_the_delay() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 0s 300ms");
        let new = states(with_opacity(1.0), "transition: opacity 0s 300ms");
        a.on_computed(1, Some(&old), &new, now);
        assert!(opacity_of(&a.overlay_test(1, now + Duration::from_millis(100)).unwrap()) < 1e-6);
        assert!(
            (opacity_of(&a.overlay_test(1, now + Duration::from_millis(400)).unwrap()) - 1.0).abs()
                < 1e-6
        );
    }

    #[test]
    fn retarget_preserves_visual_continuity() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear";
        let zero = states(with_opacity(0.0), css);
        let one = states(with_opacity(1.0), css);

        a.on_computed(1, Some(&zero), &one, now);
        let mid = now + Duration::from_millis(400);
        let at_interrupt = opacity_of(&a.overlay(1, &StyleRefinement::default(), mid).unwrap());
        assert!((at_interrupt - 0.4).abs() < 1e-3, "{at_interrupt}");

        a.on_computed(1, Some(&one), &zero, mid);
        let resumed = opacity_of(&a.overlay(1, &StyleRefinement::default(), mid).unwrap());
        assert!(
            (resumed - at_interrupt).abs() < 1e-4,
            "jumped on retarget: {at_interrupt} -> {resumed}"
        );

        let later = mid + Duration::from_millis(200);
        let v = opacity_of(&a.overlay(1, &StyleRefinement::default(), later).unwrap());
        assert!(v < at_interrupt, "{v} should be below {at_interrupt}");
        assert!((v - 0.32).abs() < 1e-2, "{v}");
        let done = mid + Duration::from_millis(1000);
        assert!(opacity_of(&a.overlay(1, &StyleRefinement::default(), done).unwrap()).abs() < 1e-6);
    }

    #[test]
    fn restyle_with_the_same_target_does_not_restart() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear";
        let zero = states(with_opacity(0.0), css);
        let one = states(with_opacity(1.0), css);
        a.on_computed(1, Some(&zero), &one, now);

        let mid = now + Duration::from_millis(500);
        a.on_computed(1, Some(&one), &one, mid);
        let v = opacity_of(&a.overlay(1, &StyleRefinement::default(), mid).unwrap());
        assert!((v - 0.5).abs() < 1e-3, "restarted: {v}");
        assert!(!a.any_active(now + Duration::from_millis(1000)));
    }

    #[test]
    fn colors_interpolate_in_srgb() {
        let mut a = Animator::new();
        let now = t0();
        let red: Hsla = Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        }
        .into();
        let blue: Hsla = Rgba {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        }
        .into();
        fn mk(c: Hsla) -> StyleRefinement {
            let mut r = StyleRefinement::default();
            r.background = Some(gpui::Fill::Color(c.into()));
            r
        }
        let old = states(mk(red), "transition: background-color 1s linear");
        let new = states(mk(blue), "transition: background-color 1s linear");
        a.on_computed(1, Some(&old), &new, now);

        let half = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        let c = half
            .background
            .unwrap()
            .color()
            .unwrap()
            .as_solid()
            .unwrap();
        // Half of each channel: `rgb(128 0 128)`, the purple a browser shows, not the
        // full-brightness magenta of the hue arc between them.
        let c = Rgba::from(c);
        assert!(
            (c.r - 0.5).abs() < 1e-3 && c.g < 1e-3 && (c.b - 0.5).abs() < 1e-3,
            "{c:?}"
        );
    }

    /// Mixed unpremultiplied, a background fading in from it is gray at half alpha part way; a
    /// browser shows the colour it is heading for, at a growing alpha.
    #[test]
    fn a_colour_fading_in_from_transparent_keeps_its_colour() {
        let white: Hsla = Rgba {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
        .into();
        let clear = Hsla {
            h: 0.0,
            s: 0.0,
            l: 0.0,
            a: 0.0,
        };
        let quarter = Rgba::from(lerp_color(clear, white, 0.25));
        assert!((quarter.a - 0.25).abs() < 1e-3, "{quarter:?}");
        assert!(
            quarter.r > 0.999 && quarter.g > 0.999 && quarter.b > 0.999,
            "{quarter:?}"
        );
        let gray: Hsla = Rgba {
            r: 0.5,
            g: 0.5,
            b: 0.5,
            a: 1.0,
        }
        .into();
        let blue: Hsla = Rgba {
            r: 0.0,
            g: 0.0,
            b: 1.0,
            a: 1.0,
        }
        .into();
        let half = Rgba::from(lerp_color(gray, blue, 0.5));
        assert!(
            (half.r - 0.25).abs() < 1e-3 && (half.g - 0.25).abs() < 1e-3,
            "{half:?}"
        );
        assert!((half.b - 0.75).abs() < 1e-3, "{half:?}");
        let over = lerp_color(gray, blue, 1.4);
        assert!((0.0..=1.0).contains(&over.l) && (0.0..=1.0).contains(&over.a));
    }

    #[test]
    fn pixel_lengths_interpolate() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_width(px(100.0).into()), "transition: width 1s linear");
        let new = states(with_width(px(200.0).into()), "transition: width 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        let overlay = a.overlay_test(1, now + Duration::from_millis(250)).unwrap();
        assert_eq!(overlay.size.width, Some(px(125.0).into()));
    }

    #[test]
    fn object_position_interpolates_from_its_centred_initial_value() {
        let css = "transition: object-position 1s linear";
        let at = |x_fraction: f32, x_offset: f32| {
            let mut r = StyleRefinement::default();
            r.vn_object_position = Some(gpui::ObjectPosition {
                x_fraction,
                x_offset: px(x_offset),
                ..Default::default()
            });
            r
        };
        let mut a = Animator::new();
        let now = t0();
        a.on_computed(
            1,
            Some(&states(at(0.0, 0.0), css)),
            &states(at(1.0, -20.0), css),
            now,
        );
        let quarter = a.overlay_test(1, now + Duration::from_millis(250)).unwrap();
        assert_eq!(
            quarter.vn_object_position,
            Some(at(0.25, -5.0).vn_object_position.unwrap())
        );

        let mut a = Animator::new();
        a.on_computed(
            1,
            Some(&states(StyleRefinement::default(), css)),
            &states(at(1.0, 0.0), css),
            now,
        );
        let half = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        assert_eq!(half.vn_object_position.map(|p| p.x_fraction), Some(0.75));
    }

    #[test]
    fn text_shadows_interpolate_pairwise_and_pad_with_transparent() {
        fn shadowed(shadows: &[gpui::TextShadow]) -> StyleRefinement {
            let mut r = StyleRefinement::default();
            r.text.text_shadow = Some(shadows.to_vec());
            r
        }
        let shadow = |x: f32, blur: f32, color: Option<Hsla>| gpui::TextShadow {
            color,
            offset: gpui::point(px(x), px(x)),
            blur_radius: px(blur),
        };
        let red = Hsla::from(Rgba {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        });
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: text-shadow 1s linear";
        let old = states(shadowed(&[shadow(0.0, 0.0, None)]), css);
        let new = states(
            shadowed(&[shadow(4.0, 8.0, Some(red)), shadow(2.0, 2.0, Some(red))]),
            css,
        );
        a.on_computed(1, Some(&old), &new, now);
        let half = a
            .overlay_test(1, now + Duration::from_millis(500))
            .and_then(|r| r.text.text_shadow)
            .expect("text-shadow transitions");
        assert_eq!(half.len(), 2);
        assert_eq!(half[0].offset.x, px(2.0));
        assert_eq!(half[0].blur_radius, px(4.0));
        let first = half[0].color.expect("currentColor meets red");
        assert!(
            first.h != red.h || first.l != red.l,
            "mixed with black: {first:?}"
        );
        assert_eq!(
            half[1].offset.x,
            px(1.0),
            "the shorter list is padded with zeros"
        );
        assert_eq!(half[1].blur_radius, px(1.0));
        let second = half[1].color.expect("a padded shadow is transparent red");
        assert!(
            (second.a - 0.5).abs() < 1e-3 && second.h == red.h,
            "{second:?}"
        );
    }

    #[test]
    fn padding_from_unset_uses_the_css_initial_zero() {
        let mut a = Animator::new();
        let now = t0();
        let mut to = StyleRefinement::default();
        to.padding.top = Some(px(20.0).into());
        let old = states(StyleRefinement::default(), "transition: padding 1s linear");
        let new = states(to, "transition: padding 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        let overlay = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        assert_eq!(overlay.padding.top, Some(px(10.0).into()));
        assert!(a.take_diagnostics().is_empty());
    }

    #[test]
    fn percentage_endpoints_snap() {
        let mut a = Animator::new();
        let now = t0();
        let from = with_width(px(100.0).into());
        let to = with_width(Length::Definite(relative(0.5)));
        let old = states(from, "transition: width 1s linear");
        let new = states(to, "transition: width 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
    }

    #[test]
    fn auto_endpoint_snaps() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_width(Length::Auto), "transition: width 1s linear");
        let new = states(with_width(px(200.0).into()), "transition: width 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
        assert_eq!(a.take_diagnostics().len(), 1);
    }

    #[test]
    fn unchanged_non_animatable_value_is_silent() {
        let mut a = Animator::new();
        let now = t0();
        let auto = states(with_width(Length::Auto), "transition: width 1s linear");
        a.on_computed(1, Some(&auto), &auto, now);
        assert!(a.take_diagnostics().is_empty());
    }

    #[test]
    fn an_unsupported_property_does_not_animate() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: filter 1s");
        let new = states(with_opacity(1.0), "transition: filter 1s");
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
    }

    #[test]
    fn all_covers_every_animatable_property() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(
            with_opacity_and_radius(0.0, 0.0),
            "transition: all 1s linear",
        );
        let new = states(
            with_opacity_and_radius(1.0, 8.0),
            "transition: all 1s linear",
        );
        a.on_computed(1, Some(&old), &new, now);
        let overlay = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        assert!((overlay.opacity.unwrap() - 0.5).abs() < 1e-3);
        assert_eq!(overlay.corner_radii.top_left, Some(px(4.0).into()));
    }

    #[test]
    fn removing_the_transition_cancels_running_tracks() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear";
        let old = states(with_opacity(0.0), css);
        let new = states(with_opacity(1.0), css);
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.any_active(now));

        let mid = now + Duration::from_millis(300);
        let bare = states(with_opacity(1.0), "");
        a.on_computed(1, Some(&new), &bare, mid);
        assert!(a.overlay(1, &StyleRefinement::default(), mid).is_none());
        assert!(!a.any_active(mid));
    }

    #[test]
    fn narrowing_transition_property_cancels_the_dropped_one() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(
            with_opacity_and_radius(0.0, 0.0),
            "transition: all 1s linear",
        );
        let new = states(
            with_opacity_and_radius(1.0, 8.0),
            "transition: all 1s linear",
        );
        a.on_computed(1, Some(&old), &new, now);
        let mid = now + Duration::from_millis(200);
        assert!(a
            .overlay(1, &StyleRefinement::default(), mid)
            .unwrap()
            .corner_radii
            .top_left
            .is_some());

        let narrowed = states(
            with_opacity_and_radius(1.0, 8.0),
            "transition: opacity 1s linear",
        );
        a.on_computed(1, Some(&new), &narrowed, mid);
        let overlay = a.overlay(1, &StyleRefinement::default(), mid).unwrap();
        assert!(overlay.corner_radii.top_left.is_none());
        assert!(overlay.opacity.is_some());
    }

    #[test]
    fn gc_and_clear_drop_node_state() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear";
        let old = states(with_opacity(0.0), css);
        let new = states(with_opacity(1.0), css);
        a.on_computed(1, Some(&old), &new, now);
        a.on_computed(2, Some(&old), &new, now);
        assert!(a.overlay(2, &StyleRefinement::default(), now).is_some());

        a.gc(&|id| id == 1);
        assert!(a.overlay(2, &StyleRefinement::default(), now).is_none());
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_some());

        a.clear();
        assert!(a.is_idle());
    }

    #[test]
    fn reduce_motion_snaps_and_cancels() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear";
        let old = states(with_opacity(0.0), css);
        let new = states(with_opacity(1.0), css);
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.any_active(now));

        a.set_reduce_motion(true);
        assert!(a.is_idle());
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());

        a.set_reduce_motion(false);
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_some());
    }

    #[test]
    fn settles_at_reports_the_last_end() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 1s linear");
        let new = states(with_opacity(1.0), "transition: opacity 1s linear");
        a.on_computed(1, Some(&old), &new, now);
        let settle = a.settles_at().unwrap();
        assert!(settle >= now + Duration::from_millis(999));
        assert!(settle <= now + Duration::from_millis(1001));
    }

    #[test]
    fn zero_px_and_zero_rem_unify() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(
            with_radius(AbsoluteLength::Rems(rems(0.0))),
            "transition: border-radius 1s linear",
        );
        let new = states(
            with_radius(px(10.0).into()),
            "transition: border-radius 1s linear",
        );
        a.on_computed(1, Some(&old), &new, now);
        let overlay = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        assert_eq!(overlay.corner_radii.top_left, Some(px(5.0).into()));
        assert!(a.take_diagnostics().is_empty());
    }

    #[test]
    fn mixed_px_rem_endpoints_snap() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(
            with_radius(AbsoluteLength::Rems(rems(1.0))),
            "transition: border-radius 1s linear",
        );
        let new = states(
            with_radius(px(10.0).into()),
            "transition: border-radius 1s linear",
        );
        a.on_computed(1, Some(&old), &new, now);
        assert!(a.overlay(1, &StyleRefinement::default(), now).is_none());
        assert_eq!(a.take_diagnostics().len(), 1);
    }

    #[test]
    fn per_property_specs_use_their_own_timing() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s linear, border-top-left-radius 2s linear";
        let old = states(with_opacity_and_radius(0.0, 0.0), css);
        let new = states(with_opacity_and_radius(1.0, 100.0), css);
        a.on_computed(1, Some(&old), &new, now);

        let at = now + Duration::from_millis(1000);
        let overlay = a.overlay(1, &StyleRefinement::default(), at).unwrap();
        assert!((overlay.opacity.unwrap() - 1.0).abs() < 1e-6);
        assert_eq!(overlay.corner_radii.top_left, Some(px(50.0).into()));
        assert!(a.any_active(at));
        assert!(!a.any_active(now + Duration::from_millis(2000)));
    }

    #[test]
    fn easing_shapes_the_curve() {
        let mut a = Animator::new();
        let now = t0();
        let old = states(with_opacity(0.0), "transition: opacity 1s ease-in");
        let new = states(with_opacity(1.0), "transition: opacity 1s ease-in");
        a.on_computed(1, Some(&old), &new, now);
        let v = opacity_of(&a.overlay_test(1, now + Duration::from_millis(500)).unwrap());
        assert!((v - 0.3154).abs() < 2e-3, "{v}");
    }

    #[test]
    fn overshooting_easing_is_clamped_for_opacity() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: opacity 1s cubic-bezier(0.34, 1.8, 0.64, 1)";
        let old = states(with_opacity(0.0), css);
        let new = states(with_opacity(1.0), css);
        a.on_computed(1, Some(&old), &new, now);
        for step in 0..=20 {
            let v = opacity_of(
                &a.overlay_test(1, now + Duration::from_millis(step * 50))
                    .unwrap(),
            );
            assert!((0.0..=1.0).contains(&v), "opacity left 0..1: {v}");
        }
    }

    fn with_shading(value: &str) -> StyleRefinement {
        let mut diagnostics = Vec::new();
        let r = crate::parse_inline(&format!("--shading: {value}"), &mut diagnostics);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(r.vn_effect.is_some(), "--shading: {value} did not map");
        r
    }

    fn shading_of(r: &StyleRefinement) -> EffectSpec {
        r.vn_effect.expect("overlay carries --shading")
    }

    #[test]
    fn effect_params_interpolate() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("frost(0.2, 8px)"), css);
        let new = states(with_shading("frost(0.8, 20px)"), css);
        a.on_computed(1, Some(&old), &new, now);

        let half = shading_of(&a.overlay_test(1, now + Duration::from_millis(500)).unwrap());
        assert_eq!(half.id, gpui::effect_id::FROST);
        assert!((half.params[0] - 0.5).abs() < 1e-3, "{}", half.params[0]);
        assert!((half.params[1] - 14.0).abs() < 1e-3, "{}", half.params[1]);
        assert_eq!(half.backdrop_blur_radius(), half.params[1]);
        assert!(a.take_diagnostics().is_empty());

        let end = shading_of(
            &a.overlay_test(1, now + Duration::from_millis(1000))
                .unwrap(),
        );
        assert_eq!(end.params[0], 0.8);
        assert_eq!(end.params[1], 20.0);
    }

    #[test]
    fn effect_color_params_interpolate_componentwise() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("glow(#000000, 10px, 0)"), css);
        let new = states(with_shading("glow(#ffffff, 30px, 1)"), css);
        a.on_computed(1, Some(&old), &new, now);
        let half = shading_of(&a.overlay_test(1, now + Duration::from_millis(500)).unwrap());
        assert!((half.params[0] - 20.0).abs() < 1e-3);
        assert!((half.params[1] - 0.5).abs() < 1e-3);
        for channel in 4..7 {
            assert!((half.params[channel] - 0.5).abs() < 1e-3, "slot {channel}");
        }
        assert!((half.bleed() - 20.0).abs() < 1e-3);
    }

    #[test]
    fn the_same_effect_with_another_image_cross_fades() {
        crate::mapper::effect::tests::with_user_effects();
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("grade(url(a.png), 0)"), css);
        let new = states(with_shading("grade(url(b.png), 1)"), css);
        a.on_computed(1, Some(&old), &new, now);
        let half = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        let fade = half.vn_effect_fade.expect("a different image cross-fades");
        let source = |spec: &EffectSpec| spec.textures[0].unwrap().source;
        assert_eq!(source(&fade.from), sk_effects::texture_source("a.png"));
        assert_eq!(
            source(&shading_of(&half)),
            sk_effects::texture_source("b.png")
        );
        assert_eq!(
            fade.from.params[2], 0.0,
            "each side keeps its own parameters"
        );

        let same = states(with_shading("grade(url(b.png), 0.5)"), css);
        a.on_computed(2, Some(&new), &same, now);
        let r = a.overlay_test(2, now + Duration::from_millis(500)).unwrap();
        assert!(r.vn_effect_fade.is_none(), "the same image interpolates");
        assert!((shading_of(&r).params[2] - 0.75).abs() < 1e-3);
    }

    #[test]
    fn different_effects_cross_fade() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("frost(0.5)"), css);
        let new = states(with_shading("glow(10px, 0.5)"), css);
        a.on_computed(1, Some(&old), &new, now);
        let half = a.overlay_test(1, now + Duration::from_millis(250)).unwrap();
        let fade = half.vn_effect_fade.expect("a cross-fade");
        assert_eq!(fade.from.id, gpui::effect_id::FROST);
        assert_eq!(shading_of(&half).id, gpui::effect_id::GLOW);
        assert!((fade.progress - 0.25).abs() < 1e-3);
        assert!(a.take_diagnostics().is_empty());

        // Back to frost halfway in: frost grows from the share it had, glow is what fades.
        let back = states(with_shading("frost(0.5)"), css);
        a.on_computed(1, Some(&new), &back, now + Duration::from_millis(250));
        let r = a.overlay_test(1, now + Duration::from_millis(750)).unwrap();
        let fade = r.vn_effect_fade.expect("still a cross-fade");
        assert_eq!(fade.from.id, gpui::effect_id::GLOW);
        assert_eq!(shading_of(&r).id, gpui::effect_id::FROST);
        assert!((fade.progress - 0.875).abs() < 1e-3);
    }

    #[test]
    fn all_does_not_cover_shading_but_naming_it_does() {
        let mut a = Animator::new();
        let now = t0();
        let all = "transition: all 1s linear";
        let old = states(with_shading("frost(0.2)"), all);
        let new = states(with_shading("frost(0.8)"), all);
        a.on_computed(1, Some(&old), &new, now);
        assert!(
            a.overlay(1, &StyleRefinement::default(), now).is_none(),
            "`all` must not cover --shading"
        );
        assert!(a.take_diagnostics().is_empty());

        let named = "transition: all 1s linear, --shading 1s linear";
        let old = states(with_shading("frost(0.2)"), named);
        let new = states(with_shading("frost(0.8)"), named);
        a.on_computed(2, Some(&old), &new, now);
        assert!(a.overlay(2, &StyleRefinement::default(), now).is_some());
    }

    #[test]
    fn transitioning_to_none_fades_out() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("frost(0.5)"), css);
        let new = states(with_shading("none"), css);
        a.on_computed(1, Some(&old), &new, now);
        let r = a.overlay_test(1, now + Duration::from_millis(500)).unwrap();
        assert!(!shading_of(&r).is_some());
        assert_eq!(
            r.vn_effect_fade.map(|f| f.from.id),
            Some(gpui::effect_id::FROST)
        );
    }

    #[test]
    fn an_effect_fades_in_from_and_out_to_an_unset_value() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let unset = states(StyleRefinement::default(), css);
        let glow = states(with_shading("glow(10px, 0.5)"), css);

        a.on_computed(1, Some(&unset), &glow, now);
        let r = a.overlay_test(1, now + Duration::from_millis(250)).unwrap();
        assert_eq!(shading_of(&r).id, gpui::effect_id::GLOW);
        let fade = r.vn_effect_fade.expect("appearing fades in");
        assert!(!fade.from.is_some());
        assert!((fade.progress - 0.25).abs() < 1e-3);

        let mut a = Animator::new();
        a.on_computed(1, Some(&glow), &unset, now);
        let r = a.overlay_test(1, now + Duration::from_millis(250)).unwrap();
        assert!(!shading_of(&r).is_some());
        let fade = r.vn_effect_fade.expect("disappearing fades out");
        assert_eq!(fade.from.id, gpui::effect_id::GLOW);
        assert!((fade.progress - 0.25).abs() < 1e-3);
    }

    #[test]
    fn an_animated_endpoint_keeps_the_whole_transition_animated() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: --shading 1s linear";
        let old = states(with_shading("noise(0.1, 2px, 0)"), css);
        let new = states(with_shading("noise(0.6, 2px, 24)"), css);
        a.on_computed(1, Some(&old), &new, now);
        let half = shading_of(&a.overlay_test(1, now + Duration::from_millis(500)).unwrap());
        assert!(half.animated, "grain must not freeze mid-transition");
        assert!((half.params[0] - 0.35).abs() < 1e-3);
        assert!((half.params[2] - 12.0).abs() < 1e-3);
    }

    fn rgb(r: f32, g: f32, b: f32) -> Hsla {
        Rgba { r, g, b, a: 1.0 }.into()
    }

    fn svg_states(c: Hsla, paint: Option<crate::SvgPaint>, css: &str) -> ComputedStates {
        let mut base = StyleRefinement::default();
        base.text.color = Some(c);
        let mut s = states(base, css);
        s.extras = crate::Extras {
            fill: paint,
            stroke: paint,
            ..Default::default()
        };
        s.color = Some(c);
        s
    }

    #[test]
    fn a_fill_track_follows_the_colour_the_sprite_is_tinted_with() {
        for css in ["transition: fill 1s linear", "transition: stroke 1s linear"] {
            let mut a = Animator::new();
            let now = t0();
            let (red, blue) = (rgb(1.0, 0.0, 0.0), rgb(0.0, 0.0, 1.0));
            let old = svg_states(red, Some(crate::SvgPaint::Current), css);
            let new = svg_states(blue, Some(crate::SvgPaint::Current), css);

            a.on_computed(1, Some(&old), &new, now);
            assert!(a.any_active(now), "{css} must start a track");

            let at = |ms: u64| {
                a.overlay_test(1, now + Duration::from_millis(ms))
                    .and_then(|r| r.text.color)
                    .unwrap_or_else(|| panic!("{css}: no tint at {ms}ms"))
            };
            let same = |a: Hsla, b: Hsla| {
                (a.h - b.h).abs() < 1e-4
                    && (a.s - b.s).abs() < 1e-4
                    && (a.l - b.l).abs() < 1e-4
                    && (a.a - b.a).abs() < 1e-4
            };
            assert!(same(at(0), red), "{css} starts where it was: {:?}", at(0));
            assert!(
                same(at(1000), blue),
                "{css} ends where it is going: {:?}",
                at(1000)
            );
            let half = at(500);
            assert!(
                !same(half, red) && !same(half, blue),
                "{css} interpolates: {half:?}"
            );
        }
    }

    #[test]
    fn an_svg_paint_of_none_snaps_instead_of_interpolating() {
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: fill 1s linear";
        let old = svg_states(rgb(1.0, 0.0, 0.0), Some(crate::SvgPaint::None), css);
        let new = svg_states(rgb(0.0, 0.0, 1.0), Some(crate::SvgPaint::Current), css);

        a.on_computed(1, Some(&old), &new, now);
        assert!(
            !a.any_active(now),
            "an unpainted endpoint has nothing to interpolate"
        );
        assert!(a.overlay_test(1, now).is_none());

        let bare = svg_states(rgb(0.0, 0.0, 1.0), None, css);
        a.on_computed(2, Some(&bare), &bare, now);
        assert!(a.overlay_test(2, now).is_none());
    }

    #[test]
    fn text_decoration_color_animates_without_refining_the_line_away() {
        fn decorated(c: Hsla) -> StyleRefinement {
            let mut r = StyleRefinement::default();
            r.text.underline = Some(gpui::UnderlineStyle {
                thickness: px(3.0),
                color: Some(c),
                wavy: true,
                offset: px(2.0),
            });
            r
        }
        let mut a = Animator::new();
        let now = t0();
        let css = "transition: text-decoration-color 1s linear";
        let (red, blue) = (rgb(1.0, 0.0, 0.0), rgb(0.0, 0.0, 1.0));
        let old = states(decorated(red), css);
        let new = states(decorated(blue), css);

        a.on_computed(1, Some(&old), &new, now);
        assert!(a.any_active(now));
        let base = decorated(red);
        let half = a
            .overlay(1, &base, now + Duration::from_millis(500))
            .unwrap();
        let u = half
            .text
            .underline
            .clone()
            .expect("the overlay carries a decoration");
        assert_eq!(
            u.thickness,
            px(3.0),
            "the base's thickness survives the overlay"
        );
        assert!(u.wavy, "…and so does its waviness");
        assert_eq!(u.offset, px(2.0), "…and its offset");
        let c = u.color.expect("with a colour of its own");
        assert!(c != red && c != blue, "interpolated, not snapped: {c:?}");

        let end = a
            .overlay(1, &base, now + Duration::from_millis(1000))
            .unwrap();
        let ended = end
            .text
            .underline
            .and_then(|u| u.color)
            .expect("a colour at the end");
        assert!(
            (ended.h - blue.h).abs() < 1e-4 && (ended.l - blue.l).abs() < 1e-4,
            "{ended:?}"
        );
    }
}

/// It holds its own `Rc<Keyframes>` rather than a name, because `overlay()` is called from the
/// render walk — which has a node id and a clock and no engine.
struct AnimRun {
    spec: AnimationSpec,
    frames: Rc<Keyframes>,
    custom_context: Option<Rc<crate::CustomAnimationContext>>,
    start: Instant,
    pending: bool,
    started: bool,
    ended: bool,
    paused_for: Duration,
    paused_at: Option<Instant>,
}

impl AnimRun {
    fn elapsed(&self, now: Instant) -> Duration {
        let raw = now.saturating_duration_since(self.start);
        let paused = self.paused_for
            + self
                .paused_at
                .map(|at| now.saturating_duration_since(at))
                .unwrap_or_default();
        raw.saturating_sub(paused)
    }

    fn phase(&self, now: Instant) -> Phase {
        self.spec.phase(self.elapsed(now))
    }

    fn paint_progress(&self, now: Instant) -> Option<f32> {
        match self.phase(now) {
            Phase::Running { progress, .. } => Some(progress),
            Phase::Delayed if matches!(self.spec.fill, FillMode::Backwards | FillMode::Both) => {
                Some(self.spec.phase(self.spec.delay.wait()).progress_or(0.0))
            }
            Phase::Finished { progress }
                if matches!(self.spec.fill, FillMode::Forwards | FillMode::Both) =>
            {
                Some(progress)
            }
            _ => None,
        }
    }

    fn elapsed_seconds(&self, now: Instant) -> f32 {
        self.spec.active_time(self.elapsed(now)).as_secs_f32()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnimEventKind {
    TransitionEnd,
    AnimationStart,
    AnimationEnd,
    AnimationCancel,
}

#[derive(Clone, Debug)]
pub struct AnimEvent {
    pub node: u32,
    pub kind: AnimEventKind,
    pub name: String,
    /// `elapsedTime` in SECONDS, excluding the delay — the DOM's definition.
    pub elapsed: f32,
}

impl Animator {
    pub fn on_computed_animations(
        &mut self,
        node: u32,
        new: &ComputedStates,
        now: Instant,
        resolve: &dyn Fn(&str) -> Option<Rc<Keyframes>>,
    ) {
        self.registered_samples.get_mut().remove(&node);
        let wanted: Vec<AnimationSpec> = if self.reduce_motion {
            Vec::new()
        } else {
            new.animations
                .iter()
                .filter(|s| !s.is_inert())
                .cloned()
                .collect()
        };

        if wanted.is_empty() {
            if let Some(runs) = self.anims.remove(&node) {
                for run in runs {
                    self.cancel_event(node, &run, now);
                }
            }
            return;
        }

        let mut previous = self.anims.remove(&node).unwrap_or_default();
        let mut runs: Vec<AnimRun> = Vec::with_capacity(wanted.len());
        for spec in wanted {
            // CSS restarts an animation when its NAME changes at that position in the list, and
            // only then: changing the duration or the easing retargets a RUNNING animation rather
            // than replaying it.
            let existing = previous.iter().position(|r| r.spec.name == spec.name);
            let Some(frames) = resolve(&spec.name) else {
                self.warn_once(format!(
                    "`animation-name: {}` names no @keyframes block in the stylesheet — \
                     nothing animates (a browser behaves the same way)",
                    spec.name
                ));
                continue;
            };
            match existing {
                Some(i) => {
                    let mut run = previous.remove(i);
                    match (run.paused_at, spec.play_state) {
                        (None, PlayState::Paused) => run.paused_at = Some(now),
                        (Some(at), PlayState::Running) => {
                            run.paused_for += now.saturating_duration_since(at);
                            run.paused_at = None;
                        }
                        _ => {}
                    }
                    run.spec = spec;
                    run.frames = frames;
                    run.custom_context = new.custom_animation.clone();
                    runs.push(run);
                }
                None => {
                    runs.push(AnimRun {
                        paused_at: (spec.play_state == PlayState::Paused).then_some(now),
                        spec,
                        frames,
                        custom_context: new.custom_animation.clone(),
                        start: now,
                        pending: true,
                        started: false,
                        ended: false,
                        paused_for: Duration::ZERO,
                    });
                }
            }
        }
        let leftovers: Vec<AnimRun> = previous;
        for run in leftovers {
            self.cancel_event(node, &run, now);
        }
        if runs.is_empty() {
            self.anims.remove(&node);
        } else {
            self.anims.insert(node, runs);
        }
    }

    pub fn cancel_animations(&mut self, node: u32, now: Instant) {
        self.registered_samples.get_mut().remove(&node);
        if let Some(runs) = self.anims.remove(&node) {
            for run in runs {
                self.cancel_event(node, &run, now);
            }
        }
    }

    pub fn has_animations(&self, node: u32) -> bool {
        self.anims.contains_key(&node)
    }

    /// No CSS animation of `node` is still to finish; a paused one is not finished.
    pub fn animations_finished(&self, node: u32, now: Instant) -> bool {
        self.anims.get(&node).is_none_or(|runs| {
            runs.iter().all(|run| {
                run.paused_at.is_none() && matches!(run.phase(now), Phase::Finished { .. })
            })
        })
    }

    fn cancel_event(&mut self, node: u32, run: &AnimRun, now: Instant) {
        // A run that already reported `animationend` finished; it was not cancelled, and the DOM
        // fires only one of the two.
        if run.ended {
            return;
        }
        self.events.push(AnimEvent {
            node,
            kind: AnimEventKind::AnimationCancel,
            name: run.spec.name.to_string(),
            elapsed: run.elapsed_seconds(now).max(0.0),
        });
    }

    pub fn tick(&mut self, now: Instant) {
        self.registered_samples.get_mut().clear();
        let mut events = Vec::new();
        for (&node, runs) in self.anims.iter_mut() {
            for run in runs.iter_mut() {
                if run.pending {
                    run.pending = false;
                    run.start = now;
                    if run.paused_at.is_some() {
                        run.paused_at = Some(now);
                    }
                }
                let phase = run.phase(now);
                if !run.started && !matches!(phase, Phase::Delayed) {
                    run.started = true;
                    events.push(AnimEvent {
                        node,
                        kind: AnimEventKind::AnimationStart,
                        name: run.spec.name.to_string(),
                        elapsed: run.spec.active_time(Duration::ZERO).as_secs_f32(),
                    });
                }
                if !run.ended && matches!(phase, Phase::Finished { .. }) {
                    run.ended = true;
                    events.push(AnimEvent {
                        node,
                        kind: AnimEventKind::AnimationEnd,
                        name: run.spec.name.to_string(),
                        elapsed: run
                            .spec
                            .total()
                            .map(|t| run.spec.active_time(t).as_secs_f32())
                            .unwrap_or(0.0),
                    });
                }
            }
        }
        self.events.append(&mut events);
    }

    pub fn take_events(&mut self) -> Vec<AnimEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn moving_nodes(&self, now: Instant) -> impl Iterator<Item = u32> + '_ {
        let tracks = self
            .nodes
            .iter()
            .filter(move |(_, tracks)| tracks.iter().any(|t| !t.finished(now)))
            .map(|(&node, _)| node);
        let runs = self
            .anims
            .iter()
            .filter(move |(_, runs)| {
                runs.iter().any(|run| match run.phase(now) {
                    Phase::Finished { .. } => false,
                    Phase::Delayed | Phase::Running { .. } => run.paused_at.is_none(),
                })
            })
            .map(|(&node, _)| node);
        let timeline = self.timeline_ms(now);
        let scripts = self
            .scripts
            .iter()
            .filter(move |(_, list)| list.iter().any(|a| a.running(timeline)))
            .map(|(&node, _)| node);
        tracks.chain(runs).chain(scripts)
    }

    pub fn any_animation(&self, now: Instant) -> bool {
        let css = self
            .anims
            .values()
            .flatten()
            .any(|run| match run.phase(now) {
                Phase::Finished { .. } => false,
                Phase::Delayed | Phase::Running { .. } => run.paused_at.is_none(),
            });
        css || {
            let timeline = self.timeline_ms(now);
            self.scripts.values().flatten().any(|a| a.running(timeline))
        }
    }

    pub fn animated_extras(&self, node: u32, now: Instant) -> Option<crate::Extras> {
        self.registered_overlay(node, now)
            .map(|sample| sample.1.clone())
    }

    fn registered_overlay(
        &self,
        node: u32,
        now: Instant,
    ) -> Option<Rc<(StyleRefinement, crate::Extras)>> {
        if let Some((at, sample)) = self.registered_samples.borrow().get(&node) {
            if *at == now {
                return Some(sample.clone());
            }
        }
        let runs = self.anims.get(&node)?;
        let mut context = None;
        let mut samples = Vec::new();
        for run in runs {
            let Some(scope) = &run.custom_context else {
                continue;
            };
            let Some(progress) = run.paint_progress(now) else {
                continue;
            };
            for registration in &scope.registrations {
                if scope.important_custom.contains(&registration.name) {
                    continue;
                }
                let declares = |step: &crate::Keyframe| {
                    scope.keyframe_value(step, &registration.name).is_some()
                };
                let Some(segment) = run.frames.segment_steps(&declares, progress) else {
                    continue;
                };
                let implicit = scope
                    .vars
                    .get(&registration.name)
                    .and_then(crate::registered::percentage)
                    .unwrap_or(registration.initial);
                let from = segment
                    .from
                    .and_then(|step| scope.keyframe_value(step, &registration.name))
                    .unwrap_or(implicit);
                let to = segment
                    .to
                    .and_then(|step| scope.keyframe_value(step, &registration.name))
                    .unwrap_or(implicit);
                let t = segment
                    .easing
                    .unwrap_or(run.spec.easing)
                    .eval(segment.t.clamp(0., 1.));
                samples.push((registration.name.clone(), from + (to - from) * t));
                context = Some(scope);
            }
        }
        context.map(|scope| {
            let sample = Rc::new(scope.sample(&samples));
            self.registered_samples
                .borrow_mut()
                .insert(node, (now, sample.clone()));
            sample
        })
    }

    fn animation_overlay(
        &self,
        node: u32,
        base: &StyleRefinement,
        now: Instant,
    ) -> Option<StyleRefinement> {
        let runs = self.anims.get(&node)?;
        let mut out = self
            .registered_overlay(node, now)
            .map(|sample| sample.0.clone());
        for run in runs {
            let progress = match run.phase(now) {
                Phase::Running { progress, .. } => progress,
                Phase::Delayed => {
                    if !matches!(run.spec.fill, FillMode::Backwards | FillMode::Both) {
                        continue;
                    }
                    run.spec.phase(run.spec.delay.wait()).progress_or(0.0)
                }
                Phase::Finished { progress } => {
                    if !matches!(run.spec.fill, FillMode::Forwards | FillMode::Both) {
                        continue;
                    }
                    progress
                }
            };
            let r = out.get_or_insert_with(StyleRefinement::default);
            let keywords = self.keywords.contains(&node);
            apply_keyframes(r, base, &run.frames, &run.spec, progress, keywords);
        }
        out
    }
}

impl Phase {
    fn progress_or(self, default: f32) -> f32 {
        match self {
            Phase::Running { progress, .. } | Phase::Finished { progress } => progress,
            Phase::Delayed => default,
        }
    }
}

fn apply_keyframes(
    out: &mut StyleRefinement,
    base: &StyleRefinement,
    frames: &Keyframes,
    spec: &AnimationSpec,
    progress: f32,
    keywords: bool,
) {
    for property in TransitionProperty::keyframed() {
        let declares: &dyn Fn(&StyleRefinement) -> bool =
            &|r: &StyleRefinement| slot_of(r, &property) != Slot::Missing;
        let Some(segment) = frames.segment(declares, progress) else {
            continue;
        };
        // Needed for Tailwind's `pulse` (`50% { opacity: .5 }` only): without the fallback both
        // ends are unset and the animation sits at 0.5.
        let implicit = || match slot_of(base, &property) {
            Slot::Missing => initial(&property).map_or(Slot::Missing, Slot::Value),
            slot => slot,
        };
        let from = segment
            .from
            .map(|k| slot_of(&k.style, &property))
            .unwrap_or_else(implicit);
        let to = segment
            .to
            .map(|k| slot_of(&k.style, &property))
            .unwrap_or_else(implicit);
        let (from, to) = size_pair(from, to, &property, keywords).unwrap_or((from, to));
        let (Slot::Value(a), Slot::Value(b)) = (from, to) else {
            if let Slot::Value(v) = from {
                install(out, base, &property, v);
            } else if let Slot::Value(v) = to {
                install(out, base, &property, v);
            }
            continue;
        };
        let Some((a, b)) = unify(a, b) else {
            install(out, base, &property, if segment.t < 0.5 { a } else { b });
            continue;
        };
        let easing = segment.easing.unwrap_or(spec.easing);
        install(
            out,
            base,
            &property,
            lerp(a, b, easing.eval(segment.t.clamp(0.0, 1.0))),
        );
    }
}

#[cfg(test)]
mod border_colors_regressions {
    use super::*;
    #[test]
    fn transition_interpolates_each_border_side_and_uniform_shorthand_resets_it() {
        let mut before = StyleRefinement::default();
        before.border_color = Some(gpui::red());
        before.vn_border_colors.left = Some(Some(gpui::blue()));
        let mut after = StyleRefinement::default();
        after.border_color = Some(gpui::green());
        let (Slot::Value(a), Slot::Value(b)) = (
            slot_of(&before, &TransitionProperty::BorderColor),
            slot_of(&after, &TransitionProperty::BorderColor),
        ) else {
            panic!("border endpoints missing")
        };
        let (a, b) = unify(a, b).expect("uniform and per-side colours interpolate");
        let mut overlay = StyleRefinement::default();
        install(
            &mut overlay,
            &after,
            &TransitionProperty::BorderColor,
            lerp(a, b, 0.5),
        );
        assert_ne!(overlay.vn_border_colors.left, overlay.vn_border_colors.top);
        install(
            &mut overlay,
            &before,
            &TransitionProperty::BorderColor,
            Animatable::Color(gpui::green()),
        );
        assert_eq!(overlay.vn_border_colors.left, Some(None));
        clear(&mut overlay, &TransitionProperty::BorderColor);
        assert_eq!(overlay.vn_border_colors.left, None);
        assert_eq!(overlay.border_color, None);
    }
}

#[cfg(test)]
mod size_interpolation {
    use super::*;
    use gpui::{px, VnCalcSize, VnSizeBasis};
    use std::rc::Rc;

    fn computed(style: &str) -> ComputedStates {
        let mut diagnostics = Vec::new();
        let inline = crate::parse_inline_style(
            &format!("{style}; transition: height 1s linear"),
            &mut diagnostics,
        );
        let mut base = inline.refinement.map(|r| (*r).clone()).unwrap_or_default();
        let transitions = inline.transitions.map(|t| (*t).clone()).unwrap_or_default();
        for (name, value) in inline.dynamic.iter() {
            crate::mapper::apply_dynamic(
                name,
                value,
                &crate::Vars::default(),
                &crate::Env::default(),
                &mut base,
                &mut crate::Extras::default(),
                &mut diagnostics,
            );
        }
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        ComputedStates {
            base: Rc::new(base),
            transitions: Rc::new(transitions),
            ..Default::default()
        }
    }

    fn height_at(keywords: bool, from: &str, to: &str, ms: u64) -> Option<Length> {
        let mut a = Animator::new();
        a.set_interpolate_keywords(1, keywords);
        let now = Instant::now();
        a.on_computed(1, Some(&computed(from)), &computed(to), now);
        a.overlay(
            1,
            &StyleRefinement::default(),
            now + Duration::from_millis(ms),
        )
        .and_then(|r| r.size.height)
    }

    fn auto_times(scale: f32, pixels: f32) -> Option<Length> {
        Some(Length::VnCalcSize(VnCalcSize {
            basis: VnSizeBasis::Auto,
            scale,
            offset: px(pixels).into(),
        }))
    }

    #[test]
    fn allow_keywords_interpolates_auto_with_a_length() {
        assert_eq!(
            height_at(true, "height: 0px", "height: auto", 250),
            auto_times(0.25, 0.)
        );
        assert_eq!(
            height_at(true, "height: auto", "height: 40px", 250),
            auto_times(0.75, 10.)
        );
        assert_eq!(
            height_at(true, "height: 10px", "width: 1px", 500),
            auto_times(0.5, 5.),
            "an unset height is auto"
        );
    }

    #[test]
    fn numeric_only_snaps_between_auto_and_a_length() {
        assert_eq!(height_at(false, "height: 0px", "height: auto", 250), None);
    }

    #[test]
    fn calc_size_interpolates_without_allow_keywords() {
        assert_eq!(
            height_at(false, "height: 0px", "height: calc-size(auto, size)", 500),
            auto_times(0.5, 0.)
        );
        assert_eq!(
            height_at(
                false,
                "height: calc-size(auto, size + 20px)",
                "height: 100px",
                500
            ),
            auto_times(0.5, 60.)
        );
        assert_eq!(
            height_at(false, "height: 50%", "height: calc-size(auto, size)", 500),
            Some(Length::VnCalcSize(VnCalcSize {
                basis: VnSizeBasis::Auto,
                scale: 0.5,
                offset: DefiniteLength::Fraction(0.25),
            }))
        );
    }

    #[test]
    fn different_bases_do_not_interpolate() {
        assert_eq!(
            height_at(
                true,
                "height: calc-size(fit-content, size)",
                "height: calc-size(min-content, size)",
                500
            ),
            None
        );
    }

    #[test]
    fn a_script_animation_to_auto_interpolates_under_allow_keywords() {
        let frames = [
            ScriptFrame {
                offset: 0.,
                easing: None,
                decls: vec![("height".into(), "0px".into())],
            },
            ScriptFrame {
                offset: 1.,
                easing: None,
                decls: vec![("height".into(), "auto".into())],
            },
        ];
        let (frames, notes) =
            script_keyframes(&frames, &crate::Vars::default(), &crate::Env::default());
        assert!(notes.diagnostics.is_empty(), "{notes:?}");
        let mut a = Animator::new();
        let t0 = Instant::now();
        a.set_timeline_origin(t0);
        a.set_interpolate_keywords(7, true);
        a.script_set(
            1,
            7,
            frames,
            ScriptTiming {
                duration: 1000.,
                ..ScriptTiming::default()
            },
        );
        a.script_clock(
            1,
            ScriptClock {
                start: Some(0.),
                hold: None,
                rate: 1.,
            },
        );
        let height = a
            .overlay(
                7,
                &StyleRefinement::default(),
                t0 + Duration::from_millis(400),
            )
            .and_then(|r| r.size.height);
        assert_eq!(height, auto_times(0.4, 0.));
    }
}
