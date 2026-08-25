//! `--shading: <effect>(<args>)` — the CSS surface of the effect system.
//!
//! ```css
//! .glass  { --shading: frost(0.6, 14px, #7fd4ff28); }
//! .grain  { --shading: noise(0.3, 2px, 12); }
//! .halo   { --shading: glow(#7fd4ff, 24px, 0.9); }
//! .plain  { --shading: none; }
//! ```
//!
//! lightningcss hands an unknown-but-valid custom property through as
//! `Property::Custom(CustomProperty { name, value: TokenList })` — a raw token
//! list, because a browser stores custom properties without interpreting them.
//! This module is the interpreter: token list in, [`EffectSpec`] out.
//!
//! # Compat notes (house policy: never diverge silently)
//!
//! * `--shading` is a **valid** custom property, so a browser parses it, stores
//!   it and renders nothing. The same `.vue` file therefore renders sanely in
//!   both places — that is why the syntax is a custom property rather than a
//!   `-vn-` prefix, which browsers *drop*.
//! * The one divergence: browsers **inherit** custom properties; vue-native
//!   applies `--shading` to the declaring element only. See
//!   `crates/vn-effects/src/lib.rs`.
//! * Every degradation is an advisory: an unknown effect name, a wrong
//!   argument type, a missing required argument, an out-of-range value, a
//!   `var()` reference, or a second effect function. Nothing is ever dropped in
//!   silence.
//! * `--shading: none` clears, and is a VALUE ([`EffectSpec::NONE`]) rather
//!   than an absent field, because a refinement cannot spell "explicitly
//!   nothing" and that is what has to cancel an effect set by an earlier rule.

use gpui::EffectSpec;
use lightningcss::properties::custom::{Token, TokenList, TokenOrValue};
use lightningcss::values::color::CssColor;
use vn_effects::{EffectSchema, ParamKind, ParamSchema};

use crate::mapper::once;

/// The custom property this module owns.
pub const SHADING: &str = "--shading";

/// One argument, already reduced to the kind of value it is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Arg {
    Number(f32),
    Length(f32),
    Color([f32; 4]),
}

impl Arg {
    fn matches(self, kind: ParamKind) -> bool {
        match (self, kind) {
            (Arg::Number(_), ParamKind::Number) => true,
            (Arg::Length(_), ParamKind::Length) => true,
            (Arg::Color(_), ParamKind::Color) => true,
            // CSS lets a length be spelled as a unitless zero.
            (Arg::Number(n), ParamKind::Length) => n == 0.0,
            _ => false,
        }
    }

    fn kind_name(self) -> &'static str {
        match self {
            Arg::Number(_) => "<number>",
            Arg::Length(_) => "<length>",
            Arg::Color(_) => "<color>",
        }
    }
}

