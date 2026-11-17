//! CSS Scroll Snap: `scroll-snap-type`, `scroll-snap-align`, `scroll-snap-stop`, `scroll-padding`
//! and `scroll-margin`, carried to the renderer (`sk_tree::snap`) that applies them. Writing mode is
//! always horizontal-tb, left to right: block is y, inline is x.

use std::rc::Rc;

use lightningcss::properties::custom::{Token, TokenList, TokenOrValue};
use lightningcss::properties::Property;
use lightningcss::values::length::LengthPercentageOrAuto;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapType {
    pub x: bool,
    pub y: bool,
    pub mandatory: bool,
}

impl SnapType {
    pub const NONE: Self = Self {
        x: false,
        y: false,
        mandatory: false,
    };

    pub fn snaps(self) -> bool {
        self.x || self.y
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SnapAlign {
    #[default]
    None,
    Start,
    End,
    Center,
}

/// A `scroll-padding` edge; a percentage is of the scrollport's size on that axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollInset {
    Auto,
    Length { px: f32, fraction: f32 },
}

impl ScrollInset {
    pub fn resolve(self, basis: f32) -> f32 {
        match self {
            ScrollInset::Auto => 0.0,
            ScrollInset::Length { px, fraction } => px + fraction * basis,
        }
    }
}

/// Top, right, bottom, left; `None` is an edge no declaration set.
pub type Edges<T> = [Option<T>; 4];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnapPatch {
    pub snap_type: Option<SnapType>,
    /// (x, y).
    pub align: Option<(SnapAlign, SnapAlign)>,
    pub stop_always: Option<bool>,
    pub padding: Option<Rc<Edges<ScrollInset>>>,
    pub margin: Option<Rc<Edges<f32>>>,
}

impl SnapPatch {
    pub(crate) fn refine(&mut self, other: &Self) {
        if other.snap_type.is_some() {
            self.snap_type = other.snap_type;
        }
        if other.align.is_some() {
            self.align = other.align;
        }
        if other.stop_always.is_some() {
            self.stop_always = other.stop_always;
        }
        merge(&mut self.padding, &other.padding);
        merge(&mut self.margin, &other.margin);
    }

    /// `scroll-padding` resolved against a scrollport of `width` × `height`: top, right, bottom, left.
    pub fn padding_px(&self, width: f32, height: f32) -> [f32; 4] {
        let edges = self.padding.as_deref().copied().unwrap_or_default();
        let basis = [height, width, height, width];
        std::array::from_fn(|i| edges[i].map_or(0.0, |e| e.resolve(basis[i]).max(0.0)))
    }

    pub fn margin_px(&self) -> [f32; 4] {
        let edges = self.margin.as_deref().copied().unwrap_or_default();
        edges.map(|e| e.unwrap_or(0.0))
    }
}

fn merge<T: Copy>(mine: &mut Option<Rc<Edges<T>>>, theirs: &Option<Rc<Edges<T>>>) {
    let Some(theirs) = theirs else { return };
    let next = match mine.as_deref() {
        None => **theirs,
        Some(edges) => {
            let mut edges = *edges;
            for (e, t) in edges.iter_mut().zip(theirs.iter()) {
                if t.is_some() {
                    *e = *t;
                }
            }
            edges
        }
    };
    *mine = Some(Rc::new(next));
}

fn set_edges<T: Copy>(slot: &mut Option<Rc<Edges<T>>>, which: [bool; 4], value: T) {
    let mut edges: Edges<T> = [None; 4];
    for (e, on) in edges.iter_mut().zip(which) {
        if on {
            *e = Some(value);
        }
    }
    merge(slot, &Some(Rc::new(edges)));
}

const TOP: [bool; 4] = [true, false, false, false];
const RIGHT: [bool; 4] = [false, true, false, false];
const BOTTOM: [bool; 4] = [false, false, true, false];
const LEFT: [bool; 4] = [false, false, false, true];

/// The keywords of a token list, or `None` when anything else is in it.
fn keywords(value: &TokenList<'_>) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for t in value.0.iter() {
        match t {
            TokenOrValue::Token(Token::WhiteSpace(_)) => {}
            TokenOrValue::Token(Token::Ident(id)) => out.push(id.as_ref().to_ascii_lowercase()),
            _ => return None,
        }
    }
    Some(out)
}

