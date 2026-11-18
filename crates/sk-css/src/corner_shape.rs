//! `corner-shape` and its longhands and side shorthands (css-borders-4 §3.7–3.8). The flow-relative
//! names map as horizontal-tb, left-to-right.

use gpui::CornerShape;
use lightningcss::traits::Parse;

/// Corner indices in `corner-shape` value order, matching the `RadiusInherit` bit order.
const TL: usize = 0;
const TR: usize = 1;
const BR: usize = 2;
const BL: usize = 3;

/// For each number of values a property takes, the corners each value sets.
type Layout = &'static [&'static [&'static [usize]]];

fn layout(name: &str) -> Option<Layout> {
    const ALL: Layout = &[
        &[&[TL, TR, BR, BL]],
        &[&[TL, BR], &[TR, BL]],
        &[&[TL], &[TR, BL], &[BR]],
        &[&[TL], &[TR], &[BR], &[BL]],
    ];
    Some(match name.to_ascii_lowercase().as_str() {
        "corner-shape" => ALL,
        "corner-top-left-shape" | "corner-start-start-shape" => &[&[&[TL]]],
        "corner-top-right-shape" | "corner-start-end-shape" => &[&[&[TR]]],
        "corner-bottom-right-shape" | "corner-end-end-shape" => &[&[&[BR]]],
        "corner-bottom-left-shape" | "corner-end-start-shape" => &[&[&[BL]]],
        "corner-top-shape" | "corner-block-start-shape" => &[&[&[TL, TR]], &[&[TL], &[TR]]],
        "corner-bottom-shape" | "corner-block-end-shape" => &[&[&[BL, BR]], &[&[BL], &[BR]]],
        "corner-left-shape" | "corner-inline-start-shape" => &[&[&[TL, BL]], &[&[TL], &[BL]]],
        "corner-right-shape" | "corner-inline-end-shape" => &[&[&[TR, BR]], &[&[TR], &[BR]]],
        _ => return None,
    })
}

pub(crate) fn is_property(name: &str) -> bool {
    layout(name).is_some()
}

/// The RadiusInherit bit of a corner's shape, above the four radius bits.
pub(crate) fn inherit_bit(corner: usize) -> u8 {
    1 << (4 + corner)
}

#[derive(Debug, PartialEq)]
pub(crate) enum Declared {
    Shapes([Option<CornerShape>; 4]),
    /// The `RadiusInherit` bits of the corners that take the parent's shape.
    Inherit(u8),
}

pub(crate) fn parse(name: &str, value: &str) -> Result<Declared, String> {
    let layout = layout(name).ok_or_else(|| format!("`{name}` is not a corner shape property"))?;
    let corners = || layout[0].iter().flat_map(|c| c.iter().copied());
    let value = value.trim();
    match value.to_ascii_lowercase().as_str() {
        "inherit" => return Ok(Declared::Inherit(corners().map(inherit_bit).sum())),
        "initial" | "unset" | "revert" | "revert-layer" => {
            let mut shapes = [None; 4];
            corners().for_each(|c| shapes[c] = Some(CornerShape::ROUND));
            return Ok(Declared::Shapes(shapes));
        }
        _ => {}
    }
    let values = split(value)
        .into_iter()
        .map(shape)
        .collect::<Result<Vec<_>, _>>()?;
    let targets = values
        .len()
        .checked_sub(1)
        .and_then(|i| layout.get(i))
        .ok_or_else(|| {
            format!(
                "`{name}` takes 1 to {} values, got {}",
                layout.len(),
                values.len()
            )
        })?;
    let mut shapes = [None; 4];
    for (value, corners) in values.into_iter().zip(targets.iter()) {
        corners.iter().for_each(|&c| shapes[c] = Some(value));
    }
    Ok(Declared::Shapes(shapes))
}

/// The top-level space-separated parts, keeping each function call whole.
fn split(value: &str) -> Vec<&str> {
    let (mut parts, mut depth, mut start) = (Vec::new(), 0usize, None);
    for (i, ch) in value.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            c if c.is_whitespace() && depth == 0 => {
                if let Some(s) = start.take() {
                    parts.push(&value[s..i]);
                }
                continue;
            }
            _ => {}
        }
        start.get_or_insert(i);
    }
    if let Some(s) = start {
        parts.push(&value[s..]);
    }
    parts
}