/// Parses a `--shading` declaration.
///
/// Returns `None` when nothing usable was found — the caller leaves the
/// refinement untouched — and always leaves an advisory behind when it does.
pub fn parse(value: &TokenList, diagnostics: &mut Vec<String>) -> Option<EffectSpec> {
    let significant: Vec<&TokenOrValue> = value
        .0
        .iter()
        .filter(|t| !t.is_whitespace() && !matches!(t, TokenOrValue::Token(Token::Comment(_))))
        .collect();

    match significant.as_slice() {
        [] => {
            advise(diagnostics, "`--shading` is empty — expected `none` or an effect function");
            None
        }
        // `--shading: none` — clears an effect declared earlier in the cascade.
        [TokenOrValue::Token(Token::Ident(ident))] if ident.eq_ignore_ascii_case("none") => {
            Some(EffectSpec::NONE)
        }
        _ => {
            let functions: Vec<_> = significant
                .iter()
                .filter_map(|t| match t {
                    TokenOrValue::Function(f) => Some(f),
                    _ => None,
                })
                .collect();

            if functions.is_empty() {
                // A `var()` reference is the overwhelmingly likely case, and it
                // is blocked on the same thing every other `var()` is.
                if significant
                    .iter()
                    .any(|t| matches!(t, TokenOrValue::Var(_) | TokenOrValue::Env(_)))
                {
                    advise(
                        diagnostics,
                        "`--shading` cannot use var()/env() yet (v1) — custom property \
                         resolution is not implemented, so the effect is skipped",
                    );
                } else {
                    advise(
                        diagnostics,
                        &format!(
                            "`--shading` expects `none` or one of {} — got something else",
                            known_effects()
                        ),
                    );
                }
                return None;
            }

            if functions.len() > 1 {
                advise(
                    diagnostics,
                    &format!(
                        "`--shading`: only one effect function is applied (v1) — kept `{}()`, \
                         dropped {}",
                        functions[0].name.0.as_ref(),
                        functions[1..]
                            .iter()
                            .map(|f| format!("`{}()`", f.name.0.as_ref()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                );
            }

            let function = functions[0];
            let name = function.name.0.as_ref();
            let Some(schema) = vn_effects::lookup(name) else {
                advise(
                    diagnostics,
                    &format!(
                        "`--shading: {name}(…)` is not a built-in effect (v1) — available: {}",
                        known_effects()
                    ),
                );
                return None;
            };

            build(schema, &function.arguments, diagnostics)
        }
    }
}

/// Splits the argument list on commas and matches it against the schema.
///
/// Matching is POSITIONAL but type-directed: an argument that does not fit the
/// next OPTIONAL parameter skips it rather than failing, which is what makes
/// `glow(12px, 0.8)` and `glow(#7fd4ff, 12px, 0.8)` both parse against a schema
/// whose first parameter is the colour. `vn_effects`' own tests enforce the
/// precondition that makes this unambiguous (an optional parameter never has
/// the same kind as the one after it).
fn build(
    schema: &'static EffectSchema,
    arguments: &TokenList,
    diagnostics: &mut Vec<String>,
) -> Option<EffectSpec> {
    let mut args = Vec::new();
    for group in split_on_commas(arguments) {
        match argument(&group) {
            Ok(arg) => args.push(arg),
            Err(reason) => {
                advise(
                    diagnostics,
                    &format!("`--shading: {}(…)`: {reason} — expected {}", schema.name, schema.signature),
                );
                return None;
            }
        }
    }

    let mut spec = schema.defaults();
    let mut next = 0usize;
    for param in schema.params {
        let Some(arg) = args.get(next).copied() else {
            if param.required {
                advise(
                    diagnostics,
                    &format!(
                        "`--shading: {}(…)` is missing the required `{}` argument — expected {}",
                        schema.name, param.name, schema.signature
                    ),
                );
                return None;
            }
            continue;
        };

        if !arg.matches(param.kind) {
            if !param.required {
                // Skip the optional parameter, keep its default, retry this
                // same argument against the next parameter.
                continue;
            }
            advise(
                diagnostics,
                &format!(
                    "`--shading: {}(…)`: argument {} (`{}`) must be {}, got {} — expected {}",
                    schema.name,
                    next + 1,
                    param.name,
                    param.kind.name(),
                    arg.kind_name(),
                    schema.signature
                ),
            );
            return None;
        }

        write(&mut spec.params, param, arg, schema, diagnostics);
        next += 1;
    }

    if next < args.len() {
        advise(
            diagnostics,
            &format!(
                "`--shading: {}(…)` takes at most {} arguments, got {} — extra ones ignored, \
                 expected {}",
                schema.name,
                schema.params.len(),
                args.len(),
                schema.signature
            ),
        );
    }

    spec.animated = schema.is_animated(&spec.params);
    Some(spec)
}

/// Writes one matched argument into its slots, clamping to the declared range
/// with an advisory. Out-of-range is CLAMPED, not dropped: a browser would
/// clamp a used value too, and dropping the whole declaration over a `1.4`
/// would be a far bigger surprise than saturating it.
fn write(
    params: &mut [f32; 8],
    param: &ParamSchema,
    arg: Arg,
    schema: &EffectSchema,
    diagnostics: &mut Vec<String>,
) {
    let slot = param.slot as usize;
    match arg {
        Arg::Color(rgba) => {
            for (offset, channel) in rgba.iter().enumerate() {
                if let Some(target) = params.get_mut(slot + offset) {
                    *target = *channel;
                }
            }
        }
        Arg::Number(value) | Arg::Length(value) => {
            let clamped = match param.range {
                Some((min, max)) if value < min || value > max => {
                    advise(
                        diagnostics,
                        &format!(
                            "`--shading: {}(…)`: `{}` is {value}, outside {min}..{max} — clamped",
                            schema.name, param.name
                        ),
                    );
                    value.clamp(min, max)
                }
                _ => value,
            };
            if let Some(target) = params.get_mut(slot) {
                *target = clamped;
            }
        }
    }
}

/// Splits a function's argument list on top-level commas.
fn split_on_commas<'a, 'i>(arguments: &'a TokenList<'i>) -> Vec<Vec<&'a TokenOrValue<'i>>> {
    let mut groups = Vec::new();
    let mut current: Vec<&TokenOrValue> = Vec::new();
    for token in &arguments.0 {
        match token {
            TokenOrValue::Token(Token::Comma) => {
                groups.push(std::mem::take(&mut current));
            }
            t if t.is_whitespace() || matches!(t, TokenOrValue::Token(Token::Comment(_))) => {}
            t => current.push(t),
        }
    }
    if !current.is_empty() || !groups.is_empty() {
        groups.push(current);
    }
    groups
}

/// Reduces one comma-separated group to a single typed value.
fn argument(group: &[&TokenOrValue]) -> Result<Arg, String> {
    let [token] = group else {
        return Err(if group.is_empty() {
            "empty argument".to_string()
        } else {
            format!("an argument must be a single value, got {} tokens", group.len())
        });
    };
    match token {
        TokenOrValue::Color(color) => rgba(color)
            .map(Arg::Color)
            .ok_or_else(|| "a colour that does not resolve to concrete channels".to_string()),
        // `currentColor`, `color-mix()`, system colours — the same gap every
        // other colour-taking property has.
        TokenOrValue::UnresolvedColor(_) => Err(
            "a colour that does not resolve to concrete channels (currentColor / color-mix() \
             are not computed in v1)"
                .to_string(),
        ),
        TokenOrValue::Length(length) => length
            .to_px()
            .map(Arg::Length)
            .ok_or_else(|| "a length in a relative unit (em/rem/vw/… are not resolved in v1)".to_string()),
        TokenOrValue::Var(_) | TokenOrValue::Env(_) => Err(
            "a var()/env() reference (custom property resolution is not implemented in v1)"
                .to_string(),
        ),
        TokenOrValue::Token(Token::Number { value, .. }) => Ok(Arg::Number(*value)),
        // A percentage is the natural spelling of a 0..1 knob: `frost(60%)`.
        TokenOrValue::Token(Token::Percentage { unit_value, .. }) => Ok(Arg::Number(*unit_value)),
        TokenOrValue::Token(Token::Dimension { value, unit, .. }) => {
            match unit.as_ref().to_ascii_lowercase().as_str() {
                "px" => Ok(Arg::Length(*value)),
                other => Err(format!("a length in `{other}` (only absolute units are resolved in v1)")),
            }
        }
        other => Err(format!("an unsupported value ({other:?})")),
    }
}

/// A concrete straight-alpha RGBA quadruple, or `None` for anything that does
/// not resolve without inheritance or a colour-space computation.
fn rgba(color: &CssColor) -> Option<[f32; 4]> {
    match color {
        CssColor::RGBA(c) => Some([
            c.red as f32 / 255.0,
            c.green as f32 / 255.0,
            c.blue as f32 / 255.0,
            c.alpha_f32(),
        ]),
        _ => None,
    }
}

fn known_effects() -> String {
    vn_effects::names()
        .map(|n| format!("`{n}()`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn advise(diagnostics: &mut Vec<String>, message: &str) {
    once(diagnostics, message.to_string());
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::effect_id;
    use lightningcss::declaration::DeclarationBlock;
    use lightningcss::properties::custom::{CustomPropertyName, CustomProperty};
    use lightningcss::properties::Property;
    use lightningcss::stylesheet::ParserOptions;

    /// Runs a `--shading` declaration through lightningcss and this parser,
    /// exactly as the mapper does.
    fn shading(body: &str) -> (Option<EffectSpec>, Vec<String>) {
        let mut diagnostics = Vec::new();
        let block = DeclarationBlock::parse_string(body, ParserOptions::default())
            .expect("declaration block parses");
        for property in block.declarations.iter() {
            if let Property::Custom(CustomProperty {
                name: CustomPropertyName::Custom(name),
                value,
            }) = property
            {
                if name.0.as_ref() == SHADING {
                    return (parse(value, &mut diagnostics), diagnostics);
                }
            }
        }
        panic!("no --shading declaration in {body:?}");
    }

    fn spec(body: &str) -> EffectSpec {
        let (spec, diagnostics) = shading(body);
        assert!(diagnostics.is_empty(), "unexpected advisories: {diagnostics:?}");
        spec.expect("parsed")
    }

    #[test]
    fn frost_with_every_argument() {
        let s = spec("--shading: frost(0.6, 14px, rgba(127, 212, 255, 0.25))");
        assert_eq!(s.id, effect_id::FROST);
        assert_eq!(s.params[0], 0.6);
        assert_eq!(s.params[1], 14.0);
        assert!((s.params[4] - 127.0 / 255.0).abs() < 1e-4);
        // lightningcss stores alpha as a u8, so 0.25 round-trips as 64/255.
        assert!((s.params[7] - 0.25).abs() < 2e-3, "{}", s.params[7]);
        assert_eq!(s.backdrop_blur_radius(), 14.0);
        assert_eq!(s.flags, gpui::EFFECT_FLAG_NEEDS_BACKDROP);
        assert!(!s.animated);
    }

    #[test]
    fn omitted_optional_arguments_keep_their_defaults() {
        let s = spec("--shading: frost(0.6)");
        assert_eq!(s.params[0], 0.6);
        assert_eq!(s.params[1], 12.0);
        assert_eq!(&s.params[4..8], &[1.0, 1.0, 1.0, 0.06]);
    }

    #[test]
    fn percentages_are_numbers() {
        assert_eq!(spec("--shading: frost(60%)").params[0], 0.6);
    }

    /// The type-directed half of the matcher: `glow`'s first parameter is an
    /// OPTIONAL colour, so both arities have to land in the right slots.
    #[test]
    fn glow_skips_its_optional_leading_color() {
        let with = spec("--shading: glow(#7fd4ff, 24px, 0.9)");
        assert_eq!(with.id, effect_id::GLOW);
        assert_eq!(with.params[0], 24.0);
        assert_eq!(with.params[1], 0.9);
        assert!((with.params[4] - 127.0 / 255.0).abs() < 1e-3);
        assert_eq!(with.params[7], 1.0);
        // `radius` is also the quad dilation, so the halo has room to land.
        assert_eq!(with.bleed(), 24.0);

        let without = spec("--shading: glow(24px, 0.9)");
        assert_eq!(without.params[0], 24.0);
        assert_eq!(without.params[1], 0.9);
        assert_eq!(&without.params[4..8], &[1.0, 1.0, 1.0, 1.0]);
    }

    #[test]
    fn noise_speed_marks_the_effect_animated() {
        assert!(!spec("--shading: noise(0.3)").animated);
        assert!(!spec("--shading: noise(0.3, 2px, 0)").animated);
        let animated = spec("--shading: noise(0.3, 2px, 24)");
        assert!(animated.animated);
        assert_eq!(animated.params[2], 24.0);
        assert_eq!(animated.flags, 0);
    }

    #[test]
    fn none_clears() {
        let s = spec("--shading: none");
        assert!(!s.is_some());
        assert_eq!(s, EffectSpec::NONE);
        assert_eq!(spec("--shading: NONE"), EffectSpec::NONE);
    }

    #[test]
    fn unitless_zero_is_a_valid_length() {
        assert_eq!(spec("--shading: glow(0, 0.5)").params[0], 0.0);
    }

    // -- advisories --------------------------------------------------------

    #[test]
    fn unknown_effect_names_the_available_ones() {
        let (s, diagnostics) = shading("--shading: plasma(1)");
        assert!(s.is_none());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].contains("plasma"), "{diagnostics:?}");
        assert!(diagnostics[0].contains("frost"), "{diagnostics:?}");
    }

    #[test]
    fn a_missing_required_argument_names_the_signature() {
        let (s, diagnostics) = shading("--shading: glow(24px)");
        assert!(s.is_none());
        assert!(diagnostics[0].contains("strength"), "{diagnostics:?}");
        assert!(diagnostics[0].contains("glow(<color>?"), "{diagnostics:?}");
    }

    #[test]
    fn a_wrong_argument_type_is_reported_not_coerced() {
        let (s, diagnostics) = shading("--shading: frost(#ff0000)");
        assert!(s.is_none());
        assert!(diagnostics[0].contains("strength"), "{diagnostics:?}");
        assert!(diagnostics[0].contains("<number>"), "{diagnostics:?}");
    }

    #[test]
    fn out_of_range_values_are_clamped_with_an_advisory() {
        let (s, diagnostics) = shading("--shading: frost(1.4)");
        assert_eq!(s.unwrap().params[0], 1.0);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert!(diagnostics[0].contains("clamped"), "{diagnostics:?}");
    }

    #[test]
    fn extra_arguments_are_ignored_with_an_advisory() {
        let (s, diagnostics) = shading("--shading: noise(0.3, 2px, 10, 4, 5)");
        assert!(s.is_some());
        assert!(diagnostics[0].contains("at most"), "{diagnostics:?}");
    }

    #[test]
    fn a_second_effect_function_is_dropped_with_an_advisory() {
        let (s, diagnostics) = shading("--shading: frost(0.5) noise(0.2)");
        assert_eq!(s.unwrap().id, effect_id::FROST);
        assert!(diagnostics[0].contains("only one effect function"), "{diagnostics:?}");
        assert!(diagnostics[0].contains("noise"), "{diagnostics:?}");
    }

    #[test]
    fn var_references_advise_rather_than_silently_doing_nothing() {
        let (s, diagnostics) = shading("--shading: var(--panel-shading)");
        assert!(s.is_none());
        assert!(diagnostics[0].contains("var()"), "{diagnostics:?}");
    }

    #[test]
    fn a_relative_length_advises() {
        let (s, diagnostics) = shading("--shading: glow(2em, 0.5)");
        assert!(s.is_none());
        assert!(diagnostics[0].contains("em"), "{diagnostics:?}");
    }

    #[test]
    fn advisories_are_one_shot() {
        let mut diagnostics = Vec::new();
        for _ in 0..3 {
            let block = DeclarationBlock::parse_string(
                "--shading: plasma(1)",
                ParserOptions::default(),
            )
            .unwrap();
            for property in block.declarations.iter() {
                if let Property::Custom(CustomProperty { value, .. }) = property {
                    parse(value, &mut diagnostics);
                }
            }
        }
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }
}
