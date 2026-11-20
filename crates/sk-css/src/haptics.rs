//! Declarative input feedback. Longhands cascade independently and repeat to the trigger list's length.
use cssparser::{ParseError, Parser, ParserInput, Token};
use std::rc::Rc;
use std::time::Duration;

type ResultAt<'i, T> = Result<T, ParseError<'i, &'static str>>;

macro_rules! keywords {
    ($name:ident { $($variant:ident => $css:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub fn from_name(s: &str) -> Option<Self> {
                $(if s.eq_ignore_ascii_case($css) { return Some(Self::$variant); })+
                None
            }
            pub fn name(self) -> &'static str {
                match self { $(Self::$variant => $css),+ }
            }
            fn parse<'i>(p: &mut Parser<'i, '_>) -> ResultAt<'i, Self> {
                let word = p.expect_ident_cloned()?;
                Self::from_name(&word).ok_or_else(|| p.new_custom_error("unknown keyword"))
            }
        }
    }
}

keywords!(Trigger { Press => "press", Activate => "activate", Change => "change", Step => "step",
    Boundary => "boundary", Snap => "snap", Hover => "hover" });
keywords!(Cue { Tick => "tick", Hint => "hint", Align => "align", Edge => "edge", Success => "success",
    Error => "error", None => "none" });
keywords!(Target { Auto => "auto", Mouse => "mouse", Pen => "pen" });
keywords!(Texture { None => "none", Ink => "ink", Pencil => "pencil", Brush => "brush", Eraser => "eraser" });

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Cue(Cue),
    Pattern(String),
}
impl Effect {
    fn parse<'i>(p: &mut Parser<'i, '_>) -> ResultAt<'i, Self> {
        if p.try_parse(|p| p.expect_function_matching("pattern"))
            .is_ok()
        {
            return p.parse_nested_block(|p| dashed(p).map(Self::Pattern));
        }
        Cue::parse(p).map(Self::Cue)
    }
    fn css(&self) -> String {
        match self {
            Self::Cue(c) => c.name().into(),
            Self::Pattern(name) => format!("pattern({name})"),
        }
    }
}
fn dashed<'i>(p: &mut Parser<'i, '_>) -> ResultAt<'i, String> {
    let name = p.expect_ident_cloned()?;
    if name.starts_with("--") && name.len() > 2 {
        Ok(name.to_string())
    } else {
        Err(p.new_custom_error("expected a --pattern name"))
    }
}
fn unit<'i>(p: &mut Parser<'i, '_>) -> ResultAt<'i, f32> {
    let n = p.expect_number()?;
    if (0.0..=1.0).contains(&n) {
        Ok(n)
    } else {
        Err(p.new_custom_error("expected a number from 0 to 1"))
    }
}
fn time<'i>(p: &mut Parser<'i, '_>) -> ResultAt<'i, Duration> {
    let seconds = match p.next()? {
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("ms") => {
            *value as f64 / 1000.
        }
        Token::Dimension { value, unit, .. } if unit.eq_ignore_ascii_case("s") => *value as f64,
        Token::Number { value, .. } if *value == 0. => 0.,
        _ => return Err(p.new_custom_error("expected a nonnegative time in ms or s")),
    };
    Duration::try_from_secs_f64(seconds).map_err(|_| p.new_custom_error("time is out of range"))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Haptics {
    pub triggers: Option<Vec<Trigger>>,
    effects: Option<Vec<Effect>>,
    intensities: Option<Vec<f32>>,
    targets: Option<Vec<Target>>,
    intervals: Option<Vec<Duration>>,
    sharpness: Option<Vec<f32>>,
    pub texture: Option<Texture>,
    pub texture_intensity: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct Binding<'a> {
    pub effect: &'a Effect,
    pub intensity: f32,
    pub target: Target,
    pub interval: Duration,
    pub sharpness: Option<f32>,
}

fn item<T>(values: &Option<Vec<T>>, i: usize) -> Option<&T> {
    let v = values.as_ref().filter(|v| !v.is_empty())?;
    Some(&v[i % v.len()])
}
fn list<T>(values: &Option<Vec<T>>, initial: &str, css: impl Fn(&T) -> String) -> String {
    match values.as_ref().filter(|v| !v.is_empty()) {
        Some(v) => v.iter().map(css).collect::<Vec<_>>().join(", "),
        None => initial.to_owned(),
    }
}

impl Haptics {
    pub fn binding(&self, trigger: Trigger) -> Option<Binding<'_>> {
        let i = self
            .triggers
            .as_ref()?
            .iter()
            .rposition(|t| *t == trigger)?;
        Some(self.at(i))
    }
    fn at(&self, i: usize) -> Binding<'_> {
        Binding {
            effect: item(&self.effects, i).unwrap_or(&Effect::Cue(Cue::None)),
            intensity: item(&self.intensities, i).copied().unwrap_or(1.),
            target: item(&self.targets, i).copied().unwrap_or(Target::Auto),
            interval: item(&self.intervals, i).copied().unwrap_or_default(),
            sharpness: item(&self.sharpness, i).copied(),
        }
    }
    pub fn texture_target(&self) -> Target {
        item(&self.targets, 0).copied().unwrap_or(Target::Auto)
    }
    pub fn css(&self, name: &str) -> Option<String> {
        Some(match name {
            "haptic" => match self.triggers.as_ref().filter(|v| !v.is_empty()) {
                Some(v) => v
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        let b = self.at(i);
                        format!("{} {} {}", t.name(), b.effect.css(), b.intensity)
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                None => "none".into(),
            },
            "haptic-trigger" => list(&self.triggers, "none", |v| v.name().into()),
            "haptic-effect" => list(&self.effects, "none", Effect::css),
            "haptic-intensity" => list(&self.intensities, "1", |v| v.to_string()),
            "haptic-target" => list(&self.targets, "auto", |v| v.name().into()),
            "haptic-min-interval" => list(&self.intervals, "0ms", |v| {
                format!("{}ms", v.as_secs_f64() * 1000.)
            }),
            "haptic-sharpness" => list(&self.sharpness, "auto", |v| v.to_string()),
            "haptic-texture" => self.texture.unwrap_or(Texture::None).name().into(),
            "haptic-texture-intensity" => self.texture_intensity.unwrap_or(1.).to_string(),
            _ => return None,
        })
    }
    pub(crate) fn refine(&mut self, other: &Self) {
        macro_rules! take { ($($field:ident),+) => { $(if other.$field.is_some() {
            self.$field.clone_from(&other.$field);
        })+ } }
        take!(
            triggers,
            effects,
            intensities,
            targets,
            intervals,
            sharpness,
            texture,
            texture_intensity
        );
    }
}

