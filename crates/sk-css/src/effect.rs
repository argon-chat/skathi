//! Interprets the `--shading` custom property (token list in, `EffectSpec` out). It is a custom
//! property so a browser stores and ignores it instead of dropping it; unlike browsers, it applies
//! to the declaring element only. `none` is a value so it can cancel an earlier rule, and every
//! degradation is an advisory. The effect schema lives in `sk-effects`; this module knows only the
//! grammar.

use gpui::EffectSpec;
use lightningcss::properties::custom::{Token, TokenList, TokenOrValue};
use lightningcss::values::color::CssColor;
use sk_effects::{EffectSchema, ParamKind, ParamSchema};

use crate::mapper::once;

pub const SHADING: &str = "--shading";

#[derive(Clone, Copy, Debug, PartialEq)]
enum Arg {
    Number(f32),
    Length(f32),
    Color([f32; 4]),
    /// `url(...)`, as `sk_effects::texture_source` numbered it.
    Texture(u32),
}

impl Arg {
    fn matches(self, kind: ParamKind) -> bool {
        match (self, kind) {
            (Arg::Number(_), ParamKind::Number) => true,
            (Arg::Length(_), ParamKind::Length) => true,
            (Arg::Color(_), ParamKind::Color) => true,
            (Arg::Texture(_), ParamKind::Texture) => true,
            (Arg::Number(n), ParamKind::Length) => n == 0.0,
            _ => false,
        }
    }

    fn kind_name(self) -> &'static str {
        match self {
            Arg::Number(_) => "<number>",
            Arg::Length(_) => "<length>",
            Arg::Color(_) => "<color>",
            Arg::Texture(_) => "url()",
        }
    }
}

pub fn parse(value: &TokenList, diagnostics: &mut Vec<String>) -> Option<EffectSpec> {
    let significant: Vec<&TokenOrValue> = value
        .0
        .iter()
        .filter(|t| !t.is_whitespace() && !matches!(t, TokenOrValue::Token(Token::Comment(_))))
        .collect();

    match significant.as_slice() {
        [] => {
            advise(
                diagnostics,
                "`--shading` is empty — expected `none` or an effect function",
            );
            None
        }
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
            let Some(schema) = sk_effects::lookup(name) else {
                // The registry is built-ins PLUS the app's own, so "unknown" here can mean either
                // "typo" or "this effect's shader did not make it into the app".
                advise(
                    diagnostics,
                    &format!(
                        "`--shading: {name}(…)` is not a known effect — available: {}. An app's \
                         own effect is a `shaders/<name>.slang` declaring [SkEffect(\"{name}\")]; \
                         sk build and sk dev compile it into the app.",
                        known_effects(),
                    ),
                );
                return None;
            };

            build(schema, &function.arguments, diagnostics)
        }
    }
}

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
                    &format!(
                        "`--shading: {}(…)`: {reason} — expected {}",
                        schema.name, schema.signature
                    ),
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

        write(&mut spec, param, arg, schema, diagnostics);
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