/// `scroll-snap-type`, `scroll-snap-align` or `scroll-snap-stop`, which lightningcss leaves as tokens.
pub(crate) fn apply_keywords(
    name: &str,
    value: &TokenList<'_>,
    snap: &mut SnapPatch,
    diagnostics: &mut Vec<String>,
) {
    let words = keywords(value).unwrap_or_default();
    let words: Vec<&str> = words.iter().map(String::as_str).collect();
    if let [w] = words.as_slice() {
        match *w {
            "inherit" => return,
            "initial" | "unset" | "revert" | "revert-layer" => {
                match name {
                    "scroll-snap-type" => snap.snap_type = Some(SnapType::NONE),
                    "scroll-snap-align" => snap.align = Some((SnapAlign::None, SnapAlign::None)),
                    _ => snap.stop_always = Some(false),
                }
                return;
            }
            _ => {}
        }
    }
    let ok = match name {
        "scroll-snap-type" => snap_type(&words).map(|t| snap.snap_type = Some(t)),
        "scroll-snap-align" => snap_align(&words).map(|a| snap.align = Some(a)),
        _ => match words.as_slice() {
            ["normal"] => Some(snap.stop_always = Some(false)),
            ["always"] => Some(snap.stop_always = Some(true)),
            _ => None,
        },
    };
    if ok.is_none() {
        let expected = match name {
            "scroll-snap-type" => {
                "`none`, or `x` | `y` | `block` | `inline` | `both` then optionally `mandatory` | `proximity`"
            }
            "scroll-snap-align" => "one or two of `none` | `start` | `end` | `center`",
            _ => "`normal` | `always`",
        };
        crate::mapper::once(
            diagnostics,
            format!("`{name}` takes {expected} — the declaration was dropped"),
        );
    }
}

fn snap_type(words: &[&str]) -> Option<SnapType> {
    let (axis, strictness) = match words {
        ["none"] => return Some(SnapType::NONE),
        [axis] => (*axis, "proximity"),
        [axis, strictness] => (*axis, *strictness),
        _ => return None,
    };
    let (x, y) = match axis {
        "x" | "inline" => (true, false),
        "y" | "block" => (false, true),
        "both" => (true, true),
        _ => return None,
    };
    let mandatory = match strictness {
        "mandatory" => true,
        "proximity" => false,
        _ => return None,
    };
    Some(SnapType { x, y, mandatory })
}

fn snap_align(words: &[&str]) -> Option<(SnapAlign, SnapAlign)> {
    let one = |w: &str| match w {
        "none" => Some(SnapAlign::None),
        "start" => Some(SnapAlign::Start),
        "end" => Some(SnapAlign::End),
        "center" => Some(SnapAlign::Center),
        _ => None,
    };
    match words {
        [both] => one(both).map(|a| (a, a)),
        [block, inline] => Some((one(inline)?, one(block)?)),
        _ => None,
    }
}

fn inset(v: &LengthPercentageOrAuto) -> Option<ScrollInset> {
    use gpui::{AbsoluteLength, DefiniteLength};
    let LengthPercentageOrAuto::LengthPercentage(lp) = v else {
        return Some(ScrollInset::Auto);
    };
    Some(match crate::mapper::lp_definite(lp)? {
        DefiniteLength::Absolute(AbsoluteLength::Pixels(p)) => ScrollInset::Length {
            px: f32::from(p),
            fraction: 0.0,
        },
        DefiniteLength::Absolute(AbsoluteLength::Rems(r)) => ScrollInset::Length {
            px: r.0 * crate::mapper::rem_basis(),
            fraction: 0.0,
        },
        DefiniteLength::Fraction(f) => ScrollInset::Length {
            px: 0.0,
            fraction: f,
        },
        DefiniteLength::Calc { fraction, pixels } => ScrollInset::Length {
            px: f32::from(pixels),
            fraction,
        },
    })
}

fn margin(v: &LengthPercentageOrAuto, diagnostics: &mut Vec<String>) -> Option<f32> {
    match inset(v) {
        Some(ScrollInset::Length { px, fraction }) if fraction == 0.0 => Some(px),
        _ => {
            crate::mapper::once(
                diagnostics,
                "`scroll-margin` takes a length — `auto` and percentages are invalid, and the \
                 declaration was dropped"
                    .into(),
            );
            None
        }
    }
}

