//! `@container` conditions and the container query length units (`cqw` … `cqmax`), answered from the
//! query containers above the element being cascaded and the content-box sizes the host's last
//! layout gave them.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use lightningcss::media_query::{
    MediaFeatureComparison, MediaFeatureName, MediaFeatureValue, Operator, QueryFeature,
};
use lightningcss::properties::Property;
use lightningcss::rules::container::{
    ContainerCondition, ContainerRule, ContainerSizeFeature, ContainerSizeFeatureId, StyleQuery,
};
use lightningcss::traits::ToCss;
use lightningcss::values::length::{Length, LengthValue};

use crate::Vars;

/// The inline (horizontal) axis: `container-type: inline-size` or `size`.
pub const INLINE: u8 = 1;
/// The block (vertical) axis: `container-type: size`.
pub const BLOCK: u8 = 2;

/// A query container among an element's ancestors: an element whose `container-type` is not
/// `normal` or that has a `container-name`. The chain runs nearest first.
#[derive(Debug)]
pub struct Container {
    pub id: u32,
    /// [`INLINE`] / [`BLOCK`]; 0 for a named style container.
    pub axes: u8,
    pub names: Option<Rc<[Rc<str>]>>,
    pub vars: Rc<Vars>,
    pub font_size: f32,
    pub parent: Chain,
}

pub type Chain = Option<Rc<Container>>;

impl Container {
    /// The chain an element's children see: `chain` with the element in front when it is a query
    /// container.
    pub fn link(
        id: u32,
        extras: &crate::Extras,
        vars: &Rc<Vars>,
        font_size: f32,
        chain: &Chain,
    ) -> Chain {
        let axes = extras.container_type.unwrap_or(0);
        let names = extras.container_name.clone().flatten();
        if axes == 0 && names.is_none() {
            return chain.clone();
        }
        Some(Rc::new(Container {
            id,
            axes,
            names,
            vars: vars.clone(),
            font_size,
            parent: chain.clone(),
        }))
    }

    fn iter(chain: &Chain) -> impl Iterator<Item = &Container> {
        std::iter::successors(chain.as_deref(), |c| c.parent.as_deref())
    }

    fn named(&self, name: &str) -> bool {
        self.names
            .as_ref()
            .is_some_and(|names| names.iter().any(|n| &**n == name))
    }
}

/// The content box `(width, height)` of a container as last laid out; `None` before its first
/// layout. The host records each question: an answer that moves must restyle what asked.
pub type SizeSource = fn(id: u32, axes: u8) -> Option<(f32, f32)>;

thread_local! {
    static SOURCE: Cell<Option<SizeSource>> = const { Cell::new(None) };
    static CHAIN: RefCell<Chain> = const { RefCell::new(None) };
    static UNITS_SEEN: Cell<bool> = const { Cell::new(false) };
}

pub fn set_size_source(source: SizeSource) {
    SOURCE.with(|s| s.set(Some(source)));
}

/// Runs one element's cascade with `chain` as its query containers.
pub fn with_chain<R>(chain: &Chain, f: impl FnOnce() -> R) -> R {
    struct Restore(Chain);
    impl Drop for Restore {
        fn drop(&mut self) {
            let outer = self.0.take();
            CHAIN.with(|c| *c.borrow_mut() = outer);
        }
    }
    let _restore = Restore(CHAIN.with(|c| c.replace(chain.clone())));
    f()
}

fn size_of(id: u32, axes: u8) -> Option<(f32, f32)> {
    SOURCE.with(Cell::get).and_then(|source| source(id, axes))
}

pub(crate) fn noting_units<R>(f: impl FnOnce() -> R) -> (R, bool) {
    let before = UNITS_SEEN.with(|c| c.replace(false));
    let out = f();
    let seen = UNITS_SEEN.with(|c| c.replace(before));
    UNITS_SEEN.with(|c| c.set(before || seen));
    (out, seen)
}

/// `cqw`, `cqh`, `cqi`, `cqb`, `cqmin`, `cqmax` in px: against the nearest container with that
/// axis, else the viewport (CSS's "small viewport" fallback, the same here).
pub(crate) fn unit_px(v: &LengthValue) -> Option<f32> {
    let (value, axes) = match *v {
        LengthValue::Cqw(p) | LengthValue::Cqi(p) => (p, INLINE),
        LengthValue::Cqh(p) | LengthValue::Cqb(p) => (p, BLOCK),
        LengthValue::Cqmin(p) | LengthValue::Cqmax(p) => (p, INLINE | BLOCK),
        _ => return None,
    };
    UNITS_SEEN.with(|c| c.set(true));
    crate::mapper::note_font_relative();
    let (vw, vh) = crate::mapper::viewport_basis();
    let along = |axis: u8| -> f32 {
        let size = CHAIN.with(|c| {
            let chain = c.borrow();
            let found = Container::iter(&chain).find(|c| c.axes & axis != 0)?;
            size_of(found.id, axis)
        });
        match (size, axis) {
            (Some((w, _)), INLINE) => w,
            (Some((_, h)), _) => h,
            (None, INLINE) => vw,
            (None, _) => vh,
        }
    };
    let basis = match (v, axes) {
        (_, INLINE) => along(INLINE),
        (_, BLOCK) => along(BLOCK),
        (LengthValue::Cqmin(_), _) => along(INLINE).min(along(BLOCK)),
        _ => along(INLINE).max(along(BLOCK)),
    };
    Some(value / 100.0 * basis)
}