/// Out-of-range is CLAMPED, not dropped: a browser would clamp a used value too, and dropping the
/// whole declaration over a `1.4` would be a far bigger surprise than saturating it.
fn write(
    spec: &mut EffectSpec,
    param: &ParamSchema,
    arg: Arg,
    schema: &EffectSchema,
    diagnostics: &mut Vec<String>,
) {
    let params = &mut spec.params;
    let slot = param.slot as usize;
    match arg {
        Arg::Texture(source) => {
            let texture = param
                .texture
                .and_then(|(index, _)| spec.textures.get_mut(index as usize)?.as_mut());
            if let Some(texture) = texture {
                texture.source = source;
            }
        }
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

fn argument(group: &[&TokenOrValue]) -> Result<Arg, String> {
    let [token] = group else {
        return Err(if group.is_empty() {
            "empty argument".to_string()
        } else {
            format!(
                "an argument must be a single value, got {} tokens",
                group.len()
            )
        });
    };
    match token {
        TokenOrValue::Color(color) => rgba(color)
            .map(Arg::Color)
            .ok_or_else(|| "a colour that does not resolve to concrete channels".to_string()),
        TokenOrValue::UnresolvedColor(_) => Err(
            "a colour that does not resolve to concrete channels (currentColor / color-mix() \
             are not computed in v1)"
                .to_string(),
        ),
        TokenOrValue::Length(length) => length.to_px().map(Arg::Length).ok_or_else(|| {
            "a length in a relative unit (em/rem/vw/… are not resolved in v1)".to_string()
        }),
        TokenOrValue::Url(url) if url.url.trim().is_empty() => Err("an empty url()".to_string()),
        TokenOrValue::Url(url) => Ok(Arg::Texture(sk_effects::texture_source(url.url.trim()))),
        TokenOrValue::Var(_) | TokenOrValue::Env(_) => Err(
            "a var()/env() reference (custom property resolution is not implemented in v1)"
                .to_string(),
        ),
        TokenOrValue::Token(Token::Number { value, .. }) => Ok(Arg::Number(*value)),
        TokenOrValue::Token(Token::Percentage { unit_value, .. }) => Ok(Arg::Number(*unit_value)),
        TokenOrValue::Token(Token::Dimension { value, unit, .. }) => {
            match unit.as_ref().to_ascii_lowercase().as_str() {
                "px" => Ok(Arg::Length(*value)),
                other => Err(format!(
                    "a length in `{other}` (only absolute units are resolved in v1)"
                )),
            }
        }
        other => Err(format!("an unsupported value ({other:?})")),
    }
}

fn rgba(color: &CssColor) -> Option<[f32; 4]> {
    super::srgb(color)
}

fn known_effects() -> String {
    sk_effects::names()
        .map(|n| format!("`{n}()`"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn advise(diagnostics: &mut Vec<String>, message: &str) {
    once(diagnostics, message.to_string());
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gpui::effect_id;
    use lightningcss::declaration::DeclarationBlock;
    use lightningcss::properties::custom::{CustomProperty, CustomPropertyName};
    use lightningcss::properties::Property;
    use lightningcss::stylesheet::ParserOptions;

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
        assert!(
            diagnostics.is_empty(),
            "unexpected advisories: {diagnostics:?}"
        );
        spec.expect("parsed")
    }

    #[test]
    fn frost_with_every_argument() {
        let s = spec("--shading: frost(0.6, 14px, rgba(127, 212, 255, 0.25))");
        assert_eq!(s.id, effect_id::FROST);
        assert_eq!(s.params[0], 0.6);
        assert_eq!(s.params[1], 14.0);
        assert!((s.params[4] - 127.0 / 255.0).abs() < 1e-4);
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

    #[test]
    fn glow_skips_its_optional_leading_color() {
        let with = spec("--shading: glow(#7fd4ff, 24px, 0.9)");
        assert_eq!(with.id, effect_id::GLOW);
        assert_eq!(with.params[0], 24.0);
        assert_eq!(with.params[1], 0.9);
        assert!((with.params[4] - 127.0 / 255.0).abs() < 1e-3);
        assert_eq!(with.params[7], 1.0);
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

    const USER_MANIFEST: &str = r#"{
      "schema": 10,
      "kind": "user",
      "effects": [
        {
          "name": "ripple",
          "backdrop": false,
          "signature": "ripple(<number:speed>, <length:wavelength>?, <color:tint>?)",
          "backdropRadiusParam": null,
          "bleedParam": null,
          "animatedParam": 0,
          "params": [
            { "name": "speed", "kind": "number", "slot": 0, "required": true,
              "default": [1.5, 0, 0, 0], "range": [0, 8] },
            { "name": "wavelength", "kind": "length", "slot": 1, "required": false,
              "default": [24, 0, 0, 0], "range": [1, 512] },
            { "name": "tint", "kind": "color", "slot": 4, "required": false,
              "default": [0.5, 0.8, 1, 1], "range": null }
          ]
        },
        {
          "name": "aurora",
          "backdrop": true,
          "signature": "aurora(<number:intensity>, <length:blur>?)",
          "backdropRadiusParam": 1,
          "bleedParam": null,
          "animatedParam": null,
          "params": [
            { "name": "intensity", "kind": "number", "slot": 0, "required": true,
              "default": [0.6, 0, 0, 0], "range": [0, 1] },
            { "name": "blur", "kind": "length", "slot": 1, "required": false,
              "default": [8, 0, 0, 0], "range": [0, 25] }
          ]
        },
        {
          "name": "grade",
          "backdrop": true,
          "signature": "grade(<texture:lut>, <number:amount>?, <texture:normals>?)",
          "backdropRadiusParam": null,
          "bleedParam": null,
          "animatedParam": null,
          "params": [
            { "name": "lut", "kind": "texture", "slot": 0, "required": true,
              "default": [0, 0, 0, 0], "range": null, "texture": 0, "linear": true },
            { "name": "amount", "kind": "number", "slot": 2, "required": false,
              "default": [1, 0, 0, 0], "range": [0, 1] },
            { "name": "normals", "kind": "texture", "slot": 4, "required": false,
              "default": [0.5, 0.5, 1, 1], "range": null, "texture": 1, "linear": false }
          ]
        }
      ]
    }"#;

    pub(crate) fn with_user_effects() {
        static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        ONCE.get_or_init(|| {
            let user =
                sk_effects::parse_manifest(USER_MANIFEST, gpui::effect_id::COUNT, false, "<test>")
                    .expect("fixture manifest parses");
            sk_effects::install_user_effects(user);
        });
    }

    #[test]
    fn an_app_effect_parses_from_its_reflected_schema() {
        with_user_effects();
        let s = spec("--shading: ripple(2.5, 40px, #ff8800)");
        assert_eq!(s.id, effect_id::COUNT);
        assert_eq!(s.params[0], 2.5);
        assert_eq!(s.params[1], 40.0);
        assert_eq!(s.params[4], 1.0);
        assert!((s.params[5] - 136.0 / 255.0).abs() < 1e-3);
        assert_eq!(s.params[6], 0.0);
        assert_eq!(s.length_mask, 1 << 1);
        assert!(s.animated);
        assert_eq!(s.flags, 0);
    }

    #[test]
    fn an_app_effects_optional_arguments_default_from_the_shader() {
        with_user_effects();
        let s = spec("--shading: ripple(0)");
        assert_eq!(s.params[0], 0.0);
        assert_eq!(s.params[1], 24.0);
        assert_eq!(&s.params[4..8], &[0.5, 0.8, 1.0, 1.0]);
        assert!(!s.animated, "speed 0 must not pin the window at 60 fps");
    }

    #[test]
    fn an_app_effect_can_ask_for_the_backdrop() {
        with_user_effects();
        let s = spec("--shading: aurora(0.8, 20px)");
        assert_eq!(s.id, effect_id::COUNT + 1);
        assert_eq!(s.flags, gpui::EFFECT_FLAG_NEEDS_BACKDROP);
        assert_eq!(s.backdrop_blur_radius(), 20.0);
    }

    #[test]
    fn a_url_argument_names_the_image_for_its_texture_binding() {
        with_user_effects();
        let s = spec("--shading: grade(url(assets/lut.png), 0.25)");
        let lut = s.textures[0].expect("lut is declared");
        assert_eq!(lut.source, sk_effects::texture_source("assets/lut.png"));
        assert_eq!((lut.size_param, lut.linear), (0, true));
        assert_eq!(s.params[2], 0.25);
        let normals = s.textures[1].expect("normals is declared");
        assert_eq!(normals.source, 0, "an omitted image is missing");

        let both = spec("--shading: grade(url(\"assets/lut.png\"), url(assets/n.png))");
        assert_eq!(both.textures[0], s.textures[0]);
        assert_eq!(
            both.textures[1].unwrap().source,
            sk_effects::texture_source("assets/n.png")
        );
        assert_eq!(both.params[2], 1.0);
    }

    #[test]
    fn a_texture_parameter_takes_only_a_url() {
        with_user_effects();
        let (s, diagnostics) = shading("--shading: grade(0.5)");
        assert!(s.is_none());
        assert!(!diagnostics.is_empty());
        let (s, _) = shading("--shading: ripple(url(a.png))");
        assert!(s.is_none());
    }

    #[test]
    fn an_app_effect_validates_like_a_builtin() {
        with_user_effects();
        let (s, _) = shading("--shading: ripple(#ff0000)");
        assert!(s.is_none());

        let (clamped, _) = shading("--shading: ripple(99)");
        assert_eq!(clamped.unwrap().params[0], 8.0);
    }

    #[test]
    fn an_unknown_effect_is_dropped() {
        let (s, diagnostics) = shading("--shading: plasma(1)");
        assert!(s.is_none());
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn a_missing_required_argument_is_refused() {
        let (s, _) = shading("--shading: glow(24px)");
        assert!(s.is_none());
    }

    #[test]
    fn a_wrong_argument_type_is_refused_not_coerced() {
        let (s, _) = shading("--shading: frost(#ff0000)");
        assert!(s.is_none());
    }

    #[test]
    fn out_of_range_values_are_clamped_with_an_advisory() {
        let (s, diagnostics) = shading("--shading: frost(1.4)");
        assert_eq!(s.unwrap().params[0], 1.0);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn extra_arguments_are_ignored() {
        let (s, _) = shading("--shading: noise(0.3, 2px, 10, 4, 5)");
        assert!(s.is_some());
    }

    #[test]
    fn a_second_effect_function_is_dropped() {
        let (s, _) = shading("--shading: frost(0.5) noise(0.2)");
        assert_eq!(s.unwrap().id, effect_id::FROST);
    }

    #[test]
    fn var_references_advise_rather_than_silently_doing_nothing() {
        let (s, diagnostics) = shading("--shading: var(--panel-shading)");
        assert!(s.is_none());
        assert!(!diagnostics.is_empty());
    }

    #[test]
    fn a_relative_length_advises() {
        let (s, diagnostics) = shading("--shading: glow(2em, 0.5)");
        assert!(s.is_none());
        assert!(!diagnostics.is_empty());
    }
}