fn shape(part: &str) -> Result<CornerShape, String> {
    let lower = part.to_ascii_lowercase();
    Ok(match lower.as_str() {
        "round" => CornerShape::ROUND,
        "scoop" => CornerShape::SCOOP,
        "bevel" => CornerShape::BEVEL,
        "notch" => CornerShape::NOTCH,
        "square" => CornerShape::SQUARE,
        "squircle" => CornerShape::SQUIRCLE,
        _ => {
            let argument = lower
                .strip_prefix("superellipse(")
                .and_then(|rest| rest.strip_suffix(')'))
                .map(str::trim)
                .ok_or_else(|| {
                    format!(
                        "`{part}` is not a corner shape: use `round`, `scoop`, `bevel`, `notch`, \
                         `square`, `squircle` or `superellipse(<number>)`"
                    )
                })?;
            match argument {
                "infinity" => CornerShape::SQUARE,
                "-infinity" => CornerShape::NOTCH,
                text => match f32::parse_string(text) {
                    Ok(k) if !k.is_nan() => CornerShape(k),
                    _ => {
                        return Err(format!(
                            "`{part}` needs a number, `infinity` or `-infinity` inside \
                             `superellipse()`"
                        ))
                    }
                },
            }
        }
    })
}

/// A shape as CSS: its keyword when it has one.
pub fn to_css(shape: CornerShape) -> String {
    let k = shape.0;
    match k {
        _ if k == 1. => "round".into(),
        _ if k == -1. => "scoop".into(),
        _ if k == 0. => "bevel".into(),
        _ if k == 2. => "squircle".into(),
        f32::INFINITY => "square".into(),
        f32::NEG_INFINITY => "notch".into(),
        _ => format!("superellipse({k})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shapes(name: &str, value: &str) -> [Option<f32>; 4] {
        match parse(name, value) {
            Ok(Declared::Shapes(s)) => s.map(|s| s.map(|s| s.0)),
            other => panic!("{name}: {value} gave {other:?}"),
        }
    }

    const INF: f32 = f32::INFINITY;

    #[test]
    fn keywords_and_superellipse_parse() {
        let one = |v: &str| shapes("corner-top-left-shape", v)[0].unwrap();
        assert_eq!(one("round"), 1.);
        assert_eq!(one("SCOOP"), -1.);
        assert_eq!(one("bevel"), 0.);
        assert_eq!(one("notch"), -INF);
        assert_eq!(one("square"), INF);
        assert_eq!(one("squircle"), 2.);
        assert_eq!(one("superellipse(0.5)"), 0.5);
        assert_eq!(one("superellipse(-3)"), -3.);
        assert_eq!(one("Superellipse( infinity )"), INF);
        assert_eq!(one("superellipse(-infinity)"), -INF);
        assert_eq!(one("superellipse(calc(1 + 2))"), 3.);
    }

    #[test]
    fn values_expand_like_border_radius() {
        let (r, s, b) = (Some(1.), Some(INF), Some(0.));
        assert_eq!(shapes("corner-shape", "round"), [r; 4]);
        assert_eq!(shapes("corner-shape", "round square"), [r, s, r, s]);
        assert_eq!(shapes("corner-shape", "round square bevel"), [r, s, b, s]);
        assert_eq!(
            shapes("corner-shape", "round square bevel notch"),
            [r, s, b, Some(-INF)]
        );
    }

    #[test]
    fn side_and_logical_names_set_their_corners() {
        let (r, s) = (Some(1.), Some(INF));
        assert_eq!(
            shapes("corner-top-shape", "round square"),
            [r, s, None, None]
        );
        assert_eq!(
            shapes("corner-bottom-shape", "round square"),
            [None, None, s, r]
        );
        assert_eq!(
            shapes("corner-left-shape", "round square"),
            [r, None, None, s]
        );
        assert_eq!(shapes("corner-right-shape", "square"), [None, s, s, None]);
        assert_eq!(
            shapes("corner-block-start-shape", "round square"),
            [r, s, None, None]
        );
        assert_eq!(
            shapes("corner-inline-end-shape", "round square"),
            [None, r, s, None]
        );
        assert_eq!(
            shapes("corner-start-end-shape", "square"),
            [None, s, None, None]
        );
        assert_eq!(
            shapes("corner-end-start-shape", "square"),
            [None, None, None, s]
        );
        assert_eq!(
            shapes("corner-end-end-shape", "square"),
            [None, None, s, None]
        );
    }

    #[test]
    fn css_wide_keywords() {
        assert_eq!(
            shapes("corner-top-shape", "initial"),
            [Some(1.), Some(1.), None, None]
        );
        assert_eq!(shapes("corner-shape", "unset"), [Some(1.); 4]);
        assert_eq!(
            parse("corner-right-shape", "inherit"),
            Ok(Declared::Inherit(inherit_bit(TR) | inherit_bit(BR)))
        );
    }

    #[test]
    fn invalid_values_are_errors() {
        for (name, value) in [
            ("corner-shape", "round round round round round"),
            ("corner-top-shape", "round square bevel"),
            ("corner-top-left-shape", "round square"),
            ("corner-shape", "rounded"),
            ("corner-shape", "superellipse()"),
            ("corner-shape", "superellipse(1px)"),
            ("corner-shape", "superellipse(1, 2)"),
            ("corner-shape", "8px"),
            ("corner-shape", ""),
        ] {
            assert!(parse(name, value).is_err(), "{name}: {value}");
        }
    }

    fn mapped(body: &str) -> (gpui::StyleRefinement, crate::Extras, Vec<String>) {
        let mut d = Vec::new();
        let (r, _, x) = crate::mapper::parse_inline_full(body, &mut d);
        (r, x, d)
    }

    #[test]
    fn the_mapper_sets_the_shapes_and_drops_an_invalid_value() {
        let (r, _, d) = mapped("corner-shape: bevel; corner-bottom-right-shape: superellipse(3)");
        assert!(d.is_empty(), "{d:?}");
        let s = r.vn_corner_shapes;
        assert_eq!(s.top_left, Some(CornerShape::BEVEL));
        assert_eq!(s.bottom_right, Some(CornerShape(3.)));

        let (r, _, d) = mapped("corner-shape: bevel; corner-shape: wobbly");
        assert_eq!(r.vn_corner_shapes.top_left, Some(CornerShape::BEVEL));
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn var_is_substituted() {
        let mut d = Vec::new();
        let vars = crate::vars::resolve(
            &std::rc::Rc::new(crate::Vars::default()),
            &[&(
                "--s".to_string(),
                "scoop superellipse(-infinity)".to_string(),
            )],
            &crate::Env::default(),
            &mut d,
        );
        let (mut r, mut x) = (gpui::StyleRefinement::default(), crate::Extras::default());
        crate::mapper::apply_dynamic(
            "corner-top-shape",
            "var(--s)",
            &vars,
            &crate::Env::default(),
            &mut r,
            &mut x,
            &mut d,
        );
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(r.vn_corner_shapes.top_left, Some(CornerShape::SCOOP));
        assert_eq!(r.vn_corner_shapes.top_right, Some(CornerShape::NOTCH));
    }

    #[test]
    fn inherit_takes_the_parents_shape_and_a_later_value_cancels_it() {
        let (r, _, _) = mapped("corner-shape: squircle bevel");
        let parent = crate::InheritedRadii::of(&r);
        let child = |body: &str| {
            let (r, extras, d) = mapped(body);
            assert!(d.is_empty(), "{d:?}");
            let mut states = crate::ComputedStates {
                base: std::rc::Rc::new(r),
                extras,
                ..Default::default()
            };
            crate::resolve_inherited_radii(&mut states, parent);
            states.base.vn_corner_shapes.clone()
        };
        let s = child("corner-right-shape: inherit");
        assert_eq!(s.top_right, Some(CornerShape::BEVEL));
        assert_eq!(s.bottom_right, Some(CornerShape::SQUIRCLE));
        assert_eq!(s.top_left, None);
        let s = child("corner-shape: inherit; corner-top-left-shape: notch");
        assert_eq!(s.top_left, Some(CornerShape::NOTCH));
        assert_eq!(s.bottom_left, Some(CornerShape::BEVEL));
    }

    fn transitioned(from: &str, to: &str, at_ms: u64) -> gpui::CornersRefinement<CornerShape> {
        use std::time::{Duration, Instant};
        let states = |shape: &str| {
            let mut d = Vec::new();
            let (r, t) = crate::parse_inline_with_transitions(
                &format!("corner-shape: {shape}; transition: corner-shape 1s linear"),
                &mut d,
            );
            assert!(d.is_empty(), "{d:?}");
            crate::ComputedStates {
                base: std::rc::Rc::new(r),
                transitions: std::rc::Rc::new(t),
                ..Default::default()
            }
        };
        let mut a = crate::Animator::new();
        let now = Instant::now();
        a.on_computed(1, Some(&states(from)), &states(to), now);
        a.overlay(
            1,
            &gpui::StyleRefinement::default(),
            now + Duration::from_millis(at_ms),
        )
        .expect("a running transition")
        .vn_corner_shapes
    }

    #[test]
    fn transitions_move_the_half_corner() {
        let s = transitioned("bevel", "square", 500);
        let mid = CornerShape::BEVEL.interpolate(CornerShape::SQUARE, 0.5);
        assert!((mid.half_corner() - 0.75).abs() < 1e-4);
        assert_eq!([s.top_left, s.bottom_left], [Some(mid); 2]);
        let s = transitioned("round", "scoop", 500);
        assert!(s.top_right.is_some_and(|k| k.0.abs() < 1e-3), "{s:?}");
        let s = transitioned("superellipse(infinity)", "round", 250);
        assert!(
            s.top_left.is_some_and(|k| k.0.is_finite() && k.0 > 1.),
            "{s:?}"
        );
    }

    #[test]
    fn serialization_round_trips() {
        for v in [
            "round",
            "notch",
            "square",
            "superellipse(0.5)",
            "superellipse(-4)",
        ] {
            let parsed = shapes("corner-top-left-shape", v)[0].unwrap();
            assert_eq!(to_css(CornerShape(parsed)), v);
        }
    }
}