pub fn property(name: &str) -> bool {
    matches!(
        name,
        "haptic"
            | "haptic-trigger"
            | "haptic-effect"
            | "haptic-intensity"
            | "haptic-target"
            | "haptic-min-interval"
            | "haptic-sharpness"
            | "haptic-texture"
            | "haptic-texture-intensity"
    )
}

pub(crate) fn declaration(name: &str, value: &str) -> Result<Haptics, String> {
    let initial = matches!(value.to_ascii_lowercase().trim(), "initial" | "unset");
    let value = if initial {
        match name {
            "haptic-target" | "haptic-sharpness" => "auto",
            "haptic-intensity" | "haptic-texture-intensity" => "1",
            "haptic-min-interval" => "0ms",
            _ => "none",
        }
    } else {
        value
    };
    let mut input = ParserInput::new(value);
    let mut p = Parser::new(&mut input);
    let parsed = p.parse_entirely(|p| {
        let mut result = Haptics::default();
        match name {
            "haptic" => {
                let mut triggers = vec![];
                let mut effects = vec![];
                let mut intensities = vec![];
                if p.try_parse(|p| p.expect_ident_matching("none")).is_err() {
                    for (trigger, effect, intensity) in p.parse_comma_separated(|p| {
                        Ok((
                            Trigger::parse(p)?,
                            Effect::parse(p)?,
                            p.try_parse(unit).unwrap_or(1.),
                        ))
                    })? {
                        triggers.push(trigger);
                        effects.push(effect);
                        intensities.push(intensity);
                    }
                }
                result.triggers = Some(triggers);
                result.effects = Some(effects);
                result.intensities = Some(intensities);
            }
            "haptic-trigger" => {
                result.triggers = Some(
                    if p.try_parse(|p| p.expect_ident_matching("none")).is_ok() {
                        Vec::new()
                    } else {
                        p.parse_comma_separated(Trigger::parse)?
                    },
                )
            }
            "haptic-effect" => result.effects = Some(p.parse_comma_separated(Effect::parse)?),
            "haptic-intensity" => result.intensities = Some(p.parse_comma_separated(unit)?),
            "haptic-target" => result.targets = Some(p.parse_comma_separated(Target::parse)?),
            "haptic-min-interval" => result.intervals = Some(p.parse_comma_separated(time)?),
            "haptic-sharpness" => {
                result.sharpness = Some(
                    if p.try_parse(|p| p.expect_ident_matching("auto")).is_ok() {
                        Vec::new()
                    } else {
                        p.parse_comma_separated(unit)?
                    },
                )
            }
            "haptic-texture" => result.texture = Some(Texture::parse(p)?),
            "haptic-texture-intensity" => result.texture_intensity = Some(unit(p)?),
            _ => return Err(p.new_custom_error("unknown property")),
        }
        Ok(result)
    });
    parsed.map_err(|e: ParseError<'_, &str>| format!("Invalid `{name}: {value}` ({})", e.kind))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pulse {
    pub at: Duration,
    pub intensity: f32,
    pub sharpness: Option<f32>,
}
#[derive(Debug, PartialEq)]
pub struct Pattern {
    pub fallback: Cue,
    pub pulses: Vec<Pulse>,
}

impl Pattern {
    pub fn scaled_pulses(&self, gain: f32) -> impl Iterator<Item = (Duration, f32)> + '_ {
        self.pulses.iter().filter_map(move |p| {
            let strength = p.intensity * gain;
            (strength > 0.).then_some((p.at, strength))
        })
    }
}