/// `scroll-padding*` and `scroll-margin*`, physical and logical.
pub(crate) fn apply_box(p: &Property, snap: &mut SnapPatch, diagnostics: &mut Vec<String>) {
    use Property::*;
    let mut pad: Vec<([bool; 4], &LengthPercentageOrAuto)> = Vec::new();
    let mut mar: Vec<([bool; 4], &LengthPercentageOrAuto)> = Vec::new();
    match p {
        ScrollPaddingTop(v) | ScrollPaddingBlockStart(v) => pad.push((TOP, v)),
        ScrollPaddingBottom(v) | ScrollPaddingBlockEnd(v) => pad.push((BOTTOM, v)),
        ScrollPaddingLeft(v) | ScrollPaddingInlineStart(v) => pad.push((LEFT, v)),
        ScrollPaddingRight(v) | ScrollPaddingInlineEnd(v) => pad.push((RIGHT, v)),
        ScrollPaddingBlock(v) => pad.extend([(TOP, &v.block_start), (BOTTOM, &v.block_end)]),
        ScrollPaddingInline(v) => pad.extend([(LEFT, &v.inline_start), (RIGHT, &v.inline_end)]),
        ScrollPadding(v) => pad.extend([
            (TOP, &v.top),
            (RIGHT, &v.right),
            (BOTTOM, &v.bottom),
            (LEFT, &v.left),
        ]),
        ScrollMarginTop(v) | ScrollMarginBlockStart(v) => mar.push((TOP, v)),
        ScrollMarginBottom(v) | ScrollMarginBlockEnd(v) => mar.push((BOTTOM, v)),
        ScrollMarginLeft(v) | ScrollMarginInlineStart(v) => mar.push((LEFT, v)),
        ScrollMarginRight(v) | ScrollMarginInlineEnd(v) => mar.push((RIGHT, v)),
        ScrollMarginBlock(v) => mar.extend([(TOP, &v.block_start), (BOTTOM, &v.block_end)]),
        ScrollMarginInline(v) => mar.extend([(LEFT, &v.inline_start), (RIGHT, &v.inline_end)]),
        ScrollMargin(v) => mar.extend([
            (TOP, &v.top),
            (RIGHT, &v.right),
            (BOTTOM, &v.bottom),
            (LEFT, &v.left),
        ]),
        _ => {}
    }
    for (which, v) in pad {
        if let Some(i) = inset(v) {
            set_edges(&mut snap.padding, which, i);
        }
    }
    for (which, v) in mar {
        if let Some(m) = margin(v, diagnostics) {
            set_edges(&mut snap.margin, which, m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_inline_style;

    fn snap(body: &str) -> (SnapPatch, Vec<String>) {
        let mut d = Vec::new();
        let s = parse_inline_style(body, &mut d);
        (s.extras.snap, d)
    }

    #[test]
    fn snap_type_reads_axis_and_strictness() {
        let t = |body: &str| snap(body).0.snap_type;
        let mk = |x, y, mandatory| Some(SnapType { x, y, mandatory });
        assert_eq!(t("scroll-snap-type: x mandatory"), mk(true, false, true));
        assert_eq!(t("scroll-snap-type: block"), mk(false, true, false));
        assert_eq!(t("scroll-snap-type: both proximity"), mk(true, true, false));
        assert_eq!(
            t("scroll-snap-type: inline mandatory"),
            mk(true, false, true)
        );
        assert_eq!(t("scroll-snap-type: none"), Some(SnapType::NONE));
        let (s, d) = snap("scroll-snap-type: x sideways");
        assert_eq!((s.snap_type, d.len()), (None, 1));
    }

    #[test]
    fn the_two_value_align_is_block_then_inline() {
        let a = |body: &str| snap(body).0.align;
        assert_eq!(
            a("scroll-snap-align: center"),
            Some((SnapAlign::Center, SnapAlign::Center))
        );
        assert_eq!(
            a("scroll-snap-align: start end"),
            Some((SnapAlign::End, SnapAlign::Start))
        );
        assert_eq!(snap("scroll-snap-align: start end center").1.len(), 1);
        assert_eq!(snap("scroll-snap-stop: always").0.stop_always, Some(true));
    }

    #[test]
    fn padding_and_margin_merge_per_edge_and_logical_maps_to_physical() {
        let (s, d) = snap(
            "scroll-padding: 10px; scroll-padding-inline: 5%; scroll-padding-block-end: auto; \
             scroll-margin: 4px; scroll-margin-block-start: 1rem",
        );
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(s.padding_px(400.0, 100.0), [10.0, 20.0, 0.0, 20.0]);
        assert_eq!(s.margin_px(), [16.0, 4.0, 4.0, 4.0]);
        let (s, d) = snap("scroll-margin: 10%");
        assert_eq!((s.margin, d.len()), (None, 1));
    }

    #[test]
    fn a_later_rule_overrides_one_edge_and_keeps_the_rest() {
        let mut base = snap("scroll-padding: 8px").0;
        base.refine(&snap("scroll-padding-top: 20px").0);
        assert_eq!(base.padding_px(100.0, 100.0), [20.0, 8.0, 8.0, 8.0]);
    }
}