pub(crate) fn unit_name_px(unit: &str, value: f32) -> Option<f32> {
    let v = match unit {
        "cqw" => LengthValue::Cqw(value),
        "cqh" => LengthValue::Cqh(value),
        "cqi" => LengthValue::Cqi(value),
        "cqb" => LengthValue::Cqb(value),
        "cqmin" => LengthValue::Cqmin(value),
        "cqmax" => LengthValue::Cqmax(value),
        _ => return None,
    };
    unit_px(&v)
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Cmp {
    fn from(op: MediaFeatureComparison) -> Cmp {
        match op {
            MediaFeatureComparison::LessThan => Cmp::Lt,
            MediaFeatureComparison::LessThanEqual => Cmp::Le,
            MediaFeatureComparison::GreaterThan => Cmp::Gt,
            MediaFeatureComparison::GreaterThanEqual => Cmp::Ge,
            MediaFeatureComparison::Equal => Cmp::Eq,
        }
    }

    fn flip(self) -> Cmp {
        match self {
            Cmp::Lt => Cmp::Gt,
            Cmp::Le => Cmp::Ge,
            Cmp::Gt => Cmp::Lt,
            Cmp::Ge => Cmp::Le,
            Cmp::Eq => Cmp::Eq,
        }
    }

    fn eval(self, lhs: f32, rhs: f32) -> bool {
        match self {
            Cmp::Lt => lhs < rhs,
            Cmp::Le => lhs <= rhs,
            Cmp::Gt => lhs > rhs,
            Cmp::Ge => lhs >= rhs,
            Cmp::Eq => (lhs - rhs).abs() < 0.01,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Len {
    Px(f32),
    /// Of the query container's own `font-size`.
    Em(f32),
}

#[derive(Clone, Debug, PartialEq)]
enum Feature {
    Width(Cmp, Len),
    Height(Cmp, Len),
    AspectRatio(Cmp, f32),
    Portrait(bool),
    /// `(width)`: true when the size is not zero.
    NonZero(u8),
}

#[derive(Clone, Debug, PartialEq)]
enum Cond {
    Always,
    Feature(Feature),
    /// `style(--name)` or `style(--name: value)`.
    Style(Box<str>, Option<Box<str>>),
    Not(Box<Cond>),
    And(Vec<Cond>),
    Or(Vec<Cond>),
}

/// One `@container` prelude.
#[derive(Clone, Debug, PartialEq)]
struct Part {
    name: Option<Box<str>>,
    axes: u8,
    cond: Cond,
}

/// The `@container` rules around a style rule, outermost first; every one must hold.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct Query {
    parts: Vec<Part>,
}

pub(crate) const SUPPORTED: &str = "supported: an optional container name, then `width`, \
     `height`, `inline-size`, `block-size`, `aspect-ratio` and `orientation` (px, rem and em \
     lengths, range and interval syntax) and `style(--custom-property: value)`, combined with \
     `and`/`or`/`not`";

type Unsupported = &'static str;

impl Query {
    pub(crate) fn nest(&self, rule: &ContainerRule<'_>) -> Result<Query, Unsupported> {
        let cond = match &rule.condition {
            Some(c) => condition(c)?,
            None => Cond::Always,
        };
        let mut parts = self.parts.clone();
        parts.push(Part {
            name: rule.name.as_ref().map(|n| n.0 .0.as_ref().into()),
            axes: axes_of(&cond),
            cond,
        });
        Ok(Query { parts })
    }

    pub(crate) fn eval(&self, parent_vars: &Rc<Vars>) -> bool {
        CHAIN.with(|c| {
            let chain = c.borrow();
            self.parts
                .iter()
                .all(|part| part.eval(&chain, parent_vars) == Some(true))
        })
    }
}

impl Part {
    /// `None` when no container answers, which CSS calls "unknown" and does not apply.
    fn eval(&self, chain: &Chain, parent_vars: &Rc<Vars>) -> Option<bool> {
        let found = Container::iter(chain).find(|c| {
            c.axes & self.axes == self.axes && self.name.as_deref().is_none_or(|n| c.named(n))
        });
        let at = match found {
            Some(c) => At {
                size: (self.axes != 0).then(|| size_of(c.id, self.axes)).flatten(),
                vars: &c.vars,
                font_size: c.font_size,
            },
            // Every element is a style container: with no name, a pure style query asks the
            // parent.
            None if self.axes == 0 && self.name.is_none() => At {
                size: None,
                vars: parent_vars,
                font_size: crate::mapper::rem_basis(),
            },
            None => return None,
        };
        at.eval(&self.cond)
    }
}

struct At<'a> {
    size: Option<(f32, f32)>,
    vars: &'a Rc<Vars>,
    font_size: f32,
}

impl At<'_> {
    fn eval(&self, cond: &Cond) -> Option<bool> {
        match cond {
            Cond::Always => Some(true),
            Cond::Feature(f) => self.feature(f),
            Cond::Style(name, value) => Some(match (self.vars.get(name), value) {
                (Some(have), Some(want)) => normalized(have) == normalized(want),
                (Some(_), None) => true,
                (None, _) => false,
            }),
            Cond::Not(c) => self.eval(c).map(|b| !b),
            Cond::And(cs) => {
                let mut unknown = false;
                for c in cs {
                    match self.eval(c) {
                        Some(false) => return Some(false),
                        None => unknown = true,
                        Some(true) => {}
                    }
                }
                (!unknown).then_some(true)
            }
            Cond::Or(cs) => {
                let mut unknown = false;
                for c in cs {
                    match self.eval(c) {
                        Some(true) => return Some(true),
                        None => unknown = true,
                        Some(false) => {}
                    }
                }
                (!unknown).then_some(false)
            }
        }
    }

    fn px(&self, len: Len) -> f32 {
        match len {
            Len::Px(px) => px,
            Len::Em(em) => em * self.font_size,
        }
    }

    fn feature(&self, f: &Feature) -> Option<bool> {
        let (w, h) = self.size?;
        Some(match *f {
            Feature::Width(cmp, len) => cmp.eval(w, self.px(len)),
            Feature::Height(cmp, len) => cmp.eval(h, self.px(len)),
            Feature::AspectRatio(cmp, ratio) => h > 0.0 && cmp.eval(w / h, ratio),
            Feature::Portrait(portrait) => (h >= w) == portrait,
            Feature::NonZero(axes) => {
                (axes & INLINE == 0 || w != 0.0) && (axes & BLOCK == 0 || h != 0.0)
            }
        })
    }
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn axes_of(cond: &Cond) -> u8 {
    match cond {
        Cond::Always | Cond::Style(..) => 0,
        Cond::Feature(Feature::Width(..)) => INLINE,
        Cond::Feature(Feature::Height(..)) => BLOCK,
        Cond::Feature(Feature::NonZero(axes)) => *axes,
        Cond::Feature(Feature::AspectRatio(..) | Feature::Portrait(_)) => INLINE | BLOCK,
        Cond::Not(c) => axes_of(c),
        Cond::And(cs) | Cond::Or(cs) => cs.iter().fold(0, |axes, c| axes | axes_of(c)),
    }
}

fn condition(c: &ContainerCondition<'_>) -> Result<Cond, Unsupported> {
    Ok(match c {
        ContainerCondition::Feature(f) => feature(f)?,
        ContainerCondition::Not(inner) => Cond::Not(Box::new(condition(inner)?)),
        ContainerCondition::Operation {
            operator,
            conditions,
        } => {
            let parsed = conditions
                .iter()
                .map(condition)
                .collect::<Result<Vec<_>, _>>()?;
            match operator {
                Operator::And => Cond::And(parsed),
                Operator::Or => Cond::Or(parsed),
            }
        }
        ContainerCondition::Style(q) => style(q)?,
        ContainerCondition::ScrollState(_) => return Err("a scroll-state query"),
        ContainerCondition::Unknown(_) => {
            return Err("tokens the container query parser could not interpret")
        }
    })
}

fn style(q: &StyleQuery<'_>) -> Result<Cond, Unsupported> {
    use lightningcss::properties::custom::CustomPropertyName;
    const STANDARD: &str = "a style query on a standard property (only custom properties are)";
    Ok(match q {
        StyleQuery::Declaration(p @ Property::Custom(c)) => match &c.name {
            CustomPropertyName::Custom(name) => {
                let text = p
                    .to_css_string(false, Default::default())
                    .map_err(|_| "a style query value that cannot be read")?;
                let value = text.split_once(':').map_or("", |(_, v)| v.trim());
                Cond::Style(name.0.as_ref().into(), Some(value.into()))
            }
            CustomPropertyName::Unknown(_) => return Err(STANDARD),
        },
        StyleQuery::Property(id) if id.name().starts_with("--") => {
            Cond::Style(id.name().into(), None)
        }
        StyleQuery::Declaration(_) | StyleQuery::Property(_) => return Err(STANDARD),
        StyleQuery::Not(inner) => Cond::Not(Box::new(style(inner)?)),
        StyleQuery::Operation {
            operator,
            conditions,
        } => {
            let parsed = conditions
                .iter()
                .map(style)
                .collect::<Result<Vec<_>, _>>()?;
            match operator {
                Operator::And => Cond::And(parsed),
                Operator::Or => Cond::Or(parsed),
            }
        }
    })
}

fn length(value: &MediaFeatureValue<'_>) -> Result<Len, Unsupported> {
    const NOT_A_LENGTH: &str =
        "a container length that is not px, rem or em (`%`, `calc()`, viewport units)";
    match value {
        MediaFeatureValue::Length(Length::Value(v)) => match v {
            LengthValue::Em(em) => Ok(Len::Em(*em)),
            LengthValue::Rem(rem) => Ok(Len::Px(rem * crate::mapper::rem_basis())),
            other => other.to_px().map(Len::Px).ok_or(NOT_A_LENGTH),
        },
        MediaFeatureValue::Length(_) => Err(NOT_A_LENGTH),
        MediaFeatureValue::Integer(0) => Ok(Len::Px(0.0)),
        MediaFeatureValue::Number(n) if *n == 0.0 => Ok(Len::Px(0.0)),
        _ => Err("a container feature value of the wrong type"),
    }
}

fn ratio(value: &MediaFeatureValue<'_>) -> Result<f32, Unsupported> {
    match value {
        MediaFeatureValue::Ratio(r) if r.1 > 0.0 => Ok(r.0 / r.1),
        MediaFeatureValue::Number(n) => Ok(*n),
        MediaFeatureValue::Integer(i) => Ok(*i as f32),
        _ => Err("an `aspect-ratio` that is not a ratio"),
    }
}

fn standard(
    name: &MediaFeatureName<'_, ContainerSizeFeatureId>,
) -> Result<ContainerSizeFeatureId, Unsupported> {
    match name {
        MediaFeatureName::Standard(id) => Ok(*id),
        MediaFeatureName::Custom(_) => Err("a custom feature (`--name`)"),
        MediaFeatureName::Unknown(_) => Err("an unknown container feature"),
    }
}

fn ranged(
    id: ContainerSizeFeatureId,
    cmp: Cmp,
    value: &MediaFeatureValue<'_>,
) -> Result<Feature, Unsupported> {
    use ContainerSizeFeatureId as Id;
    Ok(match id {
        Id::Width | Id::InlineSize => Feature::Width(cmp, length(value)?),
        Id::Height | Id::BlockSize => Feature::Height(cmp, length(value)?),
        Id::AspectRatio => Feature::AspectRatio(cmp, ratio(value)?),
        Id::Orientation => return Err("a range comparison on `orientation`"),
    })
}

fn feature(f: &ContainerSizeFeature<'_>) -> Result<Cond, Unsupported> {
    use ContainerSizeFeatureId as Id;
    Ok(Cond::Feature(match f {
        QueryFeature::Plain { name, value } => match standard(name)? {
            Id::Orientation => match value {
                MediaFeatureValue::Ident(i) if i.0.eq_ignore_ascii_case("portrait") => {
                    Feature::Portrait(true)
                }
                MediaFeatureValue::Ident(i) if i.0.eq_ignore_ascii_case("landscape") => {
                    Feature::Portrait(false)
                }
                _ => return Err("an unknown `orientation` value"),
            },
            id => ranged(id, Cmp::Eq, value)?,
        },
        QueryFeature::Boolean { name } => Feature::NonZero(match standard(name)? {
            Id::Width | Id::InlineSize => INLINE,
            Id::Height | Id::BlockSize => BLOCK,
            Id::AspectRatio | Id::Orientation => INLINE | BLOCK,
        }),
        QueryFeature::Range {
            name,
            operator,
            value,
        } => ranged(standard(name)?, Cmp::from(*operator), value)?,
        QueryFeature::Interval {
            name,
            start,
            start_operator,
            end,
            end_operator,
        } => {
            let id = standard(name)?;
            return Ok(Cond::And(vec![
                Cond::Feature(ranged(id, Cmp::from(*start_operator).flip(), start)?),
                Cond::Feature(ranged(id, Cmp::from(*end_operator), end)?),
            ]));
        }
    }))
}

pub(crate) fn prelude_text(rule: &ContainerRule<'_>) -> String {
    let name = rule.name.as_ref().map(|n| n.0 .0.to_string());
    let cond = rule
        .condition
        .as_ref()
        .and_then(|c| c.to_css_string(Default::default()).ok());
    [name, cond]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
}