pub(crate) fn pattern_rule(css: &str) -> Result<(String, Rc<Pattern>), String> {
    let mut input = ParserInput::new(css);
    let mut p = Parser::new(&mut input);
    p.parse_entirely(|p| {
        match p.next()? {
            Token::AtKeyword(id) if id.eq_ignore_ascii_case("haptic-pattern") => {}
            _ => return Err(p.new_custom_error("expected @haptic-pattern")),
        }
        let name = dashed(p)?;
        p.expect_curly_bracket_block()?;
        let pattern = p.parse_nested_block(|p| {
            let mut pat = Pattern {
                fallback: Cue::None,
                pulses: Vec::new(),
            };
            while !p.is_exhausted() {
                if p.try_parse(|p| p.expect_ident_matching("fallback")).is_ok() {
                    p.expect_colon()?;
                    pat.fallback = Cue::parse(p)?;
                    if !p.is_exhausted() {
                        p.expect_semicolon()?;
                    }
                    continue;
                }
                let at = time(p)?;
                p.expect_curly_bracket_block()?;
                let pulse = p.parse_nested_block(|p| {
                    let mut pulse = Pulse {
                        at,
                        intensity: 1.,
                        sharpness: None,
                    };
                    while !p.is_exhausted() {
                        let field = p.expect_ident_cloned()?;
                        p.expect_colon()?;
                        if field.eq_ignore_ascii_case("intensity") {
                            pulse.intensity = unit(p)?;
                        } else if field.eq_ignore_ascii_case("sharpness") {
                            pulse.sharpness = Some(unit(p)?);
                        } else {
                            return Err(p.new_custom_error("expected intensity or sharpness"));
                        }
                        if !p.is_exhausted() {
                            p.expect_semicolon()?;
                        }
                    }
                    Ok(pulse)
                })?;
                pat.pulses.push(pulse);
            }
            if pat.pulses.is_empty() {
                return Err(p.new_custom_error("expected at least one timed pulse"));
            }
            pat.pulses.sort_by_key(|p| p.at);
            if pat.pulses.windows(2).any(|p| p[0].at == p[1].at) {
                return Err(p.new_custom_error("duplicate pulse time"));
            }
            Ok(pat)
        })?;
        Ok((name, Rc::new(pattern)))
    })
    .map_err(|e: ParseError<'_, &str>| format!("Invalid @haptic-pattern ({})", e.kind))
}
